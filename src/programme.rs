//! Synthetic science programme: a population of (invented) investigator
//! teams who keep submitting observing requests, so the constellation runs
//! the way a busy, oversubscribed observatory does, plus the ledger that
//! measures what the fleet delivers ("science return").
//!
//! Every target, team and designation here is synthetic. The case mix,
//! integration times, team sizes and deadlines are ASSUMED, chosen to look
//! like a general-purpose small-telescope facility with a planetary-defence
//! emphasis; they are knobs, not data.

use crate::math::{rad, V3};
use serde::Serialize;

#[derive(Clone, Copy)]
pub enum Sky {
    /// Anywhere outside the Sun keep-out.
    Any,
    /// |b| > 30° (extragalactic sky).
    HighLat,
    /// |b| < 10° (Galactic plane).
    Plane,
    /// Near the ecliptic at a solar elongation range (degrees).
    Ecliptic(f64, f64),
    /// A planet (Mercury … Neptune).
    Planet,
}

/// What a science case needs from the schedule, beyond telescope-seconds.
/// The ASSUMED thresholds are typical of the science, not from a user study.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Need {
    /// Any mix of telescopes and pieces adds up (deep stacks, monitoring).
    #[default]
    Pooled,
    /// A fixed event window `exposure_s` long that opens `lead_h` after
    /// submission (a predicted transit or occultation). The requested number
    /// of telescopes must be locked on for at least `min_cover` of the
    /// window, with no gap longer than `max_gap_s`. Otherwise the
    /// measurement fails: no partial credit.
    Window { lead_h: (f64, f64), min_cover: f64, max_gap_s: f64 },
    /// `visits` separate visits, each a 1/visits share of the integration,
    /// at least `spacing_s` apart (end of one to start of the next), so the
    /// motion of a moving target can be measured.
    Cadence { visits: u32, spacing_s: f64 },
}

impl Need {
    pub fn describe(&self) -> String {
        match *self {
            Need::Pooled => "telescope-seconds pool".into(),
            Need::Window { min_cover, max_gap_s, .. } => {
                format!("fixed window, ≥ {:.0} % covered, no gap > {}", 100.0 * min_cover, crate::allsky::fmt_span(max_gap_s))
            }
            Need::Cadence { visits, spacing_s } => format!("{visits} visits ≥ {} apart", crate::allsky::fmt_span(spacing_s)),
        }
    }
}

pub struct Case {
    pub need: Need,
    pub key: &'static str,
    pub label: &'static str,
    team: &'static str,
    /// Relative share of submissions.
    weight: f64,
    tel: (u32, u32),
    exp_s: (f64, f64),
    prio: (i32, i32),
    /// Data are only useful if the request completes within this time.
    pub deadline_h: f64,
    sky: Sky,
}

/// Indices into the deep-sky catalogue for the cases that use real targets.
pub struct RealLists {
    /// Clusters and nearby galaxies brighter than V/B 11.
    pub clusters_galaxies: Vec<usize>,
    /// Galaxies brighter than 14th magnitude at |b| > 20° (transient hosts).
    pub host_galaxies: Vec<usize>,
}

pub fn real_lists() -> &'static RealLists {
    use std::sync::OnceLock;
    static L: OnceLock<RealLists> = OnceLock::new();
    L.get_or_init(|| {
        let d = crate::catalog::dso();
        let gal_pole = gal_to_eq(V3::Z);
        RealLists {
            clusters_galaxies: (0..d.len())
                .filter(|&k| (d[k].is_cluster() || d[k].is_galaxy()) && d[k].mag.map_or(false, |m| m <= 11.0))
                .collect(),
            host_galaxies: (0..d.len())
                .filter(|&k| d[k].is_galaxy() && d[k].mag.map_or(false, |m| m <= 14.0) && d[k].dir.dot(gal_pole).abs() > rad(20.0).sin())
                .collect(),
        }
    })
}

pub const CASES: [Case; 10] = [
    Case { need: Need::Cadence { visits: 3, spacing_s: 1200.0 }, key: "neo", label: "NEO follow-up astrometry", team: "NEO", weight: 18.0, tel: (1, 3), exp_s: (120.0, 600.0), prio: (6, 8), deadline_h: 24.0, sky: Sky::Ecliptic(60.0, 175.0) },
    Case { need: Need::Pooled, key: "transient", label: "Supernova / transient follow-up", team: "transient", weight: 10.0, tel: (4, 16), exp_s: (600.0, 1800.0), prio: (7, 8), deadline_h: 48.0, sky: Sky::HighLat },
    Case { need: Need::Pooled, key: "grb", label: "GRB afterglow (target of opportunity)", team: "GRB", weight: 1.0, tel: (10, 40), exp_s: (600.0, 1200.0), prio: (10, 10), deadline_h: 2.0, sky: Sky::Any },
    Case { need: Need::Window { lead_h: (0.5, 1.5), min_cover: 0.99, max_gap_s: 5.0 }, key: "occult", label: "Stellar occultation timing", team: "occultation", weight: 3.0, tel: (4, 12), exp_s: (120.0, 300.0), prio: (9, 9), deadline_h: 2.0, sky: Sky::Ecliptic(90.0, 180.0) },
    Case { need: Need::Window { lead_h: (1.0, 8.0), min_cover: 0.95, max_gap_s: 300.0 }, key: "transit", label: "Exoplanet transit photometry", team: "exoplanet", weight: 14.0, tel: (2, 6), exp_s: (1800.0, 7200.0), prio: (4, 5), deadline_h: 12.0, sky: Sky::Any },
    Case { need: Need::Pooled, key: "variable", label: "Variable-star monitoring", team: "variable-star", weight: 16.0, tel: (1, 2), exp_s: (300.0, 900.0), prio: (3, 4), deadline_h: 72.0, sky: Sky::Plane },
    Case { need: Need::Pooled, key: "comet", label: "Comet monitoring", team: "comet", weight: 5.0, tel: (2, 4), exp_s: (300.0, 900.0), prio: (5, 6), deadline_h: 48.0, sky: Sky::Ecliptic(50.0, 110.0) },
    Case { need: Need::Pooled, key: "planet", label: "Planetary atmospheres", team: "planetary", weight: 6.0, tel: (2, 8), exp_s: (300.0, 1200.0), prio: (4, 6), deadline_h: 24.0, sky: Sky::Planet },
    Case { need: Need::Pooled, key: "cluster", label: "Clusters & nearby galaxies", team: "stellar-pops", weight: 8.0, tel: (4, 12), exp_s: (900.0, 2400.0), prio: (2, 3), deadline_h: 168.0, sky: Sky::Any },
    Case { need: Need::Pooled, key: "deep", label: "Deep fields", team: "deep-field", weight: 3.0, tel: (20, 80), exp_s: (1800.0, 5400.0), prio: (1, 2), deadline_h: 336.0, sky: Sky::HighLat },
];

const ADJ: [&str; 25] = [
    "Aurora", "Basalt", "Cedar", "Delta", "Ember", "Fjord", "Granite", "Harbor", "Iris", "Juniper", "Kestrel", "Lumen", "Meridian",
    "Nimbus", "Orchid", "Pinnacle", "Quarry", "Riverbend", "Sable", "Tamarack", "Umber", "Vesper", "Willow", "Yarrow", "Zephyr",
];

pub struct Investigator {
    pub name: String,
    pub case: usize,
}

pub struct Draft {
    pub case: usize,
    pub inv: usize,
    pub target_name: String,
    pub body: Option<usize>,
    /// A known near-Earth asteroid (index into neo::all()), tracked.
    pub neo: Option<usize>,
    pub dir: V3,
    pub telescopes: u32,
    pub exposure_s: f64,
    pub priority: i32,
}

impl Case {
    pub fn weight(&self) -> f64 {
        self.weight
    }
}

impl Draft {
    pub fn cost(&self) -> f64 {
        self.telescopes as f64 * self.exposure_s
    }
}

/// Request generator. Demand is metered in telescope-seconds: `load` = 1
/// submits work at the rate the whole fleet could integrate it.
pub struct Generator {
    pub on: bool,
    pub load: f64,
    pub investigators: Vec<Investigator>,
    rng: u64,
    credit: f64,
    pending: Option<Draft>,
    serial: u64,
    primed: bool,
    /// Real NEO follow-up targets (direction, name), refreshed by `App::neo_tick`.
    pub neo_targets: Vec<(V3, String, usize)>,
}

impl Generator {
    pub fn new(n_inv: usize, seed: u64) -> Generator {
        let mut g = Generator { on: true, load: 1.5, investigators: Vec::new(), rng: seed | 1, credit: 0.0, pending: None, serial: 0, primed: false, neo_targets: Vec::new() };
        g.set_investigators(n_inv);
        g
    }

    pub fn set_investigators(&mut self, n: usize) {
        let n = n.clamp(1, 999);
        while self.investigators.len() < n {
            let k = self.investigators.len();
            let case = self.pick_case();
            let name = format!("{} {} team (INV-{:03})", ADJ[(k * 7 + case) % ADJ.len()], CASES[case].team, k + 1);
            self.investigators.push(Investigator { name, case });
        }
        self.investigators.truncate(n);
    }

    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.f()
    }
    /// Log-uniform: as many short requests as long ones per decade.
    fn log_range(&mut self, a: f64, b: f64) -> f64 {
        (a.ln() + (b.ln() - a.ln()) * self.f()).exp()
    }
    fn letter(&mut self) -> char {
        (b'A' + (self.next() % 26) as u8) as char
    }
    fn pick_case(&mut self) -> usize {
        let total: f64 = CASES.iter().map(|c| c.weight).sum();
        let mut x = self.f() * total;
        for (i, c) in CASES.iter().enumerate() {
            if x < c.weight {
                return i;
            }
            x -= c.weight;
        }
        CASES.len() - 1
    }

    fn sky_dir(&mut self, sky: Sky, sun: V3) -> V3 {
        match sky {
            Sky::Any | Sky::Planet => self.uniform(),
            Sky::HighLat | Sky::Plane => {
                let l = self.range(0.0, std::f64::consts::TAU);
                let (lo, hi) = if matches!(sky, Sky::Plane) { (-10.0f64, 10.0f64) } else { (30.0, 90.0) };
                let s = if matches!(sky, Sky::HighLat) && self.f() < 0.5 { -1.0 } else { 1.0 };
                // uniform in area between the latitude limits
                let (a, b) = (rad(lo).sin(), rad(hi).sin());
                let bl = s * self.range(a, b).asin();
                gal_to_eq(V3::new(bl.cos() * l.cos(), bl.cos() * l.sin(), bl.sin()))
            }
            Sky::Ecliptic(e0, e1) => {
                let eps = rad(23.439_3);
                let pole = V3::new(0.0, -eps.sin(), eps.cos());
                let s = (sun - pole * sun.dot(pole)).unit();
                let w = pole.cross(s);
                let e = rad(self.range(e0, e1));
                let side = if self.f() < 0.5 { -1.0 } else { 1.0 };
                let beta = rad(self.range(-8.0, 8.0));
                let d = s * e.cos() + w * (side * e.sin());
                (d * beta.cos() + pole * beta.sin()).unit()
            }
        }
    }
    /// A random entry of `list` whose direction passes `ok` (30 tries).
    fn pick_from<T>(&mut self, list: &[T], dir: impl Fn(&T) -> V3, ok: impl Fn(V3) -> bool) -> Option<T>
    where
        T: Copy,
    {
        if list.is_empty() {
            return None;
        }
        for _ in 0..30 {
            let k = list[(self.next() % list.len() as u64) as usize];
            if ok(dir(&k)) {
                return Some(k);
            }
        }
        None
    }

    fn uniform(&mut self) -> V3 {
        let z = self.range(-1.0, 1.0);
        let a = self.range(0.0, std::f64::consts::TAU);
        let r = (1.0 - z * z).sqrt();
        V3::new(r * a.cos(), r * a.sin(), z)
    }

    fn draft(&mut self, sun: V3, sun_excl_deg: f64, moon: V3, moon_excl_deg: f64, planets: &[V3]) -> Draft {
        let inv = (self.next() % self.investigators.len() as u64) as usize;
        let case = if self.f() < 0.8 { self.investigators[inv].case } else { self.pick_case() };
        let c = &CASES[case];
        self.serial += 1;
        let ser = self.serial;
        let cos_ex = rad(sun_excl_deg + 5.0).cos();
        let cos_mx = rad(moon_excl_deg + 5.0).cos();
        // Real targets where a catalogue exists: (direction, name).
        let ok_dir = |d: V3| d.dot(sun) < cos_ex && d.dot(moon) < cos_mx;
        let mut neo = None;
        let real: Option<(V3, String)> = match c.key {
            "cluster" => {
                let l = real_lists();
                self.pick_from(&l.clusters_galaxies, |&k| crate::catalog::dso()[k].dir, ok_dir)
                    .map(|k| { let d = &crate::catalog::dso()[k]; (d.dir, d.name()) })
            }
            "transient" => {
                let l = real_lists();
                let ser3 = (self.letter().to_ascii_lowercase(), self.letter().to_ascii_lowercase(), self.letter().to_ascii_lowercase());
                self.pick_from(&l.host_galaxies, |&k| crate::catalog::dso()[k].dir, ok_dir).map(|k| {
                    let d = &crate::catalog::dso()[k];
                    (d.dir, format!("AT 2026{}{}{} in {}", ser3.0, ser3.1, ser3.2, d.short()))
                })
            }
            "variable" => {
                let v = crate::catalog::variables();
                let idx: Vec<usize> = (0..v.len()).collect();
                self.pick_from(&idx, |&k| v[k].dir, ok_dir)
                    .map(|k| (v[k].dir, format!("{} (V {:.1}–{:.1})", v[k].name, v[k].bright, v[k].faint)))
            }
            "neo" => {
                let t = std::mem::take(&mut self.neo_targets);
                let idx: Vec<usize> = (0..t.len()).collect();
                let r = self.pick_from(&idx, |&k| t[k].0, ok_dir).map(|k| {
                    neo = Some(t[k].2);
                    (t[k].0, t[k].1.clone())
                });
                self.neo_targets = t;
                r
            }
            "deep" => {
                let f = &crate::catalog::DEEP_FIELDS;
                let idx: Vec<usize> = (0..f.len()).collect();
                let dir = |k: &usize| V3::from_radec(rad(f[*k].1), rad(f[*k].2));
                self.pick_from(&idx, dir, ok_dir).map(|k| (dir(&k), f[k].0.to_string()))
            }
            _ => None,
        };
        let (body, dir) = if let Some((d, _)) = &real {
            (None, *d)
        } else if matches!(c.sky, Sky::Planet) {
            // Mercury … Neptune (body indices 2..=8) that are clear of the Sun.
            let ok: Vec<usize> = (2..=8).filter(|&b| planets.get(b).map_or(false, |d| d.dot(sun) < cos_ex)).collect();
            let b = if ok.is_empty() { 5 } else { ok[(self.next() % ok.len() as u64) as usize] };
            (Some(b), planets.get(b).copied().unwrap_or(V3::Z))
        } else {
            let mut d = self.sky_dir(c.sky, sun);
            for _ in 0..30 {
                // Proposers avoid targets the Sun or Moon will block.
                if d.dot(sun) < cos_ex && d.dot(moon) < cos_mx {
                    break;
                }
                d = self.sky_dir(c.sky, sun);
            }
            (None, d)
        };
        let target_name = if let Some((_, n)) = real { n } else { match c.key {
            "neo" => format!("NEO candidate 2026 {}{}{}", self.letter(), self.letter(), 1 + self.next() % 99),
            "transient" => format!("AT 2026{}{}{}", self.letter().to_ascii_lowercase(), self.letter().to_ascii_lowercase(), self.letter().to_ascii_lowercase()),
            "grb" => format!("GRB afterglow G-{ser:04}"),
            "occult" => format!("Occultation OCC-{ser:04}"),
            "transit" => format!("Transit host FX-{:05}", self.next() % 100_000),
            "variable" => format!("Variable V-{:05}", self.next() % 100_000),
            "comet" => format!("Comet C/2026 {}{}", self.letter(), 1 + self.next() % 9),
            "planet" => crate::astro::BODY_NAMES[body.unwrap_or(5)].to_string(),
            "cluster" => format!("Cluster/galaxy CG-{:04}", self.next() % 10_000),
            _ => format!("Deep field DF-{:02}", 1 + self.next() % 40),
        } };
        let telescopes = self.log_range(c.tel.0 as f64, c.tel.1 as f64 + 0.999).floor() as u32;
        let exposure_s = (self.log_range(c.exp_s.0, c.exp_s.1) / 60.0).round() * 60.0;
        let priority = c.prio.0 + (self.next() % (c.prio.1 - c.prio.0 + 1) as u64) as i32;
        Draft { case, inv, target_name, body, neo, dir, telescopes: telescopes.max(1), exposure_s: exposure_s.max(60.0), priority }
    }

    /// Advance the generator by `dt` sim seconds; returns the requests that
    /// arrive. `room` caps how many more programme requests may be open.
    #[allow(clippy::too_many_arguments)]
    /// `room` caps the total of new open requests; `case_room` caps each
    /// science case, so a backlog in one case (low-priority deep fields
    /// waiting for their deadline) cannot starve the others.
    pub fn tick(
        &mut self,
        dt: f64,
        n_sats: usize,
        room: usize,
        case_room: &mut [usize],
        sun: V3,
        sun_excl_deg: f64,
        moon: V3,
        moon_excl_deg: f64,
        planets: &[V3],
    ) -> Vec<Draft> {
        let mut out = Vec::new();
        if !self.on || n_sats == 0 || self.investigators.is_empty() {
            return out;
        }
        let rate = self.load * n_sats as f64; // telescope-seconds per second
        if !self.primed {
            // An oversubscribed facility starts with a backlog: ~30 min of fleet time.
            self.credit = rate * 1800.0;
            self.primed = true;
        }
        self.credit = (self.credit + rate * dt).min(rate * 3600.0 + 1e6);
        let mut skipped = 0;
        while out.len() < 1500 && out.len() < room && skipped < 200 {
            let d = match self.pending.take() {
                Some(d) => d,
                None => self.draft(sun, sun_excl_deg, moon, moon_excl_deg, planets),
            };
            if case_room.get(d.case).map_or(false, |&r| r == 0) {
                // This team's case is saturated: the proposal waits for the next cycle.
                skipped += 1;
                continue;
            }
            if self.credit >= d.cost() {
                if let Some(r) = case_room.get_mut(d.case) {
                    *r -= 1;
                }
                self.credit -= d.cost();
                out.push(d);
            } else {
                self.pending = Some(d);
                break;
            }
        }
        out
    }
}

/// Galactic (l, b unit vector) → J2000 equatorial.
fn gal_to_eq(g: V3) -> V3 {
    // Rows of the equatorial→galactic rotation (Hipparcos/ICRS convention).
    const T: [[f64; 3]; 3] = [
        [-0.054_875_560_4, -0.873_437_090_2, -0.483_835_015_5],
        [0.494_109_427_9, -0.444_829_630_0, 0.746_982_244_5],
        [-0.867_666_149_0, -0.198_076_373_4, 0.455_983_776_2],
    ];
    V3::new(
        T[0][0] * g.x + T[1][0] * g.y + T[2][0] * g.z,
        T[0][1] * g.x + T[1][1] * g.y + T[2][1] * g.z,
        T[0][2] * g.x + T[1][2] * g.y + T[2][2] * g.z,
    )
}

// ---------------------------------------------------------------- ledger

#[derive(Clone, Default, Serialize)]
pub struct CaseStats {
    pub key: &'static str,
    pub label: &'static str,
    pub deadline_h: f64,
    pub submitted: u32,
    pub completed: u32,
    pub on_time: u32,
    pub expired: u32,
    /// Finished, but the schedule broke the science requirement (gaps,
    /// short coverage): no partial credit.
    pub failed: u32,
    /// Why the most recent failures failed (newest last, up to 6).
    pub recent_failures: Vec<String>,
    pub requirement: String,
    pub open: u32,
    /// Telescope-seconds integrated.
    pub tel_s: f64,
    #[serde(skip)]
    pub resp: Vec<f32>,
    #[serde(skip)]
    pub depth: Vec<f32>,
    /// Achieved photometric precision per hour (transits), ppm.
    #[serde(skip)]
    pub prec: Vec<f32>,
    pub median_response_s: Option<f64>,
    pub median_depth_mag: Option<f64>,
    pub median_precision_ppm: Option<f64>,
}

/// What the fleet did with its time, and what it delivered.
#[derive(Clone, Default, Serialize)]
pub struct Science {
    pub since_utc: String,
    /// Sim seconds accounted.
    pub elapsed_s: f64,
    /// Satellite-seconds by state.
    pub sat_s: f64,
    pub integrating_s: f64,
    pub overhead_s: f64,
    pub idle_s: f64,
    pub hold_s: f64,
    /// Detectors safed in the South Atlantic Anomaly.
    pub saa_s: f64,
    /// Exposure lost to Earthshine (raw − dark-sky-equivalent), included in integrating_s.
    pub earthshine_s: f64,
    /// Integrating satellite-seconds by use.
    pub by_use: UseSplit,
    /// Σ A·Ω·t over all science integration, m²·deg²·s.
    pub etendue_s: f64,
    /// Sub-frames read out and their raw (16-bit) volume.
    pub frames: f64,
    pub bytes: f64,
    pub cases: Vec<CaseStats>,
    /// Telescope-seconds and completions per investigator.
    #[serde(skip)]
    pub inv_tel_s: Vec<f64>,
    #[serde(skip)]
    pub inv_done: Vec<u32>,
}

#[derive(Clone, Default, Serialize)]
pub struct UseSplit {
    pub programme: f64,
    pub public: f64,
    pub stare: f64,
    pub director: f64,
    pub survey: f64,
    pub neo_sweep: f64,
}

impl Science {
    pub fn new(utc: String) -> Science {
        Science {
            since_utc: utc,
            cases: CASES
                .iter()
                .map(|c| CaseStats { key: c.key, label: c.label, deadline_h: c.deadline_h, requirement: c.need.describe(), ..Default::default() })
                .collect(),
            ..Default::default()
        }
    }
    /// Dark-sky-equivalent integration per working satellite-second.
    /// None until an hour has been simulated: the first minutes are
    /// dominated by start-up slews and would understate the steady state.
    pub fn duty(&self) -> Option<f64> {
        let d = self.sat_s - self.hold_s;
        if d > 0.0 && self.elapsed_s >= 3600.0 {
            Some((self.integrating_s - self.earthshine_s) / d)
        } else {
            None
        }
    }
    pub fn inv(&mut self, i: usize) -> (&mut f64, &mut u32) {
        if self.inv_tel_s.len() <= i {
            self.inv_tel_s.resize(i + 1, 0.0);
            self.inv_done.resize(i + 1, 0);
        }
        (&mut self.inv_tel_s[i], &mut self.inv_done[i])
    }
    pub fn push_capped(v: &mut Vec<f32>, x: f64) {
        if x.is_finite() {
            if v.len() >= 4000 {
                v.remove(0);
            }
            v.push(x as f32);
        }
    }

    /// JSON summary with derived figures of merit.
    pub fn summary(&self, investigators: &[Investigator]) -> serde_json::Value {
        let days = (self.elapsed_s / 86_400.0).max(1e-9);
        let mut cases = self.cases.clone();
        for c in &mut cases {
            c.median_response_s = median(&c.resp);
            c.median_depth_mag = median(&c.depth);
            c.median_precision_ppm = median(&c.prec);
        }
        let done: u32 = cases.iter().map(|c| c.completed).sum();
        let on_time: u32 = cases.iter().map(|c| c.on_time).sum();
        let expired: u32 = cases.iter().map(|c| c.expired + c.failed).sum();
        let mut top: Vec<(usize, f64)> = self.inv_tel_s.iter().copied().enumerate().filter(|x| x.1 > 0.0).collect();
        top.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let served = self.inv_done.iter().filter(|&&d| d > 0).count();
        let pct = |x: f64| if self.sat_s > 0.0 { 100.0 * x / self.sat_s } else { 0.0 };
        let u = &self.by_use;
        let sci = u.programme + u.public + u.stare + u.director;
        serde_json::json!({
            "since_utc": self.since_utc,
            "elapsed_s": self.elapsed_s,
            "efficiency": {
                "integrating_pct": pct(self.integrating_s),
                "overhead_pct": pct(self.overhead_s),
                "idle_pct": pct(self.idle_s),
                "hold_pct": pct(self.hold_s),
                "saa_pct": pct(self.saa_s),
                "earthshine_pct": pct(self.earthshine_s),
                "pointed_science_pct": pct(sci),
                "survey_pct": pct(u.survey + u.neo_sweep),
            },
            "use_h": {
                "programme": u.programme / 3600.0, "public": u.public / 3600.0, "stare": u.stare / 3600.0,
                "director": u.director / 3600.0, "survey": u.survey / 3600.0, "neo_sweep": u.neo_sweep / 3600.0,
            },
            "per_day": {
                "completed": done as f64 / days,
                "tel_h": self.integrating_s / 3600.0 / days,
                "etendue_m2deg2h": self.etendue_s / 3600.0 / days,
                "frames": self.frames / days,
                "tb": self.bytes / 1e12 / days,
            },
            "completed": done,
            "on_time": on_time,
            "expired": expired,
            "on_time_pct": if on_time + expired > 0 { Some(100.0 * on_time as f64 / (done + expired) as f64) } else { None },
            "investigators": investigators.len(),
            "investigators_served": served,
            "top": top.iter().take(8).map(|&(i, s)| serde_json::json!({
                "name": investigators.get(i).map(|v| v.name.as_str()).unwrap_or("?"),
                "case": investigators.get(i).map(|v| CASES[v.case].label).unwrap_or(""),
                "tel_h": s / 3600.0,
                "done": self.inv_done.get(i).copied().unwrap_or(0),
            })).collect::<Vec<_>>(),
            "cases": cases,
        })
    }
}

fn median(v: &[f32]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(s[s.len() / 2] as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn galactic_poles_and_centre() {
        let ngp = gal_to_eq(V3::Z);
        let (ra, dec) = ngp.to_radec();
        assert!((ra.to_degrees() - 192.859).abs() < 0.01 && (dec.to_degrees() - 27.128).abs() < 0.01);
        let gc = gal_to_eq(V3::new(1.0, 0.0, 0.0));
        let (ra, dec) = gc.to_radec();
        assert!((ra.to_degrees() - 266.405).abs() < 0.01 && (dec.to_degrees() + 28.936).abs() < 0.01);
    }

    #[test]
    fn generator_meters_demand_and_avoids_the_sun() {
        let sun = V3::new(1.0, 0.0, 0.0);
        let planets = vec![sun; 9];
        let mut g = Generator::new(120, 42);
        g.load = 1.0;
        let moon = V3::new(-1.0, 0.0, 0.0);
        let mut work = 0.0;
        let mut n = 0;
        for _ in 0..3600 {
            let mut cr = [10_000usize; 10];
            for d in g.tick(1.0, 1000, 10_000, &mut cr, sun, 45.0, moon, 15.0, &planets) {
                assert!(d.dir.dot(sun) < rad(45.0).cos() + 1e-9 || d.body.is_some());
                assert!(d.dir.dot(moon) < rad(15.0).cos() + 1e-9 || d.body.is_some());
                work += d.cost();
                n += 1;
            }
        }
        // ~1.5 fleet-hours submitted (0.5 h backlog + 1 h at load 1).
        let fleet_h = work / 1000.0 / 3600.0;
        assert!(n > 100 && (1.2..1.6).contains(&fleet_h), "{n} requests, {fleet_h:.2} fleet-hours");
        assert_eq!(g.investigators.len(), 120);
    }
}
