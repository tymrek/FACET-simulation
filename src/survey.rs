//! All-sky survey: the fleet tiles the whole observable sky and the sim times
//! each complete pass.
//!
//! Unlike the throughput estimate in `allsky.rs`, nothing here is assumed
//! about overheads: every tile is a real pointing that a telescope must slew
//! to, settle on and integrate through the attitude model, and only locked,
//! dark-sky-equivalent integration counts (so slews, the South Atlantic
//! Anomaly, Earthshine and sunward stray light all cost what they cost).
//!
//! * Tiling: rings of constant declination, spacing = field side ÷ √1.1
//!   (≈ 10 % overlap), each ring holding ⌈360° cos δ ÷ spacing⌉ tiles.
//! * A pass is complete when every tile is either done or inside the Sun or
//!   Moon keep-out at that moment. Then all tiles reset and the next begins.
//! * A free telescope takes the nearest undone, unclaimed tile it can see
//!   (above its limb margin, outside the keep-outs), searched outward from
//!   its current pointing so slews stay short.
//! * A tile whose exposure is interrupted (the telescope is taken for other
//!   work or loses sight of it) loses its partial integration.

use crate::app::App;
use crate::math::{deg, rad, V3};
use crate::obs::{self, job, job_tag, job_val, TAG_ALLSKY, TAG_SURVEY};
use serde::Serialize;

const NONE: u32 = u32::MAX;
/// Tiles are taken only within this zenith distance (deg), and weighted toward the zenith.
const MAX_ZA_DEG: f64 = 75.0;
const ZA_WEIGHT: f64 = 0.35;
/// Furthest a free telescope will slew for a tile, deg.
const MAX_SLEW_DEG: f64 = 32.0;
/// A free telescope this much closer may take over a tile whose claimant has
/// not started integrating, deg.
const STEAL_MARGIN_DEG: f64 = 8.0;

#[derive(Default)]
pub struct Tiling {
    pub dirs: Vec<V3>,
    /// First tile index of each declination ring, plus a final sentinel.
    ring_start: Vec<u32>,
    ring_dec: Vec<f64>,
    step_deg: f64,
    fov_key: i64,
}

impl Tiling {
    pub fn new(fov_deg: f64) -> Tiling {
        let step = fov_deg / 1.1f64.sqrt();
        let nr = (180.0 / step).ceil() as usize;
        let dstep = 180.0 / nr as f64;
        let (mut dirs, mut ring_start, mut ring_dec) = (Vec::new(), Vec::new(), Vec::new());
        for r in 0..nr {
            let dec = -90.0 + (r as f64 + 0.5) * dstep;
            let m = ((360.0 * rad(dec).cos()) / step).ceil().max(1.0) as usize;
            ring_start.push(dirs.len() as u32);
            ring_dec.push(dec);
            let off = if r % 2 == 0 { 0.0 } else { 0.5 };
            for k in 0..m {
                let ra = (k as f64 + off) / m as f64 * 360.0;
                dirs.push(V3::from_radec(rad(ra), rad(dec)));
            }
        }
        ring_start.push(dirs.len() as u32);
        Tiling { dirs, ring_start, ring_dec, step_deg: dstep, fov_key: (fov_deg * 1000.0) as i64 }
    }
    fn ring_of(&self, dec_deg: f64) -> usize {
        (((dec_deg + 90.0) / self.step_deg) as isize).clamp(0, self.ring_dec.len() as isize - 1) as usize
    }
    /// Tiles of ring `r` within ±`half_deg` of right ascension `ra_deg`.
    fn ring_window(&self, r: usize, ra_deg: f64, half_deg: f64) -> impl Iterator<Item = usize> + '_ {
        let (a, b) = (self.ring_start[r] as usize, self.ring_start[r + 1] as usize);
        let m = b - a;
        let cosd = rad(self.ring_dec[r]).cos().max(1e-3);
        let span = (half_deg / cosd).min(180.0);
        let per = 360.0 / m as f64;
        let k0 = ((ra_deg - span) / per).floor() as isize;
        let k1 = ((ra_deg + span) / per).ceil() as isize;
        let cnt = ((k1 - k0 + 1) as usize).min(m);
        (0..cnt).map(move |j| a + (k0 + j as isize).rem_euclid(m as isize) as usize)
    }
}

#[derive(Clone, Serialize)]
pub struct PassRecord {
    pub pass: u32,
    pub seconds: f64,
    pub tiles: usize,
    pub eyes_mean: f64,
    /// Time to finish 95 % of the observable tiles, s.
    pub t95: f64,
}

#[derive(Default)]
pub struct Survey {
    pub tiling: Tiling,
    pub done: Vec<bool>,
    pub progress: Vec<f32>,
    pub claimed_by: Vec<u32>,
    /// Tile each satellite is working on (NONE = free).
    pub tile_of: Vec<u32>,
    pub pass: u32,
    pub pass_start_jd: f64,
    pub n_done: usize,
    pub n_blocked: usize,
    pub blocked_check_jd: f64,
    pub eyes_now: usize,
    pub eye_s: f64,
    pub history: Vec<PassRecord>,
    /// Per-tile cost accounting for the pass: wall-clock seconds telescopes
    /// spent on tiles (slew + settle + integrate) and tiles completed.
    pub busy_s: f64,
    pub t95: Option<f64>,
    pub tiles_completed: usize,
    /// Also tile with free telescopes in Science operations (instead of the
    /// zenith stare).
    pub in_queue: bool,
    pub show: bool,
}

impl Survey {
    fn ensure(&mut self, fov: f64, n_sat: usize, jd: f64) {
        if self.tiling.dirs.is_empty() || self.tiling.fov_key != (fov * 1000.0) as i64 {
            self.tiling = Tiling::new(fov);
            let nt = self.tiling.dirs.len();
            self.done = vec![false; nt];
            self.progress = vec![0.0; nt];
            self.claimed_by = vec![NONE; nt];
            self.tile_of.clear();
            self.restart(jd);
            self.history.clear();
            self.pass = 0;
        }
        if self.tile_of.len() != n_sat {
            self.tile_of = vec![NONE; n_sat];
            self.claimed_by.iter_mut().for_each(|c| *c = NONE);
        }
    }
    pub fn restart(&mut self, jd: f64) {
        self.done.iter_mut().for_each(|d| *d = false);
        self.progress.iter_mut().for_each(|p| *p = 0.0);
        self.n_done = 0;
        self.pass_start_jd = jd;
        self.eye_s = 0.0;
        self.blocked_check_jd = 0.0;
        self.busy_s = 0.0;
        self.tiles_completed = 0;
        self.t95 = None;
    }
    pub fn reset(&mut self, jd: f64) {
        self.restart(jd);
        self.history.clear();
        self.pass = 0;
        self.claimed_by.iter_mut().for_each(|c| *c = NONE);
        self.tile_of.iter_mut().for_each(|t| *t = NONE);
    }
    fn release(&mut self, i: usize) {
        let t = self.tile_of[i];
        if t != NONE {
            if self.claimed_by[t as usize] == i as u32 {
                self.claimed_by[t as usize] = NONE;
                self.progress[t as usize] = 0.0;
            }
            self.tile_of[i] = NONE;
        }
    }
    pub fn tiles(&self) -> usize {
        self.tiling.dirs.len()
    }
}

impl App {
    pub fn survey_active(&self) -> bool {
        self.mode == obs::Mode::AllSky || (self.survey.in_queue && self.mode == obs::Mode::Queue)
    }

    /// Hand free (zenith-surveying) telescopes all-sky tiles. Runs after STARE
    /// and queue assignment, so it only ever takes otherwise idle eyes.
    pub fn survey_assign(&mut self) {
        let n = self.obs.job.len();
        let jd = self.jd;
        let fov = self.obs_params.fov_deg;
        if !self.survey_active() {
            // Leaving the mode: let every claim go.
            if self.survey.tile_of.iter().any(|&t| t != NONE) {
                for i in 0..self.survey.tile_of.len() {
                    self.survey.release(i);
                }
            }
            self.survey.eyes_now = 0;
            return;
        }
        self.survey.ensure(fov, n, jd);
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let o = self.obs_params.clone();
        let ok = |d: V3| obs::sky_blocked(&o, d, sun, moon).is_none();
        let cos_lim = self.obs.cos_lim;
        let s = &mut self.survey;
        let mut eyes = 0;
        // Telescopes whose detectors are safed in the South Atlantic Anomaly
        // can't integrate: they give their tiles back rather than sit on them.
        let safed = self.env.cfg.saa_policy == crate::environment::SaaPolicy::Pause;
        for i in 0..n {
            let in_saa = safed && self.env.in_saa.get(i).copied().unwrap_or(false);
            let free = job_tag(self.obs.job[i]) == TAG_SURVEY && !self.obs.hold[i] && !in_saa;
            if !free {
                s.release(i);
                continue;
            }
            // Keep the current tile while it stays visible and undone.
            let t = s.tile_of[i];
            if t != NONE {
                let d = s.tiling.dirs[t as usize];
                if !s.done[t as usize] && d.dot(self.obs.rhat[i]) > cos_lim && ok(d) {
                    self.obs.pointing[i] = Some(d);
                    self.obs.job[i] = job(TAG_ALLSKY, t as u64);
                    self.obs.fine[i] = crate::imaging::guide_star_available(d, fov, o.guide_mag);
                    eyes += 1;
                    continue;
                }
                s.release(i);
            }
            // Nearest undone, unclaimed, visible tile to where we point now.
            let here = self.obs.att[i].dir;
            let (ra, dec) = here.to_radec();
            let (ra, dec) = (deg(ra).rem_euclid(360.0), deg(dec));
            let rhat = self.obs.rhat[i];
            let cos_za = rad(MAX_ZA_DEG).cos();
            let mut best: Option<(f64, usize)> = None;
            let mut radius = 4.0 * s.tiling.step_deg;
            while best.is_none() && radius <= MAX_SLEW_DEG {
                let r0 = s.tiling.ring_of(dec - radius);
                let r1 = s.tiling.ring_of(dec + radius);
                for r in r0..=r1 {
                    for k in s.tiling.ring_window(r, ra, radius) {
                        if s.done[k] {
                            continue;
                        }
                        let owner = s.claimed_by[k];
                        if owner != NONE {
                            // Steal only from a claimant still slewing toward it
                            // from well further away.
                            let oj = owner as usize;
                            if s.progress[k] > 0.0 || oj >= n {
                                continue;
                            }
                            let d = s.tiling.dirs[k];
                            if self.obs.att[oj].dir.angle(d) < here.angle(d) + rad(STEAL_MARGIN_DEG) {
                                continue;
                            }
                        }
                        let d = s.tiling.dirs[k];
                        let cz = d.dot(rhat);
                        if cz <= cos_lim.max(cos_za) {
                            continue;
                        }
                        // Cost: slew angle plus a zenith-distance penalty, so
                        // telescopes work high in their sky (less Earthshine,
                        // longer visibility) instead of drifting to the limb.
                        let c = -(here.angle(d) + ZA_WEIGHT * cz.clamp(-1.0, 1.0).acos());
                        if best.map_or(true, |b| c > b.0) && ok(d) {
                            best = Some((c, k));
                        }
                    }
                }
                radius *= 2.0;
            }
            if let Some((_, k)) = best {
                let prev = s.claimed_by[k];
                if prev != NONE {
                    // Hand the old claimant back to the free pool (zenith).
                    let j = prev as usize;
                    s.tile_of[j] = NONE;
                    let z = self.obs.rhat[j];
                    if j < i {
                        self.obs.pointing[j] = Some(z);
                        self.obs.job[j] = job(TAG_SURVEY, 0);
                        eyes -= 1;
                    }
                }
                s.claimed_by[k] = i as u32;
                s.tile_of[i] = k as u32;
                let d = s.tiling.dirs[k];
                self.obs.pointing[i] = Some(d);
                self.obs.job[i] = job(TAG_ALLSKY, k as u64);
                self.obs.fine[i] = crate::imaging::guide_star_available(d, fov, o.guide_mag);
                eyes += 1;
            }
        }
        s.eyes_now = eyes;
    }

    /// Credit locked integration to tiles; close the pass when the whole
    /// observable sky is done.
    pub fn survey_credit(&mut self, dt: f64) {
        if !self.survey_active() || self.survey.tiles() == 0 || dt <= 0.0 {
            return;
        }
        let exp = self.obs_params.exp_s.max(1.0) as f32;
        let s = &mut self.survey;
        for i in 0..self.obs.job.len().min(s.tile_of.len()) {
            if job_tag(self.obs.job[i]) != TAG_ALLSKY {
                continue;
            }
            s.busy_s += dt;
            let k = job_val(self.obs.job[i]) as usize;
            s.progress[k] += self.obs.obs_dt[i] as f32;
            if s.progress[k] >= exp && !s.done[k] {
                s.done[k] = true;
                s.n_done += 1;
                s.tiles_completed += 1;
                s.claimed_by[k] = NONE;
                s.tile_of[i] = NONE;
            }
        }
        s.eye_s += s.eyes_now as f64 * dt;
        // Every simulated minute, count undone tiles hidden by the keep-outs.
        if self.jd - s.blocked_check_jd > 60.0 / 86_400.0 {
            s.blocked_check_jd = self.jd;
            let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
            let o = &self.obs_params;
            use rayon::prelude::*;
            let done = &s.done;
            s.n_blocked = s
                .tiling
                .dirs
                .par_iter()
                .enumerate()
                .filter(|(k, d)| !done[*k] && obs::sky_blocked(o, **d, sun, moon).is_some())
                .count();
            if s.t95.is_none() && s.n_done as f64 >= 0.95 * (s.tiles() - s.n_blocked) as f64 {
                s.t95 = Some((self.jd - s.pass_start_jd) * 86_400.0);
            }
            if s.n_done + s.n_blocked >= s.tiles() {
                let secs = (self.jd - s.pass_start_jd) * 86_400.0;
                s.pass += 1;
                let rec = PassRecord { pass: s.pass, seconds: secs, tiles: s.n_done, eyes_mean: s.eye_s / secs.max(1e-9), t95: s.t95.unwrap_or(secs) };
                s.history.push(rec);
                if s.history.len() > 50 {
                    s.history.remove(0);
                }
                let jd = self.jd;
                s.restart(jd);
                for i in 0..s.tile_of.len() {
                    s.tile_of[i] = NONE;
                }
                s.claimed_by.iter_mut().for_each(|c| *c = NONE);
            }
        }
    }

    pub fn survey_json(&self) -> serde_json::Value {
        let s = &self.survey;
        let el = if s.pass_start_jd > 0.0 { (self.jd - s.pass_start_jd) * 86_400.0 } else { 0.0 };
        let per_tile = if s.tiles_completed > 0 { Some(s.busy_s / s.tiles_completed as f64) } else { None };
        serde_json::json!({
            "active": self.survey_active(),
            "in_queue": s.in_queue,
            "tiles": s.tiles(),
            "done": s.n_done,
            "blocked": s.n_blocked,
            "remaining": s.tiles().saturating_sub(s.n_done + s.n_blocked),
            "pass": s.pass + 1,
            "elapsed_s": el,
            "eyes_now": s.eyes_now,
            "eye_seconds_per_tile": per_tile,
            "overhead_per_tile_s": per_tile.map(|p| (p - self.obs_params.exp_s).max(0.0)),
            "history": s.history,
            "last_pass_s": s.history.last().map(|h| h.seconds),
            "last_t95_s": s.history.last().map(|h| h.t95),
        })
    }

    pub fn survey_panel(&mut self, ui: &mut egui::Ui) {
        use egui::RichText;
        ui.horizontal(|ui| { ui.label(RichText::new("All-sky survey").strong()); crate::provenance::chip(ui, "allsky_measured"); });
        ui.horizontal_wrapped(|ui| {
            if ui.selectable_label(self.mode == obs::Mode::AllSky, "Dedicate the fleet").on_hover_text("Every telescope not on a STARE campaign tiles the sky.").clicked() {
                self.mode = if self.mode == obs::Mode::AllSky { obs::Mode::Queue } else { obs::Mode::AllSky };
            }
            ui.checkbox(&mut self.survey.in_queue, "free telescopes tile during Science operations")
                .on_hover_text("Idle telescopes tile the sky instead of staring at the zenith.");
        });
        ui.checkbox(&mut self.survey.show, "show coverage in sky view");
        if !self.survey_active() {
            ui.label(RichText::new("Not running.").small().weak());
            return;
        }
        let s = &self.survey;
        let el = (self.jd - s.pass_start_jd) * 86_400.0;
        let frac = s.n_done as f64 / (s.tiles() - s.n_blocked).max(1) as f64;
        ui.add(egui::ProgressBar::new(frac as f32).text(format!(
            "pass {} · {} of {} tiles · {} · {} telescopes",
            s.pass + 1,
            s.n_done,
            s.tiles() - s.n_blocked,
            crate::allsky::fmt_span(el),
            s.eyes_now
        )));
        if let Some(h) = s.history.last() {
            let mean = s.history.iter().map(|h| h.seconds).sum::<f64>() / s.history.len() as f64;
            crate::provenance::line(ui, "allsky_measured", format!(
                "Last pass: {} (95 % in {}; {} tiles, {:.0} telescopes on average) · mean of {} passes: {}",
                crate::allsky::fmt_span(h.seconds),
                crate::allsky::fmt_span(h.t95),
                h.tiles,
                h.eyes_mean,
                s.history.len(),
                crate::allsky::fmt_span(mean)
            ));
        }
        if s.tiles_completed > 0 {
            let per = s.busy_s / s.tiles_completed as f64;
            crate::provenance::line(ui, "allsky_measured", format!(
                "Measured telescope time per tile: {:.0} s ({:.0} s exposure + {:.0} s slew, settle and losses)",
                per,
                self.obs_params.exp_s,
                (per - self.obs_params.exp_s).max(0.0)
            ));
        }
        if ui.small_button("Restart pass").clicked() {
            let jd = self.jd;
            self.survey.reset(jd);
        }
    }
}
