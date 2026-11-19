//! Intensity (Hanbury Brown–Twiss) interferometry with small satellite
//! clusters, plus the propellant budget it and the rest of the shell need.
//!
//! ## Clusters
//! Each cluster is a "chief" shell satellite and up to five deputies flying
//! on concentric 2:1 relative-motion ellipses around it (a projected
//! circular-orbit formation): radial −ρ cos(u+φ), along-track 2ρ sin(u+φ),
//! cross-track ζ sin(u+ψ), in the chief's local orbital frame (Hill/Clohessy–
//! Wiltshire, valid for these km-scale separations). Deputies share the
//! chief's semi-major axis and inclination and differ only in eccentricity
//! vector, node and phase, so the J2 perturbation acts on all of them alike
//! (a J2-invariant formation) and the nested ellipses never cross. Two
//! clusters sit in planes ~90° apart in node, whose orbit poles are
//! complementary for uv coverage.
//!
//! ## Measurement
//! Every pair of members on the target records |V|² at its projected
//! baseline b. Signal-to-noise per pair (Hanbury Brown 1974):
//!     SNR = A α n |V|² √(m Δf / 2) √T
//! with collecting area A, detection efficiency α, spectral photon flux
//! density n (photons m⁻² s⁻¹ Hz⁻¹), m spectral channels and electronic
//! bandwidth Δf = 1 / (2 τ), τ the combined timing error (detector jitter,
//! clock synchronisation over the laser links, position knowledge / c).
//! The star is a uniform disk, |V|² = (2 J₁(x)/x)², x = π θ b / λ. Noisy
//! measurements are binned by baseline and θ fitted by least squares.
//! All detector and timing inputs are ASSUMED; star diameters are
//! approximate published values.

use crate::app::App;
use crate::math::{rad, V3};
use serde::Serialize;

/// Bright stars with approximate published uniform-disk diameters (mas).
pub const STARS: [(&str, f64); 12] = [
    ("Mirzam", 0.52),
    ("Bellatrix", 0.70),
    ("Adhara", 0.77),
    ("Spica", 0.90),
    ("Regulus", 1.40),
    ("Achernar", 1.80),
    ("Deneb", 2.40),
    ("Rigel", 2.75),
    ("Altair", 3.20),
    ("Vega", 3.30),
    ("Procyon", 5.40),
    ("Sirius", 6.00),
];

const LAMBDA_M: f64 = 450e-9;
const C: f64 = 299_792_458.0;

#[derive(Clone, Serialize)]
pub struct IiCfg {
    pub on: bool,
    pub star: usize,
    /// Telescopes per cluster (chief + deputies), 2–6.
    pub members: usize,
    /// Largest deputy ellipse ρ (along-track excursion is 2ρ), m.
    pub size_m: f64,
    /// Detector timing jitter, clock sync error, position knowledge.
    pub jitter_ps: f64,
    pub sync_ps: f64,
    pub pos_cm: f64,
    /// Detection efficiency (SPAD QE × optics) and spectral channels.
    pub alpha: f64,
    pub channels: u32,
}

impl Default for IiCfg {
    fn default() -> Self {
        IiCfg { on: false, star: 1, members: 6, size_m: 300.0, jitter_ps: 50.0, sync_ps: 10.0, pos_cm: 1.0, alpha: 0.35, channels: 4 }
    }
}

const NBINS: usize = 40;
const BMIN: f64 = 5.0;
const BMAX: f64 = 3000.0;

#[derive(Default)]
pub struct Cluster {
    pub chief: usize,
    pub label: &'static str,
    /// Current member positions relative to the chief in its orbital frame
    /// (radial, along-track, cross-track), m.
    pub rel: Vec<[f64; 3]>,
    pub observing: bool,
    pub why: String,
}

pub struct Ii {
    pub cfg: IiCfg,
    pub clusters: Vec<Cluster>,
    key: (usize, usize),
    // accumulators for the current star
    w: Vec<f64>,
    wy: Vec<f64>,
    pub uv: Vec<(f32, f32, u8)>,
    pub on_target_s: f64,
    pub pair_s: f64,
    rng: u64,
    star_key: usize,
    pub fit: Option<(f64, f64)>,
}

impl Default for Ii {
    fn default() -> Self {
        Ii {
            cfg: IiCfg::default(),
            clusters: Vec::new(),
            key: (0, 0),
            w: vec![0.0; NBINS],
            wy: vec![0.0; NBINS],
            uv: Vec::new(),
            on_target_s: 0.0,
            pair_s: 0.0,
            rng: 0x51ab_cdef_0123_4567,
            star_key: usize::MAX,
            fit: None,
        }
    }
}

fn bessel_j1(x: f64) -> f64 {
    // Abramowitz & Stegun polynomial approximations.
    let ax = x.abs();
    if ax < 8.0 {
        let y = x * x;
        let n = x * (72362614232.0 + y * (-7895059235.0 + y * (242396853.1 + y * (-2972611.439 + y * (15704.48260 + y * (-30.16036606))))));
        let d = 144725228442.0 + y * (2300535178.0 + y * (18583304.74 + y * (99447.43394 + y * (376.9991397 + y))));
        n / d
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 2.356194491;
        let p = 1.0 + y * (0.183105e-2 + y * (-0.3516396496e-4 + y * (0.2457520174e-5 + y * (-0.240337019e-6))));
        let q = 0.04687499995 + y * (-0.2002690873e-3 + y * (0.8449199096e-5 + y * (-0.88228987e-6 + y * 0.105787412e-6)));
        let r = (0.636619772 / ax).sqrt() * (xx.cos() * p - z * xx.sin() * q);
        if x < 0.0 { -r } else { r }
    }
}

/// Uniform-disk squared visibility at baseline `b_m` for diameter `theta_mas`.
pub fn v2(theta_mas: f64, b_m: f64) -> f64 {
    let x = std::f64::consts::PI * rad(theta_mas / 3.6e6) * b_m / LAMBDA_M;
    if x < 1e-6 {
        return 1.0;
    }
    (2.0 * bessel_j1(x) / x).powi(2)
}

fn bin_of(b: f64) -> Option<usize> {
    if !(BMIN..BMAX).contains(&b) {
        return None;
    }
    Some((((b / BMIN).ln() / (BMAX / BMIN).ln()) * NBINS as f64) as usize)
}
fn bin_centre(k: usize) -> f64 {
    BMIN * (BMAX / BMIN).powf((k as f64 + 0.5) / NBINS as f64)
}

impl Ii {
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
    pub fn reset_data(&mut self) {
        self.w = vec![0.0; NBINS];
        self.wy = vec![0.0; NBINS];
        self.uv.clear();
        self.on_target_s = 0.0;
        self.pair_s = 0.0;
        self.fit = None;
    }
    /// Combined timing error, s.
    pub fn tau_s(&self) -> f64 {
        let c = &self.cfg;
        let pos_s = c.pos_cm / 100.0 / C;
        ((c.jitter_ps * 1e-12).powi(2) + 2.0 * (c.sync_ps * 1e-12).powi(2) + 2.0 * pos_s.powi(2)).sqrt()
    }
    /// SNR per √s for |V|² = 1, one pair, V magnitude `vmag`.
    pub fn k_rate(&self, area_m2: f64, ph0_cm2: f64, vmag: f64) -> f64 {
        // ph0 is photons s⁻¹ cm⁻² over 400–700 nm at V = 0.
        let band_hz = C / 400e-9 - C / 700e-9;
        let n = ph0_cm2 * 1e4 / band_hz * 10f64.powf(-0.4 * vmag);
        let df = 1.0 / (2.0 * self.tau_s());
        area_m2 * self.cfg.alpha * n * (self.cfg.channels as f64 * df / 2.0).sqrt()
    }
    /// Deputy ellipse parameters (ρ, φ, ζ, ψ) for member k (0 = chief).
    fn member(&self, k: usize) -> (f64, f64, f64, f64) {
        if k == 0 {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let n = (self.cfg.members.max(2) - 1) as f64;
        let f = k as f64 / n;
        let rho = self.cfg.size_m * (0.15 + 0.85 * f);
        let phi = std::f64::consts::TAU * (k as f64 * 0.382);
        (rho, phi, rho * 1.2, phi + std::f64::consts::FRAC_PI_2)
    }
    fn fit_theta(&self) -> Option<(f64, f64)> {
        let used = (0..NBINS).filter(|&k| self.w[k] > 0.0).count();
        if used < 3 {
            return None;
        }
        let chi = |t: f64| -> f64 {
            (0..NBINS)
                .filter(|&k| self.w[k] > 0.0)
                .map(|k| {
                    let y = self.wy[k] / self.w[k];
                    self.w[k] * (y - v2(t, bin_centre(k))).powi(2)
                })
                .sum()
        };
        let grid: Vec<f64> = (0..800).map(|i| 0.05 * (200.0f64).powf(i as f64 / 799.0)).collect();
        let (mut bt, mut bc) = (0.0, f64::MAX);
        for &t in &grid {
            let c = chi(t);
            if c < bc {
                bc = c;
                bt = t;
            }
        }
        let ok: Vec<f64> = grid.iter().copied().filter(|&t| chi(t) <= bc + 1.0).collect();
        let err = 0.5 * (ok.last().copied().unwrap_or(bt) - ok.first().copied().unwrap_or(bt));
        Some((bt, err.max(1e-4)))
    }
}

impl App {
    fn ii_setup(&mut self) {
        let n = self.shell.len();
        let key = (n, self.shell.params.planes);
        if self.ii.key == key && !self.ii.clusters.is_empty() {
            return;
        }
        self.ii.key = key;
        self.ii.clusters.clear();
        if n == 0 {
            return;
        }
        // Chief A: first satellite of plane 0; chief B: the satellite whose
        // plane's node is closest to 90° from plane 0's.
        let r0 = self.shell.raan_now[0];
        let mut b = 0;
        let mut best = f64::MAX;
        for i in 0..n {
            let d = ((self.shell.raan_now[i] - r0 - std::f64::consts::FRAC_PI_2 + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI).abs();
            if d < best - 1e-9 && self.shell.sats[i].slot == 0 {
                best = d;
                b = i;
            }
        }
        self.ii.clusters.push(Cluster { chief: 0, label: "Cluster A", ..Default::default() });
        if b != 0 {
            self.ii.clusters.push(Cluster { chief: b, label: "Cluster B", ..Default::default() });
        }
        if self.ii.rng == 0 {
            self.ii.rng = 0x51ab_cdef_0123_4567;
        }
        if self.ii.w.len() != NBINS {
            self.ii.reset_data();
        }
    }

    /// Advance the clusters and record |V|² on the selected star.
    pub fn ii_step(&mut self, dt: f64) {
        if !self.ii.cfg.on {
            return;
        }
        self.ii_setup();
        let (name, theta) = STARS[self.ii.cfg.star.min(STARS.len() - 1)];
        if self.ii.star_key != self.ii.cfg.star {
            self.ii.star_key = self.ii.cfg.star;
            self.ii.reset_data();
        }
        let Some(star) = self.names.iter().find(|s| s.name == name) else { return };
        let (sdir, vmag) = (star.dir, star.mag as f64);
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let o = self.obs_params.clone();
        let hw = crate::imaging::Hardware::from_obs(&o);
        let area = hw.area_cm2() / 1e4;
        let k = self.ii.k_rate(area, hw.ph0, vmag);
        let cos_lim = self.obs.cos_lim;
        // Sky basis at the star: east, north.
        let east = V3::Z.cross(sdir).unit();
        let north = sdir.cross(east);
        let safed = self.env.cfg.saa_policy == crate::environment::SaaPolicy::Pause;
        let members = self.ii.cfg.members.clamp(2, 6);
        let params: Vec<(f64, f64, f64, f64)> = (0..members).map(|m| self.ii.member(m)).collect();
        for c in 0..self.ii.clusters.len() {
            let i = self.ii.clusters[c].chief;
            if i >= self.shell.len() {
                continue;
            }
            let u = self.shell.u_now[i];
            let (pp, qq) = self.shell.plane_basis(i);
            let rhat = pp * u.cos() + qq * u.sin();
            let that = pp * (-u.sin()) + qq * u.cos();
            let hhat = pp.cross(qq);
            let rel: Vec<[f64; 3]> = params
                .iter()
                .map(|&(rho, phi, zeta, psi)| [-rho * (u + phi).cos(), 2.0 * rho * (u + phi).sin(), zeta * (u + psi).sin()])
                .collect();
            let pos: Vec<V3> = rel.iter().map(|r| rhat * r[0] + that * r[1] + hhat * r[2]).collect();
            // Can the cluster see the star?
            let why = if sdir.dot(self.shell.pos[i].unit()) <= cos_lim {
                "Earth in the way"
            } else if crate::obs::sky_blocked(&o, sdir, sun, moon).is_some() {
                "inside a Sun or Moon keep-out"
            } else if safed && self.env.in_saa.get(i).copied().unwrap_or(false) {
                "detectors safed in the South Atlantic Anomaly"
            } else {
                ""
            };
            let observing = why.is_empty();
            {
                let cl = &mut self.ii.clusters[c];
                cl.rel = rel;
                cl.observing = observing;
                cl.why = why.to_string();
            }
            if !observing || dt <= 0.0 {
                continue;
            }
            self.ii.on_target_s += dt;
            for a in 0..members {
                for bb in (a + 1)..members {
                    let d = pos[bb] - pos[a];
                    let proj = d - sdir * d.dot(sdir);
                    let b = proj.norm();
                    self.ii.pair_s += dt;
                    let (uu, vv) = (proj.dot(east), proj.dot(north));
                    if self.ii.uv.len() < 60_000 {
                        self.ii.uv.push((uu as f32, vv as f32, c as u8));
                    }
                    if let Some(kb) = bin_of(b) {
                        let sigma = 1.0 / (k * dt.sqrt());
                        let y = v2(theta, b) + sigma * self.ii.gauss();
                        let w = 1.0 / (sigma * sigma);
                        self.ii.w[kb] += w;
                        self.ii.wy[kb] += w * y;
                    }
                }
            }
        }
        self.ii.fit = self.ii.fit_theta();
    }

    pub fn ii_json(&self) -> serde_json::Value {
        let ii = &self.ii;
        let (name, theta) = STARS[ii.cfg.star.min(STARS.len() - 1)];
        let bins: Vec<serde_json::Value> = (0..NBINS)
            .filter(|&k| ii.w[k] > 0.0)
            .map(|k| serde_json::json!({"b_m": bin_centre(k), "v2": ii.wy[k] / ii.w[k], "err": 1.0 / ii.w[k].sqrt()}))
            .collect();
        serde_json::json!({
            "on": ii.cfg.on,
            "star": name,
            "theta_true_mas": theta,
            "fit": ii.fit.map(|(t, e)| serde_json::json!({"theta_mas": t, "err_mas": e})),
            "on_target_h": ii.on_target_s / 3600.0,
            "pair_h": ii.pair_s / 3600.0,
            "timing_ps": ii.tau_s() * 1e12,
            "bins": bins,
            "clusters": ii.clusters.iter().map(|c| serde_json::json!({"label": c.label, "chief": c.chief, "observing": c.observing, "why": c.why})).collect::<Vec<_>>(),
        })
    }

    pub fn ii_panel(&mut self, ui: &mut egui::Ui) {
        use egui::{Color32, RichText, Stroke};
        ui.label(RichText::new("Intensity interferometry").strong());
        let c = &mut self.ii.cfg;
        ui.checkbox(&mut c.on, "two satellite clusters, planes ~90° apart (assumed design)");
        if !c.on {
            return;
        }
        let star_names: Vec<String> = STARS.iter().map(|(n, t)| format!("{n} (~{t:.2} mas)")).collect();
        egui::ComboBox::from_id_source("ii_star").selected_text(&star_names[c.star]).show_ui(ui, |ui| {
            for (k, s) in star_names.iter().enumerate() {
                ui.selectable_value(&mut c.star, k, s);
            }
        });
        let before = (c.members, c.size_m as i64);
        ui.add(egui::Slider::new(&mut c.members, 2..=6).text("telescopes per cluster"));
        ui.add(egui::Slider::new(&mut c.size_m, 20.0..=2000.0).logarithmic(true).text("cluster size ρ m"));
        egui::CollapsingHeader::new(RichText::new("Detectors and timing (assumed)").small()).id_source("ii_det").show(ui, |ui| {
            ui.add(egui::Slider::new(&mut c.jitter_ps, 5.0..=500.0).logarithmic(true).text("detector jitter ps"));
            ui.add(egui::Slider::new(&mut c.sync_ps, 1.0..=200.0).logarithmic(true).text("clock sync ps"));
            ui.add(egui::Slider::new(&mut c.pos_cm, 0.1..=10.0).logarithmic(true).text("position knowledge cm"));
            ui.add(egui::Slider::new(&mut c.alpha, 0.05..=0.8).text("detection efficiency"));
            ui.add(egui::Slider::new(&mut c.channels, 1..=32).text("spectral channels"));
        });
        if before != (c.members, c.size_m as i64) {
            self.ii.reset_data();
        }
        if ui.small_button("Clear measurements").clicked() {
            self.ii.reset_data();
        }
        let ii = &self.ii;
        let (name, theta) = STARS[ii.cfg.star];
        for cl in &ii.clusters {
            ui.label(
                RichText::new(format!("{}: {}", cl.label, if cl.observing { "on target".to_string() } else { format!("waiting — {}", cl.why) }))
                    .small()
                    .color(if cl.observing { Color32::from_rgb(120, 220, 140) } else { Color32::GRAY }),
            );
        }
        let vmag = self.names.iter().find(|s| s.name == name).map(|s| s.mag as f64).unwrap_or(1.0);
        let hw = crate::imaging::Hardware::from_obs(&self.obs_params);
        let k = ii.k_rate(hw.area_cm2() / 1e4, hw.ph0, vmag);
        ui.label(
            RichText::new(format!(
                "{name}, V {vmag:.2}: one pair reaches SNR {:.2} per √s at |V|² = 1 ({:.0} ps timing) · on target {:.1} h, {:.0} pair-hours",
                k,
                ii.tau_s() * 1e12,
                ii.on_target_s / 3600.0,
                ii.pair_s / 3600.0
            ))
            .small(),
        );
        match ii.fit {
            Some((t, e)) => {
                ui.label(RichText::new(format!("Fitted diameter {t:.3} ± {e:.3} mas (published ~{theta:.2} mas)")).small().strong());
            }
            None => {
                ui.label(RichText::new("Not enough baselines measured yet for a fit.").small().weak());
            }
        }
        // Three small plots: relative motion, uv coverage, |V|² against baseline.
        let w = ui.available_width().min(420.0);
        let h = 150.0;
        ui.horizontal(|ui| {
            let (resp, p) = ui.allocate_painter(egui::vec2(w * 0.5 - 4.0, h), egui::Sense::hover());
            let r = resp.rect;
            p.rect_filled(r, 2.0, Color32::from_gray(18));
            p.text(r.left_top() + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, "relative motion (along-track ↔, radial ↕)", egui::FontId::proportional(10.0), Color32::GRAY);
            let s = (r.width() as f64 * 0.45) / (2.0 * ii.cfg.size_m * 1.05);
            let ctr = r.center();
            for (ci, cl) in ii.clusters.iter().enumerate().take(1) {
                for m in 1..ii.cfg.members {
                    let (rho, _, _, _) = ii.member(m);
                    let pts: Vec<egui::Pos2> = (0..48).map(|t| { let a = t as f32 / 48.0 * std::f32::consts::TAU; ctr + egui::vec2(2.0 * rho as f32 * a.sin() * s as f32, -(rho as f32) * a.cos() * s as f32) }).collect();
                    p.add(egui::Shape::closed_line(pts, Stroke::new(0.6_f32, Color32::from_gray(70))));
                }
                for (m, rel) in cl.rel.iter().enumerate() {
                    let q = ctr + egui::vec2((rel[1] * s) as f32, (-rel[0] * s) as f32);
                    p.circle_filled(q, if m == 0 { 3.5 } else { 2.5 }, if m == 0 { Color32::WHITE } else { Color32::from_rgb(120, 200, 255) });
                }
                let _ = ci;
            }
            let (resp, p) = ui.allocate_painter(egui::vec2(w * 0.5 - 4.0, h), egui::Sense::hover());
            let r = resp.rect;
            p.rect_filled(r, 2.0, Color32::from_gray(18));
            p.text(r.left_top() + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, "uv coverage (projected baselines)", egui::FontId::proportional(10.0), Color32::GRAY);
            // Scale to the 98th percentile so a few long baselines don't
            // shrink everything else.
            let mut mags: Vec<f32> = ii.uv.iter().map(|x| x.0.hypot(x.1)).collect();
            mags.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let bmax = mags.get(mags.len() * 98 / 100).copied().unwrap_or(1.0).max(1.0);
            let s = r.height() * 0.44 / bmax;
            let ctr = r.center();
            for (u, v, c) in ii.uv.iter().step_by((ii.uv.len() / 8000).max(1)) {
                let col = if *c == 0 { Color32::from_rgba_unmultiplied(120, 200, 255, 170) } else { Color32::from_rgba_unmultiplied(255, 170, 90, 170) };
                for q in [ctr + egui::vec2(u * s, -v * s), ctr + egui::vec2(-u * s, v * s)] {
                    if r.contains(q) {
                        p.circle_filled(q, 1.3, col);
                    }
                }
            }
            p.text(r.left_bottom() + egui::vec2(4.0, -2.0), egui::Align2::LEFT_BOTTOM, "blue: cluster A · orange: B", egui::FontId::proportional(10.0), Color32::GRAY);
            p.text(r.right_bottom() - egui::vec2(4.0, 2.0), egui::Align2::RIGHT_BOTTOM, format!("±{:.0} m", bmax), egui::FontId::proportional(10.0), Color32::GRAY);
        });
        let (resp, p) = ui.allocate_painter(egui::vec2(w, h), egui::Sense::hover());
        let r = resp.rect;
        p.rect_filled(r, 2.0, Color32::from_gray(18));
        p.text(r.left_top() + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, "|V|² against baseline (log, 5 m – 3 km): measured, published model, fit", egui::FontId::proportional(10.0), Color32::GRAY);
        let xb = |b: f64| r.left() + ((b / BMIN).ln() / (BMAX / BMIN).ln()) as f32 * r.width();
        let yv = |v: f64| r.bottom() - 6.0 - (v.clamp(-0.1, 1.1) as f32 + 0.1) / 1.2 * (r.height() - 20.0);
        let curve = |t: f64, col: Color32| {
            let pts: Vec<egui::Pos2> = (0..200).map(|i| { let b = BMIN * (BMAX / BMIN).powf(i as f64 / 199.0); egui::pos2(xb(b), yv(v2(t, b))) }).collect();
            egui::Shape::line(pts, Stroke::new(1.2_f32, col))
        };
        p.add(curve(theta, Color32::from_gray(110)));
        if let Some((t, _)) = ii.fit {
            p.add(curve(t, Color32::from_rgb(120, 220, 140)));
        }
        for kb in 0..NBINS {
            if ii.w[kb] > 0.0 {
                let (b, y, e) = (bin_centre(kb), ii.wy[kb] / ii.w[kb], 1.0 / ii.w[kb].sqrt());
                let x = xb(b);
                p.line_segment([egui::pos2(x, yv(y - e)), egui::pos2(x, yv(y + e))], Stroke::new(1.0_f32, Color32::from_rgb(120, 200, 255)));
                p.circle_filled(egui::pos2(x, yv(y)), 2.0, Color32::from_rgb(120, 200, 255));
            }
        }
    }
}

// ------------------------------------------------------------ propellant

/// Propulsion presets: name, specific impulse s. ASSUMED typical values.
pub const PROPULSION: [(&str, f64); 3] = [("Hall-effect thruster (krypton)", 1400.0), ("Green monopropellant", 230.0), ("Cold gas", 65.0)];

#[derive(Clone, Serialize)]
pub struct FuelCfg {
    pub propulsion: usize,
    pub life_yr: f64,
    /// Δv per collision-avoidance manoeuvre, m/s (ASSUMED).
    pub cam_dv: f64,
    /// Margin on the total, fraction.
    pub margin: f64,
}

impl Default for FuelCfg {
    fn default() -> Self {
        FuelCfg { propulsion: 0, life_yr: 5.0, cam_dv: 0.05, margin: 0.25 }
    }
}

#[derive(Clone, Serialize)]
pub struct FuelBudget {
    pub drag_dv_yr: f64,
    pub cam_per_yr: f64,
    pub cam_dv_yr: f64,
    pub disposal_dv: f64,
    pub cluster_dv_yr: f64,
    pub total_dv: f64,
    pub total_dv_cluster: f64,
    pub isp_s: f64,
    pub propellant_kg: f64,
    pub propellant_kg_cluster: f64,
    pub wet_kg: f64,
}

impl App {
    pub fn fuel_budget(&self) -> FuelBudget {
        let c = &self.fuel;
        let alt = self.shell.params.alt_km;
        let a = (crate::astro::EARTH_RADIUS_KM + alt) * 1e3;
        let v = (crate::astro::MU_EARTH * 1e9 / a).sqrt();
        // Drag make-up averaged over the solar cycle (mean of min, mean, max).
        let rho = crate::environment::density_mean(alt);
        let k = crate::environment::solar_ratio(alt);
        let rho_avg = rho * (1.0 / k + 1.0 + k) / 3.0;
        let bc = self.env.cfg.ballistic();
        let drag_dv_yr = 0.5 * rho_avg * v * v * bc * 365.25 * 86_400.0;
        let cam_per_yr = self.traffic.manoeuvres_per_sat_yr().unwrap_or(2.3);
        // Measured by the traffic model when available, else rate × assumed size.
        let cam_dv_yr = self.traffic.cam_dv_per_sat_yr().unwrap_or(cam_per_yr * c.cam_dv);
        let disposal_dv = crate::environment::deorbit_dv(alt, self.env.cfg.disposal_perigee_km);
        // Cluster keeping: cancel a 5 % differential drag, plus 1 m/s/yr of
        // reconfiguration and trims (ASSUMED).
        let cluster_dv_yr = 0.05 * drag_dv_yr + 1.0;
        let base = (drag_dv_yr + cam_dv_yr) * c.life_yr + disposal_dv;
        let total_dv = base * (1.0 + c.margin);
        let total_dv_cluster = (base + cluster_dv_yr * c.life_yr) * (1.0 + c.margin);
        let isp = PROPULSION[c.propulsion.min(PROPULSION.len() - 1)].1;
        let wet = self.env.cfg.mass_kg;
        let mp = |dv: f64| wet * (1.0 - (-dv / (9.80665 * isp)).exp());
        FuelBudget {
            drag_dv_yr,
            cam_per_yr,
            cam_dv_yr,
            disposal_dv,
            cluster_dv_yr,
            total_dv,
            total_dv_cluster,
            isp_s: isp,
            propellant_kg: mp(total_dv),
            propellant_kg_cluster: mp(total_dv_cluster),
            wet_kg: wet,
        }
    }

    pub fn fuel_panel(&mut self, ui: &mut egui::Ui) {
        use egui::RichText;
        ui.label(RichText::new("Propulsion and propellant").strong());
        let c = &mut self.fuel;
        egui::ComboBox::from_id_source("prop").selected_text(PROPULSION[c.propulsion].0).show_ui(ui, |ui| {
            for (k, p) in PROPULSION.iter().enumerate() {
                ui.selectable_value(&mut c.propulsion, k, format!("{} (Isp {:.0} s)", p.0, p.1));
            }
        });
        ui.add(egui::Slider::new(&mut c.life_yr, 1.0..=15.0).text("design life yr"));
        ui.add(egui::Slider::new(&mut c.cam_dv, 0.005..=0.5).logarithmic(true).text("Δv per avoidance manoeuvre m/s"));
        let b = self.fuel_budget();
        egui::Grid::new("fuel").striped(true).show(ui, |ui| {
            let row = |ui: &mut egui::Ui, a: &str, v: String| {
                ui.label(RichText::new(a).small());
                ui.label(RichText::new(v).small());
                ui.end_row();
            };
            row(ui, "drag make-up (solar-cycle average)", format!("{:.1} m/s per year", b.drag_dv_yr));
            row(ui, "collision avoidance", format!("{:.2} m/s per year ({:.1} manoeuvres)", b.cam_dv_yr, b.cam_per_yr));
            row(ui, "end-of-life disposal", format!("{:.0} m/s once", b.disposal_dv));
            row(ui, "interferometry cluster keeping (deputies)", format!("{:.1} m/s per year", b.cluster_dv_yr));
            row(ui, "total, shell satellite (with margin)", format!("{:.0} m/s, {:.1} kg propellant", b.total_dv, b.propellant_kg));
            row(ui, "total, cluster deputy (with margin)", format!("{:.0} m/s, {:.1} kg propellant", b.total_dv_cluster, b.propellant_kg_cluster));
        });
        ui.label(RichText::new(format!("{:.0} kg satellite, Isp {:.0} s, {:.0} % margin. Δv figures are modelled; Isp, manoeuvre size and margin are assumed.", b.wet_kg, b.isp_s, 100.0 * self.fuel.margin)).small().weak());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_and_snr() {
        // First null of a uniform disk at 1.22 λ/θ.
        let theta = 0.9;
        let b0 = 1.2197 * LAMBDA_M / rad(theta / 3.6e6);
        assert!(v2(theta, b0) < 1e-4 && (v2(theta, 1.0) - 1.0).abs() < 1e-3);
        // Hanbury Brown scale: a V = 0 star with ~0.03 m² and ~60 ps timing
        // gives roughly 0.1 per √s per pair.
        let ii = Ii::default();
        let k = ii.k_rate(0.027, 3.0e6, 0.0);
        eprintln!("SNR per sqrt(s) at V=0: {k:.3}, timing {:.0} ps, null at {b0:.0} m", ii.tau_s() * 1e12);
        assert!(k > 0.03 && k < 0.5);
    }
}

#[cfg(test)]
mod default_tests {
    #[test]
    fn json_and_fit_work_when_off() {
        let ii = super::Ii::default();
        assert_eq!(ii.w.len(), super::NBINS);
        assert!(ii.fit_theta().is_none());
    }
}
