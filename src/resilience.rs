//! Resilience demo (satellite failures and replacement) and the periodic
//! space-vs-ground comparison, as `App` methods.
//!
//! Failed satellites go dark: they take no work, their laser links drop and
//! they show as failed on the consoles. Failures can be injected (a share of
//! the fleet, or a whole plane) or arrive at a random rate; with a
//! replacement delay set, failed satellites are restored after that many days
//! (spares launched). An hourly record of fleet health and science output
//! shows how gracefully the system degrades.

use crate::app::App;
use egui::RichText;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Default)]
pub struct Resilience {
    pub failed: Vec<bool>,
    pub fail_jd: Vec<f64>,
    /// Random failures, % of the fleet per year.
    pub rate_pct_yr: f64,
    /// Replacement delay, days (0 = no replacement).
    pub replace_days: f64,
    rng: u64,
    /// Hourly record: [jd, alive fraction, output vs baseline, completions vs baseline].
    pub series: Vec<[f64; 4]>,
    last: Option<(f64, f64, f64)>, // jd, integrating_s, completed
    base_out: Option<(f64, f64)>,  // integrating per sat-hour, completions per sat-hour (healthy)
    base_n: f64,
    pub events: Vec<(&'static str, String)>,
    pub total_failed: u64,
    pub total_replaced: u64,
    /// Laser links down because an end satellite has failed.
    pub links_down: usize,
    /// Scripted failure: (sim hours after start, fraction), from --fail-at.
    pub scripted: Option<(f64, f64)>,
    start_jd: f64,
    // space-vs-ground comparison (runs in the background)
    pub ground: Option<crate::ground::Comparison>,
    ground_job: Option<Arc<Mutex<Option<crate::ground::Comparison>>>>,
    ground_last: Option<Instant>,
}

#[derive(Serialize)]
pub struct ResilienceJson<'a> {
    pub failed: usize,
    pub alive_frac: f64,
    pub rate_pct_yr: f64,
    pub replace_days: f64,
    pub total_failed: u64,
    pub total_replaced: u64,
    pub series: &'a [[f64; 4]],
}

impl Resilience {
    fn rand(&mut self) -> f64 {
        if self.rng == 0 {
            self.rng = 0xD1B5_4A32_D192_ED03;
        }
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn n_failed(&self) -> usize {
        self.failed.iter().filter(|&&f| f).count()
    }
}

impl App {
    /// Called each sub-step before scheduling.
    pub fn apply_failures(&mut self, dt: f64) {
        if self.resil.start_jd == 0.0 {
            self.resil.start_jd = self.jd;
        }
        if let Some((h, frac)) = self.resil.scripted {
            if (self.jd - self.resil.start_jd) * 24.0 >= h && self.resil.failed.len() == self.shell.len() {
                self.resil.scripted = None;
                self.fail_now(frac, "scripted failure");
            }
        }
        let n = self.shell.len();
        let r = &mut self.resil;
        if r.failed.len() != n {
            r.failed = vec![false; n];
            r.fail_jd = vec![0.0; n];
            r.series.clear();
            r.last = None;
            r.base_out = None;
        }
        // Random failures (Poisson in this step).
        if r.rate_pct_yr > 0.0 && dt > 0.0 {
            let alive = n - r.n_failed();
            let lam = alive as f64 * r.rate_pct_yr / 100.0 * dt / (365.25 * 86_400.0);
            let mut k = 0;
            let mut p = (-lam).exp();
            let mut acc = p;
            let u = r.rand();
            while u > acc && k < alive {
                k += 1;
                p *= lam / k as f64;
                acc += p;
            }
            for _ in 0..k {
                let i = (r.rand() * n as f64) as usize % n;
                if !r.failed[i] {
                    r.failed[i] = true;
                    r.fail_jd[i] = self.jd;
                    r.total_failed += 1;
                    r.events.push(("warn", format!("FACET-{:05} failed (random failure)", i + 1)));
                }
            }
        }
        // Replacement of failed satellites after the delay.
        if r.replace_days > 0.0 {
            let mut back = 0;
            for i in 0..n {
                if r.failed[i] && self.jd - r.fail_jd[i] >= r.replace_days {
                    r.failed[i] = false;
                    back += 1;
                    if let Some(h) = self.obs.hold.get_mut(i) {
                        *h = false;
                    }
                }
            }
            if back > 0 {
                r.total_replaced += back as u64;
                r.events.push(("good", format!("{back} replacement satellite{} in service", if back > 1 { "s" } else { "" })));
            }
        }
        if self.obs.hold.len() == n {
            for i in 0..n {
                if r.failed[i] {
                    self.obs.hold[i] = true;
                }
            }
        }
    }

    /// Called after the scheduler: failed satellites' laser links are down.
    pub fn failed_links(&mut self) {
        let f = &self.resil.failed;
        let mut down = 0;
        if !f.is_empty() {
            for (k, &(a, b)) in self.shell.links.iter().enumerate() {
                if f.get(a as usize).copied().unwrap_or(false) || f.get(b as usize).copied().unwrap_or(false) {
                    down += 1;
                    // drawn as down, but not counted as atmospheric
                    if let Some(x) = self.obs.link_blocked.get_mut(k) {
                        *x = true;
                    }
                }
            }
        }
        self.resil.links_down = down;
    }

    pub fn fail_now(&mut self, frac: f64, why: &str) {
        let n = self.shell.len();
        if self.resil.failed.len() != n {
            return;
        }
        let want = (frac * n as f64).round() as usize;
        let mut done = 0;
        let mut guard = 0;
        while done < want && guard < 50 * n {
            guard += 1;
            let i = (self.resil.rand() * n as f64) as usize % n;
            if !self.resil.failed[i] {
                self.resil.failed[i] = true;
                self.resil.fail_jd[i] = self.jd;
                done += 1;
            }
        }
        self.resil.total_failed += done as u64;
        self.resil.events.push(("bad", format!("{done} satellites failed ({why})")));
    }

    pub fn fail_plane(&mut self, plane: u32) {
        let n = self.shell.len();
        if self.resil.failed.len() != n {
            return;
        }
        let mut done = 0;
        for i in 0..n {
            if self.shell.sats[i].plane == plane && !self.resil.failed[i] {
                self.resil.failed[i] = true;
                self.resil.fail_jd[i] = self.jd;
                done += 1;
            }
        }
        self.resil.total_failed += done as u64;
        self.resil.events.push(("bad", format!("Orbital plane {} lost: {done} satellites failed", plane + 1)));
    }

    pub fn restore_all(&mut self) {
        let mut back = 0;
        for i in 0..self.resil.failed.len() {
            if self.resil.failed[i] {
                self.resil.failed[i] = false;
                back += 1;
                if let Some(h) = self.obs.hold.get_mut(i) {
                    *h = false;
                }
            }
        }
        if back > 0 {
            self.resil.events.push(("good", format!("All {back} failed satellites restored")));
        }
    }

    /// Hourly record of fleet health vs science output (after the science ledger).
    pub fn resilience_record(&mut self) {
        let n = self.shell.len() as f64;
        let s = &self.science;
        let done: f64 = s.cases.iter().map(|c| c.completed as f64).sum();
        let r = &mut self.resil;
        let Some((jd0, i0, c0)) = r.last else {
            r.last = Some((self.jd, s.integrating_s, done));
            return;
        };
        let hours = (self.jd - jd0) * 24.0;
        if hours < 1.0 {
            return;
        }
        let alive = 1.0 - r.n_failed() as f64 / n.max(1.0);
        let per_sat_h = (s.integrating_s - i0) / 3600.0 / (n * hours);
        let comp = (done - c0) / (n * hours);
        if alive >= 0.999 && hours < 3.0 {
            // healthy baseline: running mean
            let (bo, bc) = r.base_out.unwrap_or((per_sat_h, comp));
            let w = (r.base_n + 1.0).min(24.0);
            r.base_out = Some((bo + (per_sat_h - bo) / w, bc + (comp - bc) / w));
            r.base_n += 1.0;
        }
        let (bo, bc) = r.base_out.unwrap_or((per_sat_h.max(1e-9), comp.max(1e-9)));
        r.series.push([self.jd, alive, per_sat_h / bo.max(1e-9), comp / bc.max(1e-9)]);
        if r.series.len() > 480 {
            r.series.remove(0);
        }
        r.last = Some((self.jd, s.integrating_s, done));
    }

    pub fn resilience_json(&self) -> serde_json::Value {
        let r = &self.resil;
        serde_json::to_value(ResilienceJson {
            failed: r.n_failed(),
            alive_frac: 1.0 - r.n_failed() as f64 / self.shell.len().max(1) as f64,
            rate_pct_yr: r.rate_pct_yr,
            replace_days: r.replace_days,
            total_failed: r.total_failed,
            total_replaced: r.total_replaced,
            series: &r.series,
        })
        .unwrap_or_default()
    }

    /// Start / collect the background space-vs-ground comparison (every ~20 s).
    pub fn ground_tick(&mut self) {
        if let Some(job) = &self.resil.ground_job {
            let done = job.lock().unwrap().take();
            if let Some(c) = done {
                self.resil.ground = Some(c);
                self.resil.ground_job = None;
            } else {
                return;
            }
        }
        if self.resil.ground_last.map_or(false, |t| t.elapsed().as_secs_f64() < 20.0) {
            return;
        }
        self.resil.ground_last = Some(Instant::now());
        // Targets: the open request queue (sample), else a uniform sky sample.
        let mut targets: Vec<crate::math::V3> = match &self.web {
            Some(w) => w.lock().unwrap().requests.iter().filter(|r| r.state.is_open()).map(|r| r.dir).take(300).collect(),
            None => Vec::new(),
        };
        if targets.len() < 30 {
            targets = (0..200)
                .map(|k| {
                    let z = -1.0 + 2.0 * (k as f64 + 0.5) / 200.0;
                    let a = k as f64 * 2.399_963;
                    crate::math::V3::new((1.0 - z * z).sqrt() * a.cos(), (1.0 - z * z).sqrt() * a.sin(), z)
                })
                .collect();
        }
        let s = &self.science;
        // Duty cycle of working telescopes (failed and held satellites excluded).
        let duty = s.duty().unwrap_or(0.9);
        let mut resp: Vec<f32> = s.cases.iter().flat_map(|c| c.resp.iter().copied()).collect();
        resp.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = resp.get(resp.len() / 2).map(|&x| x as f64);
        let (jd, n, sky, excl) = (self.jd, self.shell.len(), self.cov_frac, self.obs_params.sun_excl_deg);
        let hw = crate::imaging::Hardware::from_obs(&self.obs_params);
        let slot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        std::thread::spawn(move || {
            let c = crate::ground::compare(jd, &targets, n, duty, sky, excl, med, &hw);
            *out.lock().unwrap() = Some(c);
        });
        self.resil.ground_job = Some(slot);
    }

    pub fn resilience_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Resilience (failures & replacement)").strong());
        let n = self.shell.len();
        let failed = self.resil.n_failed();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Fail 10 %").clicked() {
                self.fail_now(0.10, "injected");
            }
            if ui.button("Fail 30 %").clicked() {
                self.fail_now(0.30, "injected");
            }
            if ui.button("Lose a plane").clicked() {
                let p = (self.resil.rand() * self.shell.params.effective_planes() as f64) as u32;
                self.fail_plane(p);
            }
            if ui.button("Restore all").clicked() {
                self.restore_all();
            }
        });
        ui.add(egui::Slider::new(&mut self.resil.rate_pct_yr, 0.0..=50.0).text("random failures %/yr"));
        ui.add(egui::Slider::new(&mut self.resil.replace_days, 0.0..=180.0).text("replacement after days (0 = none)"));
        let last = self.resil.series.last();
        ui.label(
            RichText::new(format!(
                "{} of {} failed ({:.1} %) · last hour: output {} of healthy, observations {}",
                failed,
                n,
                100.0 * failed as f64 / n.max(1) as f64,
                last.map(|s| format!("{:.0} %", 100.0 * s[2])).unwrap_or("–".into()),
                last.map(|s| format!("{:.0} %", 100.0 * s[3])).unwrap_or("–".into()),
            ))
            .small(),
        );
        if let Some(g) = &self.resil.ground {
            ui.label(
                RichText::new(format!(
                    "vs ground network: duty {:.0} % vs {:.0} % · ≈{:.0} ground telescopes per FACET telescope at equal depth",
                    100.0 * g.facet_duty,
                    100.0 * g.ground_duty,
                    g.equivalent_ground_telescopes
                ))
                .small(),
            );
        }
    }

    pub fn resilience_events_to_log(&mut self) {
        if self.resil.events.is_empty() {
            return;
        }
        if let Some(w) = &self.web {
            let mut sh = w.lock().unwrap();
            for (l, m) in self.resil.events.drain(..) {
                sh.log(l, m);
            }
        } else {
            self.resil.events.clear();
        }
    }
}

pub fn failed_code(app: &App, i: usize) -> bool {
    app.resil.failed.get(i).copied().unwrap_or(false)
}
