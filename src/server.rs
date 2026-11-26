//! Embedded web access: a token-protected director's control port and an open
//! public-access port. Both are plain HTTP/1.1 (tiny_http), one thread each.
//!
//! The simulator owns the truth. The server threads only:
//!   * read the latest JSON status snapshot the sim publishes (~4 Hz),
//!   * queue `Command`s for the sim to apply on its next frame,
//!   * manage the observation-request book (shared, under one mutex).
//!
//! Put a TLS-terminating reverse proxy (nginx, caddy) in front of either port
//! before exposing it beyond a trusted network: tokens travel in clear text.

use crate::math::V3;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Method, Request, Response, Server};

const CONTROL_HTML: &str = include_str!("../web/control.html");
const PUBLIC_HTML: &str = include_str!("../web/public.html");
const SKYMAP_JS: &str = include_str!("../web/skymap.js");
const CONTROL_JS: &str = include_str!("../web/control.js");
const PUBLIC_JS: &str = include_str!("../web/public.js");
const STYLE_CSS: &str = include_str!("../web/style.css");
/// The FACET compound-eye icon (16 and 32 px), as on the proposal site.
const FAVICON: &[u8] = include_bytes!("../web/favicon.ico");

const MAX_BODY: u64 = 8 * 1024;
const MAX_OPEN_PER_IP: usize = 5;
const MAX_OPEN_TOTAL: usize = 500;
const MAX_KEPT: usize = 2000;
/// Closed requests from real visitors kept beyond MAX_KEPT, newest first, so
/// their results stay reachable while the programme churns.
const MAX_KEPT_VISITOR: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReqState {
    Pending,
    Approved,
    Observing,
    Done,
    Rejected,
    Cancelled,
    /// Science programme request whose deadline passed before completion.
    Expired,
    /// Observed, but the schedule broke its science requirement (see `Need`).
    Failed,
}

impl ReqState {
    pub fn is_open(self) -> bool {
        matches!(self, ReqState::Pending | ReqState::Approved | ReqState::Observing)
    }
    pub fn is_schedulable(self) -> bool {
        matches!(self, ReqState::Approved | ReqState::Observing)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ObsRequest {
    pub id: u64,
    pub submitter: String,
    pub target_name: String,
    /// Index into astro::bodies for moving targets; otherwise fixed RA/Dec.
    pub body: Option<usize>,
    /// Index into neo::all() for a tracked near-Earth asteroid.
    #[serde(skip)]
    pub neo: Option<usize>,
    /// Latest ephemeris of a tracked asteroid (rate, distance, parallax).
    pub moving: Option<crate::neo::Ephem>,
    pub ra_deg: f64,
    pub dec_deg: f64,
    /// Telescopes requested simultaneously (they stack).
    pub telescopes: u32,
    /// Integration per telescope, seconds.
    pub exposure_s: f64,
    pub priority: i32,
    pub state: ReqState,
    /// Accumulated telescope-seconds.
    pub progress_s: f64,
    pub assigned_now: u32,
    pub blocked: Option<String>,
    pub submitted_utc: String,
    pub completed_utc: Option<String>,
    pub note: String,
    #[serde(skip)]
    pub ip: String,
    #[serde(skip)]
    pub dir: V3,
    /// Hardware snapshot taken when the request first started observing.
    #[serde(skip)]
    pub hw: Option<crate::imaging::Hardware>,
    /// Sim time of the most recent integration.
    #[serde(skip)]
    pub obs_jd: f64,
    /// Telescopes locked on and integrating right now.
    pub locked_now: u32,
    #[serde(skip)]
    pub approved_jd: Option<f64>,
    #[serde(skip)]
    pub first_light_jd: Option<f64>,
    /// First time every requested telescope was locked on.
    #[serde(skip)]
    pub full_lock_jd: Option<f64>,
    /// Σ(locked seconds × jitter²) and fine-guided seconds, for the result model.
    #[serde(skip)]
    pub jit2_s: f64,
    #[serde(skip)]
    pub fine_s: f64,
    pub slews: u32,
    /// Science-programme case key (None = public / director request).
    pub programme: Option<&'static str>,
    /// Data are only useful if complete within this many seconds of submission.
    pub deadline_s: Option<f64>,
    #[serde(skip)]
    pub submit_jd: f64,
    #[serde(skip)]
    pub case_idx: Option<usize>,
    #[serde(skip)]
    pub inv_idx: Option<usize>,
    #[serde(skip)]
    pub slew_deg_sum: f64,
    #[serde(skip)]
    pub agility: Option<crate::attitude::Agility>,
    /// Scheduling requirement and its bookkeeping (programme cases).
    #[serde(skip)]
    pub sched: Sched,
    /// Tile of a public mosaic (see mosaic.rs).
    pub mosaic: Option<crate::mosaic::Mosaic>,
    /// A pointing that drifts at a fixed rate (NEO recovery search tiles).
    #[serde(skip)]
    pub drift: Option<crate::recovery::Drift>,
}

/// Per-request state for science requirements beyond telescope-seconds.
#[derive(Clone, Debug, Default)]
pub struct Sched {
    pub need: crate::programme::Need,
    /// Event window (JD start, end) for `Need::Window`.
    pub win: Option<(f64, f64)>,
    pub cover_s: f64,
    pub gap_s: f64,
    pub gap_max_s: f64,
    pub last_jd: f64,
    pub visits_done: u32,
    pub next_visit_jd: f64,
    pub visit_s: f64,
    /// Host-star V magnitude for transit photometry (0: not applicable).
    pub host_v: f64,
    /// First moment the window was covered (JD), for the late-start metric.
    pub first_cover_jd: Option<f64>,
    /// Nominal-telescope-equivalent seconds: each contributing telescope's
    /// time × its sensitivity factor (fleet optical variation).
    pub eff_s: f64,
    /// What the members were doing when the current / longest gap began.
    pub gap_note_now: Option<String>,
    pub gap_note: Option<String>,
}

impl Sched {
    /// Is the request schedulable at `jd`? (`prep_s`: lead before a window
    /// opens, to slew and settle in.) Returns why not, for display.
    pub fn waiting(&self, jd: f64, prep_s: f64) -> Option<String> {
        if let Some((a, _)) = self.win {
            let t = (a - jd) * 86_400.0 - prep_s;
            if t > 0.0 {
                return Some(format!("event window opens in {}", crate::allsky::fmt_span(t + prep_s)));
            }
        }
        if self.next_visit_jd > jd {
            return Some(format!("visit {} in {}", self.visits_done + 1, crate::allsky::fmt_span((self.next_visit_jd - jd) * 86_400.0)));
        }
        None
    }
    /// Inside an event window that needs unbroken coverage.
    pub fn continuous(&self, jd: f64, prep_s: f64) -> bool {
        self.win.map_or(false, |(a, b)| jd >= a - prep_s / 86_400.0 && jd <= b)
    }
}

impl ObsRequest {
    /// Approval → first locked photon, seconds of sim time.
    pub fn response_s(&self) -> Option<f64> {
        Some((self.first_light_jd? - self.approved_jd?) * 86_400.0)
    }
    /// Approval → all requested telescopes locked, seconds.
    pub fn full_lock_s(&self) -> Option<f64> {
        Some((self.full_lock_jd? - self.approved_jd?) * 86_400.0)
    }
    /// Integration-weighted pointing jitter, arcsec RMS.
    pub fn eff_jitter(&self) -> Option<f64> {
        (self.progress_s > 0.0 && self.jit2_s > 0.0).then(|| (self.jit2_s / self.progress_s).sqrt())
    }

    /// Integration in nominal-telescope seconds (for depth).
    pub fn eff_total(&self) -> f64 {
        if self.sched.eff_s > 0.0 { self.sched.eff_s } else { self.progress_s }
    }
}

impl ObsRequest {
    pub fn required_s(&self) -> f64 {
        self.telescopes as f64 * self.exposure_s
    }
}

/// A STARE campaign (Sustained Tracking And Rotating Eyes): keep `eyes`
/// telescopes locked on a target continuously, handing over as satellites
/// lose line of sight.
#[derive(Clone, Debug, Serialize)]
pub struct Campaign {
    pub id: u64,
    pub target_name: String,
    #[serde(skip)]
    pub body: Option<usize>,
    #[serde(skip)]
    pub neo: Option<usize>,
    pub moving: Option<crate::neo::Ephem>,
    pub ra_deg: f64,
    pub dec_deg: f64,
    #[serde(skip)]
    pub dir: V3,
    pub eyes: u32,
    /// Cadence of the published frames (each frame stacks eyes × frame_s).
    pub frame_s: f64,
    pub active: bool,
    /// Built-in campaign for a solar-system body (stopping returns it to ready).
    pub preset: bool,
    /// Ready to start at the push of a button (not running).
    pub armed: bool,
    /// Running, but the target is inside a keep-out zone: eyes released,
    /// resumes automatically when it leaves.
    pub paused: bool,
    pub started_utc: String,
    pub stopped_utc: Option<String>,
    pub duration_h: Option<f64>,
    #[serde(skip)]
    pub start_jd: f64,
    pub assigned_now: u32,
    pub locked_now: u32,
    pub blocked: Option<String>,
    /// Time with the target trackable / inside keep-out zones, s.
    pub eligible_s: f64,
    pub excluded_s: f64,
    /// Time with ≥ eyes locked / ≥ 1 locked, s.
    pub full_s: f64,
    pub any_s: f64,
    pub gaps: u32,
    pub max_gap_s: f64,
    #[serde(skip)]
    pub cur_gap_s: f64,
    #[serde(skip)]
    pub acquired: bool,
    pub handovers: u32,
    pub integ_s: f64,
    #[serde(skip)]
    pub jit2_s: f64,
    #[serde(skip)]
    pub fine_s: f64,
    #[serde(skip)]
    pub hw: Option<crate::imaging::Hardware>,
    #[serde(skip)]
    pub agility: Option<crate::attitude::Agility>,
    #[serde(skip)]
    pub last_jd: f64,
}

/// Commands from the director's console, applied by the simulator thread.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    // NOTE: the operations console deliberately cannot change the simulated
    // universe (fleet size, orbits, hardware, clock). Those live only in the
    // desktop simulator window.
    Mode {
        mode: String,
    },
    Target {
        name: Option<String>,
        ra_deg: Option<f64>,
        dec_deg: Option<f64>,
    },
    /// Take a satellite out of tasking (safe hold) or return it to service.
    SafeHold {
        sat: usize,
        on: bool,
    },
    // Applied directly by the server thread (request book):
    Request {
        id: u64,
        action: String,
        priority: Option<i32>,
        telescopes: Option<u32>,
    },
    AutoApprove {
        on: bool,
    },
    PublicShare {
        share: f64,
    },
    /// Start a STARE campaign on a named target or RA/Dec.
    StareStart {
        target: Option<String>,
        ra_deg: Option<f64>,
        dec_deg: Option<f64>,
        eyes: Option<u32>,
        frame_s: Option<f64>,
        duration_h: Option<f64>,
    },
    StareStop {
        id: u64,
    },
    /// Start a ready (armed) campaign.
    StareActivate {
        id: u64,
    },
    /// Start every ready campaign.
    StareActivateAll,
    StareEyes {
        id: u64,
        eyes: u32,
    },
    /// Raise a synthetic new-discovery alert and search for it.
    RecoveryStart {
        target: Option<String>,
        age_h: Option<f64>,
        telescopes_per_tile: Option<u32>,
        exposure_s: Option<f64>,
    },
    /// Keep raising alerts, one after another (for measuring many).
    RecoveryAuto {
        on: bool,
    },
}

/// A resolvable target: named star, solar-system body, or catalogue object.
#[derive(Clone)]
pub enum Resolved {
    Body(usize, String),
    Fixed(String, V3),
    /// A known near-Earth asteroid (index into neo::all()), tracked.
    Neo(usize, String),
}

pub struct Shared {
    pub status_json: String,
    pub stars_json: String,
    pub commands: Vec<Command>,
    pub requests: Vec<ObsRequest>,
    pub next_id: u64,
    pub auto_approve: bool,
    pub token: String,
    pub utc_now: String,
    /// Lower-case name → target.
    pub names: HashMap<String, Resolved>,
    pub control_url: Option<String>,
    pub public_url: Option<String>,
    pub errors: Vec<String>,
    pub results: ResultCache,
    /// Sim Julian date at the last status publish.
    pub jd_now: f64,
    /// Operations event log, newest last.
    pub events: std::collections::VecDeque<serde_json::Value>,
    /// Sub-satellite points etc. for the ground-track map (published ~1 Hz).
    pub fleet_json: String,
    /// Satellite the console is inspecting, and its latest detail record.
    pub inspect_req: Option<usize>,
    pub inspect_json: String,
    pub coast_json: String,
    pub campaigns: Vec<Campaign>,
    pub next_cid: u64,
    /// Current telescope field, east-west × north-south, degrees (for
    /// mosaic planning).
    pub field_w_deg: f64,
    pub field_h_deg: f64,
}

impl Shared {
    /// Bound memory: drop the oldest closed requests, but keep the most
    /// recent closed requests from real visitors (and the director): their
    /// results are what people come back for. Programme and modelled public
    /// requests (no submitting address) go first.
    pub fn prune(&mut self) {
        if self.requests.len() <= MAX_KEPT {
            return;
        }
        let visitor = |r: &ObsRequest| r.programme.is_none() && !r.ip.is_empty();
        // Closed visitor requests, newest by completion first: keep the top N.
        let mut closed_v: Vec<(u64, f64)> =
            self.requests.iter().filter(|r| !r.state.is_open() && visitor(r)).map(|r| (r.id, r.obs_jd.max(r.submit_jd))).collect();
        closed_v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let keep_v: std::collections::HashSet<u64> = closed_v.iter().take(MAX_KEPT_VISITOR).map(|x| x.0).collect();
        let excess = self.requests.len() - MAX_KEPT;
        let mut removed = 0;
        self.requests.retain(|r| {
            if removed < excess && !r.state.is_open() && !keep_v.contains(&r.id) {
                removed += 1;
                false
            } else {
                true
            }
        });
    }

    /// Append to the operations event log. level: "info" | "good" | "warn".
    pub fn log(&mut self, level: &str, msg: impl Into<String>) {
        let e = serde_json::json!({ "utc": self.utc_now, "level": level, "msg": msg.into() });
        self.events.push_back(e);
        while self.events.len() > 300 {
            self.events.pop_front();
        }
    }
}

/// Rendered observation results, keyed by (request id, progress bucket).
#[derive(Default)]
pub struct ResultCache {
    map: HashMap<(u64, u32), Arc<crate::imaging::Output>>,
    order: std::collections::VecDeque<(u64, u32)>,
}

impl ResultCache {
    const CAP: usize = 40;
    fn get(&self, k: (u64, u32)) -> Option<Arc<crate::imaging::Output>> {
        self.map.get(&k).cloned()
    }
    fn put(&mut self, k: (u64, u32), v: Arc<crate::imaging::Output>) {
        if self.map.insert(k, v).is_none() {
            self.order.push_back(k);
        }
        while self.order.len() > Self::CAP {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }
}

/// Start a campaign from the console/CLI. If the target is a solar-system
/// body that already has a built-in (preset) campaign, that one is started.
pub fn start_campaign(
    sh: &mut Shared,
    target: Option<&str>,
    ra: Option<f64>,
    dec: Option<f64>,
    eyes: Option<u32>,
    frame_s: Option<f64>,
    duration_h: Option<f64>,
) -> Result<(), &'static str> {
    if let Some(t) = target {
        if let Some(Resolved::Body(b, _)) = sh.resolve(t) {
            if let Some(id) = sh.campaigns.iter().find(|c| c.preset && c.body == Some(b) && !c.active).map(|c| c.id) {
                if let Some(c) = sh.campaigns.iter_mut().find(|c| c.id == id) {
                    if let Some(e) = eyes {
                        c.eyes = e.clamp(1, 5000);
                    }
                    if let Some(f) = frame_s {
                        c.frame_s = f.clamp(1.0, 3600.0);
                    }
                    c.duration_h = duration_h.filter(|h| h.is_finite() && *h > 0.0);
                }
                return activate_campaign(sh, id);
            }
        }
    }
    create_campaign(sh, target, ra, dec, eyes, frame_s, duration_h, true, false).map(|_| ())
}

#[allow(clippy::too_many_arguments)]
fn create_campaign(
    sh: &mut Shared,
    target: Option<&str>,
    ra: Option<f64>,
    dec: Option<f64>,
    eyes: Option<u32>,
    frame_s: Option<f64>,
    duration_h: Option<f64>,
    active: bool,
    preset: bool,
) -> Result<u64, &'static str> {
    let (body, neo, name, dir) = match (target, ra, dec) {
        (Some(t), _, _) if !t.trim().is_empty() => match sh.resolve(t) {
            Some(Resolved::Body(i, n)) => (Some(i), None, n, V3::Z),
            Some(Resolved::Fixed(n, d)) => (None, None, n, d),
            Some(Resolved::Neo(k, n)) => (None, Some(k), n, V3::Z),
            None => return Err("unknown target name"),
        },
        (_, Some(ra), Some(dec)) if ra.is_finite() && (-90.0..=90.0).contains(&dec) => {
            let ra = ra.rem_euclid(360.0);
            (None, None, format!("RA {ra:.3}° Dec {dec:+.3}°"), V3::from_radec(ra.to_radians(), dec.to_radians()))
        }
        _ => return Err("give a target name or ra_deg/dec_deg"),
    };
    if active && sh.campaigns.iter().filter(|c| c.active).count() >= 50 {
        return Err("too many active campaigns (50)");
    }
    let id = sh.next_cid;
    sh.next_cid += 1;
    let (ra_r, dec_r) = dir.to_radec();
    let eyes = eyes.unwrap_or(10).clamp(1, 5000);
    let frame_s = frame_s.unwrap_or(60.0).clamp(1.0, 3600.0);
    let duration_h = duration_h.filter(|h| h.is_finite() && *h > 0.0);
    let c = Campaign {
        id,
        target_name: name.clone(),
        body,
        neo,
        moving: None,
        ra_deg: ra_r.to_degrees(),
        dec_deg: dec_r.to_degrees(),
        dir,
        eyes,
        frame_s,
        active,
        preset,
        armed: !active,
        paused: false,
        started_utc: if active { sh.utc_now.clone() } else { String::new() },
        stopped_utc: None,
        duration_h,
        start_jd: sh.jd_now,
        assigned_now: 0,
        locked_now: 0,
        blocked: None,
        eligible_s: 0.0,
        excluded_s: 0.0,
        full_s: 0.0,
        any_s: 0.0,
        gaps: 0,
        max_gap_s: 0.0,
        cur_gap_s: 0.0,
        acquired: false,
        handovers: 0,
        integ_s: 0.0,
        jit2_s: 0.0,
        fine_s: 0.0,
        hw: None,
        agility: None,
        last_jd: 0.0,
    };
    sh.campaigns.push(c);
    if active {
        sh.log(
            "good",
            format!(
                "STARE #{id} started on {name}: {eyes} eyes, {frame_s:.0} s frames{}",
                duration_h.map(|h| format!(", {h} h")).unwrap_or_else(|| ", until stopped".into())
            ),
        );
    }
    Ok(id)
}

/// Start (or restart) a ready campaign, with fresh statistics.
pub fn activate_campaign(sh: &mut Shared, id: u64) -> Result<(), &'static str> {
    if sh.campaigns.iter().filter(|c| c.active).count() >= 50 {
        return Err("too many active campaigns (50)");
    }
    let (jd, utc) = (sh.jd_now, sh.utc_now.clone());
    let c = sh.campaigns.iter_mut().find(|c| c.id == id).ok_or("no such campaign")?;
    if c.active {
        return Err("campaign already running");
    }
    if !c.armed {
        return Err("campaign has ended; start a new one");
    }
    c.active = true;
    c.armed = false;
    c.paused = false;
    c.start_jd = jd;
    c.started_utc = utc;
    c.stopped_utc = None;
    c.eligible_s = 0.0;
    c.excluded_s = 0.0;
    c.full_s = 0.0;
    c.any_s = 0.0;
    c.gaps = 0;
    c.max_gap_s = 0.0;
    c.cur_gap_s = 0.0;
    c.acquired = false;
    c.handovers = 0;
    c.integ_s = 0.0;
    c.jit2_s = 0.0;
    c.fine_s = 0.0;
    c.hw = None;
    c.agility = None;
    let msg = format!(
        "STARE #{} started on {}: {} eyes, {:.0} s frames{}",
        c.id,
        c.target_name,
        c.eyes,
        c.frame_s,
        c.duration_h.map(|h| format!(", {h} h")).unwrap_or_else(|| ", until stopped".into())
    );
    sh.log("good", msg);
    Ok(())
}

/// Arm the built-in campaigns: the Moon and the seven planets, ready to start.
pub fn arm_presets(sh: &mut Shared, eyes: u32) {
    let names: Vec<&str> = crate::astro::BODY_NAMES[1..].to_vec();
    for n in &names {
        if sh.campaigns.iter().any(|c| c.preset && c.target_name == *n) {
            continue;
        }
        let _ = create_campaign(sh, Some(n), None, None, Some(eyes), None, None, false, true);
    }
    sh.log("info", format!("{} STARE campaigns armed (Moon and planets), ready to start", names.len()));
}

/// Serve `/api/campaign/{id}[/full.png|/crop.png]`: the latest STARE frame.
fn handle_campaign(shared: SharedRef, req: Request, path: String) {
    let rest = path.trim_start_matches("/api/campaign/");
    let (id_s, part) = rest.split_once('/').unwrap_or((rest, ""));
    let Ok(id) = id_s.parse::<u64>() else {
        return err(req, 400, "bad id");
    };
    let (key, spec) = {
        let sh = shared.lock().unwrap();
        let Some(c) = sh.campaigns.iter().find(|c| c.id == id) else {
            drop(sh);
            return err(req, 404, "no such campaign");
        };
        let (Some(hw), Some(ag), true) = (c.hw, c.agility, c.integ_s > 0.0) else {
            drop(sh);
            return err(req, 404, "no frames yet: the campaign has not locked on");
        };
        let jd = if c.active { sh.jd_now } else { c.last_jd };
        let bucket = ((jd * 86_400.0 / c.frame_s).floor() as u64 & 0xFFFF_FFFF) as u32;
        // One frame = every locked eye integrating for one cadence interval.
        let eyes = c.locked_now.max(1).min(c.eyes.max(1));
        let total = eyes as f64 * c.frame_s;
        let mut hw = hw;
        if c.integ_s > 0.0 && c.jit2_s > 0.0 {
            hw.jitter_arcsec = (c.jit2_s / c.integ_s).sqrt();
        }
        (
            (id | (1u64 << 62), bucket),
            crate::imaging::Spec {
                id,
                target_name: format!("STARE #{id} · {}", c.target_name),
                dir: c.dir,
                body: c.body,
                jd,
                total_s: total,
                required_s: total,
                telescopes: eyes,
                exposure_s: c.frame_s,
                hw,
                done: true,
                fine_frac: if c.integ_s > 0.0 { c.fine_s / c.integ_s } else { 1.0 },
                response_s: None,
                full_lock_s: None,
                slews: c.handovers,
                mean_slew_deg: 0.0,
                agility: Some(ag),
                mover: c.moving,
            },
        )
    };
    let cached = shared.lock().unwrap().results.get(key);
    let out = match cached {
        Some(o) => o,
        None => {
            let _g = RENDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let again = shared.lock().unwrap().results.get(key);
            match again {
                Some(o) => o,
                None => {
                    let o = Arc::new(crate::imaging::render(&spec));
                    shared.lock().unwrap().results.put(key, o.clone());
                    o
                }
            }
        }
    };
    match part {
        "" => json(req, 200, out.report.clone()),
        "full.png" => send_bytes(req, "image/png", out.full_png.clone()),
        "crop.png" => send_bytes(req, "image/png", out.crop_png.clone()),
        "psf.png" => send_bytes(req, "image/png", out.psf_png.clone()),
        _ => err(req, 404, "not found"),
    }
}

/// Renders are CPU-heavy: do one at a time.
static RENDER_LOCK: Mutex<()> = Mutex::new(());

/// The render spec for a request's current data, and its cache key.
fn result_spec(r: &ObsRequest) -> Option<((u64, u32), crate::imaging::Spec)> {
    let hw = r.hw?;
    if r.progress_s <= 0.0 {
        return None;
    }
    let done = r.state == ReqState::Done;
    let required = r.required_s();
    // Quantise live stacks to 5 % steps so repeated polls hit the cache.
    let bucket = if done { u32::MAX } else { ((r.progress_s / required * 20.0).floor() as u32).max(1) };
    let total = if done { r.progress_s } else { (bucket as f64 / 20.0 * required).min(r.progress_s.max(1e-6)) };
    // Telescopes differ: scale to nominal-equivalent time.
    let total = if r.sched.eff_s > 0.0 { total * r.sched.eff_s / r.progress_s } else { total };
    Some((
        (r.id, bucket),
        crate::imaging::Spec {
            id: r.id,
            target_name: r.target_name.clone(),
            dir: r.dir,
            body: r.body,
            jd: r.obs_jd,
            total_s: total,
            required_s: required,
            telescopes: r.telescopes,
            exposure_s: r.exposure_s,
            hw: {
                let mut hw = hw;
                if let Some(j) = r.eff_jitter() {
                    hw.jitter_arcsec = j;
                }
                hw
            },
            done,
            fine_frac: if r.progress_s > 0.0 { r.fine_s / r.progress_s } else { 1.0 },
            response_s: r.response_s(),
            full_lock_s: r.full_lock_s(),
            slews: r.slews,
            mean_slew_deg: if r.slews > 0 { r.slew_deg_sum / r.slews as f64 } else { 0.0 },
            agility: r.agility,
            mover: r.moving,
        },
    ))
}

/// Render through the cache, one render at a time; `post` adjusts a fresh render.
fn render_cached(
    shared: &SharedRef,
    key: (u64, u32),
    spec: &crate::imaging::Spec,
    post: impl FnOnce(crate::imaging::Output) -> crate::imaging::Output,
) -> Arc<crate::imaging::Output> {
    if let Some(o) = shared.lock().unwrap().results.get(key) {
        return o;
    }
    let _g = RENDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Another thread may have rendered it while we waited.
    if let Some(o) = shared.lock().unwrap().results.get(key) {
        return o;
    }
    let o = Arc::new(post(crate::imaging::render(spec)));
    shared.lock().unwrap().results.put(key, o.clone());
    o
}

fn send_output(req: Request, out: &crate::imaging::Output, part: &str) {
    match part {
        "" => json(req, 200, out.report.clone()),
        "full.png" => send_bytes(req, "image/png", out.full_png.clone()),
        "crop.png" => send_bytes(req, "image/png", out.crop_png.clone()),
        "psf.png" => send_bytes(req, "image/png", out.psf_png.clone()),
        _ => err(req, 404, "not found"),
    }
}

/// Serve `/api/result/{id}[/full.png|/crop.png]`. Runs on its own thread.
fn handle_result(shared: SharedRef, req: Request, path: String) {
    let rest = path.trim_start_matches("/api/result/");
    let (id_s, part) = rest.split_once('/').unwrap_or((rest, ""));
    let Ok(id) = id_s.parse::<u64>() else {
        return err(req, 400, "bad id");
    };
    let spec = {
        let sh = shared.lock().unwrap();
        let Some(r) = sh.requests.iter().find(|r| r.id == id) else {
            drop(sh);
            return err(req, 404, "no such request");
        };
        let Some(s) = result_spec(r) else {
            drop(sh);
            return err(req, 404, "no data yet: the request has not been observed");
        };
        s
    };
    let (key, spec) = spec;
    let out = render_cached(&shared, key, &spec, |o| o);
    send_output(req, &out, part)
}

/// Serve `/api/mosaic/{group}[/full.png|/crop.png]`: the stitched overview of
/// a mosaic, rendered over the whole footprint at the depth of its shallowest
/// tile, with cells that were not observed greyed out.
fn handle_mosaic(shared: SharedRef, req: Request, path: String) {
    let rest = path.trim_start_matches("/api/mosaic/");
    let (id_s, part) = rest.split_once('/').unwrap_or((rest, ""));
    let Ok(group) = id_s.parse::<u64>() else {
        return err(req, 400, "bad id");
    };
    let prep = {
        let sh = shared.lock().unwrap();
        let tiles: Vec<&ObsRequest> = sh.requests.iter().filter(|r| r.mosaic.as_ref().is_some_and(|m| m.group == group)).collect();
        let Some(m) = tiles.first().and_then(|r| r.mosaic.clone()) else {
            drop(sh);
            return err(req, 404, "no such mosaic");
        };
        let started = tiles.iter().filter(|r| r.hw.is_some() && r.progress_s > 0.0).count();
        if started < tiles.len() {
            let msg = format!("no overview yet: {started} of {} tiles observed so far", tiles.len());
            drop(sh);
            return err(req, 404, &msg);
        }
        // The shallowest tile sets the depth.
        let Some(shallow) = tiles
            .iter()
            .min_by(|a, b| (a.progress_s / a.required_s()).partial_cmp(&(b.progress_s / b.required_s())).unwrap_or(std::cmp::Ordering::Equal))
        else {
            drop(sh);
            return err(req, 404, "no such mosaic");
        };
        let Some((_, mut spec)) = result_spec(shallow) else {
            drop(sh);
            return err(req, 404, "no data yet");
        };
        let done = tiles.iter().all(|r| r.state == ReqState::Done);
        let frac = (shallow.progress_s / shallow.required_s()).min(1.0);
        let bucket = if done { u32::MAX } else { ((frac * 20.0).floor() as u32).max(1) };
        let span = crate::mosaic::Plan::span_deg(&m);
        spec.id = group;
        spec.target_name = m.object.clone();
        spec.dir = V3::from_radec(m.ra_deg.to_radians(), m.dec_deg.to_radians());
        spec.hw.fov_deg = span;
        spec.done = done;
        spec.slews = tiles.iter().map(|r| r.slews).sum();
        let observed: Vec<(u32, u32)> = tiles.iter().filter_map(|r| r.mosaic.as_ref().map(|x| (x.col, x.row))).collect();
        ((group | 1 << 62, bucket), spec, m, observed, span, tiles.len())
    };
    let (key, spec, m, observed, span, n) = prep;
    let out = render_cached(&shared, key, &spec, |mut o| {
        // Grey out grid cells that no tile covers.
        let mut dec = png::Decoder::new(std::io::Cursor::new(o.full_png.clone()));
        dec.set_transformations(png::Transformations::IDENTITY);
        if let Ok(mut rd) = dec.read_info() {
            let mut buf = vec![0u8; rd.output_buffer_size()];
            if let Ok(info) = rd.next_frame(&mut buf) {
                if info.color_type == png::ColorType::Grayscale && info.width == info.height {
                    let w = info.width as usize;
                    buf.truncate(w * w);
                    crate::mosaic::mask_unobserved(&mut buf, w, span, &m, &observed);
                    o.full_png = crate::imaging::encode_png(w, w, &buf);
                }
            }
        }
        let note = format!(
            "Mosaic overview: {n} tiles of {:.0}′ × {:.0}′ on a {} × {} grid ({:.0} % overlap) covering {:.0}′ × {:.0}′. \
             The overview is drawn at the depth of the shallowest tile and a coarser pixel scale than the tiles; \
             each tile's full-resolution result is under its own request.{}",
            m.field_w_deg * 60.0,
            m.field_h_deg * 60.0,
            m.cols,
            m.rows,
            crate::mosaic::OVERLAP * 100.0,
            crate::mosaic::Plan::extent_deg(&m).0 * 60.0,
            crate::mosaic::Plan::extent_deg(&m).1 * 60.0,
            if (n as u32) < m.cols * m.rows { " Grid cells outside the object were not observed and are greyed out." } else { "" }
        );
        if let Some(a) = o.report["notes"].as_array_mut() {
            a.insert(0, serde_json::json!(note));
        }
        // The overview is rendered as one wide frame: put the hardware table
        // back to one telescope's field.
        let ps = o.report["hardware"]["pixel_scale_arcsec"].as_f64().unwrap_or(0.0);
        o.report["hardware"]["fov_deg"] = serde_json::json!((m.field_w_deg * m.field_h_deg).sqrt());
        if let (Some(sensor), true) = (o.report["hardware"]["sensor"].as_str().map(str::to_string), ps > 0.0) {
            if let Some((_, rest)) = sensor.split_once(" px across") {
                o.report["hardware"]["sensor"] = serde_json::json!(format!("{:.0} px across{rest}", m.field_w_deg * 3600.0 / ps));
            }
        }
        o.report["mosaic"] = serde_json::json!({
            "group": m.group, "tiles": n, "cols": m.cols, "rows": m.rows,
            "field_w_deg": m.field_w_deg, "field_h_deg": m.field_h_deg, "span_deg": span, "overlap": crate::mosaic::OVERLAP,
        });
        o
    });
    send_output(req, &out, part)
}

fn send_bytes(req: Request, ctype: &str, body: Vec<u8>) {
    let resp = Response::from_data(body)
        .with_header(header("Content-Type", ctype))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("X-Content-Type-Options", "nosniff"));
    let _ = req.respond(resp);
}

pub type SharedRef = Arc<Mutex<Shared>>;

impl Shared {
    pub fn resolve(&self, name: &str) -> Option<Resolved> {
        self.names
            .get(&name.trim().to_lowercase())
            .cloned()
            .or_else(|| crate::neo::find(name).map(|k| Resolved::Neo(k, crate::neo::all()[k].name.to_string())))
    }
}

pub fn random_token() -> String {
    let mut buf = [0u8; 16];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok();
    if !ok {
        // Fallback: not cryptographic, but never empty.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
            ^ (std::process::id() as u128) << 64;
        buf.copy_from_slice(&t.to_le_bytes());
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("static header")
}

fn send(req: Request, code: u16, ctype: &str, body: String) {
    let resp = Response::from_string(body)
        .with_status_code(code)
        .with_header(header("Content-Type", ctype))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("X-Content-Type-Options", "nosniff"))
        .with_header(header("Referrer-Policy", "no-referrer"))
        .with_header(header(
            "Content-Security-Policy",
            "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:",
        ));
    let _ = req.respond(resp);
}

fn json(req: Request, code: u16, v: serde_json::Value) {
    send(req, code, "application/json", v.to_string());
}

fn err(req: Request, code: u16, msg: &str) {
    json(req, code, serde_json::json!({ "error": msg }));
}

fn read_body(req: &mut Request) -> Option<String> {
    if req.body_length().map_or(false, |l| l as u64 > MAX_BODY) {
        return None;
    }
    let mut s = String::new();
    req.as_reader().take(MAX_BODY + 1).read_to_string(&mut s).ok()?;
    if s.len() as u64 > MAX_BODY {
        return None;
    }
    Some(s)
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

fn path_of(url: &str) -> &str {
    url.split('?').next().unwrap_or("/")
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn authorised(req: &Request, token: &str) -> bool {
    let from_header = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("X-Facet-Token"))
        .map(|h| h.value.as_str().to_string());
    let given = from_header.or_else(|| query_param(req.url(), "token"));
    given.map_or(false, |g| ct_eq(&g, token))
}

fn clean(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

fn static_route(path: &str, director: bool) -> Option<(&'static str, &'static str)> {
    match path {
        "/skymap.js" => Some(("application/javascript; charset=utf-8", SKYMAP_JS)),
        "/style.css" => Some(("text/css; charset=utf-8", STYLE_CSS)),
        "/control.js" if director => Some(("application/javascript; charset=utf-8", CONTROL_JS)),
        "/public.js" if !director => Some(("application/javascript; charset=utf-8", PUBLIC_JS)),
        _ => None,
    }
}

fn requests_json(sh: &Shared, director: bool, always: &[u64]) -> serde_json::Value {
    let mut list: Vec<&ObsRequest> = sh.requests.iter().collect();
    // Open public/director requests first (they may need approval), then the
    // science programme by priority, then closed ones, newest first.
    list.sort_by_key(|r| (!r.state.is_open(), r.programme.is_some(), std::cmp::Reverse(r.priority), std::cmp::Reverse(if r.state.is_open() { 0 } else { r.id })));
    // The first 400, plus any the caller asked for by id (a visitor's own
    // requests), which would otherwise drop off the list once they finish.
    let mut chosen: Vec<&ObsRequest> = list.iter().take(400).copied().collect();
    for id in always.iter().take(100) {
        if !chosen.iter().any(|r| r.id == *id) {
            if let Some(r) = sh.requests.iter().find(|r| r.id == *id) {
                chosen.push(r);
            }
        }
    }
    let v: Vec<serde_json::Value> = chosen
        .iter()
        .map(|r| {
            let mut j = serde_json::to_value(r).unwrap_or_default();
            j["required_s"] = serde_json::json!(r.required_s());
            j["has_result"] = serde_json::json!(r.hw.is_some() && r.progress_s > 0.0);
            j["response_s"] = serde_json::json!(r.response_s());
            j["full_lock_s"] = serde_json::json!(r.full_lock_s());
            j["waiting_s"] = serde_json::json!(match (r.approved_jd, r.first_light_jd) {
                (Some(a), None) if r.state.is_schedulable() => Some((sh.jd_now - a) * 86_400.0),
                _ => None,
            });
            if director {
                j["ip"] = serde_json::json!(r.ip);
            }
            j
        })
        .collect();
    serde_json::Value::Array(v)
}

fn status_with(sh: &Shared, director: bool, url: &str) -> String {
    // status_json is an object; splice requests (and director extras) in.
    let mut v: serde_json::Value =
        serde_json::from_str(&sh.status_json).unwrap_or_else(|_| serde_json::json!({}));
    let mine: Vec<u64> = query_param(url, "mine")
        .map(|m| m.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_default();
    v["requests"] = requests_json(sh, director, &mine);
    v["auto_approve"] = serde_json::json!(sh.auto_approve);
    v["campaigns"] = serde_json::Value::Array(
        sh.campaigns
            .iter()
            .filter(|c| c.active || director)
            .rev()
            .take(80)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|c| {
                let mut j = serde_json::to_value(c).unwrap_or_default();
                j["has_frame"] = serde_json::json!(c.integ_s > 0.0 && c.hw.is_some());
                j
            })
            .collect(),
    );
    if director {
        v["events"] = serde_json::Value::Array(sh.events.iter().rev().take(120).cloned().collect());
    }
    v.to_string()
}

#[derive(Deserialize)]
struct Submit {
    submitter: Option<String>,
    target: Option<String>,
    ra_deg: Option<f64>,
    dec_deg: Option<f64>,
    telescopes: Option<u32>,
    exposure_s: Option<f64>,
    note: Option<String>,
    /// Tile a catalogue object larger than one field.
    mosaic: Option<bool>,
}

fn handle_submit(shared: &SharedRef, mut req: Request) {
    let ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();
    let Some(body) = read_body(&mut req) else {
        return err(req, 413, "request too large");
    };
    let sub: Submit = match serde_json::from_str(&body) {
        Ok(s) => s,
        Err(_) => return err(req, 400, "malformed JSON"),
    };
    let mut sh = shared.lock().unwrap();
    let open_total = sh.requests.iter().filter(|r| r.state.is_open() && r.programme.is_none()).count();
    // A mosaic counts once towards the per-visitor limit.
    let open_ip = sh
        .requests
        .iter()
        .filter(|r| r.state.is_open() && r.ip == ip)
        .map(|r| r.mosaic.as_ref().map_or(r.id, |m| m.group))
        .collect::<std::collections::HashSet<u64>>()
        .len();
    if open_total >= MAX_OPEN_TOTAL {
        drop(sh);
        return err(req, 503, "queue full, try later");
    }
    if open_ip >= MAX_OPEN_PER_IP {
        drop(sh);
        return err(req, 429, "you already have 5 open requests");
    }
    let (body_idx, neo_idx, name, dir) = match (&sub.target, sub.ra_deg, sub.dec_deg) {
        (Some(t), _, _) if !t.trim().is_empty() => match sh.resolve(t) {
            Some(Resolved::Body(i, n)) => (Some(i), None, n, V3::Z),
            Some(Resolved::Fixed(n, d)) => (None, None, n, d),
            Some(Resolved::Neo(k, n)) => (None, Some(k), n, V3::Z),
            None => {
                drop(sh);
                return err(req, 404, "unknown target name; use a planet, a near-Earth asteroid, a bright-star or deep-sky name, or RA/Dec");
            }
        },
        (_, Some(ra), Some(dec)) if ra.is_finite() && (-90.0..=90.0).contains(&dec) => {
            let ra = ra.rem_euclid(360.0);
            (None, None, format!("RA {ra:.3}° Dec {dec:+.3}°"), V3::from_radec(ra.to_radians(), dec.to_radians()))
        }
        _ => {
            drop(sh);
            return err(req, 400, "give a target name or ra_deg/dec_deg");
        }
    };
    let telescopes = sub.telescopes.unwrap_or(10).clamp(1, 1000);
    let exposure_s = sub.exposure_s.unwrap_or(300.0);
    if !(1.0..=86_400.0).contains(&exposure_s) {
        drop(sh);
        return err(req, 400, "exposure_s must be 1–86400");
    }
    let (ra, dec) = dir.to_radec();
    let id = sh.next_id;
    sh.next_id += 1;
    let state = if sh.auto_approve { ReqState::Approved } else { ReqState::Pending };
    let utc = sh.utc_now.clone();
    let r = ObsRequest {
        id,
        submitter: {
            let s = clean(sub.submitter.as_deref().unwrap_or(""), 40);
            if s.is_empty() { "anonymous".into() } else { s }
        },
        target_name: name,
        body: body_idx,
        neo: neo_idx,
        moving: None,
        ra_deg: ra.to_degrees(),
        dec_deg: dec.to_degrees(),
        telescopes,
        exposure_s,
        priority: 0,
        state,
        progress_s: 0.0,
        assigned_now: 0,
        blocked: None,
        submitted_utc: utc,
        completed_utc: None,
        note: clean(sub.note.as_deref().unwrap_or(""), 200),
        ip,
        dir,
        hw: None,
        obs_jd: 0.0,
        locked_now: 0,
        approved_jd: if state == ReqState::Approved { Some(sh.jd_now) } else { None },
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
        submit_jd: sh.jd_now,
        case_idx: None,
        inv_idx: None,
        mosaic: None,
        drift: None,
    };
    // A mosaic: one request per tile, the telescopes shared between tiles.
    let mut reqs = vec![r];
    if sub.mosaic == Some(true) {
        let plan = match &sub.target {
            Some(t) if body_idx.is_none() => match crate::catalog::find_dso(t) {
                Some(d) if d.dir.angle(dir) < 1e-3 => crate::mosaic::plan_object(d, sh.field_w_deg, sh.field_h_deg),
                _ => Err("mosaics are for catalogue deep-sky objects (galaxies, nebulae, clusters)".into()),
            },
            _ => Err("mosaics are for catalogue deep-sky objects (galaxies, nebulae, clusters)".into()),
        };
        let plan = match plan {
            Ok(p) => p,
            Err(e) => {
                sh.next_id -= 1;
                drop(sh);
                return err(req, 400, &format!("mosaic: {e}"));
            }
        };
        let n = plan.tiles.len() as u32;
        let per = telescopes.div_ceil(n).max(1);
        let t = reqs.pop().unwrap();
        let (ra0, dec0) = (t.ra_deg, t.dec_deg);
        for (k, &(col, row, d)) in plan.tiles.iter().enumerate() {
            let mut x = t.clone();
            x.id = id + k as u64;
            x.target_name = format!("{} · tile {}/{}", t.target_name, k + 1, n);
            x.dir = d;
            let (ra, dec) = d.to_radec();
            x.ra_deg = ra.to_degrees();
            x.dec_deg = dec.to_degrees();
            x.telescopes = per;
            x.mosaic = Some(crate::mosaic::Mosaic {
                group: id,
                index: k as u32,
                n,
                cols: plan.cols,
                rows: plan.rows,
                col,
                row,
                object: t.target_name.clone(),
                ra_deg: ra0,
                dec_deg: dec0,
                field_w_deg: sh.field_w_deg,
                field_h_deg: sh.field_h_deg,
                step_x_deg: plan.step_x_deg,
                step_y_deg: plan.step_y_deg,
            });
            reqs.push(x);
        }
        sh.next_id = id + n as u64;
    }
    let ids: Vec<u64> = reqs.iter().map(|r| r.id).collect();
    let mosaic_json = reqs[0].mosaic.as_ref().map(|m| serde_json::json!({
        "group": m.group, "n": m.n, "cols": m.cols, "rows": m.rows, "telescopes_per_tile": reqs[0].telescopes,
    }));
    let r = &reqs[0];
    let msg = match &r.mosaic {
        Some(m) => format!(
            "Request #{} from {}: mosaic of {} ({} tiles, {} × {:.0} s each){}",
            r.id, r.submitter, m.object, m.n, r.telescopes, r.exposure_s,
            if state == ReqState::Approved { " · auto-approved" } else { " · awaiting approval" }
        ),
        None => format!(
            "Request #{} from {}: {} ({} × {:.0} s){}",
            r.id, r.submitter, r.target_name, r.telescopes, r.exposure_s,
            if state == ReqState::Approved { " · auto-approved" } else { " · awaiting approval" }
        ),
    };
    sh.requests.extend(reqs);
    sh.log("info", msg);
    sh.prune();
    drop(sh);
    json(req, 201, serde_json::json!({ "id": id, "ids": ids, "state": state, "mosaic": mosaic_json }));
}

fn apply_request_cmd(sh: &mut Shared, id: u64, action: &str, priority: Option<i32>, telescopes: Option<u32>) -> Result<(), &'static str> {
    // Approve, reject and cancel act on a whole mosaic: its tiles go together.
    let group = sh.requests.iter().find(|r| r.id == id).and_then(|r| r.mosaic.as_ref().map(|m| m.group));
    if let (Some(g), "approve" | "reject" | "cancel") = (group, action) {
        let ids: Vec<u64> = sh.requests.iter().filter(|r| r.mosaic.as_ref().is_some_and(|m| m.group == g)).map(|r| r.id).collect();
        let mut any = false;
        for t in ids {
            any |= apply_one(sh, t, action, priority, telescopes).is_ok();
        }
        return if any { Ok(()) } else { Err("not valid in this state") };
    }
    apply_one(sh, id, action, priority, telescopes)
}

fn apply_one(sh: &mut Shared, id: u64, action: &str, priority: Option<i32>, telescopes: Option<u32>) -> Result<(), &'static str> {
    let jd = sh.jd_now;
    let r = sh.requests.iter_mut().find(|r| r.id == id).ok_or("no such request")?;
    if let Some(p) = priority {
        r.priority = p.clamp(-100, 100);
    }
    if let Some(t) = telescopes {
        r.telescopes = t.clamp(1, 10_000);
    }
    match action {
        "approve" if r.state == ReqState::Pending => {
            r.state = ReqState::Approved;
            r.approved_jd = Some(jd);
        }
        "reject" if r.state.is_open() => r.state = ReqState::Rejected,
        "cancel" if r.state.is_open() => r.state = ReqState::Cancelled,
        "requeue" if !r.state.is_open() => {
            r.state = ReqState::Approved;
            r.progress_s = 0.0;
            r.completed_utc = None;
            r.hw = None;
            r.approved_jd = Some(jd);
            r.first_light_jd = None;
            r.full_lock_jd = None;
            r.jit2_s = 0.0;
            r.fine_s = 0.0;
            r.slews = 0;
            r.slew_deg_sum = 0.0;
        }
        "update" => {}
        "approve" | "reject" | "cancel" | "requeue" => return Err("not valid in this state"),
        _ => return Err("unknown action"),
    }
    if !r.state.is_schedulable() {
        r.assigned_now = 0;
    }
    Ok(())
}

fn control_loop(server: Server, shared: SharedRef) {
    for mut req in server.incoming_requests() {
        let path = path_of(req.url()).to_string();
        let method = req.method().clone();
        let token = shared.lock().unwrap().token.clone();
        match (method, path.as_str()) {
            (Method::Get, "/") => send(req, 200, "text/html; charset=utf-8", CONTROL_HTML.into()),
            (Method::Get, "/favicon.ico") => send_bytes(req, "image/x-icon", FAVICON.to_vec()),
            (Method::Get, p) if static_route(p, true).is_some() => {
                let (ct, body) = static_route(p, true).unwrap();
                send(req, 200, ct, body.into())
            }
            _ if !authorised(&req, &token) => err(req, 401, "missing or wrong token"),
            (Method::Get, "/api/stars") => {
                let s = shared.lock().unwrap().stars_json.clone();
                send(req, 200, "application/json", s)
            }
            (Method::Get, "/api/status") => {
                let s = status_with(&shared.lock().unwrap(), true, req.url());
                send(req, 200, "application/json", s)
            }
            (Method::Get, p) if p.starts_with("/api/result/") => {
                let sh = shared.clone();
                let p = p.to_string();
                std::thread::spawn(move || handle_result(sh, req, p));
            }
            (Method::Get, p) if p.starts_with("/api/mosaic/") => {
                let sh = shared.clone();
                let p = p.to_string();
                std::thread::spawn(move || handle_mosaic(sh, req, p));
            }
            (Method::Get, p) if p.starts_with("/api/campaign/") => {
                let sh = shared.clone();
                let p = p.to_string();
                std::thread::spawn(move || handle_campaign(sh, req, p));
            }
            (Method::Get, "/api/fleet") => {
                let s = shared.lock().unwrap().fleet_json.clone();
                send(req, 200, "application/json", s)
            }
            (Method::Get, "/api/coast") => {
                let s = shared.lock().unwrap().coast_json.clone();
                send(req, 200, "application/json", s)
            }
            (Method::Get, p) if p.starts_with("/api/sat/") => {
                let Ok(i) = p.trim_start_matches("/api/sat/").parse::<usize>() else {
                    err(req, 400, "bad satellite index");
                    continue;
                };
                let mut sh = shared.lock().unwrap();
                sh.inspect_req = Some(i);
                let cur: serde_json::Value = serde_json::from_str(&sh.inspect_json).unwrap_or_default();
                drop(sh);
                if cur.get("index").and_then(|v| v.as_u64()) == Some(i as u64) {
                    json(req, 200, cur)
                } else {
                    json(req, 202, serde_json::json!({ "pending": true }))
                }
            }
            (Method::Post, "/api/cmd") => {
                let Some(body) = read_body(&mut req) else {
                    err(req, 413, "request too large");
                    continue;
                };
                let cmd: Command = match serde_json::from_str(&body) {
                    Ok(c) => c,
                    Err(e) => {
                        err(req, 400, &format!("bad command: {e}"));
                        continue;
                    }
                };
                let mut sh = shared.lock().unwrap();
                let res = match &cmd {
                    Command::Request { id, action, priority, telescopes } => {
                        let r = apply_request_cmd(&mut sh, *id, action, *priority, *telescopes);
                        if r.is_ok() && action != "update" {
                            let past = match action.as_str() {
                                "approve" => "approved",
                                "reject" => "rejected",
                                "cancel" => "cancelled",
                                "requeue" => "re-queued",
                                _ => action.as_str(),
                            };
                            sh.log(if action == "approve" || action == "requeue" { "good" } else { "warn" }, format!("Director {past} request #{id}"));
                        } else if r.is_ok() {
                            sh.log("info", format!("Director updated request #{id} (priority / telescopes)"));
                        }
                        r
                    }
                    Command::AutoApprove { on } => {
                        sh.auto_approve = *on;
                        sh.log("info", format!("Auto-approval of public requests {}", if *on { "ON" } else { "OFF" }));
                        Ok(())
                    }
                    Command::Target { name: Some(n), .. } if sh.resolve(n).is_none() => Err("unknown target name"),
                    Command::StareStart { target, ra_deg, dec_deg, eyes, frame_s, duration_h } => {
                        start_campaign(&mut sh, target.as_deref(), *ra_deg, *dec_deg, *eyes, *frame_s, *duration_h)
                    }
                    Command::StareStop { id } => match sh.campaigns.iter_mut().find(|c| c.id == *id && c.active) {
                        Some(c) => {
                            c.active = false;
                            c.paused = false;
                            c.armed = c.preset; // built-ins return to "ready"
                            c.assigned_now = 0;
                            c.locked_now = 0;
                            let msg = format!(
                                "STARE #{} on {} stopped by director{}",
                                c.id,
                                c.target_name,
                                if c.preset { " (ready to restart)" } else { "" }
                            );
                            let utc = sh.utc_now.clone();
                            if let Some(c) = sh.campaigns.iter_mut().find(|c| c.id == *id) {
                                c.stopped_utc = Some(utc);
                            }
                            sh.log("info", msg);
                            Ok(())
                        }
                        None => Err("no such active campaign"),
                    },
                    Command::StareActivate { id } => activate_campaign(&mut sh, *id),
                    Command::StareActivateAll => {
                        let ids: Vec<u64> = sh.campaigns.iter().filter(|c| c.armed && !c.active).map(|c| c.id).collect();
                        for id in ids {
                            let _ = activate_campaign(&mut sh, id);
                        }
                        Ok(())
                    }
                    Command::StareEyes { id, eyes } => match sh.campaigns.iter_mut().find(|c| c.id == *id && (c.active || c.armed)) {
                        Some(c) => {
                            c.eyes = (*eyes).clamp(1, 5000);
                            let msg = format!("STARE #{} on {} → {} eyes", c.id, c.target_name, c.eyes);
                            sh.log("info", msg);
                            Ok(())
                        }
                        None => Err("no such active campaign"),
                    },
                    _ => {
                        let msg = match &cmd {
                            Command::Mode { mode } => format!("Programme mode → {mode}"),
                            Command::Target { name: Some(n), .. } => format!("Director target → {n}"),
                            Command::Target { ra_deg: Some(r), dec_deg: Some(d), .. } => format!("Director target → RA {r:.2}° Dec {d:+.2}°"),
                            Command::PublicShare { share } => format!("Public time share → {:.0} %", share * 100.0),
                            Command::SafeHold { sat, on } => format!("FACET-{:05} {}", sat + 1, if *on { "commanded to SAFE HOLD" } else { "returned to service" }),
                            _ => String::new(),
                        };
                        if !msg.is_empty() {
                            let lvl = if matches!(cmd, Command::SafeHold { on: true, .. }) { "warn" } else { "info" };
                            sh.log(lvl, msg);
                        }
                        sh.commands.push(cmd.clone());
                        Ok(())
                    }
                };
                drop(sh);
                match res {
                    Ok(()) => json(req, 200, serde_json::json!({ "ok": true })),
                    Err(e) => err(req, 400, e),
                }
            }
            _ => err(req, 404, "not found"),
        }
    }
}

fn public_loop(server: Server, shared: SharedRef) {
    for req in server.incoming_requests() {
        let path = path_of(req.url()).to_string();
        let method = req.method().clone();
        match (method, path.as_str()) {
            (Method::Get, "/") => send(req, 200, "text/html; charset=utf-8", PUBLIC_HTML.into()),
            (Method::Get, "/favicon.ico") => send_bytes(req, "image/x-icon", FAVICON.to_vec()),
            (Method::Get, p) if static_route(p, false).is_some() => {
                let (ct, body) = static_route(p, false).unwrap();
                send(req, 200, ct, body.into())
            }
            (Method::Get, "/api/stars") => {
                let s = shared.lock().unwrap().stars_json.clone();
                send(req, 200, "application/json", s)
            }
            (Method::Get, "/api/status") => {
                let s = status_with(&shared.lock().unwrap(), false, req.url());
                send(req, 200, "application/json", s)
            }
            (Method::Get, p) if p.starts_with("/api/result/") => {
                let sh = shared.clone();
                let p = p.to_string();
                std::thread::spawn(move || handle_result(sh, req, p));
            }
            (Method::Get, p) if p.starts_with("/api/mosaic/") => {
                let sh = shared.clone();
                let p = p.to_string();
                std::thread::spawn(move || handle_mosaic(sh, req, p));
            }
            (Method::Get, p) if p.starts_with("/api/campaign/") => {
                let sh = shared.clone();
                let p = p.to_string();
                std::thread::spawn(move || handle_campaign(sh, req, p));
            }
            (Method::Post, "/api/request") => handle_submit(&shared, req),
            _ => err(req, 404, "not found"),
        }
    }
}

/// Start both listeners. Failures (port in use, …) are recorded in `errors`
/// and shown in the app rather than aborting the simulator.
pub fn start(shared: &SharedRef, control_addr: Option<&str>, public_addr: Option<&str>) {
    let token = shared.lock().unwrap().token.clone();
    if let Some(addr) = control_addr {
        match Server::http(addr) {
            Ok(s) => {
                let url = format!("http://{}/?token={}", display_addr(addr), token);
                eprintln!("FACET director console: {url}");
                shared.lock().unwrap().control_url = Some(url);
                let sh = shared.clone();
                std::thread::Builder::new()
                    .name("facet-control".into())
                    .spawn(move || control_loop(s, sh))
                    .ok();
            }
            Err(e) => shared.lock().unwrap().errors.push(format!("control port {addr}: {e}")),
        }
    }
    if let Some(addr) = public_addr {
        match Server::http(addr) {
            Ok(s) => {
                let url = format!("http://{}/", display_addr(addr));
                eprintln!("FACET public access node: {url}");
                shared.lock().unwrap().public_url = Some(url);
                let sh = shared.clone();
                std::thread::Builder::new()
                    .name("facet-public".into())
                    .spawn(move || public_loop(s, sh))
                    .ok();
            }
            Err(e) => shared.lock().unwrap().errors.push(format!("public port {addr}: {e}")),
        }
    }
}

/// 0.0.0.0 isn't browsable; show localhost for a wildcard bind.
fn display_addr(addr: &str) -> String {
    addr.replace("0.0.0.0", "localhost").replace("[::]", "localhost")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "abcd"));
    }

    #[test]
    fn query() {
        assert_eq!(query_param("/api/status?token=xyz&a=1", "token").as_deref(), Some("xyz"));
        assert_eq!(query_param("/api/status", "token"), None);
    }

    #[test]
    fn commands_parse() {
        // The console must not be able to reshape the simulated universe.
        for sim_only in [r#"{"cmd":"shell","n":500}"#, r#"{"cmd":"time","rate":60}"#, r#"{"cmd":"telescope","fov_deg":2}"#] {
            assert!(serde_json::from_str::<Command>(sim_only).is_err(), "{sim_only}");
        }
        let c: Command = serde_json::from_str(r#"{"cmd":"safe_hold","sat":12,"on":true}"#).unwrap();
        assert!(matches!(c, Command::SafeHold { sat: 12, on: true }));
        let c: Command = serde_json::from_str(r#"{"cmd":"request","id":3,"action":"approve"}"#).unwrap();
        assert!(matches!(c, Command::Request { id: 3, .. }));
    }

    #[test]
    fn token_is_hex_32() {
        let t = random_token();
        assert_eq!(t.len(), 32);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
