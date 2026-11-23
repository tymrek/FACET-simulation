//! Planetary-defence exercise: a synthetic Earth impactor, its discovery by
//! FACET and by a model ground survey, follow-up astrometry, orbit
//! determination and the evolution of the impact probability, for two
//! systems side by side: FACET + ground, and ground alone.
//!
//! Method (deliberately standard and inspectable):
//! * Heliocentric two-body orbit (universal-variable Kepler propagation,
//!   AU / day, μ = k²). The true orbit is built backwards from the impact:
//!   at the impact epoch the object sits inside Earth's capture cross-section
//!   with a chosen approach speed v∞ and radiant. Earth's own gravity on the
//!   approach is not integrated; it enters only through gravitational
//!   focusing of the capture radius. Observers are geocentric.
//! * Brightness from the H, G magnitude system (G = 0.15) with H from
//!   diameter and albedo. Search detections suffer trailing loss; follow-up
//!   is rate-tracked.
//! * FACET finds it when the searching telescopes (those surveying, not
//!   busy on other work) cover its position while it is brighter than the
//!   trailed single-exposure limit (a Poisson process at the fleet's areal
//!   search rate over the open sky); the candidate is confirmed one step
//!   later by a few rate-tracking telescopes if it is still detectable. The ground survey is a
//!   generic composite with ASSUMED parameters (`GroundModel`).
//! * Orbit determination: linearised weighted least squares about the true
//!   orbit with simulated Gaussian astrometric errors; partials by central
//!   differences through the propagator; a weak prior keeps the normal
//!   matrix invertible on very short arcs.
//! * Impact probability: the 6×6 covariance mapped linearly to the target
//!   plane (b-plane) at the impact epoch and integrated over the
//!   gravitationally focused capture disc. Linear methods are optimistic
//!   about Gaussianity on very short arcs, as real impact monitors note.
//! * Palermo scale from impact energy, probability and time to impact.

use crate::astro::{self, AU_KM, EARTH_RADIUS_KM};
use crate::math::{deg, rad, V3};
use serde::Serialize;

const K_GAUSS: f64 = 0.017_202_098_95;
const MU: f64 = K_GAUSS * K_GAUSS; // AU³/day²
const KMS_TO_AUD: f64 = 86_400.0 / AU_KM;
const V_ESC_KMS: f64 = 11.186;
const ARCSEC: f64 = std::f64::consts::PI / 648_000.0;

// ---------------------------------------------------------------- Kepler

fn stumpff(z: f64) -> (f64, f64) {
    // (C, S)
    if z > 1e-6 {
        let s = z.sqrt();
        ((1.0 - s.cos()) / z, (s - s.sin()) / (s * s * s))
    } else if z < -1e-6 {
        let s = (-z).sqrt();
        ((s.cosh() - 1.0) / (-z), (s.sinh() - s) / (s * s * s))
    } else {
        (0.5 - z / 24.0 + z * z / 720.0, 1.0 / 6.0 - z / 120.0 + z * z / 5040.0)
    }
}

/// Two-body propagation of a heliocentric state (AU, AU/day) by `dt` days.
pub fn propagate(r0: V3, v0: V3, dt: f64) -> (V3, V3) {
    if dt == 0.0 {
        return (r0, v0);
    }
    let sm = MU.sqrt();
    let r0n = r0.norm();
    let vr0 = r0.dot(v0) / r0n;
    let alpha = 2.0 / r0n - v0.dot(v0) / MU;
    let mut x = sm * alpha.abs() * dt;
    if !x.is_finite() || x == 0.0 {
        x = sm * dt / r0n;
    }
    for _ in 0..60 {
        let z = alpha * x * x;
        let (c, s) = stumpff(z);
        let f = r0n * vr0 / sm * x * x * c + (1.0 - alpha * r0n) * x * x * x * s + r0n * x - sm * dt;
        let fp = r0n * vr0 / sm * x * (1.0 - z * s) + (1.0 - alpha * r0n) * x * x * c + r0n;
        let dx = f / fp;
        x -= dx;
        if dx.abs() < 1e-13 * (1.0 + x.abs()) {
            break;
        }
    }
    let z = alpha * x * x;
    let (c, s) = stumpff(z);
    let f = 1.0 - x * x / r0n * c;
    let g = dt - x * x * x / sm * s;
    let r = r0 * f + v0 * g;
    let rn = r.norm();
    let fd = sm / (rn * r0n) * (alpha * x * x * x * s - x);
    let gd = 1.0 - x * x / rn * c;
    (r, r0 * fd + v0 * gd)
}

/// Earth's heliocentric J2000 equatorial position, AU.
pub fn earth_pos(jd: f64) -> V3 {
    // Many callers ask for the same few instants in one step (every tracked
    // asteroid needs the Earth at now and now + 60 s): remember the last few.
    thread_local!(static CACHE: std::cell::RefCell<[(f64, V3); 4]> = const { std::cell::RefCell::new([(f64::NAN, V3::new(0.0, 0.0, 0.0)); 4]) });
    if let Some(v) = CACHE.with(|c| c.borrow().iter().find(|e| e.0 == jd).map(|e| e.1)) {
        return v;
    }
    let s = astro::bodies(jd)[0];
    let v = s.dir * (-s.dist_km / AU_KM);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.rotate_right(1);
        c[0] = (jd, v);
    });
    v
}
fn earth_state(jd: f64) -> (V3, V3) {
    let h = 0.01;
    (earth_pos(jd), (earth_pos(jd + h) - earth_pos(jd - h)) * (0.5 / h))
}

// ---------------------------------------------------------------- linear algebra

type M6 = [[f64; 6]; 6];

fn inv6(a: &M6) -> Option<M6> {
    let mut m = *a;
    let mut inv = [[0.0; 6]; 6];
    for (i, row) in inv.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for c in 0..6 {
        let p = (c..6).max_by(|&i, &j| m[i][c].abs().partial_cmp(&m[j][c].abs()).unwrap())?;
        if m[p][c].abs() < 1e-300 {
            return None;
        }
        m.swap(c, p);
        inv.swap(c, p);
        let d = m[c][c];
        for k in 0..6 {
            m[c][k] /= d;
            inv[c][k] /= d;
        }
        for r in 0..6 {
            if r != c {
                let f = m[r][c];
                if f != 0.0 {
                    for k in 0..6 {
                        m[r][k] -= f * m[c][k];
                        inv[r][k] -= f * inv[c][k];
                    }
                }
            }
        }
    }
    Some(inv)
}

// ---------------------------------------------------------------- photometry

/// H from diameter (m) and geometric albedo.
pub fn h_mag(d_m: f64, albedo: f64) -> f64 {
    5.0 * (1329.0 / (d_m / 1000.0 * albedo.sqrt())).log10()
}

/// Apparent V from the H, G system.
pub fn v_mag(h: f64, r_au: f64, delta_au: f64, phase: f64) -> f64 {
    const G: f64 = 0.15;
    let t = (phase / 2.0).tan().max(0.0);
    let p1 = (-3.33 * t.powf(0.63)).exp();
    let p2 = (-1.87 * t.powf(1.22)).exp();
    let phi = ((1.0 - G) * p1 + G * p2).max(1e-12);
    h + 5.0 * (r_au * delta_au).log10() - 2.5 * phi.log10()
}

// ---------------------------------------------------------------- configuration

#[derive(Clone, Serialize)]
pub struct Config {
    pub name: String,
    /// Diameter, m.
    pub diameter_m: f64,
    pub albedo: f64,
    /// Bulk density, kg/m³ (ASSUMED stony).
    pub density: f64,
    /// Days from scenario start to impact.
    pub days: f64,
    /// Approach speed relative to Earth before focusing, km/s.
    pub v_inf_kms: f64,
    /// Angle between the radiant (where it comes from) and the Sun at impact, degrees.
    pub radiant_elong_deg: f64,
    /// Position angle of the radiant around the Sun, from the ecliptic, degrees.
    pub radiant_pa_deg: f64,
    /// FACET telescopes assigned to follow-up once it is found.
    pub eyes: u32,
    pub seed: u64,
}

pub fn presets() -> Vec<Config> {
    vec![
        Config {
            name: "Sunward Tunguska-class (50 m, 10 days out)".into(),
            diameter_m: 50.0,
            albedo: 0.14,
            density: 2600.0,
            days: 10.0,
            v_inf_kms: 18.0,
            radiant_elong_deg: 55.0,
            radiant_pa_deg: 15.0,
            eyes: 30,
            seed: 11,
        },
        Config {
            name: "Chelyabinsk-class from the Sun (20 m, 4 days out)".into(),
            diameter_m: 20.0,
            albedo: 0.14,
            density: 3300.0,
            days: 4.0,
            v_inf_kms: 17.0,
            radiant_elong_deg: 50.0,
            radiant_pa_deg: -10.0,
            eyes: 30,
            seed: 7,
        },
        Config {
            name: "Night-side city-killer (140 m, 60 days out)".into(),
            diameter_m: 140.0,
            albedo: 0.14,
            density: 2600.0,
            days: 60.0,
            v_inf_kms: 12.0,
            radiant_elong_deg: 150.0,
            radiant_pa_deg: 20.0,
            eyes: 30,
            seed: 3,
        },
    ]
}

/// Generic ground NEO survey + follow-up network. All values ASSUMED, chosen
/// to resemble current practice in broad strokes, not any one facility.
pub struct GroundModel {
    pub disc_lim_v: f64,
    pub follow_lim_v: f64,
    /// Minimum solar elongation observable at night, degrees.
    pub min_elong_deg: f64,
    /// Chance a given observable night covers the object (cadence).
    pub coverage: f64,
    /// Chance of clear weather per session.
    pub clear: f64,
    /// Sessions per day across longitudes.
    pub sessions_per_day: u32,
    pub sigma_arcsec: f64,
    pub obs_per_session: u32,
}

pub const GROUND: GroundModel = GroundModel {
    disc_lim_v: 21.0,
    follow_lim_v: 22.0,
    min_elong_deg: 60.0,
    coverage: 0.5,
    clear: 0.6,
    sessions_per_day: 3,
    sigma_arcsec: 0.3,
    obs_per_session: 4,
};

/// Positional floor for one FACET epoch (catalogue, timing, systematics), arcsec. ASSUMED.
const FACET_FLOOR_ARCSEC: f64 = 0.05;

// ---------------------------------------------------------------- orbit determination state

#[derive(Clone)]
struct Od {
    a: M6,
    b: [f64; 6],
    n_obs: u32,
    first_jd: Option<f64>,
    last_jd: f64,
}

impl Od {
    fn new() -> Od {
        // Weak prior: 0.5 AU and 10 km/s per component around the true state.
        let mut a = [[0.0; 6]; 6];
        let sr = 0.5f64;
        let sv = 10.0 * KMS_TO_AUD;
        for i in 0..3 {
            a[i][i] = 1.0 / (sr * sr);
            a[i + 3][i + 3] = 1.0 / (sv * sv);
        }
        Od { a, b: [0.0; 6], n_obs: 0, first_jd: None, last_jd: 0.0 }
    }
    fn add(&mut self, jac: &[[f64; 6]; 2], eps: [f64; 2], sigma: f64, jd: f64) {
        let w = 1.0 / (sigma * sigma);
        for r in 0..2 {
            for i in 0..6 {
                self.b[i] += jac[r][i] * w * eps[r];
                for j in 0..6 {
                    self.a[i][j] += jac[r][i] * w * jac[r][j];
                }
            }
        }
        self.n_obs += 1;
        self.first_jd.get_or_insert(jd);
        self.last_jd = jd;
    }
    fn arc_days(&self) -> f64 {
        self.first_jd.map_or(0.0, |f| self.last_jd - f)
    }
    fn solve(&self) -> Option<(M6, [f64; 6])> {
        let p = inv6(&self.a)?;
        let mut dx = [0.0; 6];
        for i in 0..6 {
            for j in 0..6 {
                dx[i] += p[i][j] * self.b[j];
            }
        }
        Some((p, dx))
    }
}

#[derive(Clone, Serialize, Default)]
pub struct SystemView {
    pub name: &'static str,
    pub discovered_utc: Option<String>,
    /// Warning time at discovery (s before impact).
    pub warning_s: Option<f64>,
    pub n_obs: u32,
    pub arc_h: f64,
    pub ip: Option<f64>,
    pub palermo: Option<f64>,
    /// 1σ semi-major axis of the b-plane uncertainty ellipse, km.
    pub sigma_b_km: Option<f64>,
    /// Nominal-orbit miss distance from Earth's centre, km.
    pub nominal_miss_km: Option<f64>,
    /// Time before impact at which IP first reached 99 %.
    pub certain_s: Option<f64>,
    pub ip_1pct_s: Option<f64>,
}

struct System {
    view: SystemView,
    od: Od,
    disc_jd: Option<f64>,
    /// Unconfirmed survey candidate pending (FACET).
    hits: u32,
    first_hit_jd: f64,
    cov_b: Option<([f64; 2], [[f64; 2]; 2])>,
    solved_n: u32,
}

impl System {
    fn new(name: &'static str) -> System {
        System { view: SystemView { name, ..Default::default() }, od: Od::new(), disc_jd: None, hits: 0, first_hit_jd: 0.0, cov_b: None, solved_n: 0 }
    }
}

/// Inputs from the rest of the simulation for one step.
pub struct StepIn<'a> {
    pub jd: f64,
    pub dt_s: f64,
    pub sun: V3,
    pub moon: V3,
    pub o: &'a crate::obs::ObsParams,
    pub hw: &'a crate::imaging::Hardware,
    /// Telescopes surveying (available for search) this step.
    pub search_eyes: usize,
    /// FACET eyes locked on the object (follow-up campaign).
    pub locked_eyes: u32,
}

#[derive(Default, Serialize, Clone)]
pub struct Now {
    pub ra_deg: f64,
    pub dec_deg: f64,
    pub v_mag: f64,
    pub dist_km: f64,
    pub elong_deg: f64,
    pub phase_deg: f64,
    pub rate_arcsec_s: f64,
    pub facet_search_lim: f64,
    pub facet_follow_lim: f64,
    pub detectable_facet: bool,
    pub observable_ground: bool,
    pub blocked: Option<String>,
    pub search_eyes: usize,
    /// Hours for the searching telescopes to cover the open sky once.
    pub sky_revisit_h: Option<f64>,
}

pub struct Scenario {
    pub cfg: Config,
    pub designation: String,
    pub start_jd: f64,
    pub impact_jd: f64,
    x0: (V3, V3),
    /// Unit approach direction (geocentric velocity) at impact.
    s_hat: V3,
    b_true: [f64; 2],
    e1: V3,
    e2: V3,
    /// Capture radius with gravitational focusing, km.
    pub b_capture_km: f64,
    pub energy_mt: f64,
    pub h: f64,
    sys: [System; 2], // 0: FACET + ground, 1: ground only
    rng: u64,
    pub now: Now,
    pub history: Vec<[f64; 3]>, // [hours to impact, IP combined, IP ground]
    last_hist_jd: f64,
    pub corridor: Vec<[f64; 2]>,
    corridor_n: u32,
    pub impacted: bool,
    pub true_impact: Option<[f64; 2]>,
    pub campaign: Option<u64>,
    pub events: Vec<(&'static str, String)>,
    last_ground_session: i64,
    ip_marks: [[bool; 4]; 2],
    m_bplane: [[f64; 6]; 3],
}

impl Scenario {
    pub fn new(cfg: Config, start_jd: f64) -> Scenario {
        let impact_jd = start_jd + cfg.days.max(0.05);
        let (re, ve) = earth_state(impact_jd);
        let sun_dir = (re * -1.0).unit();
        let pole = V3::new(0.0, -rad(23.4393).sin(), rad(23.4393).cos());
        let n1 = pole.cross(sun_dir).unit();
        let n2 = sun_dir.cross(n1).unit();
        let pa = rad(cfg.radiant_pa_deg);
        let w = n1 * pa.cos() + n2 * pa.sin();
        let el = rad(cfg.radiant_elong_deg);
        let radiant = (sun_dir * el.cos() + w * el.sin()).unit();
        let s_hat = radiant * -1.0;
        let v_rel = s_hat * (cfg.v_inf_kms * KMS_TO_AUD);
        let e1 = s_hat.any_perp().unit();
        let e2 = s_hat.cross(e1).unit();
        let b_capture_km = EARTH_RADIUS_KM * (1.0 + (V_ESC_KMS / cfg.v_inf_kms).powi(2)).sqrt();
        // Aim a third of the way out from the centre of the capture disc.
        let bvec_km = e1 * (0.33 * b_capture_km * 0.8) + e2 * (0.33 * b_capture_km * 0.6);
        let r_imp = re + bvec_km * (1.0 / AU_KM);
        let v_imp = ve + v_rel;
        let x0 = propagate(r_imp, v_imp, -(impact_jd - start_jd));
        let mass = cfg.density * std::f64::consts::PI / 6.0 * cfg.diameter_m.powi(3);
        let v_imp_ms = (cfg.v_inf_kms.powi(2) + V_ESC_KMS.powi(2)).sqrt() * 1000.0;
        let energy_mt = 0.5 * mass * v_imp_ms * v_imp_ms / 4.184e15;
        let (y, m, d, ..) = astro::calendar(impact_jd);
        let _ = (y, d);
        let letters = b"ABCDEFGHJKLMNOPQRSTUVWXY";
        let designation = format!("2026 P{}{} (simulated)", letters[(m as usize * 2) % 24] as char, 1 + cfg.seed % 9);
        let mut sc = Scenario {
            h: h_mag(cfg.diameter_m, cfg.albedo),
            designation,
            start_jd,
            impact_jd,
            x0,
            s_hat,
            b_true: [bvec_km.dot(e1), bvec_km.dot(e2)],
            e1,
            e2,
            b_capture_km,
            energy_mt,
            sys: [System::new("FACET + ground"), System::new("Ground only")],
            rng: cfg.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
            now: Now::default(),
            history: Vec::new(),
            last_hist_jd: 0.0,
            corridor: Vec::new(),
            corridor_n: 0,
            impacted: false,
            true_impact: None,
            campaign: None,
            events: Vec::new(),
            last_ground_session: i64::MIN,
            ip_marks: [[false; 4]; 2],
            m_bplane: [[0.0; 6]; 3],
            cfg,
        };
        sc.m_bplane = sc.partials_pos(impact_jd);
        sc
    }

    fn rand(&mut self) -> f64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn gauss(&mut self) -> f64 {
        let u = self.rand().max(1e-300);
        let v = self.rand();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
    fn poisson(&mut self, lam: f64) -> u32 {
        if lam > 30.0 {
            return (lam + lam.sqrt() * self.gauss()).max(0.0).round() as u32;
        }
        let l = (-lam).exp();
        let (mut k, mut p) = (0u32, 1.0);
        loop {
            p *= self.rand();
            if p <= l {
                return k;
            }
            k += 1;
        }
    }

    fn state_at(&self, x0: (V3, V3), jd: f64) -> V3 {
        propagate(x0.0, x0.1, jd - self.start_jd).0
    }
    fn perturbed(&self, k: usize, h: f64) -> (V3, V3) {
        let (mut r, mut v) = self.x0;
        match k {
            0 => r.x += h,
            1 => r.y += h,
            2 => r.z += h,
            3 => v.x += h,
            4 => v.y += h,
            _ => v.z += h,
        }
        (r, v)
    }
    fn step_h(k: usize) -> f64 {
        if k < 3 {
            1e-7
        } else {
            1e-9
        }
    }
    /// ∂(geocentric position at jd)/∂x0, AU per state unit.
    fn partials_pos(&self, jd: f64) -> [[f64; 6]; 3] {
        let mut m = [[0.0; 6]; 3];
        for k in 0..6 {
            let h = Self::step_h(k);
            let p = self.state_at(self.perturbed(k, h), jd);
            let q = self.state_at(self.perturbed(k, -h), jd);
            let d = (p - q) * (0.5 / h);
            m[0][k] = d.x;
            m[1][k] = d.y;
            m[2][k] = d.z;
        }
        m
    }

    /// True geocentric position (AU) and unit direction at jd.
    pub fn geo(&self, jd: f64) -> (V3, V3) {
        let rho = self.state_at(self.x0, jd) - earth_pos(jd);
        (rho, rho.unit())
    }

    /// One astrometric measurement: Jacobian rows (radians per state unit).
    fn jac(&self, jd: f64) -> [[f64; 6]; 2] {
        let (rho, u) = self.geo(jd);
        let (e, n) = crate::obs::tangent_basis(u);
        let m = self.partials_pos(jd);
        let r = rho.norm();
        let mut j = [[0.0; 6]; 2];
        for k in 0..6 {
            let dp = V3::new(m[0][k], m[1][k], m[2][k]);
            let du = (dp - u * u.dot(dp)) * (1.0 / r);
            j[0][k] = du.dot(e);
            j[1][k] = du.dot(n);
        }
        j
    }

    fn add_obs(&mut self, which: &[usize], jd: f64, sigma_arcsec: f64) {
        let jac = self.jac(jd);
        let sig = sigma_arcsec * ARCSEC;
        let eps = [self.gauss() * sig, self.gauss() * sig];
        for &w in which {
            self.sys[w].od.add(&jac, eps, sig, jd);
        }
    }

    fn sky_now(&mut self, inp: &StepIn) -> (f64, f64, V3) {
        let jd = inp.jd;
        let r_ast = self.state_at(self.x0, jd);
        let (rho, u) = self.geo(jd);
        let delta = rho.norm();
        let phase = (r_ast * -1.0).angle(rho * -1.0);
        let v = v_mag(self.h, r_ast.norm(), delta, phase);
        let (_, u2) = self.geo(jd + 60.0 / 86_400.0);
        let rate = u.angle(u2) / 60.0 / ARCSEC;
        let (ra, dec) = u.to_radec();
        self.now.ra_deg = deg(ra).rem_euclid(360.0);
        self.now.dec_deg = deg(dec);
        self.now.v_mag = v;
        self.now.dist_km = delta * AU_KM;
        self.now.elong_deg = deg(u.angle(inp.sun));
        self.now.phase_deg = deg(phase);
        self.now.rate_arcsec_s = rate;
        (v, rate, u)
    }

    /// Advance the exercise over the step ending at `inp.jd`.
    pub fn step(&mut self, inp: &StepIn) {
        if self.impacted || inp.dt_s <= 0.0 {
            return;
        }
        let jd = inp.jd.min(self.impact_jd);
        let inp = StepIn { jd, ..*inp };
        let (v, rate, u) = self.sky_now(&inp);
        let o = inp.o;
        let sky_sb = crate::imaging::sky_sb(u, inp.sun, o);
        let fwhm = inp.hw.fwhm(60.0).max(0.3);
        // Search: one telescope, 60 s, trailed.
        let trail = rate * 60.0;
        let lim1 = crate::imaging::point_lim_mag(inp.hw, 60.0, 60.0f64.min(inp.hw.sub_max_s), sky_sb);
        let lim_search = lim1 - 1.25 * (1.0 + trail / fwhm).log10();
        let k = inp.locked_eyes.max(1);
        let lim_follow = crate::imaging::point_lim_mag(inp.hw, 60.0 * k as f64, 60.0f64.min(inp.hw.sub_max_s), sky_sb);
        let blocked = crate::obs::sky_blocked(o, u, inp.sun, inp.moon);
        self.now.facet_search_lim = lim_search;
        self.now.facet_follow_lim = lim_follow;
        self.now.blocked = blocked.map(String::from);
        self.now.detectable_facet = blocked.is_none() && v <= lim_search;
        let elong_ok = self.now.elong_deg >= GROUND.min_elong_deg;
        self.now.observable_ground = elong_ok && v <= GROUND.follow_lim_v;
        self.now.search_eyes = inp.search_eyes;
        let open_sky = 41_253.0 * (1.0 + rad(o.sun_excl_deg).cos()) / 2.0;
        let frame = o.exp_s + o.overhead_s;
        let rate_deg2_s = inp.search_eyes as f64 * o.fov_deg * o.fov_deg / frame.max(1.0);
        self.now.sky_revisit_h = (rate_deg2_s > 0.0).then(|| open_sky / rate_deg2_s / 3600.0);
        let t_left = (self.impact_jd - jd) * 86_400.0;
        let utc = astro::format_utc(jd);

        // --- FACET discovery (combined system only). One survey detection
        // makes a candidate; the fleet confirms it automatically by tasking
        // neighbouring telescopes over the laser links (rate-tracked, a few
        // eyes), which takes about one exposure.
        if self.sys[0].disc_jd.is_none() {
            if self.sys[0].hits > 0 {
                let lim_conf = crate::imaging::point_lim_mag(inp.hw, 60.0 * 4.0, 60.0f64.min(inp.hw.sub_max_s), sky_sb);
                if blocked.is_none() && v <= lim_conf {
                    self.discover(0, jd, t_left, &utc, "FACET survey + automatic confirmation");
                } else if jd - self.sys[0].first_hit_jd > 0.5 {
                    self.sys[0].hits = 0;
                }
            } else if self.now.detectable_facet && rate_deg2_s > 0.0 {
                let lam = rate_deg2_s / open_sky * inp.dt_s;
                if self.poisson(lam) > 0 {
                    self.sys[0].hits = 1;
                    self.sys[0].first_hit_jd = jd;
                    self.events.push(("warn", format!("FACET survey: fast-moving candidate at V = {v:.1}, confirmation tasked to neighbouring telescopes")));
                }
            }
        }
        // --- Ground sessions (discovery + follow-up), a few per day.
        let per = 1.0 / GROUND.sessions_per_day as f64;
        let session = ((jd - 0.1) / per).floor() as i64;
        if self.last_ground_session == i64::MIN {
            self.last_ground_session = session;
        }
        while self.last_ground_session < session && t_left > 600.0 {
            self.last_ground_session += 1;
            let clear = self.rand() < GROUND.clear;
            if !(clear && elong_ok) {
                continue;
            }
            // Discovery by the survey.
            if v <= GROUND.disc_lim_v && self.rand() < GROUND.coverage {
                for s in 0..2 {
                    if self.sys[s].disc_jd.is_none() {
                        self.discover(s, jd, t_left, &utc, "ground survey");
                    }
                }
            }
            if v <= GROUND.follow_lim_v {
                let who: Vec<usize> = (0..2).filter(|&s| self.sys[s].disc_jd.is_some()).collect();
                if !who.is_empty() {
                    for i in 0..GROUND.obs_per_session {
                        let t = jd - (GROUND.obs_per_session - i) as f64 * 15.0 / 1440.0;
                        self.add_obs(&who, t.max(self.start_jd), GROUND.sigma_arcsec);
                    }
                }
            }
        }
        // --- FACET follow-up astrometry: one stacked epoch per minute.
        if self.sys[0].disc_jd.is_some() && inp.locked_eyes > 0 && blocked.is_none() && v <= lim_follow && t_left > 600.0 {
            let snr = 5.0 * 10f64.powf(0.4 * (lim_follow - v));
            let sig_rand = 0.6 * fwhm / snr;
            let sigma1 = (sig_rand * sig_rand + FACET_FLOOR_ARCSEC * FACET_FLOOR_ARCSEC).sqrt();
            let n_ep = ((inp.dt_s / 60.0).floor() as usize).max(1);
            let cap = 600usize;
            let (n_use, sigma) = if n_ep > cap { (cap, sigma1 / ((n_ep as f64 / cap as f64).sqrt())) } else { (n_ep, sigma1) };
            for i in 0..n_use {
                let t = jd - (inp.dt_s / 86_400.0) * (i as f64 + 0.5) / n_use as f64;
                if t > self.start_jd {
                    self.add_obs(&[0], t, sigma);
                }
            }
        }

        // --- Orbit solutions and impact probability.
        for s in 0..2 {
            self.solve(s, t_left, &utc);
        }
        if jd - self.last_hist_jd > (self.impact_jd - self.start_jd) / 600.0 || self.history.is_empty() {
            self.last_hist_jd = jd;
            self.history.push([t_left / 3600.0, self.sys[0].view.ip.unwrap_or(-1.0), self.sys[1].view.ip.unwrap_or(-1.0)]);
        }
        if self.sys[0].solved_n != self.corridor_n {
            self.corridor_n = self.sys[0].solved_n;
            self.corridor = self.sample_corridor(0, 300);
        }

        if jd >= self.impact_jd - 1e-9 {
            self.impacted = true;
            let pt = self.impact_lonlat(self.b_true);
            self.true_impact = Some(pt);
            self.events.push((
                "bad",
                format!(
                    "IMPACT {} UTC: {} struck Earth at {:.1}° {}, {:.1}° {} — {:.1} Mt",
                    astro::format_utc(self.impact_jd),
                    self.designation,
                    pt[1].abs(),
                    if pt[1] >= 0.0 { "N" } else { "S" },
                    pt[0].abs(),
                    if pt[0] >= 0.0 { "E" } else { "W" },
                    self.energy_mt
                ),
            ));
        }
    }

    fn discover(&mut self, s: usize, jd: f64, t_left: f64, utc: &str, by: &str) {
        let sy = &mut self.sys[s];
        sy.disc_jd = Some(jd);
        sy.view.discovered_utc = Some(utc.to_string());
        sy.view.warning_s = Some(t_left);
        if s == 0 || self.sys[0].disc_jd.is_none_or_ne(jd) {
            self.events.push((
                "warn",
                format!(
                    "{}: {} discovered by {} ({}), V = {:.1}, {} before impact",
                    self.sys[s].view.name,
                    self.designation,
                    by,
                    if s == 0 { "follow-up tasked" } else { "ground-only timeline" },
                    self.now.v_mag,
                    fmt_dur(t_left)
                ),
            ));
        }
        // the discovery tracklet itself
        let sig = if by.starts_with("FACET") { 0.3 } else { GROUND.sigma_arcsec };
        for i in 0..3 {
            let t = jd - i as f64 * 20.0 / 1440.0;
            if t > self.start_jd {
                self.add_obs(&[s], t, sig);
            }
        }
    }

    fn solve(&mut self, s: usize, t_left: f64, _utc: &str) {
        let (od, has) = (&self.sys[s].od, self.sys[s].disc_jd.is_some());
        if !has || od.n_obs < 6 || od.arc_days() < 1.0 / 24.0 {
            return;
        }
        if od.n_obs == self.sys[s].solved_n {
            // No new data: the solution is unchanged; only Palermo (time) moves.
            if let (Some(ip), true) = (self.sys[s].view.ip, ip_pos(self.sys[s].view.ip)) {
                let years = (t_left / 86_400.0 / 365.25).max(1.0 / 365.25);
                let fb = 0.03 * self.energy_mt.max(1e-6).powf(-0.8);
                self.sys[s].view.palermo = Some((ip / (fb * years)).log10());
            }
            return;
        }
        self.sys[s].solved_n = od.n_obs;
        let Some((p, dx)) = od.solve() else { return };
        let m = &self.m_bplane;
        // E·M (2×6)
        let mut em = [[0.0; 6]; 2];
        for k in 0..6 {
            let col = V3::new(m[0][k], m[1][k], m[2][k]) * AU_KM;
            em[0][k] = col.dot(self.e1);
            em[1][k] = col.dot(self.e2);
        }
        let mut nom = self.b_true;
        let mut cov = [[0.0; 2]; 2];
        for a in 0..2 {
            for k in 0..6 {
                nom[a] += em[a][k] * dx[k];
            }
            for b in 0..2 {
                let mut acc = 0.0;
                for i in 0..6 {
                    for j in 0..6 {
                        acc += em[a][i] * p[i][j] * em[b][j];
                    }
                }
                cov[a][b] = acc;
            }
        }
        let ip = disc_probability(nom, cov, self.b_capture_km, &mut self.rng);
        let tr = cov[0][0] + cov[1][1];
        let det = cov[0][0] * cov[1][1] - cov[0][1] * cov[1][0];
        let lmax = (tr / 2.0 + ((tr * tr / 4.0 - det).max(0.0)).sqrt()).max(0.0);
        let v = &mut self.sys[s].view;
        v.n_obs = self.sys[s].od.n_obs;
        v.arc_h = self.sys[s].od.arc_days() * 24.0;
        v.ip = Some(ip);
        v.sigma_b_km = Some(lmax.sqrt());
        v.nominal_miss_km = Some((nom[0] * nom[0] + nom[1] * nom[1]).sqrt());
        let years = (t_left / 86_400.0 / 365.25).max(1.0 / 365.25);
        let fb = 0.03 * self.energy_mt.max(1e-6).powf(-0.8);
        v.palermo = (ip > 0.0).then(|| (ip / (fb * years)).log10());
        let name = v.name;
        for (i, (thr, label)) in [(0.01, "1 %"), (0.1, "10 %"), (0.5, "50 %"), (0.99, "99 %")].iter().enumerate() {
            if ip >= *thr && !self.ip_marks[s][i] {
                self.ip_marks[s][i] = true;
                if i == 0 {
                    self.sys[s].view.ip_1pct_s = Some(t_left);
                }
                if i == 3 {
                    self.sys[s].view.certain_s = Some(t_left);
                }
                self.events.push((if i >= 2 { "bad" } else { "warn" }, format!("{name}: impact probability for {} passed {label}, {} before impact", self.designation, fmt_dur(t_left))));
            }
        }
        self.sys[s].cov_b = Some((nom, cov));
    }

    /// Point on Earth (lon, lat degrees) for a b-plane point inside the capture disc.
    fn impact_lonlat(&self, b: [f64; 2]) -> [f64; 2] {
        let scale = EARTH_RADIUS_KM / self.b_capture_km; // focusing maps the capture disc onto Earth's disc
        let bp = (self.e1 * b[0] + self.e2 * b[1]) * scale;
        let h = (EARTH_RADIUS_KM * EARTH_RADIUS_KM - bp.dot(bp)).max(0.0).sqrt();
        let p = bp - self.s_hat * h;
        let (ra, dec) = p.unit().to_radec();
        let lon = (deg(ra) - deg(astro::gmst(self.impact_jd))).rem_euclid(360.0);
        [if lon > 180.0 { lon - 360.0 } else { lon }, deg(dec)]
    }

    fn sample_corridor(&mut self, s: usize, n: usize) -> Vec<[f64; 2]> {
        let Some((nom, cov)) = self.sys[s].cov_b else { return Vec::new() };
        let (l11, l21) = (cov[0][0].max(0.0).sqrt(), if cov[0][0] > 0.0 { cov[1][0] / cov[0][0].sqrt() } else { 0.0 });
        let l22 = (cov[1][1] - l21 * l21).max(0.0).sqrt();
        let mut out = Vec::new();
        for _ in 0..n * 40 {
            if out.len() >= n {
                break;
            }
            let (g1, g2) = (self.gauss(), self.gauss());
            let b = [nom[0] + l11 * g1, nom[1] + l21 * g1 + l22 * g2];
            if b[0] * b[0] + b[1] * b[1] < self.b_capture_km * self.b_capture_km {
                out.push(self.impact_lonlat(b));
            }
        }
        out
    }

    pub fn systems(&self) -> [&SystemView; 2] {
        [&self.sys[0].view, &self.sys[1].view]
    }
    pub fn discovered(&self) -> bool {
        self.sys[0].disc_jd.is_some()
    }

    pub fn json(&self, jd: f64) -> serde_json::Value {
        let show_pos = self.discovered();
        serde_json::json!({
            "designation": self.designation,
            "scenario": self.cfg.name,
            "diameter_m": self.cfg.diameter_m,
            "h_mag": self.h,
            "albedo": self.cfg.albedo,
            "v_inf_kms": self.cfg.v_inf_kms,
            "radiant_elong_deg": self.cfg.radiant_elong_deg,
            "energy_mt": self.energy_mt,
            "impact_utc": astro::format_utc(self.impact_jd),
            "t_to_impact_s": ((self.impact_jd - jd) * 86_400.0).max(0.0),
            "start_utc": astro::format_utc(self.start_jd),
            "state": if self.impacted { "impacted" } else if show_pos { "tracking" } else { "undiscovered" },
            "now": self.now,
            "position_known": show_pos,
            "systems": self.systems(),
            "history": self.history,
            "corridor": self.corridor,
            "true_impact": self.true_impact,
            "b_capture_km": self.b_capture_km,
            "campaign": self.campaign,
            "ground_model": {
                "disc_lim_v": GROUND.disc_lim_v, "follow_lim_v": GROUND.follow_lim_v, "min_elong_deg": GROUND.min_elong_deg,
                "coverage": GROUND.coverage, "clear": GROUND.clear, "sigma_arcsec": GROUND.sigma_arcsec,
            },
        })
    }
}

fn ip_pos(ip: Option<f64>) -> bool {
    ip.map_or(false, |p| p > 0.0)
}

trait OptNe {
    fn is_none_or_ne(&self, v: f64) -> bool;
}
impl OptNe for Option<f64> {
    fn is_none_or_ne(&self, v: f64) -> bool {
        self.map_or(true, |x| x != v)
    }
}

/// P(|B| < R) for B ~ N(nom, cov).
fn disc_probability(nom: [f64; 2], cov: [[f64; 2]; 2], r: f64, rng: &mut u64) -> f64 {
    let tr = cov[0][0] + cov[1][1];
    let det = (cov[0][0] * cov[1][1] - cov[0][1] * cov[1][0]).max(1e-300);
    let lmin = (tr / 2.0 - ((tr * tr / 4.0 - det).max(0.0)).sqrt()).max(1e-300);
    if lmin.sqrt() > r / 8.0 {
        // Smooth over the disc: polar quadrature.
        let inv = [[cov[1][1] / det, -cov[0][1] / det], [-cov[1][0] / det, cov[0][0] / det]];
        let norm = 1.0 / (std::f64::consts::TAU * det.sqrt());
        let (nr, nt) = (60, 96);
        let mut acc = 0.0;
        for i in 0..nr {
            let rr = r * (i as f64 + 0.5) / nr as f64;
            for j in 0..nt {
                let t = std::f64::consts::TAU * (j as f64 + 0.5) / nt as f64;
                let (x, y) = (rr * t.cos() - nom[0], rr * t.sin() - nom[1]);
                let q = x * (inv[0][0] * x + inv[0][1] * y) + y * (inv[1][0] * x + inv[1][1] * y);
                acc += (-0.5 * q).exp() * rr;
            }
        }
        (acc * norm * (r / nr as f64) * (std::f64::consts::TAU / nt as f64)).clamp(0.0, 1.0)
    } else {
        // Narrow: Monte Carlo.
        let l11 = cov[0][0].max(0.0).sqrt();
        let l21 = if l11 > 0.0 { cov[1][0] / l11 } else { 0.0 };
        let l22 = (cov[1][1] - l21 * l21).max(0.0).sqrt();
        let n = 20_000;
        let mut hit = 0;
        let mut next = || {
            let mut x = *rng;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            *rng = x;
            ((x.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..n {
            let (u, v) = (next().max(1e-300), next());
            let m = (-2.0 * u.ln()).sqrt();
            let (g1, g2) = (m * (std::f64::consts::TAU * v).cos(), m * (std::f64::consts::TAU * v).sin());
            let (x, y) = (nom[0] + l11 * g1, nom[1] + l21 * g1 + l22 * g2);
            if x * x + y * y < r * r {
                hit += 1;
            }
        }
        hit as f64 / n as f64
    }
}

pub fn fmt_dur(s: f64) -> String {
    let s = s.max(0.0);
    if s >= 2.0 * 86_400.0 {
        format!("{:.1} days", s / 86_400.0)
    } else if s >= 2.0 * 3600.0 {
        format!("{:.1} h", s / 3600.0)
    } else if s >= 120.0 {
        format!("{:.0} min", s / 60.0)
    } else {
        format!("{s:.0} s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kepler_round_trip_and_energy() {
        let r0 = V3::new(1.0, 0.1, 0.05);
        let v0 = V3::new(-0.002, 0.017, 0.001);
        for dt in [0.1, 10.0, 365.0, -50.0] {
            let (r, v) = propagate(r0, v0, dt);
            let (rb, vb) = propagate(r, v, -dt);
            assert!((rb - r0).norm() < 1e-11 && (vb - v0).norm() < 1e-13, "dt {dt}");
            let e0 = v0.dot(v0) / 2.0 - MU / r0.norm();
            let e1 = v.dot(v) / 2.0 - MU / r.norm();
            assert!((e0 - e1).abs() < 1e-14);
        }
        // circular orbit at 1 AU: period one sidereal year
        let (r, _) = propagate(V3::new(1.0, 0.0, 0.0), V3::new(0.0, K_GAUSS, 0.0), std::f64::consts::TAU / K_GAUSS);
        assert!((r - V3::new(1.0, 0.0, 0.0)).norm() < 1e-9);
    }

    #[test]
    fn hg_magnitudes() {
        // H for a 1 km, p = 0.25 body = 5 log10(1329 / 0.5) ≈ 17.12
        assert!((h_mag(1000.0, 0.25) - 17.12).abs() < 0.02);
        assert!((v_mag(20.0, 1.0, 1.0, 0.0) - 20.0).abs() < 1e-9);
        assert!(v_mag(20.0, 1.0, 1.0, rad(130.0)) - 20.0 > 5.0);
    }

    #[test]
    fn truth_hits_earth_and_ip_goes_to_one() {
        let mut cfg = presets()[2].clone();
        cfg.days = 20.0;
        let jd0 = 2_461_360.5;
        let mut sc = Scenario::new(cfg, jd0);
        let (rho, _) = sc.geo(sc.impact_jd);
        let miss = rho.norm() * AU_KM;
        assert!(miss < EARTH_RADIUS_KM, "true miss {miss} km");
        // Feed a long arc of good astrometry into the combined system.
        sc.sys[0].disc_jd = Some(jd0);
        for i in 0..400 {
            let t = jd0 + i as f64 * 0.04;
            sc.add_obs(&[0], t, 0.1);
        }
        sc.solve(0, 1e5, "");
        let ip = sc.sys[0].view.ip.unwrap();
        assert!(ip > 0.99, "IP {ip}");
        let s = sc.sys[0].view.sigma_b_km.unwrap();
        assert!(s < 2000.0, "sigma_b {s}");
        // A short arc gives a small, uncertain IP.
        let mut sc2 = Scenario::new(presets()[2].clone(), jd0);
        sc2.sys[1].disc_jd = Some(jd0);
        for i in 0..8 {
            sc2.add_obs(&[1], jd0 + i as f64 * 0.01, 0.3);
        }
        sc2.solve(1, 1e6, "");
        assert!(sc2.sys[1].view.ip.unwrap() < 0.5);
    }

    #[test]
    fn disc_probability_limits() {
        let mut r = 12345u64;
        let p = disc_probability([0.0, 0.0], [[1e6, 0.0], [0.0, 1e6]], 1000.0, &mut r);
        let expect = 1.0 - (-0.5f64).exp(); // R = σ
        assert!((p - expect).abs() < 0.01, "{p} vs {expect}");
        assert_eq!(disc_probability([5000.0, 0.0], [[1.0, 0.0], [0.0, 1.0]], 1000.0, &mut r), 0.0);
        assert_eq!(disc_probability([0.0, 0.0], [[1.0, 0.0], [0.0, 1.0]], 1000.0, &mut r), 1.0);
    }

    #[test]
    fn corridor_lands_on_earth_near_truth() {
        let sc = Scenario::new(presets()[0].clone(), 2_461_360.5);
        let p = sc.impact_lonlat(sc.b_true);
        assert!(p[0].abs() <= 180.0 && p[1].abs() <= 90.0);
    }
}

#[cfg(test)]
mod scenario_tests {
    use super::*;

    fn run(i: usize, days: Option<f64>) -> (Option<f64>, Option<f64>, [SystemView; 2], Vec<String>) {
        let o = crate::obs::ObsParams::default();
        let hw = crate::imaging::Hardware::from_obs(&o);
        let jd0 = 2_461_360.5;
        let mut cfg = presets()[i].clone();
        if let Some(d) = days {
            cfg.days = d;
        }
        let mut sc = Scenario::new(cfg, jd0);
        let dt = 300.0;
        let mut jd = jd0;
        let mut found = None;
        let mut ev = Vec::new();
        while !sc.impacted {
            jd += dt / 86_400.0;
            let b = astro::bodies(jd);
            let inp = StepIn { jd, dt_s: dt, sun: b[0].dir, moon: b[1].dir, o: &o, hw: &hw, search_eyes: 100, locked_eyes: if found.is_some() { 25 } else { 0 } };
            sc.step(&inp);
            if found.is_none() && sc.discovered() {
                found = Some((sc.impact_jd - jd) * 24.0);
            }
            ev.extend(sc.events.drain(..).map(|e| e.1));
        }
        let s = sc.systems();
        (found, s[1].warning_s.map(|w| w / 3600.0), [s[0].clone(), s[1].clone()], ev)
    }

    #[test]
    fn scenarios_play_out() {
        for i in 0..3 {
            let (f, g, s, ev) = run(i, None);
            eprintln!("--- {}: FACET found {:?} h before; ground {:?} h", presets()[i].name, f, g);
            for v in &s {
                eprintln!("    {:15} IP {:?} sigma_b {:?} km  certain {:?} h  obs {}", v.name, v.ip, v.sigma_b_km.map(|x| x.round()), v.certain_s.map(|x| (x / 360.0).round() / 10.0), v.n_obs);
            }
            for e in ev.iter().take(12) {
                eprintln!("      {e}");
            }
        }
        // The night-side 140 m object is bright for weeks: both systems find it.
        let (f, g, ..) = run(2, None);
        assert!(f.is_some() && g.is_some());
        // The sunward Tunguska-class object: FACET finds it, the ground does not.
        let (f, g, ..) = run(0, None);
        assert!(f.is_some() && g.is_none(), "facet {f:?} ground {g:?}");
    }
}

#[cfg(test)]
mod rng_tests {
    use super::*;
    #[test]
    fn rng_is_uniform_and_poisson_mean() {
        let mut sc = Scenario::new(presets()[0].clone(), 2_461_360.5);
        let xs: Vec<f64> = (0..10000).map(|_| sc.rand()).collect();
        let m = xs.iter().sum::<f64>() / xs.len() as f64;
        let lo = xs.iter().filter(|&&x| x < 0.05).count();
        let k: u32 = (0..10000).map(|_| sc.poisson(0.0227)).sum();
        eprintln!("mean {m} lo {lo} poisson total {k}");
        assert!((m - 0.5).abs() < 0.02 && (k as f64 - 227.0).abs() < 60.0);
    }
}
