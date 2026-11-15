//! Spacecraft attitude: slewing, settling, fine-guidance acquisition and lock.
//!
//! Each satellite is a ~150 kg bus-dominated spacecraft steered by reaction
//! wheels. A repoint is modelled as
//!
//!   1. SLEW — time-optimal rest-to-rest bang-bang (or bang-coast-bang) eigen-
//!      axis manoeuvre, limited by wheel torque (angular acceleration α = τ/I),
//!      wheel momentum and a software rate limit (ω_max);
//!   2. SETTLE — the residual pointing error from structural/controller
//!      overshoot decays as a damped oscillation, e(t) = e₀·e^(−t/τ_s)·|cos 2πt/P|,
//!      with e₀ growing with slew size, until it is inside the fine-guidance
//!      capture range;
//!   3. ACQUIRE — the fine-guidance loop finds a guide star in the science
//!      field (or uses a bright target itself) and closes;
//!   4. LOCKED — science integration; the boresight wanders with the steady-
//!      state jitter: fine-guided RMS if a guide star exists, otherwise the much
//!      worse star-tracker-only (coarse) RMS.
//!
//! All parameters are ASSUMPTIONS (see ObsParams defaults and the panel).

use crate::math::V3;
use crate::obs::ObsParams;
use std::f64::consts::PI;

const ARCSEC: f64 = 206_264.806;

#[derive(Clone, Copy, Debug)]
pub struct Agility {
    /// rad/s²
    pub alpha: f64,
    /// rad/s
    pub omega: f64,
    pub settle_tau_s: f64,
    pub settle_period_s: f64,
    /// Initial residual = max(min, frac × slew angle), arcsec
    pub overshoot_frac: f64,
    pub overshoot_min_arcsec: f64,
    /// Fine-guidance capture radius, arcsec
    pub capture_arcsec: f64,
    pub acquire_s: f64,
    pub fine_jitter: f64,
    pub coarse_jitter: f64,
}

impl Agility {
    pub fn from_obs(o: &ObsParams) -> Agility {
        let alpha = o.wheel_torque_nm / o.inertia_kgm2;
        // Keep 20 % of wheel momentum in reserve for disturbance rejection.
        let omega_mom = 0.8 * o.wheel_momentum_nms / o.inertia_kgm2;
        Agility {
            alpha,
            omega: o.rate_limit_dps.to_radians().min(omega_mom),
            settle_tau_s: o.settle_tau_s,
            settle_period_s: 4.0,
            overshoot_frac: o.overshoot_frac,
            overshoot_min_arcsec: 20.0,
            capture_arcsec: o.capture_arcsec,
            acquire_s: o.acquire_s,
            fine_jitter: o.jitter_arcsec,
            coarse_jitter: o.coarse_jitter_arcsec,
        }
    }
    /// Rest-to-rest slew time for angle θ (rad).
    pub fn slew_time(&self, th: f64) -> f64 {
        if th <= 0.0 {
            return 0.0;
        }
        let th_c = self.omega * self.omega / self.alpha;
        if th <= th_c {
            2.0 * (th / self.alpha).sqrt()
        } else {
            th / self.omega + self.omega / self.alpha
        }
    }
    /// Angle covered after time t of a rest-to-rest slew of θ.
    pub fn slew_progress(&self, th: f64, t: f64) -> f64 {
        let tt = self.slew_time(th);
        if t >= tt {
            return th;
        }
        if t <= 0.0 {
            return 0.0;
        }
        let a = self.alpha;
        let th_c = self.omega * self.omega / a;
        if th <= th_c {
            let th_half = tt / 2.0;
            if t < th_half {
                0.5 * a * t * t
            } else {
                th - 0.5 * a * (tt - t).powi(2)
            }
        } else {
            let ta = self.omega / a;
            if t < ta {
                0.5 * a * t * t
            } else if t < tt - ta {
                0.5 * a * ta * ta + self.omega * (t - ta)
            } else {
                th - 0.5 * a * (tt - t).powi(2)
            }
        }
    }
    pub fn initial_residual(&self, th: f64) -> f64 {
        (th * ARCSEC * self.overshoot_frac).max(self.overshoot_min_arcsec)
    }
    /// Time for the residual envelope to fall inside the capture radius.
    pub fn settle_time(&self, th: f64) -> f64 {
        let e0 = self.initial_residual(th);
        if e0 <= self.capture_arcsec {
            0.0
        } else {
            self.settle_tau_s * (e0 / self.capture_arcsec).ln()
        }
    }
    /// Total repoint time: slew + settle + acquire.
    pub fn repoint_time(&self, th: f64) -> f64 {
        self.slew_time(th) + self.settle_time(th) + self.acquire_s
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Slewing,
    Settling,
    Acquiring,
    Locked,
}

#[derive(Clone, Debug)]
pub struct Att {
    /// Current boresight (unit vector, ECI).
    pub dir: V3,
    start: V3,
    pub target: V3,
    angle: f64,
    elapsed: f64,
    t_slew: f64,
    t_settle: f64,
    t_acq: f64,
    e0: f64,
    pub job: u64,
    /// Fine guidance available for the current pointing.
    pub fine: bool,
}

/// What happened to one satellite during a step.
#[derive(Clone, Copy, Debug, Default)]
pub struct StepOut {
    /// Seconds of locked science integration in this step.
    pub observed_s: f64,
    /// Started a new manoeuvre of this angle (rad).
    pub new_slew: Option<f64>,
}

impl Att {
    pub fn new(dir: V3) -> Att {
        Att {
            dir,
            start: dir,
            target: dir,
            angle: 0.0,
            elapsed: f64::INFINITY,
            t_slew: 0.0,
            t_settle: 0.0,
            t_acq: 0.0,
            e0: 0.0,
            job: 0,
            fine: true,
        }
    }

    fn total(&self) -> f64 {
        self.t_slew + self.t_settle + self.t_acq
    }

    pub fn phase(&self) -> Phase {
        if self.job == 0 {
            Phase::Idle
        } else if self.elapsed < self.t_slew {
            Phase::Slewing
        } else if self.elapsed < self.t_slew + self.t_settle {
            Phase::Settling
        } else if self.elapsed < self.total() {
            Phase::Acquiring
        } else {
            Phase::Locked
        }
    }

    /// Current steady-state jitter RMS (arcsec) while locked.
    pub fn jitter(&self, ag: &Agility) -> f64 {
        if self.fine {
            ag.fine_jitter
        } else {
            ag.coarse_jitter
        }
    }

    fn begin(&mut self, d: V3, job: u64, fine: bool, ag: &Agility) -> f64 {
        self.start = self.dir;
        self.target = d;
        self.angle = self.dir.angle(d);
        self.elapsed = 0.0;
        self.t_slew = ag.slew_time(self.angle);
        self.t_settle = ag.settle_time(self.angle);
        self.t_acq = ag.acquire_s;
        self.e0 = ag.initial_residual(self.angle);
        self.job = job;
        self.fine = fine;
        self.angle
    }

    /// Advance by `dt` seconds toward the command `cmd` = (direction, job id, fine-guidance available).
    pub fn step(&mut self, cmd: Option<(V3, u64, bool)>, dt: f64, ag: &Agility) -> StepOut {
        let mut out = StepOut::default();
        let Some((d, job, fine)) = cmd else {
            // No task: hold attitude where we are.
            if self.phase() == Phase::Slewing {
                self.target = self.dir;
            }
            self.job = 0;
            self.elapsed = f64::INFINITY;
            return out;
        };
        let moved = d.angle(self.target);
        let locked = self.phase() == Phase::Locked;
        if job != self.job {
            out.new_slew = Some(self.begin(d, job, fine, ag));
        } else if moved > 0.5f64.to_radians() {
            // Same task, but the aim point moved a lot. If it moves slowly
            // compared with our rate capability, the control loop just tracks it.
            let rate = if dt > 0.0 { moved / dt } else { f64::INFINITY };
            if locked && rate < 0.5 * ag.omega {
                self.target = d;
            } else {
                out.new_slew = Some(self.begin(d, job, fine, ag));
            }
        } else {
            self.target = d;
            self.fine = fine;
        }

        let before = self.elapsed;
        self.elapsed += dt;
        if self.elapsed < self.t_slew && self.angle > 1e-12 {
            let f = ag.slew_progress(self.angle, self.elapsed) / self.angle;
            self.dir = slerp(self.start, self.target, f);
        } else {
            self.dir = self.target;
        }
        let total = self.total();
        out.observed_s = (self.elapsed - before.max(total)).max(0.0).min(dt);
        out
    }

    /// Instantaneous pointing error (arcsec) for display: remaining slew,
    /// settling envelope, or zero-mean jitter magnitude while locked.
    pub fn error_arcsec(&self, ag: &Agility, noise: f64) -> f64 {
        match self.phase() {
            Phase::Idle => 0.0,
            Phase::Slewing => (self.angle - ag.slew_progress(self.angle, self.elapsed)) * ARCSEC + self.e0,
            Phase::Settling | Phase::Acquiring => {
                let t = self.elapsed - self.t_slew;
                self.e0 * (-t / ag.settle_tau_s).exp() * (2.0 * PI * t / ag.settle_period_s).cos().abs()
                    + self.jitter(ag) * noise.abs()
            }
            Phase::Locked => self.jitter(ag) * noise.abs(),
        }
    }
}

/// Spherical linear interpolation between unit vectors.
pub fn slerp(a: V3, b: V3, f: f64) -> V3 {
    let ang = a.angle(b);
    if ang < 1e-12 {
        return b;
    }
    let axis = a.cross(b);
    if axis.norm() < 1e-12 {
        // Antipodal: pick any axis.
        return a.rot_axis(a.any_perp(), ang * f).unit();
    }
    a.rot_axis(axis.unit(), ang * f).unit()
}

/// A sampled pointing-error trace (t s, error arcsec) for a typical repoint
/// of `th` radians, for plotting in result reports.
pub fn trace(ag: &Agility, th: f64, fine: bool, seed: u64) -> Vec<(f64, f64)> {
    let mut a = Att::new(V3::new(1.0, 0.0, 0.0));
    let target = V3::new(1.0, 0.0, 0.0).rot_z(th);
    let total = ag.repoint_time(th);
    let span = total + 30.0;
    let n = 300;
    let dt = span / n as f64;
    let mut s = seed.max(1);
    let mut gauss = move || {
        // xorshift + Box–Muller
        let mut u = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 11) as f64 / (1u64 << 53) as f64
        };
        let (u1, u2) = (u().max(1e-12), u());
        (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
    };
    let mut out = Vec::with_capacity(n + 1);
    a.step(Some((target, 1, fine)), 0.0, ag);
    out.push((0.0, a.error_arcsec(ag, gauss())));
    for k in 1..=n {
        a.step(Some((target, 1, fine)), dt, ag);
        out.push((k as f64 * dt, a.error_arcsec(ag, gauss()).max(0.01)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ag() -> Agility {
        Agility::from_obs(&ObsParams::default())
    }

    #[test]
    fn derived_agility() {
        let a = ag();
        assert!((a.alpha.to_degrees() - 0.43).abs() < 0.02, "{}", a.alpha.to_degrees());
        assert!((a.omega.to_degrees() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn slew_profile_consistent() {
        let a = ag();
        for deg in [0.5f64, 5.0, 30.0, 90.0, 180.0] {
            let th = deg.to_radians();
            let t = a.slew_time(th);
            assert!((a.slew_progress(th, t) - th).abs() < 1e-12);
            assert!((a.slew_progress(th, t / 2.0) - th / 2.0).abs() < 1e-9, "symmetry {deg}");
            // monotone
            let mut prev = 0.0;
            for k in 0..=100 {
                let p = a.slew_progress(th, t * k as f64 / 100.0);
                assert!(p >= prev - 1e-15);
                prev = p;
            }
        }
        // 90°: ~50 s at 2°/s with 0.43°/s²
        assert!((a.slew_time(90f64.to_radians()) - 49.7).abs() < 1.0);
    }

    #[test]
    fn state_machine_reaches_lock_and_counts_time() {
        let a = ag();
        let mut s = Att::new(V3::new(1.0, 0.0, 0.0));
        let tgt = V3::new(0.0, 1.0, 0.0);
        let total = a.repoint_time(90f64.to_radians());
        let mut observed = 0.0;
        let dt = 0.5;
        let mut t = 0.0;
        while t < total + 20.0 {
            observed += s.step(Some((tgt, 7, true)), dt, &a).observed_s;
            t += dt;
        }
        assert_eq!(s.phase(), Phase::Locked);
        assert!((observed - (t - total)).abs() < dt + 1e-9, "{observed} vs {}", t - total);
        // One big step gives the same answer (time-step independent)
        let mut s2 = Att::new(V3::new(1.0, 0.0, 0.0));
        let o2 = s2.step(Some((tgt, 7, true)), t, &a).observed_s;
        assert!((o2 - (t - total)).abs() < 1e-6);
    }

    #[test]
    fn tracking_slow_target_keeps_lock() {
        let a = ag();
        let mut s = Att::new(V3::new(1.0, 0.0, 0.0));
        s.step(Some((V3::new(1.0, 0.0, 0.0), 3, true)), 100.0, &a);
        assert_eq!(s.phase(), Phase::Locked);
        // Zenith-like drift: 0.064°/s for 10 minutes in 1 s steps
        let mut obs = 0.0;
        for k in 1..=600 {
            let d = V3::new(1.0, 0.0, 0.0).rot_z((0.064f64 * k as f64).to_radians());
            obs += s.step(Some((d, 3, true)), 1.0, &a).observed_s;
        }
        assert!((obs - 600.0).abs() < 1e-6, "{obs}");
    }
}
