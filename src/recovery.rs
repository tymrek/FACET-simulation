//! Recovering a new discovery: the planetary-defence follow-up exercise.
//!
//! A survey reports a new near-Earth object from a short tracklet: where it
//! was and how fast it was moving, with errors. Its position is then only
//! known within an uncertainty region that grows with time since discovery,
//! mostly along the direction of motion. FACET tiles the 3σ region with
//! fields that drift at the predicted rate (each a tracked mosaic tile), and
//! the object is recovered when a tile that contains it reaches deep enough.
//!
//! The "discovery" is a real catalogued NEO that is observable now, so its
//! true motion — including the curvature a straight-line prediction misses —
//! comes from its orbit. Only the alert's errors are synthetic:
//!
//! * astrometry: four positions over one hour at 0.5″ each (rate error
//!   ≈ 0.67″/h per axis from the least-squares slope);
//! * the rest of a short-arc solution's error, as a fraction of the rate:
//!   5 % along the motion, 2 % across it (assumed).
//!
//! The prediction is a straight line from the discovery position at the
//! estimated rate; real orbit-determination services do better than that
//! for older arcs, so this is a pessimistic case.

use crate::math::V3;
use serde::Serialize;

/// Astrometric error per position, arcsec, and the tracklet: n positions over T.
const ASTROM_ARCSEC: f64 = 0.5;
const TRACKLET_N: f64 = 4.0;
const TRACKLET_S: f64 = 3600.0;
/// Fractional rate errors of the short-arc solution (assumed).
const RATE_FRAC_ALONG: f64 = 0.05;
const RATE_FRAC_CROSS: f64 = 0.02;
/// Planning margin: size the region for this long after the alert, s.
const PLAN_AHEAD_S: f64 = 1800.0;
/// Most tiles a recovery may use.
pub const MAX_TILES: usize = 49;
/// Give up after this long, s.
const TIMEOUT_S: f64 = 12.0 * 3600.0;

#[derive(Clone, Debug, Serialize)]
pub struct RecoveryCfg {
    /// Hours between discovery and the alert reaching FACET.
    pub age_h: f64,
    pub telescopes_per_tile: u32,
    pub exposure_s: f64,
    /// Brightness range for the synthetic discovery.
    pub vmin: f64,
    pub vmax: f64,
    /// Only objects this close stand in for discoveries (new NEOs are mostly
    /// small and found near the Earth, so close and fast), AU.
    pub max_delta_au: f64,
    /// Start a new alert whenever none is running (for measuring many).
    pub auto: bool,
}

impl Default for RecoveryCfg {
    fn default() -> Self {
        RecoveryCfg { age_h: 12.0, telescopes_per_tile: 8, exposure_s: 120.0, vmin: 18.0, vmax: 21.5, max_delta_au: 0.2, auto: false }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub enum Outcome {
    Searching,
    /// Recovered: seconds after the alert, tile index, SNR.
    Recovered { after_s: f64, tile: usize, snr: f64 },
    /// Not found: why.
    Lost { after_s: f64, why: String },
}

/// A drifting pointing: tangent-plane position at `jd0` moving at a constant
/// rate (arcsec/s east and north).
#[derive(Clone, Copy, Debug)]
pub struct Drift {
    pub dir0: V3,
    pub jd0: f64,
    pub rate_e: f64,
    pub rate_n: f64,
}

impl Drift {
    pub fn at(&self, jd: f64) -> V3 {
        let dt = (jd - self.jd0) * 86_400.0;
        crate::mosaic::offset_dir(self.dir0, self.rate_e * dt / 3600.0, self.rate_n * dt / 3600.0)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Recovery {
    pub id: u64,
    /// Provisional designation shown in the alert.
    pub designation: String,
    /// The catalogued object standing in for the discovery (index; revealed
    /// only once recovered).
    #[serde(skip)]
    pub neo: usize,
    pub truth_name: Option<String>,
    pub vmag: f64,
    pub alert_jd: f64,
    pub alert_utc: String,
    pub age_h: f64,
    /// Predicted rate, arcsec/min, and position angle.
    pub rate_arcsec_min: f64,
    pub pa_deg: f64,
    /// 1σ along / across the motion at the alert, arcsec.
    pub sigma_along_arcsec: f64,
    pub sigma_cross_arcsec: f64,
    /// Searched region (3σ, sized for half an hour after the alert), deg.
    pub region_deg: (f64, f64),
    pub cols: u32,
    pub rows: u32,
    /// Tile request ids, nearest the prediction first.
    pub tiles: Vec<u64>,
    #[serde(skip)]
    pub checked: Vec<bool>,
    pub tiles_done: usize,
    /// Where the object really was relative to the prediction at the alert,
    /// in σ along / across (revealed at the end).
    pub truth_offset_sigma: Option<(f64, f64)>,
    pub telescopes_per_tile: u32,
    pub exposure_s: f64,
    pub telescope_s: f64,
    pub outcome: Outcome,
    pub followup: Option<u64>,
    #[serde(skip)]
    pub predicted: Drift,
}

/// 1σ position error (along, across the motion), arcsec, `dt` s after discovery.
pub fn sigma(rate_arcsec_s: f64, dt: f64) -> (f64, f64) {
    let n = TRACKLET_N;
    // Least-squares slope error for n evenly spaced points over T.
    let s_rate = ASTROM_ARCSEC * (12.0 / (n * (n * n - 1.0))).sqrt() * (n - 1.0) / TRACKLET_S;
    let a = ((s_rate * dt).powi(2) + (RATE_FRAC_ALONG * rate_arcsec_s * dt).powi(2) + ASTROM_ARCSEC.powi(2)).sqrt();
    let c = ((s_rate * dt).powi(2) + (RATE_FRAC_CROSS * rate_arcsec_s * dt).powi(2) + ASTROM_ARCSEC.powi(2)).sqrt();
    (a, c)
}

/// Tangent-plane offset (east, north; arcsec) of `d` from `c`.
pub fn offset_arcsec(c: V3, d: V3) -> (f64, f64) {
    let (e, n) = crate::obs::tangent_basis(c);
    let k = d.dot(c);
    ((d.dot(e) / k).atan().to_degrees() * 3600.0, (d.dot(n) / k).atan().to_degrees() * 3600.0)
}

pub fn gauss(st: &mut u64) -> f64 {
    let mut u = || {
        *st = st.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *st;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    };
    let (a, b) = (u().max(1e-300), u());
    (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncertainty_grows_mostly_along_the_motion() {
        let rate = 30.0 / 60.0; // 30″/min
        let (a1, c1) = sigma(rate, 3600.0);
        let (a12, c12) = sigma(rate, 12.0 * 3600.0);
        assert!(a12 > 10.0 * a1 && c12 > 5.0 * c1);
        assert!(a12 > 2.0 * c12);
        // 12 h at 30″/min: 5 % of 6 h of motion ≈ 1080″ ≈ 0.3° along.
        assert!((a12 - 1080.0).abs() < 60.0, "{a12}");
    }

    #[test]
    fn drift_moves_at_the_rate() {
        let d = Drift { dir0: V3::from_radec(1.0, 0.2), jd0: 2_461_000.0, rate_e: 1.0, rate_n: 0.0 };
        let (e, n) = offset_arcsec(d.dir0, d.at(2_461_000.0 + 600.0 / 86_400.0));
        assert!((e - 600.0).abs() < 1.0 && n.abs() < 1.0, "{e} {n}");
    }
}

use crate::app::App;
use crate::server::{ObsRequest, ReqState};

fn tile_request(id: u64, name: String, dir: V3, telescopes: u32, exposure_s: f64, jd: f64, utc: &str, drift: Drift, note: String) -> ObsRequest {
    let (ra, dec) = dir.to_radec();
    ObsRequest {
        id,
        submitter: "Planetary defence".into(),
        target_name: name,
        body: None,
        neo: None,
        moving: None,
        ra_deg: ra.to_degrees(),
        dec_deg: dec.to_degrees(),
        telescopes,
        exposure_s,
        priority: 9,
        state: ReqState::Approved,
        progress_s: 0.0,
        assigned_now: 0,
        blocked: None,
        submitted_utc: utc.to_string(),
        completed_utc: None,
        note,
        ip: String::new(),
        dir,
        hw: None,
        obs_jd: 0.0,
        locked_now: 0,
        approved_jd: Some(jd),
        first_light_jd: None,
        full_lock_jd: None,
        jit2_s: 0.0,
        fine_s: 0.0,
        slews: 0,
        slew_deg_sum: 0.0,
        agility: None,
        sched: Default::default(),
        programme: None,
        deadline_s: None,
        submit_jd: jd,
        case_idx: None,
        inv_idx: None,
        mosaic: None,
        drift: Some(drift),
    }
}

impl App {
    /// Raise a discovery alert and start searching for it. `target` picks the
    /// catalogued object standing in for the discovery; otherwise a random
    /// observable NEO in the configured brightness range.
    pub fn recovery_start(&mut self, target: Option<&str>) -> Result<u64, String> {
        let Some(web) = self.web.clone() else { return Err("web server not running".into()) };
        let cfg = self.recovery_cfg.clone();
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let o = &self.obs_params;
        let ok = |d: V3| d.angle(sun).to_degrees() > o.sun_excl_deg + 5.0 && d.angle(moon).to_degrees() > o.moon_excl_deg + 5.0;
        let disc_jd = self.jd - cfg.age_h / 24.0;
        let a = self.shell.params.semi_major_km();
        let v_orb = (crate::astro::MU_EARTH / a).sqrt();
        let k = match target.and_then(crate::neo::find) {
            Some(k) => k,
            None => {
                let n = self.neo.dir.len();
                let mut cands: Vec<usize> = (0..n)
                    .filter(|&i| {
                        (cfg.vmin..=cfg.vmax).contains(&(self.neo.vmag[i] as f64))
                            && (self.neo.delta_au[i] as f64) <= cfg.max_delta_au
                            && ok(self.neo.dir[i])
                    })
                    .collect();
                if cands.is_empty() {
                    return Err(format!("no observable NEO between V {} and {} within {} AU right now", cfg.vmin, cfg.vmax, cfg.max_delta_au));
                }
                self.recovery_seed = self.recovery_seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let pick = (self.recovery_seed >> 17) as usize % cands.len();
                cands.swap_remove(pick)
            }
        };
        let disc = crate::neo::ephem(k, disc_jd, a, v_orb);
        let now = crate::neo::ephem(k, self.jd, a, v_orb);
        if !ok(now.dir) {
            return Err(format!("{} is too close to the Sun or Moon right now", crate::neo::all()[k].name));
        }
        // Estimated rate at discovery: truth plus the short-arc errors.
        let rate = disc.rate_arcsec_s;
        let (ps, pc) = disc.pa_deg.to_radians().sin_cos();
        let mut st = self.recovery_seed ^ (k as u64).wrapping_mul(0xD1B5_4A32_D192_ED03);
        let s_rate = ASTROM_ARCSEC * (12.0 / (TRACKLET_N * (TRACKLET_N.powi(2) - 1.0))).sqrt() * (TRACKLET_N - 1.0) / TRACKLET_S;
        let d_along = gauss(&mut st) * (RATE_FRAC_ALONG * rate).hypot(s_rate);
        let d_cross = gauss(&mut st) * (RATE_FRAC_CROSS * rate).hypot(s_rate);
        let (est_along, est_cross) = (rate + d_along, d_cross);
        // Along = (sin PA, cos PA) in (east, north); across = (cos PA, −sin PA).
        let rate_e = est_along * ps + est_cross * pc;
        let rate_n = est_along * pc - est_cross * ps;
        // Discovery position (with its astrometric error), extrapolated.
        let d0 = crate::mosaic::offset_dir(disc.dir, gauss(&mut st) * ASTROM_ARCSEC / 3600.0, gauss(&mut st) * ASTROM_ARCSEC / 3600.0);
        let predicted = Drift { dir0: d0, jd0: disc_jd, rate_e, rate_n };
        let center = predicted.at(self.jd);
        let age_s = cfg.age_h * 3600.0;
        let (sa, sc) = sigma(rate, age_s);
        let (pa_, pc_) = sigma(rate, age_s + PLAN_AHEAD_S);
        let (fw, fh) = crate::obs::frame_ew_ns(o);
        let plan = crate::mosaic::plan_region(center, 3.0 * pa_ / 3600.0, 3.0 * pc_ / 3600.0, disc.pa_deg, fw, fh, MAX_TILES)?;
        // Nearest the prediction first.
        let mut tiles = plan.tiles.clone();
        tiles.sort_by(|x, y| x.2.angle(center).partial_cmp(&y.2.angle(center)).unwrap());
        // Truth relative to the prediction now, in σ along/across.
        let (te, tn) = offset_arcsec(center, now.dir);
        let (t_al, t_cr) = (te * ps + tn * pc, te * pc - tn * ps);
        let mut sh = web.lock().unwrap();
        let utc = sh.utc_now.clone();
        let rid = sh.next_id;
        let designation = format!("FAC{}{:04}", &utc.get(2..4).unwrap_or("26"), rid % 10_000);
        let mut ids = Vec::new();
        for (j, &(_, _, d)) in tiles.iter().enumerate() {
            let id = sh.next_id;
            sh.next_id += 1;
            let drift = Drift { dir0: d, jd0: self.jd, rate_e, rate_n };
            sh.requests.push(tile_request(
                id,
                format!("{designation} recovery · tile {}/{}", j + 1, tiles.len()),
                d,
                cfg.telescopes_per_tile,
                cfg.exposure_s,
                self.jd,
                &utc,
                drift,
                format!("NEO recovery search, tracking {:.1}″/min at PA {:.0}°", 60.0 * est_along.hypot(est_cross), disc.pa_deg),
            ));
            ids.push(id);
        }
        sh.log(
            "warn",
            format!(
                "NEO alert {designation}: V {:.1}, discovered {:.1} h ago moving {:.1}″/min; 1σ {:.0}″ × {:.0}″ — searching {} tile{} ({} × {} grid), {} telescopes each",
                now.vmag,
                cfg.age_h,
                rate * 60.0,
                sa,
                sc,
                ids.len(),
                if ids.len() == 1 { "" } else { "s" },
                plan.cols,
                plan.rows,
                cfg.telescopes_per_tile
            ),
        );
        drop(sh);
        self.recoveries.push(Recovery {
            id: rid,
            designation,
            neo: k,
            truth_name: None,
            vmag: now.vmag,
            alert_jd: self.jd,
            alert_utc: utc,
            age_h: cfg.age_h,
            rate_arcsec_min: 60.0 * est_along.hypot(est_cross),
            pa_deg: disc.pa_deg,
            sigma_along_arcsec: sa,
            sigma_cross_arcsec: sc,
            region_deg: (6.0 * pa_ / 3600.0, 6.0 * pc_ / 3600.0),
            cols: plan.cols,
            rows: plan.rows,
            checked: vec![false; ids.len()],
            tiles: ids,
            tiles_done: 0,
            truth_offset_sigma: Some((t_al / sa, t_cr / sc)),
            telescopes_per_tile: cfg.telescopes_per_tile,
            exposure_s: cfg.exposure_s,
            telescope_s: 0.0,
            outcome: Outcome::Searching,
            followup: None,
            predicted,
        });
        Ok(rid)
    }

    /// Each step: move the search tiles, check finished tiles for the object,
    /// and close the exercise when it is found or the region is exhausted.
    pub fn recovery_step(&mut self) {
        let Some(web) = self.web.clone() else { return };
        if self.recovery_cfg.auto && self.recoveries.iter().all(|r| r.outcome != Outcome::Searching) {
            let _ = self.recovery_start(None);
        }
        if self.recoveries.iter().all(|r| r.outcome != Outcome::Searching) {
            return;
        }
        let a = self.shell.params.semi_major_km();
        let v_orb = (crate::astro::MU_EARTH / a).sqrt();
        let hw = crate::imaging::Hardware::from_obs(&self.obs_params);
        let sun = self.bodies[0].dir;
        let (fw, fh) = crate::obs::frame_ew_ns(&self.obs_params);
        let jd = self.jd;
        let mut sh = web.lock().unwrap();
        let utc = sh.utc_now.clone();
        let mut logs: Vec<(&'static str, String)> = Vec::new();
        for rc in self.recoveries.iter_mut().filter(|r| r.outcome == Outcome::Searching) {
            let truth = crate::neo::ephem(rc.neo, jd, a, v_orb);
            let since = (jd - rc.alert_jd) * 86_400.0;
            let mut found: Option<(usize, f64)> = None;
            let mut done = 0;
            let mut used = 0.0;
            for (j, id) in rc.tiles.iter().enumerate() {
                let Some(r) = sh.requests.iter().find(|r| r.id == *id) else { continue };
                used += r.progress_s;
                let finished = r.state == ReqState::Done;
                if finished {
                    done += 1;
                }
                if !finished || rc.checked[j] {
                    continue;
                }
                rc.checked[j] = true;
                // Inside the tile's field now?
                let (dx, dy) = offset_arcsec(r.dir, truth.dir);
                let (half_w, half_h) = (fw * 3600.0 / 2.0 * 0.97, fh * 3600.0 / 2.0 * 0.97);
                if dx.abs() > half_w || dy.abs() > half_h {
                    continue;
                }
                // Deep enough? The tile tracks the predicted rate, so the
                // object trails by the rate error within each sub-frame.
                let sub = hw.sub_max_s.min(r.exposure_s);
                let lim = crate::imaging::point_lim_mag(&hw, r.progress_s, sub, crate::imaging::sky_sb(truth.dir, sun, &self.obs_params));
                let (ps, pc) = truth.pa_deg.to_radians().sin_cos();
                let (te, tn) = (truth.rate_arcsec_s * ps, truth.rate_arcsec_s * pc);
                let mis = ((te - rc.predicted.rate_e).powi(2) + (tn - rc.predicted.rate_n).powi(2)).sqrt();
                let fwhm = hw.psf(sub).fwhm;
                let lim = lim - 1.25 * (1.0 + mis * sub / fwhm).log10();
                if truth.vmag <= lim {
                    let snr = 5.0 * 10f64.powf(0.4 * (lim - truth.vmag));
                    found = Some((j, snr));
                    break;
                }
            }
            rc.tiles_done = done;
            rc.telescope_s = used;
            let name = crate::neo::all()[rc.neo].name.to_string();
            if let Some((j, snr)) = found {
                rc.outcome = Outcome::Recovered { after_s: since, tile: j, snr };
                rc.truth_name = Some(name.clone());
                // Stop the rest of the search and follow the object up.
                for id in &rc.tiles {
                    if let Some(r) = sh.requests.iter_mut().find(|r| r.id == *id && r.state.is_open()) {
                        r.state = ReqState::Cancelled;
                        r.assigned_now = 0;
                        r.locked_now = 0;
                        r.completed_utc = Some(utc.clone());
                    }
                }
                let fid = sh.next_id;
                sh.next_id += 1;
                let mut f = tile_request(
                    fid,
                    format!("{} follow-up (= {name})", rc.designation),
                    truth.dir,
                    rc.telescopes_per_tile.max(4),
                    rc.exposure_s,
                    jd,
                    &utc,
                    rc.predicted,
                    "NEO follow-up after recovery: tracked on its orbit".into(),
                );
                f.drift = None;
                f.neo = Some(rc.neo);
                sh.requests.push(f);
                rc.followup = Some(fid);
                logs.push((
                    "info",
                    format!(
                        "NEO {} RECOVERED {} after the alert in tile {} of {} (SNR {:.0}); it is {name}. Follow-up #{fid} tracking it.",
                        rc.designation,
                        crate::defence::fmt_dur(since),
                        j + 1,
                        rc.tiles.len(),
                        snr
                    ),
                ));
            } else if done == rc.tiles.len() || since > TIMEOUT_S {
                let (te, tn) = offset_arcsec(rc.predicted.at(jd), truth.dir);
                let (ps, pc) = rc.pa_deg.to_radians().sin_cos();
                let (sa, sc) = (rc.sigma_along_arcsec, rc.sigma_cross_arcsec);
                let outside = ((te * ps + tn * pc) / (3.0 * sa)).powi(2) + ((te * pc - tn * ps) / (3.0 * sc)).powi(2) > 1.0;
                let why = if since > TIMEOUT_S {
                    "timed out (12 h)".to_string()
                } else if outside {
                    "the object had left the searched 3σ region".to_string()
                } else {
                    format!("in the region but fainter than the tiles reached (V {:.1})", truth.vmag)
                };
                rc.outcome = Outcome::Lost { after_s: since, why: why.clone() };
                rc.truth_name = Some(name.clone());
                logs.push(("warn", format!("NEO {} NOT recovered after {}: {why}. It was {name}.", rc.designation, crate::defence::fmt_dur(since))));
            }
        }
        for (lvl, m) in logs {
            sh.log(lvl, m);
        }
    }

    /// Status JSON: configuration, recent exercises and the measured spread.
    pub fn recovery_json(&self) -> serde_json::Value {
        let mut t: Vec<f64> = self
            .recoveries
            .iter()
            .filter_map(|r| if let Outcome::Recovered { after_s, .. } = r.outcome { Some(after_s) } else { None })
            .collect();
        t.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let ended = self.recoveries.iter().filter(|r| r.outcome != Outcome::Searching).count();
        let q = |f: f64| if t.is_empty() { None } else { Some(t[((t.len() - 1) as f64 * f).round() as usize]) };
        serde_json::json!({
            "cfg": self.recovery_cfg,
            "recent": self.recoveries.iter().rev().take(60).collect::<Vec<_>>(),
            "ended": ended,
            "recovered": t.len(),
            "after_s": { "min": q(0.0), "median": q(0.5), "p90": q(0.9), "max": q(1.0) },
        })
    }
}
