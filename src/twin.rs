//! Twin-telescope satellites: two apertures on one spacecraft, a rigid boom
//! apart, doing intensity interferometry on large nearby stars.
//!
//! A cluster of separate satellites can't fly closer than a few metres, so
//! stars bigger than ~5 mas (Betelgeuse, Antares, Arcturus…) are resolved out
//! at every cluster baseline. Two telescopes on a 0.5–3 m boom sit inside
//! those stars' first visibility lobe. Rolling the spacecraft turns the
//! baseline through every position angle; observing in several spectral
//! channels samples several points of the visibility curve at once, because
//! the spatial frequency is baseline ÷ wavelength.
//!
//! The twins are a small share of the fleet (1–5 %, set in the panel) taken
//! out of the shared imaging pool and dedicated to their target. Design
//! choices here are ASSUMED: boom length, rigid-boom metrology (1 mm), a
//! shared on-board clock (no inter-satellite sync error), the same detectors
//! as the clusters, and published approximate uniform-disk diameters.

use crate::app::App;
use crate::intensity::{bessel_j1, mag_at};
use crate::math::V3;
use serde::{Deserialize, Serialize};

/// Large stars: name, approximate uniform-disk angular diameter (mas) in
/// the visible. Published values, rounded; the visible-light disk of a
/// cool supergiant depends on wavelength and the star varies.
pub const BIG_STARS: [(&str, f64); 6] = [
    ("Betelgeuse", 45.0),
    ("Antares", 41.0),
    ("Gacrux", 25.0),
    ("Arcturus", 21.0),
    ("Aldebaran", 20.0),
    ("Pollux", 8.0),
];

const C: f64 = 299_792_458.0;
const NB: usize = 48;
const FMIN: f64 = 0.3; // spatial frequency, millions of wavelengths
const FMAX: f64 = 15.0;
/// Rigid-boom path-length knowledge, m (ASSUMED).
const BOOM_METROLOGY_M: f64 = 1e-3;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TwinCfg {
    pub on: bool,
    /// Share of the fleet built as twins, percent (1–5).
    pub pct: f64,
    /// Aperture separation along the boom, m.
    pub boom_m: f64,
    pub star: usize,
}

impl Default for TwinCfg {
    fn default() -> Self {
        TwinCfg { on: false, pct: 1.0, boom_m: 1.5, star: 0 }
    }
}

pub struct Twin {
    pub cfg: TwinCfg,
    pub w: Vec<f64>,
    pub wy: Vec<f64>,
    /// Σ w·f per bin: each channel sits at one fixed spatial frequency, so
    /// the fit uses the weighted mean frequency, not the bin centre.
    pub wf: Vec<f64>,
    pub sat_s: f64,
    pub n_twins: usize,
    /// Now: observing, Earth in the way, keep-out, SAA / safed, slewing or settling.
    pub now: [usize; 5],
    pub fit: Option<(f64, f64)>,
    key: (usize, i64),
    rng: u64,
}

impl Default for Twin {
    fn default() -> Self {
        Twin { cfg: TwinCfg::default(), w: vec![0.0; NB], wy: vec![0.0; NB], wf: vec![0.0; NB], sat_s: 0.0, n_twins: 0, now: [0; 5], fit: None, key: (usize::MAX, -1), rng: 0x7a11_0f00_dead_beef }
    }
}

/// Uniform-disk |V|² at spatial frequency `f` (millions of wavelengths).
pub fn v2f(theta_mas: f64, f: f64) -> f64 {
    let x = std::f64::consts::PI * (theta_mas / 3.6e6).to_radians() * f * 1e6;
    if x < 1e-6 {
        return 1.0;
    }
    (2.0 * bessel_j1(x) / x).powi(2)
}

fn bin_of(f: f64) -> Option<usize> {
    (FMIN..FMAX).contains(&f).then(|| (((f / FMIN).ln() / (FMAX / FMIN).ln()) * NB as f64) as usize)
}
fn bin_centre(k: usize) -> f64 {
    FMIN * (FMAX / FMIN).powf((k as f64 + 0.5) / NB as f64)
}

/// Channel centre wavelengths, nm: log-spaced over 400–700 nm.
pub fn channels(n: u32) -> Vec<f64> {
    let n = n.max(1);
    (0..n).map(|k| 400.0 * (700.0f64 / 400.0).powf((k as f64 + 0.5) / n as f64)).collect()
}

impl Twin {
    fn gauss(&mut self) -> f64 {
        let mut u = || {
            let mut x = self.rng.max(1);
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.rng = x;
            ((x >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        let (a, b) = (u(), u());
        (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
    }
    pub fn reset(&mut self) {
        self.w = vec![0.0; NB];
        self.wy = vec![0.0; NB];
        self.wf = vec![0.0; NB];
        self.sat_s = 0.0;
        self.fit = None;
    }
    /// Which satellites are twins: spread evenly through the fleet.
    pub fn members(&self, n: usize) -> Vec<usize> {
        if !self.cfg.on || n == 0 {
            return Vec::new();
        }
        let k = ((n as f64 * self.cfg.pct / 100.0).round() as usize).clamp(1, n);
        (0..k).map(|j| ((j as f64 + 0.5) * n as f64 / k as f64) as usize).collect()
    }
    fn f_of(&self, k: usize) -> f64 {
        if self.w[k] > 0.0 { self.wf[k] / self.w[k] } else { bin_centre(k) }
    }
    fn fit_theta(&self) -> Option<(f64, f64)> {
        if (0..NB).filter(|&k| self.w[k] > 0.0).count() < 2 {
            return None;
        }
        let chi = |t: f64| -> f64 { (0..NB).filter(|&k| self.w[k] > 0.0).map(|k| self.w[k] * (self.wy[k] / self.w[k] - v2f(t, self.f_of(k))).powi(2)).sum() };
        let grid: Vec<f64> = (0..1000).map(|i| 1.0 * (150.0f64).powf(i as f64 / 999.0)).collect();
        let (mut bt, mut bc) = (0.0, f64::MAX);
        for &t in &grid {
            let c = chi(t);
            if c < bc {
                bc = c;
                bt = t;
            }
        }
        // Several diameters fit sidelobe data; report the error only when the
        // best fit is well separated (Δχ² = 1 interval around it).
        let ok: Vec<f64> = grid.iter().copied().filter(|&t| (t / bt - 1.0).abs() < 0.5 && chi(t) <= bc + 1.0).collect();
        let err = 0.5 * (ok.last().copied().unwrap_or(bt) - ok.first().copied().unwrap_or(bt));
        Some((bt, err.max(1e-3)))
    }
}

impl App {
    fn twin_star(&self) -> Option<(&'static str, f64, V3, f64, f64)> {
        let (name, theta) = BIG_STARS[self.twin.cfg.star.min(BIG_STARS.len() - 1)];
        let s = self.names.iter().find(|s| s.name == name)?;
        let bv = crate::catalog::bv_named(&s.name, s.dir, s.mag) as f64;
        Some((name, theta, s.dir, s.mag as f64, bv))
    }

    /// Before the schedulers run: reserve the twins and give them their target.
    pub fn twin_prepare(&mut self) {
        let n = self.shell.len();
        self.obs.reserved.clear();
        self.obs.reserved.resize(n, false);
        let m = self.twin.members(n);
        self.twin.n_twins = m.len();
        for i in m {
            self.obs.reserved[i] = true;
        }
        self.obs.twin_dir = if self.twin.cfg.on { self.twin_star().map(|s| s.2) } else { None };
    }

    /// SNR per √s at |V|² = 1 for one twin in a channel at `lambda_nm`.
    pub fn twin_k(&self, mag_v: f64, bv: f64, lambda_nm: f64) -> f64 {
        let hw = crate::imaging::Hardware::from_obs(&self.obs_params);
        let c = &self.ii.cfg;
        let tau = ((c.jitter_ps * 1e-12).powi(2) + 2.0 * (BOOM_METROLOGY_M / C).powi(2)).sqrt();
        let df = 1.0 / (2.0 * tau);
        let band_hz = C / 400e-9 - C / 700e-9;
        let n = hw.ph0 * 1e4 / band_hz * 10f64.powf(-0.4 * mag_at(mag_v, bv, lambda_nm));
        hw.area_cm2() / 1e4 * c.alpha * n * (df / 2.0).sqrt()
    }

    /// After the attitude step: credit locked twins with |V|² samples.
    pub fn twin_step(&mut self, _dt: f64) {
        self.twin.now = [0; 5];
        if !self.twin.cfg.on {
            return;
        }
        let key = (self.twin.cfg.star, (self.twin.cfg.boom_m * 1000.0) as i64);
        if key != self.twin.key {
            self.twin.key = key;
            self.twin.reset();
        }
        let Some((_, theta, sdir, mv, bv)) = self.twin_star() else { return };
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let blocked = crate::obs::sky_blocked(&self.obs_params, sdir, sun, moon).is_some();
        let chans: Vec<(f64, f64)> = channels(self.ii.cfg.channels).into_iter().map(|l| (l, self.twin_k(mv, bv, l))).collect();
        let boom = self.twin.cfg.boom_m;
        let members = self.twin.members(self.shell.len());
        let mut samples: Vec<(f64, f64)> = Vec::new();
        for i in members {
            let dt = self.obs.obs_dt.get(i).copied().unwrap_or(0.0);
            let in_saa = self.env.in_saa.get(i).copied().unwrap_or(false);
            if sdir.dot(self.obs.rhat[i]) <= self.obs.cos_lim {
                self.twin.now[1] += 1;
            } else if blocked {
                self.twin.now[2] += 1;
            } else if in_saa && self.env.cfg.saa_policy == crate::environment::SaaPolicy::Pause {
                self.twin.now[3] += 1;
            } else if dt <= 0.0 || self.obs.job.get(i).map_or(true, |&j| crate::obs::job_tag(j) != crate::obs::TAG_TWIN) {
                self.twin.now[4] += 1;
            } else {
                self.twin.now[0] += 1;
                self.twin.sat_s += dt;
                for &(lam, k) in &chans {
                    let f = boom / (lam * 1e-9) / 1e6;
                    samples.push((f, 1.0 / (k * dt.sqrt())));
                }
            }
        }
        for (f, sigma) in samples {
            if let Some(kb) = bin_of(f) {
                let y = v2f(theta, f) + sigma * self.twin.gauss();
                let w = 1.0 / (sigma * sigma);
                self.twin.w[kb] += w;
                self.twin.wy[kb] += w * y;
                self.twin.wf[kb] += w * f;
            }
        }
        self.twin.fit = self.twin.fit_theta();
    }

    pub fn twin_json(&self) -> serde_json::Value {
        let t = &self.twin;
        let star = self.twin_star();
        let bins: Vec<serde_json::Value> = (0..NB)
            .filter(|&k| t.w[k] > 0.0)
            .map(|k| serde_json::json!({"f_mlambda": t.f_of(k), "v2": t.wy[k] / t.w[k], "err": 1.0 / t.w[k].sqrt()}))
            .collect();
        serde_json::json!({
            "on": t.cfg.on,
            "pct": t.cfg.pct,
            "twins": t.n_twins,
            "boom_m": t.cfg.boom_m,
            "star": star.map(|s| s.0),
            "theta_true_mas": star.map(|s| s.1),
            "observing_now": t.now[0],
            "twin_hours": t.sat_s / 3600.0,
            "fit": t.fit.map(|(th, e)| serde_json::json!({"theta_mas": th, "err_mas": e})),
            "bins": bins,
        })
    }

    pub fn twin_panel(&mut self, ui: &mut egui::Ui) {
        use egui::{Color32, RichText, Stroke};
        ui.horizontal(|ui| {
            ui.label(RichText::new("Twin-telescope satellites").strong());
            crate::provenance::chip(ui, "twin");
        });
        ui.checkbox(&mut self.twin.cfg.on, "two telescopes on a rigid boom, for large stars (assumed design)")
            .on_hover_text("Separate satellites can't fly close enough for stars larger than ~5 mas. A twin carries both apertures on one bus.");
        if !self.twin.cfg.on {
            ui.label(RichText::new("Off: every satellite is a single telescope.").small().weak());
            return;
        }
        let n = self.shell.len();
        let c = &mut self.twin.cfg;
        ui.add(egui::Slider::new(&mut c.pct, 1.0..=5.0).step_by(0.5).suffix(" %").text("share of the fleet"));
        ui.add(egui::Slider::new(&mut c.boom_m, 0.5..=3.0).step_by(0.1).suffix(" m").text("aperture separation"));
        let names: Vec<String> = BIG_STARS.iter().map(|(s, t)| format!("{s} (~{t:.0} mas)")).collect();
        egui::ComboBox::from_id_source("twin_star").selected_text(&names[c.star.min(names.len() - 1)]).show_ui(ui, |ui| {
            for (k, s) in names.iter().enumerate() {
                ui.selectable_value(&mut c.star, k, s);
            }
        });
        let k_twins = self.twin.members(n).len();
        ui.label(RichText::new(format!("{k_twins} twin satellites, taken out of the imaging pool")).small());
        let t = &self.twin;
        let Some((name, theta, _, mv, bv)) = self.twin_star() else { return };
        let now = t.now;
        ui.label(
            RichText::new(format!(
                "Now: {} observing · {} Earth in the way · {} keep-out · {} SAA · {} slewing/settling",
                now[0], now[1], now[2], now[3], now[4]
            ))
            .small(),
        );
        let lam_mid = 530.0;
        let k = self.twin_k(mv, bv, lam_mid);
        let f_mid = t.cfg.boom_m / (lam_mid * 1e-9) / 1e6;
        let v = v2f(theta, f_mid);
        ui.label(
            RichText::new(format!(
                "{name}, V {mv:.2}, B−V {bv:+.2}: at {:.1} m and {lam_mid:.0} nm |V|² = {v:.3}; one twin reaches SNR {:.2} per √s per channel there · {:.1} twin-hours on target",
                t.cfg.boom_m,
                k * v,
                t.sat_s / 3600.0
            ))
            .small(),
        );
        match t.fit {
            Some((th, e)) => crate::provenance::line(ui, "twin", format!("Fitted diameter {th:.1} ± {e:.1} mas (assumed true ~{theta:.0} mas)")),
            None => {
                ui.label(RichText::new("Not enough data for a fit yet.").small().weak());
            }
        }
        if ui.small_button("Clear twin measurements").clicked() {
            self.twin.reset();
        }
        // |V|² against spatial frequency.
        let t = &self.twin;
        let w = ui.available_width().min(420.0);
        let (resp, p) = ui.allocate_painter(egui::vec2(w, 140.0), egui::Sense::hover());
        let r = resp.rect;
        p.rect_filled(r, 2.0, Color32::from_gray(18));
        p.text(r.left_top() + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, "|V|² against baseline/wavelength (log, 0.3–15 Mλ): measured, assumed model, fit", egui::FontId::proportional(10.0), Color32::GRAY);
        let xf = |f: f64| r.left() + ((f / FMIN).ln() / (FMAX / FMIN).ln()) as f32 * r.width();
        let yv = |v: f64| r.bottom() - 6.0 - (v.clamp(-0.1, 1.1) as f32 + 0.1) / 1.2 * (r.height() - 20.0);
        let curve = |th: f64, col: Color32| {
            let pts: Vec<egui::Pos2> = (0..200).map(|i| { let f = FMIN * (FMAX / FMIN).powf(i as f64 / 199.0); egui::pos2(xf(f), yv(v2f(th, f))) }).collect();
            egui::Shape::line(pts, Stroke::new(1.2_f32, col))
        };
        p.add(curve(theta, Color32::from_gray(110)));
        if let Some((th, _)) = t.fit {
            p.add(curve(th, Color32::from_rgb(120, 220, 140)));
        }
        for kb in 0..NB {
            if t.w[kb] > 0.0 {
                let (f, y, e) = (t.f_of(kb), t.wy[kb] / t.w[kb], 1.0 / t.w[kb].sqrt());
                let x = xf(f);
                p.line_segment([egui::pos2(x, yv(y - e)), egui::pos2(x, yv(y + e))], Stroke::new(1.0_f32, Color32::from_rgb(255, 190, 110)));
                p.circle_filled(egui::pos2(x, yv(y)), 2.0, Color32::from_rgb(255, 190, 110));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn betelgeuse_sits_in_the_first_lobe_on_a_boom() {
        // 45 mas at 1.5 m: first null at 1.22 λ/θ ≈ 2.5 m (450 nm), so a
        // 1.5 m boom samples the main lobe at every channel.
        for lam in channels(4) {
            let f = 1.5 / (lam * 1e-9) / 1e6;
            let v = v2f(45.0, f);
            assert!(v > 0.02 && v < 0.9, "λ {lam:.0} nm: |V|² {v}");
        }
        // Monotonic in the main lobe: longer wavelength, less resolved.
        let c = channels(4);
        assert!(v2f(45.0, 1.5 / (c[0] * 1e-9) / 1e6) < v2f(45.0, 1.5 / (c[3] * 1e-9) / 1e6));
    }

    #[test]
    fn members_spread_and_count() {
        let mut t = Twin::default();
        t.cfg.on = true;
        t.cfg.pct = 1.0;
        let m = t.members(1000);
        assert_eq!(m.len(), 10);
        assert!(m.windows(2).all(|w| w[1] - w[0] == 100));
        t.cfg.pct = 5.0;
        assert_eq!(t.members(10_000).len(), 500);
    }
}
