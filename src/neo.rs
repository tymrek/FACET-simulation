//! Known near-Earth asteroids: the Minor Planet Center's NEA orbit file.
//!
//! 43,182 objects (snapshot with osculating epoch 2026 Jun 9 for nearly
//! all), heliocentric ecliptic J2000 elements, H and G. Positions are
//! two-body Keplerian from each object's epoch, which is good to arcminutes
//! for months either side of the epoch; planetary perturbations, notably
//! during Earth close approaches, are ignored. Apparent magnitude from the
//! H, G system.
//!
//! This research has made use of data and/or services provided by the
//! International Astronomical Union's Minor Planet Center.

use crate::app::App;
use crate::astro::{self, AU_KM};
use crate::math::{deg, rad, V3};
use serde::Serialize;

static NEO_BIN: &[u8] = include_bytes!("../assets/neo.bin");
static NEO_NAMES: &str = include_str!("../assets/neo_names.txt");

const K_GAUSS: f64 = 0.017_202_098_95;
const LD_AU: f64 = 384_400.0 / AU_KM;
pub const FLAG_PHA: u16 = 0x8000;
pub const FLAG_1KM: u16 = 0x1000;

pub struct Neo {
    pub name: &'static str,
    pub epoch: f64,
    pub a: f64,
    pub e: f64,
    pub i: f64,
    pub node: f64,
    pub peri: f64,
    pub m0: f64,
    pub h: f32,
    pub g: f32,
    pub flags: u16,
}

impl Neo {
    pub fn pha(&self) -> bool {
        self.flags & FLAG_PHA != 0
    }
    pub fn km(&self) -> bool {
        self.flags & FLAG_1KM != 0
    }
    /// Heliocentric J2000 equatorial position, AU.
    pub fn helio(&self, jd: f64) -> V3 {
        let n = K_GAUSS / self.a.powf(1.5);
        let m = (rad(self.m0) + n * (jd - self.epoch)).rem_euclid(std::f64::consts::TAU);
        let e = self.e;
        let mut ea = if e > 0.8 { std::f64::consts::PI } else { m };
        for _ in 0..30 {
            let d = (ea - e * ea.sin() - m) / (1.0 - e * ea.cos());
            ea -= d;
            if d.abs() < 1e-12 {
                break;
            }
        }
        let (x, y) = (self.a * (ea.cos() - e), self.a * (1.0 - e * e).sqrt() * ea.sin());
        let (w, om, inc) = (rad(self.peri), rad(self.node), rad(self.i));
        let (cw, sw, co, so, ci, si) = (w.cos(), w.sin(), om.cos(), om.sin(), inc.cos(), inc.sin());
        let ecl = V3::new(
            (co * cw - so * sw * ci) * x + (-co * sw - so * cw * ci) * y,
            (so * cw + co * sw * ci) * x + (-so * sw + co * cw * ci) * y,
            (sw * si) * x + (cw * si) * y,
        );
        ecl.rot_x(rad(astro::OBLIQUITY_J2000_DEG))
    }
}

pub fn all() -> &'static [Neo] {
    use std::sync::OnceLock;
    static N: OnceLock<Vec<Neo>> = OnceLock::new();
    N.get_or_init(|| {
        let f = |b: &[u8], o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as f64;
        NEO_BIN
            .chunks_exact(44)
            .zip(NEO_NAMES.lines())
            .map(|(c, name)| Neo {
                name,
                epoch: f64::from_le_bytes(c[0..8].try_into().unwrap()),
                a: f(c, 8),
                e: f(c, 12),
                i: f(c, 16),
                node: f(c, 20),
                peri: f(c, 24),
                m0: f(c, 28),
                h: f(c, 32) as f32,
                g: f(c, 36) as f32,
                flags: u16::from_le_bytes([c[40], c[41]]),
            })
            .collect()
    })
}

/// Apparent V from H, G.
fn v_mag(h: f64, g: f64, r: f64, delta: f64, phase: f64) -> f64 {
    let t = (phase / 2.0).tan().max(0.0);
    let p1 = (-3.33 * t.powf(0.63)).exp();
    let p2 = (-1.87 * t.powf(1.22)).exp();
    let phi = ((1.0 - g) * p1 + g * p2).max(1e-12);
    h + 5.0 * (r * delta).log10() - 2.5 * phi.log10()
}

#[derive(Clone, Serialize)]
pub struct Approach {
    pub name: String,
    pub jd: f64,
    pub utc: String,
    pub dist_ld: f64,
    pub peak_v: f64,
    pub h: f64,
    pub pha: bool,
}

/// Geocentric sky state of every NEO at one instant.
#[derive(Default)]
pub struct NeoSky {
    pub jd: f64,
    pub dir: Vec<V3>,
    pub vmag: Vec<f32>,
    pub delta_au: Vec<f32>,
    pub elong_deg: Vec<f32>,
    pub approaches: Vec<Approach>,
    pub approaches_day: i64,
    /// Default diameter-albedo for size quotes.
    pub show: bool,
}

pub fn size_m(h: f64, albedo: f64) -> f64 {
    1329.0e3 / albedo.sqrt() * 10f64.powf(-h / 5.0)
}

impl App {
    /// Refresh positions every simulated half hour, close approaches daily.
    pub fn neo_tick(&mut self) {
        let jd = self.jd;
        let s = &mut self.neo;
        if s.dir.is_empty() || (jd - s.jd).abs() > 1.0 / 48.0 {
            use rayon::prelude::*;
            let earth = crate::defence::earth_pos(jd);
            let sun_dir = (-earth).unit();
            let res: Vec<(V3, f32, f32, f32)> = all()
                .par_iter()
                .map(|o| {
                    let r = o.helio(jd);
                    let rho = r - earth;
                    let d = rho.norm();
                    let phase = (-r).angle(-rho);
                    let v = v_mag(o.h as f64, o.g as f64, r.norm(), d, phase);
                    (rho.unit(), v as f32, d as f32, deg(rho.angle(sun_dir)) as f32)
                })
                .collect();
            s.jd = jd;
            s.dir = res.iter().map(|x| x.0).collect();
            s.vmag = res.iter().map(|x| x.1).collect();
            s.delta_au = res.iter().map(|x| x.2).collect();
            s.elong_deg = res.iter().map(|x| x.3).collect();
            // Real NEO follow-up targets for the science programme: bright
            // enough for a short stack and outside the keep-outs.
            let o = &self.obs_params;
            let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
            let mut cands: Vec<(V3, String)> = (0..s.dir.len())
                .filter(|&k| s.vmag[k] <= 20.5 && crate::obs::sky_blocked(o, s.dir[k], sun, moon).is_none())
                .map(|k| (s.dir[k], format!("{} (V {:.1})", all()[k].name, s.vmag[k])))
                .collect();
            cands.truncate(4000);
            self.gen.neo_targets = cands;
        }
        let day = jd.floor() as i64;
        let s = &mut self.neo;
        if s.approaches.is_empty() && s.approaches_day == 0 || (day - s.approaches_day).abs() >= 3 {
            s.approaches_day = day;
            s.approaches = close_approaches(jd, 60.0, 20.0);
        }
    }

    /// Counts for the panels and the public site.
    pub fn neo_json(&self) -> serde_json::Value {
        let s = &self.neo;
        let upcoming: Vec<&Approach> = s.approaches.iter().filter(|a| a.jd >= self.jd).collect();
        let o = &self.obs_params;
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let hw = crate::imaging::Hardware::from_obs(o);
        let lim1 = crate::imaging::point_lim_mag(&hw, 60.0, 60.0f64.min(o.sub_max_s), 21.5);
        let lim16 = crate::imaging::point_lim_mag(&hw, 960.0, 60.0f64.min(o.sub_max_s), 21.5);
        let n = all().len();
        let (mut open1, mut open16, mut blocked_bright) = (0, 0, 0);
        for k in 0..s.dir.len() {
            let v = s.vmag[k] as f64;
            let blocked = crate::obs::sky_blocked(o, s.dir[k], sun, moon).is_some();
            if v <= lim16 {
                if blocked {
                    blocked_bright += 1;
                } else {
                    open16 += 1;
                    if v <= lim1 {
                        open1 += 1;
                    }
                }
            }
        }
        serde_json::json!({
            "known": n,
            "pha": all().iter().filter(|o| o.pha()).count(),
            "km": all().iter().filter(|o| o.km()).count(),
            "lim_1tel_60s": lim1,
            "lim_16tel": lim16,
            "visible_1tel": open1,
            "visible_16tel": open16,
            "bright_but_in_keepout": blocked_bright,
            "approaches": upcoming,
            "source": "IAU Minor Planet Center NEA orbits (snapshot, epoch 2026 Jun 9); two-body positions",
        })
    }

    pub fn neo_panel(&mut self, ui: &mut egui::Ui) {
        use egui::RichText;
        ui.label(RichText::new("Known near-Earth asteroids").strong());
        ui.checkbox(&mut self.neo.show, "show in sky view (hazardous ones in red)");
        let j = self.neo_json();
        ui.label(RichText::new(format!(
            "{} known (MPC), {} potentially hazardous, {} larger than ~1 km.",
            j["known"], j["pha"], j["km"]
        ))
        .small());
        ui.label(RichText::new(format!(
            "Observable now, outside the keep-outs: {} brighter than V {:.1} (one telescope, 60 s), {} brighter than V {:.1} (16 stacked). {} more are bright enough but inside the Sun or Moon keep-out.",
            j["visible_1tel"], j["lim_1tel_60s"].as_f64().unwrap_or(0.0), j["visible_16tel"], j["lim_16tel"].as_f64().unwrap_or(0.0), j["bright_but_in_keepout"]
        ))
        .small());
        if !self.neo.approaches.is_empty() {
            egui::CollapsingHeader::new(RichText::new("Close approaches, next 60 days (within 20 lunar distances)").small())
                .id_source("neo_ca")
                .show(ui, |ui| {
                    egui::Grid::new("neo_ca_g").striped(true).show(ui, |ui| {
                        for h in ["object", "date (UTC)", "distance", "peak V", "size"] {
                            ui.label(RichText::new(h).small().strong());
                        }
                        ui.end_row();
                        for a in self.neo.approaches.iter().filter(|a| a.jd >= self.jd).take(15) {
                            let col = if a.pha { egui::Color32::from_rgb(255, 120, 100) } else { egui::Color32::from_gray(210) };
                            ui.label(RichText::new(&a.name).small().color(col));
                            ui.label(RichText::new(&a.utc[..16.min(a.utc.len())]).small());
                            ui.label(RichText::new(format!("{:.1} LD", a.dist_ld)).small());
                            ui.label(RichText::new(format!("{:.1}", a.peak_v)).small());
                            ui.label(RichText::new(format!("~{:.0} m", size_m(a.h, 0.14))).small());
                            ui.end_row();
                        }
                    });
                    ui.label(RichText::new("Two-body orbits from the MPC snapshot: distances and dates are approximate; perturbations during the approach are ignored.").small().weak());
                });
        }
    }
}

/// Minimum geocentric distances over the next `days` days below `max_ld`
/// lunar distances, nearest first. Daily sampling, then a golden-section
/// refinement around each minimum.
pub fn close_approaches(jd0: f64, days: f64, max_ld: f64) -> Vec<Approach> {
    use rayon::prelude::*;
    let nd = days as usize + 1;
    let earth: Vec<V3> = (0..nd).map(|k| crate::defence::earth_pos(jd0 + k as f64)).collect();
    let mut out: Vec<Approach> = all()
        .par_iter()
        .filter_map(|o| {
            // Objects that can't come within reach of Earth's orbit are skipped.
            let q = o.a * (1.0 - o.e);
            let qq = o.a * (1.0 + o.e);
            if q > 1.0 + 0.06 || qq < 1.0 - 0.06 {
                return None;
            }
            let d: Vec<f64> = (0..nd).map(|k| (o.helio(jd0 + k as f64) - earth[k]).norm()).collect();
            let (mut kbest, mut dbest) = (0, f64::MAX);
            for k in 0..nd {
                if d[k] < dbest {
                    dbest = d[k];
                    kbest = k;
                }
            }
            if dbest > max_ld * LD_AU * 3.0 {
                return None;
            }
            let dist = |t: f64| (o.helio(t) - crate::defence::earth_pos(t)).norm();
            let (mut a, mut b) = (jd0 + kbest as f64 - 1.0, jd0 + kbest as f64 + 1.0);
            let gr = 0.618_033_988_75;
            for _ in 0..40 {
                let c = b - gr * (b - a);
                let e = a + gr * (b - a);
                if dist(c) < dist(e) {
                    b = e;
                } else {
                    a = c;
                }
            }
            let t = 0.5 * (a + b);
            if t < jd0 || t > jd0 + days {
                return None;
            }
            let dmin = dist(t);
            if dmin > max_ld * LD_AU {
                return None;
            }
            // Peak brightness around the approach.
            let mut peak = f64::MAX;
            for h in -48..=48 {
                let tt = t + h as f64 / 24.0;
                let r = o.helio(tt);
                let rho = r - crate::defence::earth_pos(tt);
                peak = peak.min(v_mag(o.h as f64, o.g as f64, r.norm(), rho.norm(), (-r).angle(-rho)));
            }
            Some(Approach {
                name: o.name.to_string(),
                jd: t,
                utc: astro::format_utc(t),
                dist_ld: dmin / LD_AU,
                peak_v: peak,
                h: o.h as f64,
                pha: o.pha(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.dist_ld.partial_cmp(&b.dist_ld).unwrap());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_eros_is_sane() {
        let a = all();
        assert_eq!(a.len(), 43_182);
        let eros = &a[0];
        assert_eq!(eros.name, "(433) Eros");
        assert!((eros.a - 1.458).abs() < 0.01 && eros.pha() == false);
        assert!(a.iter().filter(|o| o.pha()).count() == 2537);
        // Heliocentric distance stays between perihelion and aphelion.
        for k in 0..50 {
            let r = eros.helio(2_461_200.5 + 20.0 * k as f64).norm();
            assert!(r > 1.13 && r < 1.79, "{r}");
        }
    }

    #[test]
    fn approaches_run() {
        let ca = close_approaches(2_461_363.5, 60.0, 20.0);
        eprintln!("{} approaches within 20 LD; nearest: {:?}", ca.len(), ca.first().map(|a| (&a.name, a.dist_ld, &a.utc)));
        assert!(ca.windows(2).all(|w| w[0].dist_ld <= w[1].dist_ld));
    }
}
#[cfg(test)]
mod check {
    use super::*;
    #[test]
    #[ignore]
    fn compare() {
        let jd = 2_461_363.5 + 0.1; // 2026-11-19
        let ca = close_approaches(jd, 60.0, 20.0);
        for a in ca.iter().filter(|a| a.name.contains("2023 XH3") || a.name.contains("2023 BZ")) {
            eprintln!("{} {} {:.2} LD", a.name, a.utc, a.dist_ld);
        }
        for o in all().iter().filter(|o| o.name == "2023 BZ") {
            let d = (o.helio(jd) - crate::defence::earth_pos(jd)).norm();
            eprintln!("2023 BZ now {:.1} LD", d / LD_AU);
        }
        for name in ["(99942) Apophis", "2023 XH3"] {
            if let Some(o) = all().iter().find(|o| o.name == name) {
                let (mut best, mut bt) = (f64::MAX, 0.0);
                let t0 = if name.contains("Apophis") { 2_462_239.5 } else { 2_461_370.5 };
                for k in 0..(20 * 24 * 60) {
                    let t = t0 + k as f64 / (24.0 * 60.0);
                    let d = (o.helio(t) - crate::defence::earth_pos(t)).norm();
                    if d < best { best = d; bt = t; }
                }
                eprintln!("{} min {:.3} LD at {}  (a {:.4} e {:.4} i {:.3} epoch {})", name, best / LD_AU, astro::format_utc(bt), o.a, o.e, o.i, o.epoch);
            }
        }
        for a in ca.iter().take(8) {
            eprintln!("{:<18} {} {:.2} LD V{:.1}", a.name, &a.utc[..16], a.dist_ld, a.peak_v);
        }
    }
}
