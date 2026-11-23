//! Space-traffic layer: other operators' constellations and objects around
//! FACET, and the three things they mean for operations:
//!
//! * **Data path** — Starlink's optical mesh at 480 km as a relay (an ASSUMED
//!   service), compared with conventional direct-to-ground stations, for a
//!   choice of on-board data products.
//! * **Streak avoidance** — sunlit satellites crossing a FACET field during an
//!   exposure. Their ephemerides are known, so crossings are predicted and the
//!   exposure is gated around them (a short loss of integration) instead of
//!   ruining the frame.
//! * **Neighbourhood notices** — conjunction screening of FACET against
//!   objects near its altitude: time of closest approach, miss distance,
//!   probability of collision, and the avoidance manoeuvre if one is needed.
//!
//! Orbits are circular with secular J2 (as for FACET). Populations are
//! simplified: Starlink and OneWeb as Walker shells at their published
//! altitudes and inclinations; "other objects" (debris, other operators,
//! small satellites) as an ASSUMED random population around FACET's altitude.

use crate::astro::{EARTH_RADIUS_KM, J2, MU_EARTH};
use crate::math::{rad, V3};
use serde::Serialize;
use std::f64::consts::{PI, TAU};

const C_KMS: f64 = 299_792.458;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Pop {
    Starlink,
    OneWeb,
    Other,
    Facet,
}

impl Pop {
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Pop::Starlink => "Starlink",
            Pop::OneWeb => "OneWeb",
            Pop::Other => "Other object",
            Pop::Facet => "FACET",
        }
    }
}

/// Circular orbit with secular J2 drift.
#[derive(Clone, Copy)]
pub struct Orbit {
    pub a: f64,
    pub inc: f64,
    pub raan0: f64,
    pub u0: f64,
    pub raan_dot: f64,
    pub u_dot: f64,
    pub epoch_jd: f64,
}

impl Orbit {
    pub fn new(alt_km: f64, inc: f64, raan0: f64, u0: f64, epoch_jd: f64) -> Orbit {
        let a = EARTH_RADIUS_KM + alt_km;
        let n0 = (MU_EARTH / a.powi(3)).sqrt();
        let k = J2 * (EARTH_RADIUS_KM / a).powi(2);
        let ci = inc.cos();
        let raan_dot = -1.5 * n0 * k * ci;
        let argp_dot = 0.75 * n0 * k * (5.0 * ci * ci - 1.0);
        let m_dot = n0 * (1.0 + 0.75 * k * (3.0 * ci * ci - 1.0));
        Orbit { a, inc, raan0, u0, raan_dot, u_dot: argp_dot + m_dot, epoch_jd }
    }
    fn angles(&self, jd: f64) -> (f64, f64) {
        let dt = (jd - self.epoch_jd) * 86_400.0;
        (self.raan0 + self.raan_dot * dt, self.u0 + self.u_dot * dt)
    }
    pub fn basis(&self, jd: f64) -> (V3, V3, V3) {
        let (o, _) = self.angles(jd);
        let (so, co) = o.sin_cos();
        let (si, ci) = self.inc.sin_cos();
        let p = V3::new(co, so, 0.0);
        let q = V3::new(-so * ci, co * ci, si);
        (p, q, p.cross(q))
    }
    pub fn pos(&self, jd: f64) -> V3 {
        let (o, u) = self.angles(jd);
        let (so, co) = o.sin_cos();
        let (su, cu) = u.sin_cos();
        let (si, ci) = self.inc.sin_cos();
        V3::new(self.a * (cu * co - su * ci * so), self.a * (cu * so + su * ci * co), self.a * (su * si))
    }
    pub fn u(&self, jd: f64) -> f64 {
        self.angles(jd).1
    }
    pub fn period_s(&self) -> f64 {
        TAU / self.u_dot
    }
}

pub struct Obj {
    pub pop: Pop,
    pub orbit: Orbit,
    /// Can it manoeuvre (active satellite) — for notices.
    pub active: bool,
}

#[derive(Clone, Serialize)]
pub struct Config {
    pub starlink_n: usize,
    pub starlink_alt_km: f64,
    pub oneweb: bool,
    pub other_n: usize,
    /// Half-width of the "other objects" altitude band around FACET, km.
    pub other_band_km: f64,
    pub streak_avoidance: bool,
    pub relay: bool,
    /// Optical relay terminal rate per FACET satellite, Gb/s (ASSUMED).
    pub relay_gbps: f64,
    pub relay_range_km: f64,
    pub stations: usize,
    pub antennas_per_station: usize,
    pub station_gbps: f64,
    pub product: Product,
    /// On-board storage per satellite, TB (ASSUMED).
    pub storage_tb: f64,
    /// Probability-of-collision threshold for an avoidance manoeuvre.
    pub pc_manoeuvre: f64,
    /// Combined hard-body radius, m.
    pub hbr_m: f64,
    pub show: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            starlink_n: 9_950,
            starlink_alt_km: 480.0,
            oneweb: true,
            other_n: 1_500,
            other_band_km: 50.0,
            streak_avoidance: true,
            relay: true,
            relay_gbps: 10.0,
            relay_range_km: 1_500.0,
            stations: 20,
            antennas_per_station: 2,
            station_gbps: 1.2,
            product: Product::Compressed,
            storage_tb: 2.0,
            pc_manoeuvre: 1e-4,
            hbr_m: 5.0,
            show: true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Product {
    Raw,
    Compressed,
    Coadded,
    Catalogue,
}

impl Product {
    pub const ALL: [Product; 4] = [Product::Raw, Product::Compressed, Product::Coadded, Product::Catalogue];
    pub fn label(self) -> &'static str {
        match self {
            Product::Raw => "Raw 16-bit frames",
            Product::Compressed => "Lossless compressed (Rice, ~2.5×)",
            Product::Coadded => "Co-added 4 subs + compressed (~10×)",
            Product::Catalogue => "Source catalogue + cutouts (~1 %)",
        }
    }
    /// Fraction of raw volume sent (ASSUMED reduction factors).
    pub fn factor(self) -> f64 {
        match self {
            Product::Raw => 1.0,
            Product::Compressed => 1.0 / 2.5,
            Product::Coadded => 1.0 / 10.0,
            Product::Catalogue => 0.01,
        }
    }
}

/// Generic ground-station sites (lat, lon), ASSUMED: polar-heavy, as real
/// networks are, so polar and inclined orbits see them often.
const STATIONS: [(f64, f64); 24] = [
    (78.2, 15.4), (-72.0, 2.5), (64.8, -147.5), (67.9, 21.1), (-53.0, -70.8), (19.0, -155.6), (-33.1, -70.7), (-25.9, 27.7),
    (-31.8, 115.9), (35.7, 139.7), (64.1, -21.9), (53.5, -113.5), (37.7, -25.7), (13.4, 144.8), (-45.9, 170.5), (1.3, 103.8),
    (25.2, 55.3), (-20.2, 57.5), (40.4, -3.7), (32.9, -97.0), (-12.0, -77.0), (28.5, 77.2), (50.0, 8.6), (-35.3, 149.1),
];

#[derive(Clone, Serialize)]
pub struct Notice {
    pub id: u64,
    pub facet: usize,
    pub object: String,
    pub pop: Pop,
    pub tca_jd: f64,
    pub tca_utc: String,
    pub miss_km: f64,
    pub radial_km: f64,
    pub rel_speed_kms: f64,
    pub pc: f64,
    pub level: &'static str,
    pub action: String,
    pub dv_ms: f64,
}

#[derive(Default, Clone, Serialize)]
pub struct StreakStats {
    pub frames: f64,
    pub streaked: f64,
    pub by_pop: [f64; 4],
    pub lost_s: f64,
    pub exposed_s: f64,
}

#[derive(Default, Clone, Serialize)]
pub struct ConjStats {
    pub screened_sat_days: f64,
    /// Close approaches inside the 5 km screening volume.
    pub approaches_5km: u32,
    pub notices: u32,
    pub high_risk: u32,
    pub manoeuvres: u32,
    pub coordinated: u32,
    pub dv_ms: f64,
    pub closest_km: Option<f64>,
}

pub struct Traffic {
    pub cfg: Config,
    pub objs: Vec<Obj>,
    pub pos: Vec<V3>,
    pub streaks: StreakStats,
    pub conj: ConjStats,
    pub upcoming: Vec<Notice>,
    pub recent: Vec<Notice>,
    seen: std::collections::HashSet<(usize, usize, i64)>,
    next_notice: u64,
    streak_cursor: usize,
    screen_cursor: usize,
    screen_credit: f64,
    last_screened: Vec<f64>,
    /// Backlog of satellites awaiting screening, as a fraction of the fleet.
    pub screen_lag: f64,
    pub relay: RelayStats,
    pub dtg: DtgStats,
    last_dtg_jd: f64,
    rng: u64,
    built_for: (usize, f64, bool, usize, f64, f64),
    pub events: Vec<(&'static str, String)>,
    pub elapsed_s: f64,
}

#[derive(Default, Clone, Serialize)]
pub struct RelayStats {
    pub sampled: usize,
    pub linked: usize,
    pub mean_range_km: f64,
    pub latency_ms: f64,
}

#[derive(Default, Clone, Serialize)]
pub struct DtgStats {
    /// Fraction of time a satellite is in view of at least one station.
    pub in_view_frac: f64,
    /// Median wait until the next station contact, s.
    pub median_wait_s: f64,
}

/// Inputs from the FACET simulation for one step.
pub struct FacetState<'a> {
    pub jd: f64,
    pub dt_s: f64,
    pub sun: V3,
    pub alt_km: f64,
    pub inc: f64,
    pub raan: &'a [f64],
    pub u: &'a [f64],
    pub pos: &'a [V3],
    /// Boresight of each satellite if integrating, else None.
    pub boresight: &'a [Option<V3>],
    pub fov_deg: f64,
    pub sub_s: f64,
}

impl Traffic {
    /// Measured collision-avoidance rate and Δv per satellite-year, once at
    /// least a few satellite-days have been screened.
    pub fn manoeuvres_per_sat_yr(&self) -> Option<f64> {
        (self.conj.screened_sat_days > 5.0).then(|| self.conj.manoeuvres as f64 / self.conj.screened_sat_days * 365.25)
    }
    pub fn cam_dv_per_sat_yr(&self) -> Option<f64> {
        (self.conj.screened_sat_days > 5.0).then(|| self.conj.dv_ms / self.conj.screened_sat_days * 365.25)
    }

    pub fn new(cfg: Config, jd: f64, facet_alt_km: f64) -> Traffic {
        let mut t = Traffic {
            cfg,
            objs: Vec::new(),
            pos: Vec::new(),
            streaks: Default::default(),
            conj: Default::default(),
            upcoming: Vec::new(),
            recent: Vec::new(),
            seen: Default::default(),
            next_notice: 1,
            streak_cursor: 0,
            screen_cursor: 0,
            screen_credit: 0.0,
            last_screened: Vec::new(),
            screen_lag: 0.0,
            relay: Default::default(),
            dtg: Default::default(),
            last_dtg_jd: 0.0,
            rng: 0x7AFF_1C5E_ED00_0001,
            built_for: (0, 0.0, false, 0, 0.0, 0.0),
            events: Vec::new(),
            elapsed_s: 0.0,
        };
        t.rebuild(jd, facet_alt_km);
        t
    }

    fn rand(&mut self) -> f64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// (Re)build populations if the configuration changed.
    pub fn rebuild(&mut self, jd: f64, facet_alt_km: f64) {
        let key = (self.cfg.starlink_n, self.cfg.starlink_alt_km, self.cfg.oneweb, self.cfg.other_n, self.cfg.other_band_km, facet_alt_km);
        if key == self.built_for && !self.objs.is_empty() {
            return;
        }
        self.built_for = key;
        self.objs.clear();
        self.rng = 0x7AFF_1C5E_ED00_0001;
        // Starlink shells (published inclinations; split ASSUMED), all at the configured altitude.
        let shells = [(53.0, 0.50, 72), (43.0, 0.28, 56), (70.0, 0.07, 36), (97.6, 0.15, 48)];
        for (inc, frac, planes) in shells {
            let n = (self.cfg.starlink_n as f64 * frac).round() as usize;
            self.walker(Pop::Starlink, n, planes, self.cfg.starlink_alt_km, inc, 360.0, jd, true);
        }
        if self.cfg.oneweb {
            // OneWeb: 648 satellites, 12 near-polar planes at 87.9°, ~1200 km.
            self.walker(Pop::OneWeb, 648, 12, 1200.0, 87.9, 180.0, jd, true);
        }
        // Other objects near FACET's altitude: debris, other operators, small sats (ASSUMED mix).
        for _ in 0..self.cfg.other_n {
            let alt = facet_alt_km + (self.rand() * 2.0 - 1.0) * self.cfg.other_band_km;
            let x = self.rand();
            let inc = if x < 0.45 {
                97.4 + (self.rand() - 0.5)
            } else if x < 0.70 {
                51.6
            } else if x < 0.85 {
                53.0 + (self.rand() - 0.5) * 0.4
            } else {
                30.0 + 70.0 * self.rand()
            };
            let active = self.rand() < 0.3;
            let (raan, u) = (self.rand() * TAU, self.rand() * TAU);
            self.objs.push(Obj { pop: Pop::Other, orbit: Orbit::new(alt, rad(inc), raan, u, jd), active });
        }
        self.pos = self.objs.iter().map(|o| o.orbit.pos(jd)).collect();
    }

    #[allow(clippy::too_many_arguments)]
    fn walker(&mut self, pop: Pop, n: usize, planes: usize, alt: f64, inc: f64, span: f64, jd: f64, active: bool) {
        if n == 0 {
            return;
        }
        let planes = planes.min(n).max(1);
        let per = n / planes;
        let off = self.rand() * TAU;
        for p in 0..planes {
            let raan = off + rad(span) * p as f64 / planes as f64;
            let cnt = per + usize::from(p < n % planes);
            for s in 0..cnt {
                let u = TAU * s as f64 / cnt as f64 + TAU * p as f64 / n as f64;
                self.objs.push(Obj { pop, orbit: Orbit::new(alt, rad(inc), raan, u, jd), active });
            }
        }
    }

    pub fn count(&self, pop: Pop) -> usize {
        self.objs.iter().filter(|o| o.pop == pop).count()
    }

    pub fn step(&mut self, f: &FacetState) {
        self.rebuild(f.jd, f.alt_km);
        use rayon::prelude::*;
        self.pos.clear();
        self.pos.par_extend(self.objs.par_iter().map(|o| o.orbit.pos(f.jd)));
        if f.dt_s <= 0.0 {
            return;
        }
        self.elapsed_s += f.dt_s;
        let prof = std::env::var_os("FACET_PROFILE").is_some();
        let t0 = web_time::Instant::now();
        self.streak_sample(f);
        let t1 = web_time::Instant::now();
        self.screen(f);
        let t2 = web_time::Instant::now();
        self.age_notices(f.jd);
        if prof {
            eprintln!("    traffic: streaks {:.1} ms, screening {:.1} ms", (t1 - t0).as_secs_f64() * 1e3, (t2 - t1).as_secs_f64() * 1e3);
        }
    }

    fn facet_orbit(f: &FacetState, i: usize) -> Orbit {
        // Same elements as the FACET shell at this instant.
        Orbit::new(f.alt_km, f.inc, f.raan[i], f.u[i], f.jd)
    }

    // ------------------------------------------------------------ streaks

    fn streak_sample(&mut self, f: &FacetState) {
        use rayon::prelude::*;
        let n = f.pos.len();
        if n == 0 {
            return;
        }
        let tau = f.sub_s.max(1.0);
        let jd1 = f.jd + tau / 86_400.0;
        let fov = rad(f.fov_deg);
        let r_fov = (fov * 0.5 * std::f64::consts::SQRT_2).tan();
        let sun = f.sun;
        let sunlit = |p: V3| {
            let along = p.dot(sun);
            along > 0.0 || (p - sun * along).norm() > EARTH_RADIUS_KM
        };
        // Sources: other operators' satellites and other FACETs (debris is too
        // faint to matter), sunlit, at the start and end of the sub-exposure.
        let mut src: Vec<(V3, V3, Pop)> = self
            .objs
            .par_iter()
            .zip(self.pos.par_iter())
            .filter(|(o, p)| o.pop != Pop::Other && sunlit(**p))
            .map(|(o, &p)| (p, o.orbit.pos(jd1), o.pop))
            .collect();
        src.par_extend((0..n).into_par_iter().filter(|&k| sunlit(f.pos[k])).map(|k| (f.pos[k], Self::facet_orbit(f, k).pos(jd1), Pop::Facet)));
        // Sample integrating telescopes (round robin).
        // Bound the work per step (~400k segment tests natively; a quarter of
        // that in the browser, where it runs on the frame thread: the
        // estimate is unbiased either way, it just converges more slowly).
        const TESTS: usize = if cfg!(target_arch = "wasm32") { 100_000 } else { 400_000 };
        let want = (TESTS / src.len().max(1)).clamp(if cfg!(target_arch = "wasm32") { 2 } else { 8 }, 64).min(n);
        let mut picks = Vec::with_capacity(want);
        let mut tries = 0;
        while picks.len() < want && tries < n {
            tries += 1;
            let i = self.streak_cursor % n;
            self.streak_cursor += 1;
            if let Some(b) = f.boresight[i] {
                picks.push((i, b));
            }
        }
        let results: Vec<([u32; 4], f64)> = picks
            .par_iter()
            .map(|&(i, b)| {
                let fo = Self::facet_orbit(f, i);
                let (p0, p1) = (f.pos[i], fo.pos(jd1));
                let (e, nn) = crate::obs::tangent_basis(b);
                let gn = |d: V3| -> Option<(f64, f64)> {
                    let z = d.dot(b);
                    (z > 0.0).then(|| (d.dot(e) / z, d.dot(nn) / z))
                };
                let mut hits = [0u32; 4];
                let mut lost = 0.0f64;
                for &(q0, q1, pop) in &src {
                    if (q0 - p0).norm() < 1e-3 {
                        continue; // itself
                    }
                    let (d0, d1) = (q0 - p0, q1 - p1);
                    let (Some(a), Some(c)) = (gn(d0), gn(d1)) else { continue };
                    // distance from the boresight (origin) to the gnomonic segment a→c
                    let (vx, vy) = (c.0 - a.0, c.1 - a.1);
                    let l2 = vx * vx + vy * vy;
                    let t = if l2 > 0.0 { (-(a.0 * vx + a.1 * vy) / l2).clamp(0.0, 1.0) } else { 0.0 };
                    let (mx, my) = (a.0 + t * vx, a.1 + t * vy);
                    if mx * mx + my * my > r_fov * r_fov {
                        continue;
                    }
                    // line of sight must clear the Earth and atmosphere
                    let pm = (p0 + p1) * 0.5;
                    let mid = (q0 + q1) * 0.5 - pm;
                    let s = (-pm.dot(mid) / mid.dot(mid).max(1e-9)).clamp(0.0, 1.0);
                    if (pm + mid * s).norm() < EARTH_RADIUS_KM + 100.0 {
                        continue;
                    }
                    let omega = d0.angle(d1) / tau;
                    let dur = if omega > 0.0 { fov / omega } else { tau };
                    lost += (dur + 2.0).min(tau);
                    hits[pop as usize] += 1;
                }
                (hits, lost.min(tau))
            })
            .collect();
        let s = &mut self.streaks;
        for (hits, lost) in results {
            s.frames += 1.0;
            s.exposed_s += tau;
            if hits.iter().any(|&h| h > 0) {
                s.streaked += 1.0;
            }
            for k in 0..4 {
                s.by_pop[k] += hits[k] as f64;
            }
            s.lost_s += lost;
        }
    }

    // ------------------------------------------------------------ conjunctions

    fn screen(&mut self, f: &FacetState) {
        let n = f.pos.len();
        if n == 0 {
            return;
        }
        // Re-screen every satellite every 6 h over a 30 h window, within a
        // compute budget per step. At extreme time rates screening falls
        // behind; statistics are normalised by the satellite-days actually
        // screened, so rates stay unbiased.
        if self.last_screened.len() != n {
            self.last_screened = vec![f64::NAN; n];
        }
        self.screen_credit = (self.screen_credit + n as f64 * f.dt_s / 21_600.0).min(n as f64);
        let window_d = 30.0 / 24.0;
        // Circular orbits never come closer than their altitude difference.
        let cands: Vec<usize> = (0..self.objs.len())
            .filter(|&j| (self.objs[j].orbit.a - (EARTH_RADIUS_KM + f.alt_km)).abs() < 5.0)
            .collect();
        let t0 = web_time::Instant::now();
        let mut found: Vec<(usize, usize, f64, f64)> = Vec::new();
        while self.screen_credit >= 1.0 && t0.elapsed().as_secs_f64() < 0.012 {
            let todo = (self.screen_credit.floor() as usize).min(8 * rayon::current_num_threads());
            self.screen_credit -= todo as f64;
            let sats: Vec<usize> = (0..todo).map(|k| (self.screen_cursor + k) % n).collect();
            self.screen_cursor += todo;
            for &i in &sats {
                let last = self.last_screened[i];
                let new_days = if last.is_nan() { window_d } else { (f.jd - last).clamp(0.0, window_d) };
                self.conj.screened_sat_days += new_days;
                self.last_screened[i] = f.jd;
            }
            let objs = &self.objs;
            found.extend(self.screen_batch(f, &sats, &cands, objs, window_d));
        }
        self.screen_lag = self.screen_credit / n as f64;
        for (i, j, tca, miss) in found {
            let fo = Self::facet_orbit(f, i);
            let ob = self.objs[j].orbit;
            self.add_notice(f, i, &fo, j, &ob, tca, miss);
        }
    }

    fn screen_batch(&self, f: &FacetState, sats: &[usize], cands: &[usize], objs: &[Obj], window_d: f64) -> Vec<(usize, usize, f64, f64)> {
        use rayon::prelude::*;
        sats.par_iter()
            .flat_map_iter(|&i| {
                let fo = Self::facet_orbit(f, i);
                let mut v = Vec::new();
                for &j in cands {
                    for (tca, miss) in Self::screen_pair(f.jd, &fo, &objs[j].orbit, window_d) {
                        v.push((i, j, tca, miss));
                    }
                }
                v
            })
            .collect()
    }

    /// Close approaches (TCA, miss km) of two circular orbits within the window.
    fn screen_pair(jd0: f64, fo: &Orbit, ob: &Orbit, window_d: f64) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        let tf = fo.period_s() / 86_400.0;
        let mut t = jd0;
        while t < jd0 + window_d {
            let (pf, qf, hf) = fo.basis(t);
            let (pj, qj, hj) = ob.basis(t);
            let l = hf.cross(hj);
            if l.norm() < 1e-3 {
                t += tf;
                continue;
            }
            let l = l.unit();
            for sgn in [1.0, -1.0] {
                let lv = l * sgn;
                let uf_l = lv.dot(qf).atan2(lv.dot(pf));
                let dtf = (uf_l - fo.u(t)).rem_euclid(TAU) / fo.u_dot;
                let tc = t + dtf / 86_400.0;
                if tc > jd0 + window_d {
                    continue;
                }
                let uj_l = lv.dot(qj).atan2(lv.dot(pj));
                let mut d = (uj_l - ob.u(tc)).rem_euclid(TAU);
                if d > PI {
                    d -= TAU;
                }
                let est = ((fo.a - ob.a).powi(2) + (ob.a * d).powi(2)).sqrt();
                if est > 25.0 {
                    continue;
                }
                // refine by golden section on the true separation
                let dist = |x: f64| (fo.pos(x) - ob.pos(x)).norm();
                let w = (300.0 + (d / ob.u_dot).abs()) / 86_400.0;
                let (mut lo, mut hi) = (tc - w, tc + w);
                let g = 0.618_033_988_75;
                let (mut x1, mut x2) = (hi - g * (hi - lo), lo + g * (hi - lo));
                let (mut f1, mut f2) = (dist(x1), dist(x2));
                for _ in 0..60 {
                    if f1 < f2 {
                        hi = x2;
                        x2 = x1;
                        f2 = f1;
                        x1 = hi - g * (hi - lo);
                        f1 = dist(x1);
                    } else {
                        lo = x1;
                        x1 = x2;
                        f1 = f2;
                        x2 = lo + g * (hi - lo);
                        f2 = dist(x2);
                    }
                }
                let tca = 0.5 * (lo + hi);
                let miss = dist(tca);
                if miss < 5.0 && tca > jd0 {
                    out.push((tca, miss));
                }
            }
            t += tf;
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn add_notice(&mut self, f: &FacetState, i: usize, fo: &Orbit, j: usize, ob: &Orbit, tca: f64, miss: f64) {
        let key = (i, j, (tca * 1440.0).round() as i64);
        if self.seen.contains(&key) || self.seen.contains(&(i, j, key.2 - 1)) || self.seen.contains(&(i, j, key.2 + 1)) {
            return;
        }
        self.seen.insert(key);
        self.conj.approaches_5km += 1;
        let (rf, rj) = (fo.pos(tca), ob.pos(tca));
        // velocities, km/s
        let h = 1.0 / 86_400.0;
        let vf_s = (fo.pos(tca + h) - fo.pos(tca - h)) * 0.5;
        let vj_s = (ob.pos(tca + h) - ob.pos(tca - h)) * 0.5;
        let d = rj - rf;
        let vrel = vj_s - vf_s;
        let days = (tca - f.jd).max(0.0);
        let ric_cov = |r: V3, v: V3, sr: f64, si: f64, sc: f64| -> [[f64; 3]; 3] {
            let ru = r.unit();
            let cu = r.cross(v).unit();
            let iu = cu.cross(ru);
            let axes = [ru, iu, cu];
            let s2 = [sr * sr, si * si, sc * sc];
            let mut m = [[0.0; 3]; 3];
            for (k, ax) in axes.iter().enumerate() {
                let a = [ax.x, ax.y, ax.z];
                for p in 0..3 {
                    for q in 0..3 {
                        m[p][q] += s2[k] * a[p] * a[q];
                    }
                }
            }
            m
        };
        let ob_obj = &self.objs[j];
        let (sr, si, sc) = match (ob_obj.pop, ob_obj.active) {
            (Pop::Other, true) => (0.05, 0.15 + 0.3 * days, 0.08), // operator-shared ephemeris
            _ => (0.1, 0.3 + 1.0 * days, 0.2),                      // catalogue-quality
        };
        let c1 = ric_cov(rf, vf_s, 0.01, 0.02 + 0.05 * days, 0.01);
        let c2 = ric_cov(rj, vj_s, sr, si, sc);
        let ev = vrel.unit();
        let e1 = (d - ev * d.dot(ev)).unit();
        let e2 = ev.cross(e1);
        let proj = |a: V3, b: V3| -> f64 {
            let (av, bv) = ([a.x, a.y, a.z], [b.x, b.y, b.z]);
            let mut s = 0.0;
            for p in 0..3 {
                for q in 0..3 {
                    s += av[p] * (c1[p][q] + c2[p][q]) * bv[q];
                }
            }
            s
        };
        let (cxx, cxy, cyy) = (proj(e1, e1), proj(e1, e2), proj(e2, e2));
        let det = (cxx * cyy - cxy * cxy).max(1e-18);
        let m = d.dot(e1);
        let q = m * m * cyy / det;
        let hbr = self.cfg.hbr_m / 1000.0;
        let pc = (hbr * hbr / (2.0 * det.sqrt()) * (-0.5 * q).exp()).min(1.0);
        let radial = d.dot(rf.unit());
        let level = if pc >= self.cfg.pc_manoeuvre {
            "red"
        } else if pc >= 1e-5 {
            "amber"
        } else {
            "green"
        };
        self.conj.closest_km = Some(self.conj.closest_km.map_or(miss, |c: f64| c.min(miss)));
        // Notices: inside 1 km, or any non-negligible probability.
        if miss >= 1.0 && pc < 1e-6 {
            return;
        }
        let mut dv = 0.0;
        let action = if level == "red" {
            if ob_obj.active {
                self.conj.coordinated += 1;
                "coordinated with the other operator; FACET manoeuvres".to_string()
            } else {
                "FACET avoidance manoeuvre".to_string()
            }
        } else if level == "amber" {
            "monitor; refine with tasked tracking".into()
        } else {
            "no action".into()
        };
        if level == "red" {
            // Along-track shift ≈ 3·Δv·lead; aim for 5σ in-track separation.
            let lead = (days * 86_400.0).clamp(600.0, 43_200.0);
            let target = (5.0 * (si * si + 0.02f64.powi(2)).sqrt()).max(1.0);
            dv = target / (3.0 * lead) * 1000.0;
            self.conj.manoeuvres += 1;
            self.conj.dv_ms += dv;
        }
        self.conj.notices += 1;
        if pc >= 1e-5 {
            self.conj.high_risk += 1;
        }
        let name = format!("{} {:05}", if ob_obj.active { "OBJ-ACT" } else { "OBJ" }, 40_000 + j % 60_000);
        if level == "red" {
            self.events.push((
                "warn",
                format!(
                    "Conjunction: FACET-{:05} vs {} at {} — miss {:.0} m, Pc {:.1e}: {} (Δv {:.2} m/s)",
                    i + 1,
                    name,
                    crate::astro::format_utc(tca),
                    miss * 1000.0,
                    pc,
                    action,
                    dv
                ),
            ));
        }
        let id = self.next_notice;
        self.next_notice += 1;
        self.upcoming.push(Notice {
            id,
            facet: i,
            object: name,
            pop: ob_obj.pop,
            tca_jd: tca,
            tca_utc: crate::astro::format_utc(tca),
            miss_km: miss,
            radial_km: radial,
            rel_speed_kms: vrel.norm(),
            pc,
            level,
            action,
            dv_ms: dv,
        });
        self.upcoming.sort_by(|a, b| a.tca_jd.partial_cmp(&b.tca_jd).unwrap());
    }

    fn age_notices(&mut self, jd: f64) {
        let (past, fut): (Vec<Notice>, Vec<Notice>) = self.upcoming.drain(..).partition(|n| n.tca_jd <= jd);
        self.upcoming = fut;
        for n in past {
            self.recent.insert(0, n);
        }
        self.recent.truncate(40);
        self.upcoming.truncate(400);
        if self.seen.len() > 200_000 {
            self.seen.clear();
        }
    }

    // ------------------------------------------------------------ data path

    /// Relay availability and latency from a sample of FACET satellites.
    pub fn relay_sample(&mut self, facet_pos: &[V3]) {
        let n = facet_pos.len();
        if n == 0 || !self.cfg.relay {
            self.relay = Default::default();
            return;
        }
        let star: Vec<V3> = self.objs.iter().zip(&self.pos).filter(|(o, _)| o.pop == Pop::Starlink).map(|(_, &p)| p).collect();
        use rayon::prelude::*;
        let stride = (n / 400).max(1);
        let range = self.cfg.relay_range_km;
        let bests: Vec<Option<f64>> = (0..n)
            .into_par_iter()
            .step_by(stride)
            .map(|i| {
                let p = facet_pos[i];
                let mut best = f64::MAX;
                for &q in &star {
                    let d = (q - p).norm();
                    if d < best && d <= range {
                        // line of sight clears 100 km
                        let v = q - p;
                        let s = (-p.dot(v) / v.dot(v)).clamp(0.0, 1.0);
                        if (p + v * s).norm() > EARTH_RADIUS_KM + 100.0 {
                            best = d;
                        }
                    }
                }
                (best < f64::MAX).then_some(best)
            })
            .collect();
        let sampled = bests.len();
        let linked = bests.iter().flatten().count();
        let rsum: f64 = bests.iter().flatten().sum();
        let mean = if linked > 0 { rsum / linked as f64 } else { 0.0 };
        // Mesh path to a gateway (ASSUMED ~2,500 km of laser hops at ~2 ms
        // per hop) plus ~15 ms of terrestrial backhaul.
        let latency_ms = (mean / C_KMS + 2_500.0 / C_KMS) * 1000.0 + 3.0 * 2.0 + 15.0;
        self.relay = RelayStats { sampled, linked, mean_range_km: mean, latency_ms };
    }

    /// Direct-to-ground contact statistics (every ~10 sim minutes).
    pub fn dtg_sample(&mut self, jd: f64, alt_km: f64, inc: f64, raan: &[f64], u: &[f64]) {
        if (jd - self.last_dtg_jd).abs() < 600.0 / 86_400.0 || raan.is_empty() {
            return;
        }
        self.last_dtg_jd = jd;
        let t0 = web_time::Instant::now();
        let _g = Defer(t0);
        let ns = self.cfg.stations.min(STATIONS.len());
        let a = EARTH_RADIUS_KM + alt_km;
        // Earth central angle for a 10° elevation mask
        let el = rad(10.0);
        let lam = (EARTH_RADIUS_KM / a * el.cos()).acos() - el;
        let cos_lam = lam.cos();
        // Station unit vectors in the Earth-fixed frame, computed once; the
        // satellite is rotated into that frame instead.
        let st_ecef: Vec<V3> = STATIONS[..ns].iter().map(|&(la, lo)| crate::astro::ecef_to_eci(lo, la, 1.0, 0.0)).collect();
        let in_view = |p: V3, t: f64| -> bool {
            let e = p.rot_z(-crate::astro::gmst(t)).unit();
            st_ecef.iter().any(|s| e.dot(*s) > cos_lam)
        };
        use rayon::prelude::*;
        let stride = (raan.len() / 200).max(1);
        let mut waits: Vec<f64> = (0..raan.len())
            .into_par_iter()
            .step_by(stride)
            .map(|i| {
                let o = Orbit::new(alt_km, inc, raan[i], u[i], jd);
                (0..=720)
                    .map(|k| k as f64 * 30.0)
                    .find(|&s| {
                        let t = jd + s / 86_400.0;
                        in_view(o.pos(t), t)
                    })
                    .unwrap_or(21_600.0)
            })
            .collect();
        let cnt = waits.len();
        let vis = waits.iter().filter(|&&w| w == 0.0).count();
        waits.sort_by(|a, b| a.partial_cmp(b).unwrap());
        self.dtg = DtgStats { in_view_frac: vis as f64 / cnt.max(1) as f64, median_wait_s: waits.get(waits.len() / 2).copied().unwrap_or(0.0) };
    }

    pub fn json(&self, raw_tb_day: f64, n_facet: usize) -> serde_json::Value {
        let c = &self.cfg;
        let st = &self.streaks;
        let frac_linked = if self.relay.sampled > 0 { self.relay.linked as f64 / self.relay.sampled as f64 } else { 0.0 };
        let tb_day = |gbps: f64| gbps * 1e9 / 8.0 * 86_400.0 / 1e12;
        let relay_cap = if c.relay { tb_day(c.relay_gbps * n_facet as f64 * frac_linked) } else { 0.0 };
        let dtg_cap = tb_day(c.stations.min(STATIONS.len()) as f64 * c.antennas_per_station as f64 * c.station_gbps * 0.7);
        let products: Vec<serde_json::Value> = Product::ALL
            .iter()
            .map(|p| {
                let d = raw_tb_day * p.factor();
                serde_json::json!({
                    "key": p, "label": p.label(), "tb_day": d,
                    "relay_util": if relay_cap > 0.0 { Some(d / relay_cap) } else { None },
                    "dtg_util": d / dtg_cap.max(1e-9),
                    "selected": *p == c.product,
                })
            })
            .collect();
        let demand = raw_tb_day * c.product.factor();
        let cap = relay_cap + dtg_cap;
        let fleet_store = c.storage_tb * n_facet as f64;
        // Fleet-days actually screened (rates per fleet-day = events / that × fleet size).
        let days = (self.conj.screened_sat_days / n_facet.max(1) as f64).max(1e-9);
        // Highest risk first, then soonest.
        let rank = |l: &str| match l {
            "red" => 0,
            "amber" => 1,
            _ => 2,
        };
        let mut upcoming: Vec<&Notice> = self.upcoming.iter().collect();
        upcoming.sort_by(|a, b| rank(a.level).cmp(&rank(b.level)).then(a.tca_jd.partial_cmp(&b.tca_jd).unwrap()));
        upcoming.truncate(30);
        serde_json::json!({
            "counts": { "starlink": self.count(Pop::Starlink), "oneweb": self.count(Pop::OneWeb), "other": self.count(Pop::Other) },
            "config": c,
            "data": {
                "raw_tb_day": raw_tb_day,
                "product": c.product, "product_label": c.product.label(), "demand_tb_day": demand,
                "relay_cap_tb_day": relay_cap, "dtg_cap_tb_day": dtg_cap,
                "relay_linked_frac": frac_linked, "relay_mean_range_km": self.relay.mean_range_km,
                "relay_latency_ms": if c.relay && frac_linked > 0.0 { Some(self.relay.latency_ms) } else { None },
                "dtg_in_view_frac": self.dtg.in_view_frac, "dtg_median_wait_s": self.dtg.median_wait_s,
                "margin": if cap > 0.0 { cap / demand.max(1e-9) } else { 0.0 },
                "storage_fill_days": if demand > cap { Some(fleet_store / (demand - cap)) } else { None },
                "products": products,
            },
            "streaks": {
                "frames": st.frames,
                "streaked_pct": if st.frames > 0.0 { 100.0 * st.streaked / st.frames } else { 0.0 },
                "per_1000_frames": {
                    "starlink": if st.frames > 0.0 { 1000.0 * st.by_pop[0] / st.frames } else { 0.0 },
                    "oneweb": if st.frames > 0.0 { 1000.0 * st.by_pop[1] / st.frames } else { 0.0 },
                    "facet": if st.frames > 0.0 { 1000.0 * st.by_pop[3] / st.frames } else { 0.0 },
                },
                "avoidance": c.streak_avoidance,
                "avoidance_cost_pct": if st.exposed_s > 0.0 { 100.0 * st.lost_s / st.exposed_s } else { 0.0 },
            },
            "conjunctions": {
                "stats": self.conj,
                "per_day": {
                    "approaches_5km": self.conj.approaches_5km as f64 / days,
                    "notices": self.conj.notices as f64 / days,
                    "high_risk": self.conj.high_risk as f64 / days,
                    "manoeuvres": self.conj.manoeuvres as f64 / days,
                    "dv_ms": self.conj.dv_ms / days,
                },
                "dv_per_sat_year_ms": self.conj.dv_ms / days * 365.25 / n_facet.max(1) as f64,
                "screen_backlog_pct": 100.0 * self.screen_lag,
                "upcoming": upcoming,
                "recent": self.recent.iter().take(15).collect::<Vec<_>>(),
            },
        })
    }

    /// Thinned positions for map overlays: (lon, lat, pop) for every k-th object.
    pub fn map_points(&self, jd: f64) -> Vec<(f32, f32, u8)> {
        let g = crate::astro::gmst(jd);
        let mut out = Vec::new();
        for (k, (o, p)) in self.objs.iter().zip(&self.pos).enumerate() {
            let stride = match o.pop {
                Pop::Starlink => 6,
                _ => 1,
            };
            if k % stride != 0 {
                continue;
            }
            let (ra, dec) = p.unit().to_radec();
            let mut lon = (ra - g).to_degrees().rem_euclid(360.0);
            if lon > 180.0 {
                lon -= 360.0;
            }
            out.push((lon as f32, dec.to_degrees() as f32, o.pop as u8));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn populations_and_altitudes() {
        let t = Traffic::new(Config::default(), 2_461_360.5, 550.0);
        assert!((t.count(Pop::Starlink) as i64 - 9_950).abs() < 10);
        assert_eq!(t.count(Pop::OneWeb), 648);
        assert_eq!(t.count(Pop::Other), 1_500);
        for (o, p) in t.objs.iter().zip(&t.pos) {
            let alt = p.norm() - EARTH_RADIUS_KM;
            match o.pop {
                Pop::Starlink => assert!((alt - 480.0).abs() < 1e-6),
                Pop::OneWeb => assert!((alt - 1200.0).abs() < 1e-6),
                _ => assert!((alt - 550.0).abs() <= 50.0 + 1e-6),
            }
        }
    }

    #[test]
    fn screening_finds_a_planted_conjunction() {
        let jd = 2_461_360.5;
        let mut cfg = Config::default();
        cfg.starlink_n = 0;
        cfg.oneweb = false;
        cfg.other_n = 0;
        let mut t = Traffic::new(cfg, jd, 550.0);
        // FACET sat on an inclined orbit; a debris object on a crossing plane,
        // phased to arrive at the node at the same time, 300 m higher.
        let fo = Orbit::new(550.0, rad(53.0), 0.3, 0.0, jd);
        let mut ob = Orbit::new(550.3, rad(97.4), 1.4, 0.0, jd);
        // find node crossing time for FACET then set the object's phase to match
        let (pf, qf, hf) = fo.basis(jd);
        let (pj, qj, hj) = ob.basis(jd);
        let l = hf.cross(hj).unit();
        let uf = l.dot(qf).atan2(l.dot(pf)).rem_euclid(TAU);
        let tc = jd + uf / fo.u_dot / 86_400.0;
        let uj = l.dot(qj).atan2(l.dot(pj));
        ob.u0 = uj - ob.u_dot * (tc - jd) * 86_400.0;
        t.objs.push(Obj { pop: Pop::Other, orbit: ob, active: false });
        t.pos = t.objs.iter().map(|o| o.orbit.pos(jd)).collect();
        let raan = [0.3];
        let u = [0.0];
        let pos = [fo.pos(jd)];
        let bs = [None];
        let f = FacetState { jd, dt_s: 21_600.0, sun: V3::new(1.0, 0.0, 0.0), alt_km: 550.0, inc: rad(53.0), raan: &raan, u: &u, pos: &pos, boresight: &bs, fov_deg: 1.0, sub_s: 60.0 };
        t.screen(&f);
        assert!(t.conj.approaches_5km >= 1, "no close approach found");
        // Brute force: the true closest approach within ±20 min of the planted time.
        let (mut best, mut tbest) = (f64::MAX, 0.0);
        let mut x = tc - 1200.0 / 86_400.0;
        while x < tc + 1200.0 / 86_400.0 {
            let d = (fo.pos(x) - ob.pos(x)).norm();
            if d < best {
                best = d;
                tbest = x;
            }
            x += 0.05 / 86_400.0;
        }
        let closest = t.conj.closest_km.unwrap();
        assert!((closest - best).abs() < 0.01, "screened miss {closest} vs brute force {best} at {tbest}");
        assert!(best < 5.0);
    }
}

struct Defer(web_time::Instant);
impl Drop for Defer {
    fn drop(&mut self) {
        if std::env::var_os("FACET_PROFILE").is_some() {
            eprintln!("    traffic: ground-station sampling {:.1} ms", self.0.elapsed().as_secs_f64() * 1e3);
        }
    }
}
