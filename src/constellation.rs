//! The FACET shell: Walker-pattern circular orbits propagated analytically with
//! secular J2 (nodal regression + argument-of-latitude drift). No drag, no
//! station-keeping, no collision avoidance — good for geometry and coverage
//! studies over days, not for ops.

use crate::astro::{EARTH_RADIUS_KM, J2, MU_EARTH};
use crate::math::{rad, V3};
use std::f64::consts::{PI, TAU};

#[derive(Clone, Debug, PartialEq)]
pub struct ShellParams {
    /// Total satellites T.
    pub n: usize,
    /// Orbital planes P (0 = automatic ≈ √T).
    pub planes: usize,
    /// Walker phasing factor F (0..P-1).
    pub phasing: usize,
    pub alt_km: f64,
    pub inc_deg: f64,
    /// Spread of ascending nodes: 360° = Walker delta, 180° = Walker star (polar).
    pub raan_span_deg: f64,
    /// RAAN of plane 0 at epoch.
    pub raan0_deg: f64,
}

impl Default for ShellParams {
    fn default() -> Self {
        ShellParams {
            n: 100,
            planes: 0,
            phasing: 1,
            alt_km: 550.0,
            inc_deg: 53.0,
            raan_span_deg: 360.0,
            raan0_deg: 0.0,
        }
    }
}

impl ShellParams {
    pub fn effective_planes(&self) -> usize {
        let p = if self.planes == 0 {
            (self.n as f64).sqrt().round() as usize
        } else {
            self.planes
        };
        p.clamp(1, self.n.max(1))
    }
    pub fn semi_major_km(&self) -> f64 {
        EARTH_RADIUS_KM + self.alt_km
    }
    pub fn period_s(&self) -> f64 {
        TAU * (self.semi_major_km().powi(3) / MU_EARTH).sqrt()
    }
    /// Inclination that makes the node precess at +360°/tropical year.
    pub fn sun_sync_inc_deg(alt_km: f64) -> Option<f64> {
        let a = EARTH_RADIUS_KM + alt_km;
        let n = (MU_EARTH / a.powi(3)).sqrt();
        let omega_dot = TAU / (365.2422 * 86_400.0);
        let c = -omega_dot / (1.5 * n * J2 * (EARTH_RADIUS_KM / a).powi(2));
        if c.abs() <= 1.0 {
            Some(c.acos().to_degrees())
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Sat {
    pub plane: u32,
    pub slot: u32,
    raan0: f64,
    u0: f64,
}

pub struct Shell {
    pub params: ShellParams,
    pub epoch_jd: f64,
    pub sats: Vec<Sat>,
    /// Laser-link topology (+grid: fore/aft in-plane, and to the adjacent plane).
    pub links: Vec<(u32, u32)>,
    // Secular rates (rad/s)
    raan_dot: f64,
    u_dot: f64,
    cos_i: f64,
    sin_i: f64,
    a: f64,
    /// Current ECI positions, km.
    pub pos: Vec<V3>,
    /// Current RAAN and argument of latitude per satellite (rad).
    pub raan_now: Vec<f64>,
    pub u_now: Vec<f64>,
}

impl Shell {
    pub fn new(params: ShellParams, epoch_jd: f64) -> Shell {
        let p = params.effective_planes();
        let t = params.n.max(1);
        let a = params.semi_major_km();
        let inc = rad(params.inc_deg);
        let n0 = (MU_EARTH / a.powi(3)).sqrt();
        let k = J2 * (EARTH_RADIUS_KM / a).powi(2);
        let ci = inc.cos();
        let raan_dot = -1.5 * n0 * k * ci;
        let argp_dot = 0.75 * n0 * k * (5.0 * ci * ci - 1.0);
        let m_dot = n0 * (1.0 + 0.75 * k * (3.0 * ci * ci - 1.0));

        let mut sats = Vec::with_capacity(t);
        let mut plane_start = Vec::with_capacity(p + 1);
        let base = t / p;
        let extra = t % p;
        let phase_step = params.phasing as f64 * TAU / t as f64;
        for pl in 0..p {
            plane_start.push(sats.len());
            let count = base + usize::from(pl < extra);
            let raan0 = rad(params.raan0_deg) + rad(params.raan_span_deg) * pl as f64 / p as f64;
            for s in 0..count {
                let u0 = TAU * s as f64 / count as f64 + phase_step * pl as f64;
                sats.push(Sat {
                    plane: pl as u32,
                    slot: s as u32,
                    raan0,
                    u0,
                });
            }
        }
        plane_start.push(sats.len());

        let mut links = Vec::with_capacity(sats.len() * 2);
        for pl in 0..p {
            let (s0, s1) = (plane_start[pl], plane_start[pl + 1]);
            let count = s1 - s0;
            if count >= 2 {
                for i in 0..count {
                    let j = (i + 1) % count;
                    if count == 2 && i == 1 {
                        break;
                    }
                    links.push(((s0 + i) as u32, (s0 + j) as u32));
                }
            }
            // Cross-plane link to the satellite in the next plane with the
            // closest argument of latitude (relative phases are constant, so the
            // topology is fixed). Skip the seam of a Walker star, where planes
            // counter-rotate.
            let next = pl + 1;
            let wraps = next == p;
            if p >= 2 && (!wraps || params.raan_span_deg > 270.0) && !(p == 2 && wraps) {
                let np = next % p;
                let (n0s, n1s) = (plane_start[np], plane_start[np + 1]);
                for i in s0..s1 {
                    let ui = sats[i].u0;
                    let mut best = n0s;
                    let mut best_d = f64::MAX;
                    for (j, sj) in sats.iter().enumerate().take(n1s).skip(n0s) {
                        let d = (sj.u0 - ui).rem_euclid(TAU);
                        let d = d.min(TAU - d);
                        if d < best_d {
                            best_d = d;
                            best = j;
                        }
                    }
                    if n1s > n0s {
                        links.push((i as u32, best as u32));
                    }
                }
            }
        }

        let mut shell = Shell {
            params,
            epoch_jd,
            sats,
            links,
            raan_dot,
            u_dot: argp_dot + m_dot,
            cos_i: ci,
            sin_i: inc.sin(),
            a,
            pos: Vec::new(),
            raan_now: Vec::new(),
            u_now: Vec::new(),
        };
        shell.propagate(epoch_jd);
        shell
    }

    pub fn propagate(&mut self, jd: f64) {
        let dt = (jd - self.epoch_jd) * 86_400.0;
        let (ci, si, a) = (self.cos_i, self.sin_i, self.a);
        let d_raan = self.raan_dot * dt;
        let d_u = (self.u_dot * dt).rem_euclid(TAU);
        self.pos.clear();
        self.raan_now.clear();
        self.u_now.clear();
        for s in &self.sats {
            self.raan_now.push(s.raan0 + d_raan);
            self.u_now.push(s.u0 + d_u);
        }
        self.pos.extend(self.sats.iter().map(|s| {
            let (so, co) = (s.raan0 + d_raan).sin_cos();
            let (su, cu) = (s.u0 + d_u).sin_cos();
            V3::new(
                a * (cu * co - su * ci * so),
                a * (cu * so + su * ci * co),
                a * (su * si),
            )
        }));
    }

    /// Mean elements at `jd`: (inclination°, two-body mean motion rev/day, per-satellite (RAAN°, arg. of latitude°)).
    pub fn elements_at(&self, jd: f64) -> (f64, f64, Vec<(f64, f64)>) {
        let dt = (jd - self.epoch_jd) * 86_400.0;
        let els = self
            .sats
            .iter()
            .map(|s| {
                (
                    (s.raan0 + self.raan_dot * dt).to_degrees().rem_euclid(360.0),
                    (s.u0 + self.u_dot * dt).to_degrees().rem_euclid(360.0),
                )
            })
            .collect();
        // Two-body mean motion: SGP4 applies its own secular J2 rates on top.
        let n0 = (MU_EARTH / self.a.powi(3)).sqrt();
        (self.params.inc_deg, n0 * 86_400.0 / TAU, els)
    }

    /// Orbital-plane basis for satellite i at the current time: radial unit
    /// vector is cos u · P + sin u · Q, with u = `u_now[i]`.
    pub fn plane_basis(&self, i: usize) -> (V3, V3) {
        let (so, co) = self.raan_now[i].sin_cos();
        (
            V3::new(co, so, 0.0),
            V3::new(-so * self.cos_i, co * self.cos_i, self.sin_i),
        )
    }

    /// Seconds until direction `t` leaves satellite i's field of regard (the
    /// sky outside the Earth keep-out cone: dot(t, r̂) > cos_lim). None if it
    /// is not visible now; +∞ if it never sets. Ignores the slow node drift.
    pub fn time_to_set(&self, i: usize, t: V3, cos_lim: f64) -> Option<f64> {
        let (p, q) = self.plane_basis(i);
        let (a, b) = (t.dot(p), t.dot(q));
        let r = a.hypot(b);
        if r < 1e-12 {
            return (0.0 > cos_lim).then_some(f64::INFINITY);
        }
        let c = cos_lim / r;
        if c <= -1.0 {
            return Some(f64::INFINITY);
        }
        if c >= 1.0 {
            return None;
        }
        let phi = b.atan2(a);
        let th = (self.u_now[i] - phi + PI).rem_euclid(TAU) - PI;
        let thb = c.acos();
        if th.abs() < thb {
            Some((thb - th) / self.u_dot)
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.sats.len()
    }

    pub fn raan_drift_deg_per_day(&self) -> f64 {
        self.raan_dot.to_degrees() * 86_400.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_550km() {
        let p = ShellParams::default();
        let min = p.period_s() / 60.0;
        assert!((min - 95.6).abs() < 0.2, "{min}");
    }

    #[test]
    fn sso_550km() {
        let i = ShellParams::sun_sync_inc_deg(550.0).unwrap();
        assert!((i - 97.6).abs() < 0.1, "{i}");
    }

    #[test]
    fn counts_and_radius() {
        for n in [2usize, 3, 7, 100, 1234, 10_000] {
            let p = ShellParams { n, ..Default::default() };
            let s = Shell::new(p.clone(), 2_461_000.0);
            assert_eq!(s.len(), n);
            assert_eq!(s.pos.len(), n);
            for x in &s.pos {
                assert!((x.norm() - p.semi_major_km()).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn time_to_set_matches_propagation() {
        // Predict when a target sets for a few satellites, then propagate and check.
        let jd0 = 2_461_000.0;
        let mut s = Shell::new(ShellParams { n: 60, ..Default::default() }, jd0);
        let t = V3::from_radec(1.0, 0.4);
        let cos_lim = -(69.2f64.to_radians() + 10f64.to_radians()).cos();
        for i in 0..60 {
            if let Some(tts) = s.time_to_set(i, t, cos_lim) {
                if !tts.is_finite() || tts > 3000.0 {
                    continue;
                }
                let before = jd0 + (tts - 5.0) / 86_400.0;
                let after = jd0 + (tts + 5.0) / 86_400.0;
                s.propagate(before);
                assert!(t.dot(s.pos[i].unit()) > cos_lim, "sat {i} should still see it");
                s.propagate(after);
                assert!(t.dot(s.pos[i].unit()) <= cos_lim + 1e-4, "sat {i} should have lost it");
                s.propagate(jd0);
            }
        }
    }

    #[test]
    fn nodal_regression_53deg() {
        // 550 km, 53°: about −4.5 °/day
        let s = Shell::new(ShellParams::default(), 2_461_000.0);
        let r = s.raan_drift_deg_per_day();
        assert!((r + 4.5).abs() < 0.15, "{r}");
    }
}
