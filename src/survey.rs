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
/// Polar duty: satellites whose zenith is within this many degrees of the
/// shell's turning latitude take the polar caps (sky beyond that latitude).
const POLAR_BAND_DEG: f64 = 8.0;
/// Shells whose zeniths reach within this of the poles have no caps to assign.
const POLAR_MIN_CAP_DEG: f64 = 8.0;
/// Tail search beyond the slew cap: at most this many telescopes per step,
/// once fewer than `TAIL_LIST_MAX` tiles remain.
const MAX_FALLBACKS_PER_STEP: usize = 64;
const TAIL_LIST_MAX: usize = 8000;

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
    /// Completion times by |declination| band.
    pub bands: Vec<BandStat>,
    /// Share of the last 5 % of tiles completed that lie in the polar caps
    /// (|Dec| beyond the shell's turning latitude).
    pub tail_in_caps: f64,
    pub polar: bool,
    /// Telescope-seconds per completed tile by attitude phase.
    pub slew_s_per_tile: f64,
    pub settle_s_per_tile: f64,
    pub locked_s_per_tile: f64,
    /// Mean slew per tile, deg.
    pub slew_deg_per_tile: f64,
}

#[derive(Clone, Serialize)]
pub struct BandStat {
    pub dec_lo: f64,
    pub dec_hi: f64,
    pub tiles: usize,
    pub median_s: f64,
    pub p95_s: f64,
    pub last_s: f64,
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
    /// Undone, unblocked tiles, kept once few remain (refreshed with the
    /// blocked count) so idle telescopes can reach the tail beyond the slew cap.
    pub left: Vec<u32>,
    /// Tiles handed back part-done (their integration is kept).
    pub resumed_n: u64,
    /// Completion time of each tile this pass, s after the pass start (<0 = not done).
    pub done_at: Vec<f32>,
    /// Polar duty: satellites near the shell's turning latitude tile the
    /// polar caps; the rest leave the caps until the tail.
    pub polar: bool,
    /// Shell's turning latitude (max |zenith Dec|), deg, set each assignment.
    pub turn_deg: f64,
    /// This pass: telescope-seconds of tile holders by attitude phase, and
    /// total slew angle (deg) between tiles.
    pub phase_s: [f64; 3],
    pub slew_deg: f64,
    /// Telescopes not holding a tile at the last assignment, by reason:
    /// [in the SAA, taken for other work or off, nothing visible in reach].
    pub idle_now: [usize; 3],
}

impl Survey {
    fn ensure(&mut self, fov: f64, n_sat: usize, jd: f64) {
        if self.tiling.dirs.is_empty() || self.tiling.fov_key != (fov * 1000.0) as i64 {
            self.tiling = Tiling::new(fov);
            let nt = self.tiling.dirs.len();
            self.done = vec![false; nt];
            self.progress = vec![0.0; nt];
            self.claimed_by = vec![NONE; nt];
            self.done_at = vec![-1.0; nt];
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
        self.done_at.iter_mut().for_each(|t| *t = -1.0);
        self.n_done = 0;
        self.pass_start_jd = jd;
        self.eye_s = 0.0;
        self.blocked_check_jd = 0.0;
        self.busy_s = 0.0;
        self.phase_s = [0.0; 3];
        self.slew_deg = 0.0;
        self.tiles_completed = 0;
        self.t95 = None;
        self.left.clear();
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
                // Keep the integration so far: an interrupted exposure is read
                // out early and stacked with whoever resumes the tile (an
                // ASSUMPTION; discarding it threw away most of the survey's
                // time in Science operations, where the queue keeps pulling
                // survey telescopes away).
                self.claimed_by[t as usize] = NONE;
                if self.progress[t as usize] > 0.0 {
                    self.resumed_n += 1;
                }
            }
            self.tile_of[i] = NONE;
        }
    }
    pub fn tiles(&self) -> usize {
        self.tiling.dirs.len()
    }

    /// Completion-time statistics by |Dec| band for the pass just finished,
    /// and the share of the last 5 % of tiles that lie in the polar caps.
    fn band_stats(&self) -> (Vec<BandStat>, f64) {
        let turn = self.turn_deg;
        let edges = [0.0, 30.0, (turn - POLAR_BAND_DEG).max(30.0), turn.max(30.0), 75.0f64.max(turn), 90.0];
        let mut out = Vec::new();
        let mut all: Vec<(f32, bool)> = Vec::new();
        for w in edges.windows(2) {
            let (lo, hi) = (w[0], w[1]);
            if hi <= lo {
                continue;
            }
            let mut t: Vec<f32> = (0..self.tiles())
                .filter(|&k| {
                    let ad = deg(self.tiling.dirs[k].to_radec().1).abs();
                    ad >= lo && ad < hi && self.done_at[k] >= 0.0
                })
                .map(|k| self.done_at[k])
                .collect();
            t.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let q = |f: f64| if t.is_empty() { 0.0 } else { t[((t.len() - 1) as f64 * f) as usize] as f64 };
            out.push(BandStat { dec_lo: lo, dec_hi: hi, tiles: t.len(), median_s: q(0.5), p95_s: q(0.95), last_s: q(1.0) });
        }
        for k in 0..self.tiles() {
            if self.done_at[k] >= 0.0 {
                all.push((self.done_at[k], deg(self.tiling.dirs[k].to_radec().1).abs() > turn));
            }
        }
        all.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let tail = &all[(all.len() as f64 * 0.95) as usize..];
        let share = if tail.is_empty() { 0.0 } else { tail.iter().filter(|x| x.1).count() as f64 / tail.len() as f64 };
        (out, share)
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
        let inc = self.shell_params.inc_deg;
        self.survey.turn_deg = if inc <= 90.0 { inc } else { 180.0 - inc };
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let o = self.obs_params.clone();
        let ok = |d: V3| obs::sky_blocked(&o, d, sun, moon).is_none();
        let cos_lim = self.obs.cos_lim;
        let s = &mut self.survey;
        let mut eyes = 0;
        let mut fallbacks = 0;
        let mut idle = [0usize; 3];
        // Telescopes whose detectors are safed in the South Atlantic Anomaly
        // can't integrate: they give their tiles back rather than sit on them.
        let safed = self.env.cfg.saa_policy == crate::environment::SaaPolicy::Pause;
        // Polar caps: sky beyond the shell's turning latitude, never overhead.
        let turn = s.turn_deg;
        let caps = s.polar && 90.0 - turn >= POLAR_MIN_CAP_DEG;
        let tail = !s.left.is_empty();
        for i in 0..n {
            let in_saa = safed && self.env.in_saa.get(i).copied().unwrap_or(false);
            let free = job_tag(self.obs.job[i]) == TAG_SURVEY && !self.obs.off.get(i).copied().unwrap_or(false) && !in_saa;
            if !free {
                idle[if in_saa { 0 } else { 1 }] += 1;
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
            // Polar duty for satellites near the turning latitude (their
            // zenith Dec ≈ their latitude): +1 north cap, −1 south, 0 none.
            // A duty satellite that finds no cap tile in reach searches again
            // without duty, so it never idles while other sky is left.
            let zdec = deg(self.obs.rhat[i].to_radec().1);
            let duty0: i8 = if caps && zdec.abs() > turn - POLAR_BAND_DEG { zdec.signum() as i8 } else { 0 };
            // Nearest undone, unclaimed, visible tile to where we point now.
            let here = self.obs.att[i].dir;
            let (ra, dec) = here.to_radec();
            let (ra, dec) = (deg(ra).rem_euclid(360.0), deg(dec));
            let rhat = self.obs.rhat[i];
            let cos_za = rad(MAX_ZA_DEG).cos();
            let mut best: Option<(f64, usize)> = None;
            for duty in [duty0, 0] {
            if duty != duty0 && (duty0 == 0 || best.is_some()) {
                break;
            }
            let mut search_r = 4.0 * s.tiling.step_deg;
            loop {
                // Search out to exactly the cap on the last round (doubling
                // alone stepped 18° → 36° and never searched 18–32°).
                let radius_now = search_r.min(MAX_SLEW_DEG);
                let radius = radius_now;
                let r0 = s.tiling.ring_of(dec - radius);
                let r1 = s.tiling.ring_of(dec + radius);
                for r in r0..=r1 {
                    if caps {
                        let rd = s.tiling.ring_dec[r];
                        let cap = if rd > turn { 1 } else if rd < -turn { -1 } else { 0 };
                        // On duty: only the own-hemisphere cap. Off duty: no
                        // caps until the tail.
                        if (duty != 0 && cap != duty) || (duty == 0 && cap != 0 && !tail) {
                            continue;
                        }
                    }
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
                if best.is_some() || radius_now >= MAX_SLEW_DEG {
                    break;
                }
                search_r = radius_now * 2.0;
            }
            }
            // The tail: nothing within the cap, but tiles remain somewhere this
            // telescope can see. A long slew beats idling at the zenith.
            if best.is_none() && !s.left.is_empty() && fallbacks < MAX_FALLBACKS_PER_STEP {
                fallbacks += 1;
                for &k in &s.left {
                    let k = k as usize;
                    if s.done[k] || s.claimed_by[k] != NONE {
                        continue;
                    }
                    let d = s.tiling.dirs[k];
                    let cz = d.dot(rhat);
                    if cz <= cos_lim.max(cos_za) {
                        continue;
                    }
                    let c = -(here.angle(d) + ZA_WEIGHT * cz.clamp(-1.0, 1.0).acos());
                    if best.map_or(true, |b| c > b.0) && ok(d) {
                        best = Some((c, k));
                    }
                }
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
                s.slew_deg += deg(here.angle(d));
                self.obs.pointing[i] = Some(d);
                self.obs.job[i] = job(TAG_ALLSKY, k as u64);
                self.obs.fine[i] = crate::imaging::guide_star_available(d, fov, o.guide_mag);
                eyes += 1;
            } else {
                idle[2] += 1;
            }
        }
        s.eyes_now = eyes;
        s.idle_now = idle;
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
            use crate::attitude::Phase;
            match self.obs.att[i].phase() {
                Phase::Slewing => s.phase_s[0] += dt,
                Phase::Settling | Phase::Acquiring => s.phase_s[1] += dt,
                Phase::Locked => s.phase_s[2] += dt,
                _ => {}
            }
            let k = job_val(self.obs.job[i]) as usize;
            s.progress[k] += self.obs.obs_dt[i] as f32;
            if s.progress[k] >= exp && !s.done[k] {
                s.done[k] = true;
                if let Some(t) = s.done_at.get_mut(k) {
                    *t = ((self.jd - s.pass_start_jd) * 86_400.0) as f32;
                }
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
            let remaining = s.tiles().saturating_sub(s.n_done + s.n_blocked);
            s.left = if remaining <= TAIL_LIST_MAX {
                (0..s.tiles()).filter(|&k| !s.done[k] && obs::sky_blocked(o, s.tiling.dirs[k], sun, moon).is_none()).map(|k| k as u32).collect()
            } else {
                Vec::new()
            };
            if s.t95.is_none() && s.n_done as f64 >= 0.95 * (s.tiles() - s.n_blocked) as f64 {
                s.t95 = Some((self.jd - s.pass_start_jd) * 86_400.0);
            }
            if s.n_done + s.n_blocked >= s.tiles() {
                let secs = (self.jd - s.pass_start_jd) * 86_400.0;
                s.pass += 1;
                let (bands, tail_in_caps) = s.band_stats();
                let rec = PassRecord {
                    pass: s.pass,
                    seconds: secs,
                    tiles: s.n_done,
                    eyes_mean: s.eye_s / secs.max(1e-9),
                    t95: s.t95.unwrap_or(secs),
                    bands,
                    tail_in_caps,
                    polar: s.polar,
                    slew_s_per_tile: s.phase_s[0] / s.tiles_completed.max(1) as f64,
                    settle_s_per_tile: s.phase_s[1] / s.tiles_completed.max(1) as f64,
                    locked_s_per_tile: s.phase_s[2] / s.tiles_completed.max(1) as f64,
                    slew_deg_per_tile: s.slew_deg / s.tiles_completed.max(1) as f64,
                };
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
            "polar": s.polar,
            "idle_now": { "saa": s.idle_now[0], "other_or_off": s.idle_now[1], "nothing_in_reach": s.idle_now[2] },
            "turn_deg": s.turn_deg,
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
            "remaining_detail": self.survey_remaining_detail(),
        })
    }

    /// Where the untiled, unblocked tiles are (by Sun elongation), and what
    /// the telescopes holding tiles are doing.
    pub fn survey_remaining_detail(&self) -> serde_json::Value {
        let s = &self.survey;
        if s.tiles() == 0 {
            return serde_json::Value::Null;
        }
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let o = &self.obs_params;
        let n = self.obs.job.len();
        let mut bins = [0u32; 18];
        let list: Box<dyn Iterator<Item = usize>> = if s.left.is_empty() { Box::new(0..s.tiles()) } else { Box::new(s.left.iter().map(|&k| k as usize)) };
        for k in list {
            let d = s.tiling.dirs[k];
            if s.done[k] || obs::sky_blocked(o, d, sun, moon).is_some() {
                continue;
            }
            bins[((deg(d.angle(sun)) / 10.0) as usize).min(17)] += 1;
        }
        // What the telescopes holding tiles are doing.
        use crate::attitude::Phase;
        let (mut sl, mut se, mut lk, mut eff_sum, mut el_sum) = (0u32, 0u32, 0u32, 0.0f64, 0.0f64);
        for i in 0..n {
            if job_tag(self.obs.job[i]) != TAG_ALLSKY {
                continue;
            }
            match self.obs.att[i].phase() {
                Phase::Slewing => sl += 1,
                Phase::Settling | Phase::Acquiring => se += 1,
                Phase::Locked => {
                    lk += 1;
                    eff_sum += self.env.shine.get(i).copied().unwrap_or(1.0) as f64;
                    el_sum += deg(self.obs.att[i].dir.angle(sun));
                }
                _ => {}
            }
        }
        serde_json::json!({
            "released_part_done": s.resumed_n,
            "holders_slewing": sl,
            "holders_settling": se,
            "holders_locked": lk,
            "locked_mean_efficiency": if lk > 0 { eff_sum / lk as f64 } else { 0.0 },
            "locked_mean_sun_elongation": if lk > 0 { el_sum / lk as f64 } else { 0.0 },
            "by_sun_elongation_10deg": bins,

        })
    }

    pub fn survey_panel(&mut self, ui: &mut egui::Ui) {
        use egui::RichText;
        ui.horizontal(|ui| { ui.label(RichText::new("All-sky survey").strong()); crate::provenance::chip(ui, "allsky_measured"); });
        ui.horizontal_wrapped(|ui| {
            if ui.selectable_label(self.mode == obs::Mode::AllSky, "Dedicate the fleet").on_hover_text("Every telescope not on a STARE campaign tiles the sky.").clicked() {
                self.mode = if self.mode == obs::Mode::AllSky { obs::Mode::Queue } else { obs::Mode::AllSky };
            }
            ui.checkbox(&mut self.survey.polar, "polar duty: satellites near the turning latitude tile the polar caps")
                .on_hover_text("Sky beyond the shell's turning latitude (53° by default) never passes overhead. Satellites near that latitude, where the shell is densest, take the caps; the rest leave them until the tail.");
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
