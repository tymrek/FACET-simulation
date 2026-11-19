//! Glue between the simulator and the web server: start-up, applying director
//! commands, running the request queue, and publishing status snapshots.

use crate::app::{App, Target, FIXED_TARGETS};
use crate::astro::{self, BODY_NAMES};
use crate::math::{deg, rad, V3};
use crate::obs::{self, Mode, QueueItem};
use crate::server::{self, Command, ReqState, Resolved, Shared};
use egui::RichText;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}
fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

impl App {
    pub fn start_web(&mut self) {
        if self.web_control_addr.is_none() && self.web_public_addr.is_none() {
            return;
        }
        let mut names: HashMap<String, Resolved> = HashMap::new();
        for s in &self.names {
            names.insert(s.name.to_lowercase(), Resolved::Fixed(s.name.clone(), s.dir));
        }
        // Deep-sky objects by designation, Messier number and common name.
        for o in crate::catalog::dso() {
            let r = Resolved::Fixed(o.name(), o.dir);
            names.entry(o.id.to_lowercase()).or_insert_with(|| r.clone());
            names.entry(o.id.to_lowercase().replace(' ', "")).or_insert_with(|| r.clone());
            if let Some(m) = &o.messier {
                names.insert(m.to_lowercase(), r.clone());
            }
            if let Some(c) = &o.common {
                names.entry(c.to_lowercase()).or_insert_with(|| r.clone());
            }
        }
        for (name, ra, dec) in FIXED_TARGETS {
            let d = V3::from_radec(rad(ra), rad(dec));
            names.insert(name.to_lowercase(), Resolved::Fixed(name.into(), d));
            // Also accept the short catalogue designation, e.g. "m42".
            if let Some(code) = name.split('(').nth(1).map(|c| c.trim_end_matches(')')) {
                names.insert(code.to_lowercase(), Resolved::Fixed(name.into(), d));
            }
        }
        for (i, b) in BODY_NAMES.iter().enumerate().skip(1) {
            names.insert(b.to_lowercase(), Resolved::Body(i, (*b).into()));
        }
        let stars: Vec<[f64; 4]> = self
            .stars
            .iter()
            .take_while(|s| s.mag < 5.0)
            .map(|s| {
                let (ra, dec) = s.dir.to_radec();
                [r2(deg(ra)), r2(deg(dec)), r2(s.mag as f64), r2(s.ci as f64)]
            })
            .collect();
        let shared = Shared {
            status_json: "{}".into(),
            stars_json: serde_json::to_string(&stars).unwrap_or_else(|_| "[]".into()),
            commands: Vec::new(),
            requests: Vec::new(),
            next_id: 1,
            auto_approve: false,
            token: self.web_token.clone().unwrap_or_else(server::random_token),
            utc_now: astro::format_utc(self.jd),
            names,
            control_url: None,
            public_url: None,
            errors: Vec::new(),
            results: Default::default(),
            jd_now: self.jd,
            events: Default::default(),
            fleet_json: "{}".into(),
            inspect_req: None,
            inspect_json: "{}".into(),
            campaigns: Vec::new(),
            next_cid: 1,
            coast_json: {
                let lines: Vec<Vec<[f32; 2]>> = self
                    .coast
                    .iter()
                    .map(|l| l.iter().map(|&(lo, la)| [(lo * 10.0).round() / 10.0, (la * 10.0).round() / 10.0]).collect())
                    .collect();
                serde_json::to_string(&lines).unwrap_or_else(|_| "[]".into())
            },
        };
        let shared = Arc::new(Mutex::new(shared));
        server::start(
            &shared,
            self.web_control_addr.as_deref(),
            self.web_public_addr.as_deref(),
        );
        self.web = Some(shared);
    }

    pub fn apply_web_commands(&mut self) {
        let Some(web) = &self.web else { return };
        let (cmds, names_snapshot): (Vec<Command>, Option<Vec<(String, Resolved)>>) = {
            let mut sh = web.lock().unwrap();
            let cmds = std::mem::take(&mut sh.commands);
            // Resolve target names while we hold the lock.
            let res = cmds
                .iter()
                .filter_map(|c| match c {
                    Command::Target { name: Some(n), .. } => sh.resolve(n).map(|r| (n.clone(), r)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            (cmds, Some(res))
        };
        let resolved = names_snapshot.unwrap_or_default();
        for c in cmds {
            match c {
                Command::PublicShare { share } => {
                    if share.is_finite() {
                        self.public_share = share.clamp(0.0, 1.0);
                    }
                }
                Command::Mode { mode } => {
                    if let Some(m) = Mode::from_key(&mode) {
                        self.mode = m;
                    }
                }
                Command::Target { name, ra_deg, dec_deg } => {
                    if let Some(n) = name {
                        match resolved.iter().find(|(k, _)| *k == n).map(|(_, r)| r.clone()) {
                            Some(Resolved::Body(i, _)) => self.target = Some(Target::Body(i)),
                            Some(Resolved::Fixed(nm, d)) => self.target = Some(Target::Fixed { name: nm, dir: d }),
                            None => {}
                        }
                    } else if let (Some(ra), Some(dec)) = (ra_deg, dec_deg) {
                        if ra.is_finite() && (-90.0..=90.0).contains(&dec) {
                            self.target = Some(Target::Fixed {
                                name: format!("RA {:.2}° Dec {:+.2}°", ra.rem_euclid(360.0), dec),
                                dir: V3::from_radec(rad(ra), rad(dec)),
                            });
                        }
                    }
                }
                Command::SafeHold { sat, on } => {
                    if let Some(h) = self.obs.hold.get_mut(sat) {
                        *h = on;
                    }
                }
                // Handled in the server thread.
                Command::Request { .. }
                | Command::AutoApprove { .. }
                | Command::StareStart { .. }
                | Command::StareStop { .. }
                | Command::StareActivate { .. }
                | Command::StareActivateAll
                | Command::StareEyes { .. } => {}
            }
        }
    }

    /// STARE campaigns: keep their eyes on target with predictive handover.
    pub fn stare_assign(&mut self) {
        let Some(web) = self.web.clone() else { return };
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let ag = crate::attitude::Agility::from_obs(&self.obs_params);
        let o = &self.obs_params;
        let mut sh = web.lock().unwrap();
        let jd = self.jd;
        let mut ended = Vec::new();
        for c in sh.campaigns.iter_mut().filter(|c| c.active) {
            if let Some(h) = c.duration_h {
                if (jd - c.start_jd) * 24.0 >= h {
                    c.active = false;
                    c.assigned_now = 0;
                    c.locked_now = 0;
                    ended.push(format!("STARE #{} on {} completed its {} h campaign", c.id, c.target_name, h));
                    continue;
                }
            }
            if let Some(b) = c.body {
                c.dir = self.bodies[b].dir;
                let (ra, dec) = c.dir.to_radec();
                c.ra_deg = deg(ra);
                c.dec_deg = deg(dec);
            }
        }
        // Ready (armed) campaigns: keep their target position and keep-out
        // status current so the console can show "would start paused".
        for c in sh.campaigns.iter_mut().filter(|c| c.armed && !c.active) {
            if let Some(b) = c.body {
                c.dir = self.bodies[b].dir;
                let (ra, dec) = c.dir.to_radec();
                c.ra_deg = deg(ra);
                c.dec_deg = deg(dec);
            }
            c.blocked = obs::sky_blocked(o, c.dir, sun, moon).map(String::from);
        }
        let utc = sh.utc_now.clone();
        for m in ended {
            sh.log("good", m);
        }
        for c in sh.campaigns.iter_mut().filter(|c| !c.active && !c.armed && c.stopped_utc.is_none()) {
            c.stopped_utc = Some(utc.clone());
        }
        let idx: Vec<usize> = (0..sh.campaigns.len()).filter(|&i| sh.campaigns[i].active).collect();
        let items: Vec<obs::StareItem> = idx
            .iter()
            .map(|&i| {
                let c = &sh.campaigns[i];
                obs::StareItem {
                    id: c.id,
                    dir: c.dir,
                    eyes: c.eyes as usize,
                    fine: c.body.is_some() || crate::imaging::guide_star_available(c.dir, o.fov_deg, o.guide_mag),
                }
            })
            .collect();
        let res = obs::assign_stare(&mut self.obs, o, &self.shell, &items, sun, moon, &ag);
        for (r, &i) in res.iter().zip(&idx) {
            let c = &mut sh.campaigns[i];
            c.assigned_now = r.assigned as u32;
            c.blocked = r.blocked.map(String::from);
            if r.assigned > 0 && c.hw.is_none() {
                c.hw = Some(crate::imaging::Hardware::from_obs(o));
                c.agility = Some(ag);
            }
        }
    }

    /// After the attitude step: coverage statistics, integration and events.
    pub fn stare_credit(&mut self, dt: f64) {
        let Some(web) = self.web.clone() else { return };
        if dt <= 0.0 {
            return;
        }
        let ag = crate::attitude::Agility::from_obs(&self.obs_params);
        let mut sh = web.lock().unwrap();
        let mut events: Vec<(&str, String)> = Vec::new();
        let jd = self.jd;
        for c in sh.campaigns.iter_mut().filter(|c| c.active) {
            let me = obs::job(obs::TAG_STARE, c.id);
            let mut locked = 0u32;
            for (i, a) in self.obs.att.iter().enumerate() {
                if self.obs.job[i] != me {
                    continue;
                }
                if a.phase() == crate::attitude::Phase::Locked {
                    locked += 1;
                }
                let d = self.obs.obs_dt[i];
                if d > 0.0 {
                    let j = a.jitter(&ag);
                    c.integ_s += d;
                    c.jit2_s += d * j * j;
                    if a.fine {
                        c.fine_s += d;
                    }
                }
            }
            let recruits = self.obs.new_slews.iter().filter(|(_, j, _)| *j == me).count() as u32;
            if c.acquired {
                c.handovers += recruits;
            }
            c.locked_now = locked;
            c.last_jd = jd;
            let excluded = matches!(c.blocked.as_deref(), Some("Sun exclusion") | Some("Moon exclusion"));
            if excluded && !c.paused {
                c.paused = true;
                events.push((
                    "warn",
                    format!(
                        "STARE #{} on {}: PAUSED — target inside the {}; will resume automatically",
                        c.id,
                        c.target_name,
                        c.blocked.as_deref().unwrap_or("").to_lowercase()
                    ),
                ));
            } else if !excluded && c.paused {
                c.paused = false;
                events.push(("good", format!("STARE #{} on {}: target left the keep-out zone — resuming", c.id, c.target_name)));
            }
            if excluded {
                c.excluded_s += dt;
                c.acquired = false;
                c.cur_gap_s = 0.0;
                continue;
            }
            if !c.acquired {
                if locked > 0 {
                    c.acquired = true;
                    events.push(("good", format!("STARE #{} on {}: {} eye{} locked — sustained tracking", c.id, c.target_name, locked, if locked == 1 { "" } else { "s" })));
                } else {
                    continue;
                }
            }
            c.eligible_s += dt;
            if locked >= c.eyes {
                c.full_s += dt;
            }
            if locked > 0 {
                if c.cur_gap_s > 0.0 {
                    events.push(("good", format!("STARE #{} on {}: coverage restored after {:.0} s", c.id, c.target_name, c.cur_gap_s)));
                }
                c.any_s += dt;
                c.cur_gap_s = 0.0;
            } else {
                if c.cur_gap_s == 0.0 {
                    c.gaps += 1;
                    events.push(("warn", format!("STARE #{} on {}: COVERAGE GAP — no eyes locked", c.id, c.target_name)));
                }
                c.cur_gap_s += dt;
                c.max_gap_s = c.max_gap_s.max(c.cur_gap_s);
            }
        }
        for (l, m) in events {
            sh.log(l, m);
        }
    }

    /// Allocate telescopes to approved requests (pointing commands only).
    /// Lead before an event window opens (and before a member sets) for a
    /// replacement to slew 90°, settle and acquire, plus a minute's margin.
    pub fn window_prep_s(&self) -> f64 {
        crate::attitude::Agility::from_obs(&self.obs_params).repoint_time(std::f64::consts::FRAC_PI_2) + 60.0
    }

    pub fn queue_assign(&mut self) {
        let Some(web) = self.web.clone() else { return };
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let mut sh = web.lock().unwrap();
        let (jd, prep) = (self.jd, self.window_prep_s());
        let mut all: Vec<usize> = Vec::new();
        for i in 0..sh.requests.len() {
            let r = &mut sh.requests[i];
            if !r.state.is_schedulable() {
                continue;
            }
            match r.sched.waiting(jd, prep) {
                Some(why) => {
                    r.assigned_now = 0;
                    r.blocked = Some(why);
                }
                None => all.push(i),
            }
        }
        // Time-critical first: an open event window outranks everything but
        // planetary defence, because it cannot be observed later.
        let eff = |r: &crate::server::ObsRequest| if self.handover && r.sched.continuous(jd, prep) { r.priority.max(9) } else { r.priority };
        all.sort_by_key(|&i| (std::cmp::Reverse(eff(&sh.requests[i])), sh.requests[i].id));
        let n = self.shell.len();
        // Pool 1: public / director requests on the public time share (they
        // may pre-empt director tasks outside Science operations, as before).
        // Pool 2: the science programme, on every telescope still free. It
        // only takes idle or surveying telescopes, so in Stack/Mosaic it uses
        // the eyes beyond the target cap; in Survey and NEO-sweep modes the
        // director has given the whole fleet to the sweep.
        let pools: [(Vec<usize>, usize, bool, &'static str); 2] = [
            (
                all.iter().copied().filter(|&i| sh.requests[i].programme.is_none()).take(300).collect(),
                (self.public_share * n as f64).round() as usize,
                self.mode != Mode::Queue,
                "waiting: public share of telescopes fully booked",
            ),
            (
                all.iter().copied().filter(|&i| sh.requests[i].programme.is_some()).take((n / 2).max(400)).collect(),
                if matches!(self.mode, Mode::Survey | Mode::NeoSweep | Mode::AllSky) { 0 } else { usize::MAX },
                false,
                if matches!(self.mode, Mode::Survey | Mode::NeoSweep | Mode::AllSky) {
                    "paused: fleet dedicated to the director's sweep"
                } else {
                    "waiting: every telescope that can see it is busy"
                },
            ),
        ];
        for (order, budget, steal, pool) in pools {
        // Programme budget: what is left after public work, minus the
        // planetary-defence search reserve, which keeps surveying.
        let budget = if budget == usize::MAX {
            let used = self.obs.job.iter().filter(|&&j| obs::job_tag(j) == obs::TAG_QUEUE).count();
            let reserve = (self.search_share * n as f64).round() as usize;
            n.saturating_sub(used + reserve)
        } else {
            budget
        };
        let pool = if budget == 0 && pool.starts_with("waiting: every") { "waiting: fleet fully booked (search reserve held back)" } else { pool };
        let tb = std::time::Instant::now();
        let o = &self.obs_params;
        let items: Vec<QueueItem> = order
            .iter()
            .map(|&i| {
                let r = &mut sh.requests[i];
                if let Some(b) = r.body {
                    r.dir = self.bodies[b].dir;
                    let (ra, dec) = r.dir.to_radec();
                    r.ra_deg = deg(ra);
                    r.dec_deg = deg(dec);
                }
                let fine = r.body.is_some() || crate::imaging::guide_star_available(r.dir, o.fov_deg, o.guide_mag);
                let continuous = self.handover && r.sched.continuous(jd, prep);
                QueueItem { id: r.id, dir: r.dir, want: r.telescopes as usize, fine, continuous }
            })
            .collect();
        let ta = std::time::Instant::now();
        let results = obs::assign_queue(&mut self.obs, &self.obs_params, &items, sun, moon, budget, steal, pool, &self.shell, prep);
        if std::env::var_os("FACET_PROFILE").is_some() {
            eprintln!("    queue: build {:.1} ms, assign {:.1} ms ({} items)", (ta - tb).as_secs_f64() * 1e3, ta.elapsed().as_secs_f64() * 1e3, items.len());
        }
        for (res, &i) in results.iter().zip(&order) {
            let r = &mut sh.requests[i];
            r.assigned_now = res.assigned as u32;
            r.blocked = res.blocked.map(String::from);
            if res.blocked.is_none() && r.sched.win.map_or(false, |(a, _)| jd < a) {
                r.blocked = Some("settling in before the event window".into());
            }
            if res.assigned > 0 && r.hw.is_none() {
                r.hw = Some(crate::imaging::Hardware::from_obs(&self.obs_params));
                r.agility = Some(crate::attitude::Agility::from_obs(&self.obs_params));
            }
        }
        }
    }

    /// After the attitude step: credit locked integration to requests, track
    /// slews, jitter and first light.
    pub fn queue_credit(&mut self) {
        let Some(web) = self.web.clone() else { return };
        let ag = crate::attitude::Agility::from_obs(&self.obs_params);
        let mut per: HashMap<u64, (f64, f64, f64, u32)> = HashMap::new(); // id -> (s, jit²·s, fine s, locked sats)
        let mut longest: HashMap<u64, f64> = HashMap::new(); // id -> longest integration this step
        for i in 0..self.obs.att.len() {
            let j = self.obs.job[i];
            if obs::job_tag(j) != obs::TAG_QUEUE {
                continue;
            }
            let e = per.entry(obs::job_val(j)).or_default();
            let dt = self.obs.obs_dt[i];
            let a = &self.obs.att[i];
            if a.phase() == crate::attitude::Phase::Locked {
                e.3 += 1;
            }
            if dt > 0.0 {
                let m = longest.entry(obs::job_val(j)).or_insert(0.0);
                *m = m.max(dt);
                let jit = a.jitter(&ag);
                e.0 += dt;
                e.1 += dt * jit * jit;
                if a.fine {
                    e.2 += dt;
                }
            }
        }
        let mut slews: HashMap<u64, (u32, f64)> = HashMap::new();
        for &(_, j, ang) in &self.obs.new_slews {
            if obs::job_tag(j) == obs::TAG_QUEUE {
                let e = slews.entry(obs::job_val(j)).or_default();
                e.0 += 1;
                e.1 += ang.to_degrees();
            }
        }
        let mut sh = web.lock().unwrap();
        let utc = astro::format_utc(self.jd);
        let jd = self.jd;
        let mut events: Vec<(&str, String)> = Vec::new();
        for r in sh.requests.iter_mut().filter(|r| r.state.is_schedulable()) {
            if let Some(&(n, sum)) = slews.get(&r.id) {
                r.slews += n;
                r.slew_deg_sum += sum;
            }
            let (got, jit2, fine_s, locked) = per.get(&r.id).copied().unwrap_or_default();
            r.locked_now = locked;
            if let (Some(c), true) = (r.case_idx, got > 0.0) {
                self.science.cases[c].tel_s += got;
                if let Some(v) = r.inv_idx {
                    *self.science.inv(v).0 += got;
                }
            }
            let loud = r.programme.is_none();
            if got > 0.0 {
                if r.first_light_jd.is_none() {
                    // Integration began `longest` seconds before the end of the step.
                    let start = jd - longest.get(&r.id).copied().unwrap_or(0.0) / 86_400.0;
                    r.first_light_jd = Some(r.approved_jd.map_or(start, |a| start.max(a)));
                    if let (Some(c), false) = (r.case_idx, r.sched.win.is_some()) {
                        if let Some(t) = r.response_s() {
                            crate::programme::Science::push_capped(&mut self.science.cases[c].resp, t);
                        }
                    }
                    if loud {
                    events.push((
                        "good",
                        format!(
                            "Request #{} first light on {} ({} telescope{} locked{})",
                            r.id,
                            r.target_name,
                            locked,
                            if locked == 1 { "" } else { "s" },
                            r.response_s().map(|t| format!(", +{t:.0} s after approval")).unwrap_or_default()
                        ),
                    ));
                    }
                }
                r.obs_jd = jd;
                let mut got = got;
                if let crate::programme::Need::Cadence { visits, spacing_s } = r.sched.need {
                    // Credit only up to this visit's share; then wait.
                    let per = r.required_s() / visits as f64;
                    got = got.min(per - r.sched.visit_s).max(0.0);
                    r.sched.visit_s += got;
                    if r.sched.visit_s >= per - 1e-6 && r.sched.visits_done + 1 < visits {
                        r.sched.visits_done += 1;
                        r.sched.visit_s = 0.0;
                        r.sched.next_visit_jd = jd + spacing_s / 86_400.0;
                    }
                }
                if let Some((a, _)) = r.sched.win {
                    if jd < a {
                        got = 0.0; // settling in: nothing useful before the event
                    }
                }
                r.progress_s += got;
                r.jit2_s += jit2;
                r.fine_s += fine_s;
                let windowed = r.sched.win.is_some();
                if !windowed && r.progress_s >= r.required_s() - 1e-3 {
                    r.progress_s = r.required_s();
                    r.state = ReqState::Done;
                    r.assigned_now = 0;
                    r.locked_now = 0;
                    r.completed_utc = Some(utc.clone());
                    if let Some(c) = r.case_idx {
                        let age = (jd - r.submit_jd) * 86_400.0;
                        let cs = &mut self.science.cases[c];
                        cs.completed += 1;
                        if r.deadline_s.map_or(true, |d| age <= d) {
                            cs.on_time += 1;
                        }
                        if let Some(hw) = &r.hw {
                            let sub = r.exposure_s.min(hw.sub_max_s);
                            let lim = crate::imaging::point_lim_mag(hw, r.progress_s, sub, crate::imaging::sky_sb(r.dir, self.bodies[0].dir, &self.obs_params));
                            crate::programme::Science::push_capped(&mut cs.depth, lim);
                        }
                        if let Some(v) = r.inv_idx {
                            *self.science.inv(v).1 += 1;
                        }
                    }
                    if loud {
                        events.push((
                            "good",
                            format!("Request #{} complete: {} — {:.0} telescope-seconds, {} repoints", r.id, r.target_name, r.progress_s, r.slews),
                        ));
                    }
                    continue;
                }
            }
            if let (Some((a, b)), crate::programme::Need::Window { min_cover, max_gap_s, .. }) = (r.sched.win, r.sched.need) {
                let t0 = r.sched.last_jd.max(a);
                let t1 = jd.min(b);
                if t1 > t0 {
                    let span = (t1 - t0) * 86_400.0;
                    if locked >= r.telescopes && got > 0.0 {
                        r.sched.first_cover_jd.get_or_insert(t0);
                        r.sched.cover_s += span;
                        r.sched.gap_s = 0.0;
                    } else {
                        r.sched.gap_s += span;
                        r.sched.gap_max_s = r.sched.gap_max_s.max(r.sched.gap_s);
                    }
                }
                r.sched.last_jd = jd;
                let len = (b - a) * 86_400.0;
                r.progress_s = r.progress_s.min(r.required_s());
                if jd >= b {
                    let cover = r.sched.cover_s / len;
                    let ok = cover >= min_cover && r.sched.gap_max_s <= max_gap_s;
                    r.assigned_now = 0;
                    r.locked_now = 0;
                    r.completed_utc = Some(utc.clone());
                    if let Some(c) = r.case_idx {
                        let cs = &mut self.science.cases[c];
                        // Response for an event window: how late coverage began.
                        let late = r.sched.first_cover_jd.map_or(len, |f| (f - a) * 86_400.0);
                        crate::programme::Science::push_capped(&mut cs.resp, late);
                        if ok {
                            cs.completed += 1;
                            cs.on_time += 1;
                            if let Some(hw) = &r.hw {
                                let sub = r.exposure_s.min(hw.sub_max_s);
                                let sky = crate::imaging::sky_sb(r.dir, self.bodies[0].dir, &self.obs_params);
                                let lim = crate::imaging::point_lim_mag(hw, r.progress_s, sub, sky);
                                crate::programme::Science::push_capped(&mut cs.depth, lim);
                                if r.sched.host_v > 0.0 {
                                    // Short sub-frames for bright hosts, so the peak pixel stays unsaturated.
                                    let sub = crate::imaging::unsaturated_sub(hw, r.sched.host_v, 10.0);
                                    let p = crate::imaging::precision(hw, r.sched.host_v, 3600.0, sub, sky, r.telescopes as usize);
                                    crate::programme::Science::push_capped(&mut cs.prec, p.total_ppm);
                                }
                            }
                            if let Some(v) = r.inv_idx {
                                *self.science.inv(v).1 += 1;
                            }
                        } else {
                            cs.failed += 1;
                        }
                    }
                    if ok {
                        r.state = ReqState::Done;
                    } else {
                        r.state = ReqState::Failed;
                        r.blocked = Some(format!(
                            "requirement not met: {:.1} % of the window covered, longest gap {}",
                            100.0 * cover,
                            crate::allsky::fmt_span(r.sched.gap_max_s)
                        ));
                    }
                    continue;
                }
            }
            if r.full_lock_jd.is_none() && r.first_light_jd.is_some() && locked >= r.telescopes {
                r.full_lock_jd = Some(jd);
                if let (Some(t), true) = (r.full_lock_s(), loud) {
                    events.push(("good", format!("Request #{} at full strength: {} telescopes locked (+{:.0} s)", r.id, locked, t)));
                }
            }
            r.state = if r.assigned_now > 0 { ReqState::Observing } else { ReqState::Approved };
        }
        // Programme deadlines: late data are worthless for these cases.
        let mut expired_n = 0;
        for r in sh.requests.iter_mut().filter(|r| r.state.is_open()) {
            if let (Some(d), Some(c)) = (r.deadline_s, r.case_idx) {
                if (jd - r.submit_jd) * 86_400.0 > d {
                    r.state = ReqState::Expired;
                    r.assigned_now = 0;
                    r.locked_now = 0;
                    r.completed_utc = Some(utc.clone());
                    self.science.cases[c].expired += 1;
                    expired_n += 1;
                    if r.priority >= 9 {
                        events.push(("warn", format!("Request #{} ({}) EXPIRED before completion: {}", r.id, crate::programme::CASES[c].label, r.target_name)));
                    }
                }
            }
        }
        let _ = expired_n;
        for (lvl, m) in events {
            sh.log(lvl, m);
        }
    }

    /// Science programme: investigators submit work in proportion to the
    /// fleet's capacity (`load`).
    pub fn programme_tick(&mut self, dt_sim: f64) {
        let Some(web) = self.web.clone() else { return };
        if self.science.since_utc.is_empty() {
            self.science = crate::programme::Science::new(astro::format_utc(self.jd));
            self.env.reset_stats();
            self.public.reset();
        }
        if dt_sim <= 0.0 {
            return;
        }
        let mut sh = web.lock().unwrap();
        // Open-request caps scale with the fleet; each case gets a share in
        // proportion to its weight (×2 headroom, at least 40).
        let cap = (1.5 * self.shell.len() as f64).max(1200.0) as usize;
        let mut per_case = vec![0usize; crate::programme::CASES.len()];
        let mut open = 0;
        for r in sh.requests.iter().filter(|r| r.state.is_open() && r.programme.is_some()) {
            open += 1;
            if let Some(c) = r.case_idx {
                per_case[c] += 1;
            }
        }
        let room = cap.saturating_sub(open);
        let wsum: f64 = crate::programme::CASES.iter().map(|c| c.weight()).sum();
        let mut case_room: Vec<usize> = crate::programme::CASES
            .iter()
            .enumerate()
            .map(|(k, c)| ((cap as f64 * c.weight() / wsum * 2.0).max(40.0) as usize).saturating_sub(per_case[k]))
            .collect();
        let planets: Vec<V3> = self.bodies.iter().map(|b| b.dir).collect();
        let drafts = self.gen.tick(
            dt_sim,
            self.shell.len(),
            room,
            &mut case_room,
            self.bodies[0].dir,
            self.obs_params.sun_excl_deg,
            self.bodies[1].dir,
            self.obs_params.moon_excl_deg,
            &planets,
        );
        // The attitude step that follows covers (jd − dt, jd]: treat this
        // step's arrivals as arriving at its start.
        let t0 = self.jd - dt_sim / 86_400.0;
        if drafts.is_empty() {
            return;
        }
        let utc = astro::format_utc(self.jd);
        for d in drafts {
            let c = &crate::programme::CASES[d.case];
            let id = sh.next_id;
            sh.next_id += 1;
            let (ra, dec) = d.dir.to_radec();
            let inv = self.gen.investigators[d.inv].name.clone();
            if d.priority >= 9 && self.rate <= 60.0 {
                sh.log("warn", format!("ToO request #{id} from {inv}: {} — {} × {:.0} s, priority {}", d.target_name, d.telescopes, d.exposure_s, d.priority));
            }
            self.science.cases[d.case].submitted += 1;
            let mut sched = crate::server::Sched { need: c.need, ..Default::default() };
            let mut deadline_s = c.deadline_h * 3600.0;
            if let crate::programme::Need::Window { lead_h: (lo, hi), .. } = c.need {
                // Predicted event time: uniform in the case's lead range.
                let u = (id.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 11) as f64 / (1u64 << 53) as f64;
                let a = t0 + (lo + u * (hi - lo)) / 24.0;
                let b = a + d.exposure_s / 86_400.0;
                sched.win = Some((a, b));
                if c.key == "transit" {
                    // ASSUMED host brightness: V 8–13, typical of ground follow-up targets.
                    let u2 = (id.wrapping_mul(0xD1B5_4A32_D192_ED03) >> 11) as f64 / (1u64 << 53) as f64;
                    sched.host_v = 8.0 + 5.0 * u2;
                }
                deadline_s = (b - t0) * 86_400.0 + 600.0;
            }
            sh.requests.push(crate::server::ObsRequest {
                id,
                submitter: inv,
                target_name: d.target_name,
                body: d.body,
                ra_deg: deg(ra),
                dec_deg: deg(dec),
                telescopes: d.telescopes,
                exposure_s: d.exposure_s,
                priority: d.priority,
                state: ReqState::Approved,
                progress_s: 0.0,
                assigned_now: 0,
                blocked: None,
                submitted_utc: utc.clone(),
                completed_utc: None,
                note: c.label.to_string(),
                ip: String::new(),
                dir: d.dir,
                hw: None,
                obs_jd: 0.0,
                locked_now: 0,
                approved_jd: Some(t0),
                first_light_jd: None,
                full_lock_jd: None,
                jit2_s: 0.0,
                fine_s: 0.0,
                slews: 0,
                slew_deg_sum: 0.0,
                agility: None,
                sched,
                programme: Some(c.key),
                deadline_s: Some(deadline_s),
                submit_jd: t0,
                case_idx: Some(d.case),
                inv_idx: Some(d.inv),
            });
        }
        sh.prune();
    }

    /// Fleet time accounting for the science-return figures.
    pub fn science_account(&mut self, dt: f64) {
        if dt <= 0.0 || self.science.since_utc.is_empty() {
            return;
        }
        let o = &self.obs_params;
        let prog: std::collections::HashSet<u64> = match &self.web {
            Some(w) => w.lock().unwrap().requests.iter().filter(|r| r.programme.is_some() && r.state.is_open()).map(|r| r.id).collect(),
            None => Default::default(),
        };
        let area_m2 = std::f64::consts::PI * (o.aperture_cm / 200.0).powi(2) * (1.0 - o.obstruction * o.obstruction);
        let omega = o.fov_deg * o.fov_deg;
        let sub = o.exp_s.min(o.sub_max_s).max(1.0);
        let frame_bytes = o.sensor_px * o.sensor_py * 2.0; // 16-bit
        // Streak avoidance gates exposures around predicted crossings: that
        // time is not integrated.
        let gate = if self.traffic.cfg.streak_avoidance && self.traffic.streaks.exposed_s > 0.0 {
            1.0 - self.traffic.streaks.lost_s / self.traffic.streaks.exposed_s
        } else {
            1.0
        };
        let s = &mut self.science;
        let env = &self.env;
        let safed = env.cfg.saa_policy == crate::environment::SaaPolicy::Pause;
        s.elapsed_s += dt;
        let n = self.obs.att.len();
        s.sat_s += n as f64 * dt;
        for i in 0..n {
            if self.obs.hold.get(i).copied().unwrap_or(false) {
                s.hold_s += dt;
                continue;
            }
            if safed && env.in_saa.get(i).copied().unwrap_or(false) {
                s.saa_s += dt;
                continue;
            }
            let j = self.obs.job[i];
            if j == 0 {
                s.idle_s += dt;
                continue;
            }
            // obs_dt is dark-sky-equivalent time; recover the raw exposure.
            let shine = env.shine.get(i).copied().unwrap_or(1.0).max(1e-3) as f64;
            let got = self.obs.obs_dt[i].clamp(0.0, dt) * gate;
            let raw = (got / shine).min(dt);
            s.integrating_s += raw;
            s.earthshine_s += raw - got;
            s.overhead_s += dt - raw;
            if got <= 0.0 {
                continue;
            }
            let u = &mut s.by_use;
            match obs::job_tag(j) {
                obs::TAG_QUEUE if prog.contains(&obs::job_val(j)) => u.programme += got,
                obs::TAG_QUEUE => u.public += got,
                obs::TAG_STARE => u.stare += got,
                obs::TAG_STACK | obs::TAG_MOSAIC => u.director += got,
                obs::TAG_NEO => u.neo_sweep += got,
                _ => u.survey += got,
            }
            s.etendue_s += area_m2 * omega * got;
            s.frames += raw / sub;
            s.bytes += raw / sub * frame_bytes;
        }
    }

    /// Planetary-defence exercise: advance it, task follow-up eyes, log events.
    pub fn defence_step(&mut self, dt: f64) {
        let Some(sc) = self.defence.as_mut() else { return };
        let search = self.obs.job.iter().filter(|&&j| matches!(obs::job_tag(j), obs::TAG_SURVEY | obs::TAG_NEO | obs::TAG_ALLSKY)).count();
        let mut locked = 0;
        if let (Some(id), Some(w)) = (sc.campaign, &self.web) {
            if let Some(c) = w.lock().unwrap().campaigns.iter().find(|c| c.id == id && c.active) {
                locked = c.locked_now;
            }
        }
        let hw = crate::imaging::Hardware::from_obs(&self.obs_params);
        let inp = crate::defence::StepIn {
            jd: self.jd,
            dt_s: dt,
            sun: self.bodies[0].dir,
            moon: self.bodies[1].dir,
            o: &self.obs_params,
            hw: &hw,
            search_eyes: search,
            locked_eyes: locked,
        };
        sc.step(&inp);
        let Some(web) = self.web.clone() else {
            sc.events.clear();
            return;
        };
        let mut sh = web.lock().unwrap();
        if sc.discovered() && sc.campaign.is_none() && !sc.impacted {
            let (ra, dec) = (sc.now.ra_deg, sc.now.dec_deg);
            if crate::server::start_campaign(&mut sh, None, Some(ra), Some(dec), Some(sc.cfg.eyes), Some(60.0), None).is_ok() {
                let id = sh.next_cid - 1;
                if let Some(c) = sh.campaigns.iter_mut().find(|c| c.id == id) {
                    c.target_name = sc.designation.clone();
                }
                sc.campaign = Some(id);
            }
        }
        if let Some(id) = sc.campaign {
            if let Some(c) = sh.campaigns.iter_mut().find(|c| c.id == id) {
                if sc.impacted {
                    if c.active {
                        c.active = false;
                        c.stopped_utc = Some(sh_utc(self.jd));
                        c.assigned_now = 0;
                        c.locked_now = 0;
                    }
                } else {
                    c.dir = V3::from_radec(sc.now.ra_deg.to_radians(), sc.now.dec_deg.to_radians());
                    c.ra_deg = sc.now.ra_deg;
                    c.dec_deg = sc.now.dec_deg;
                }
            }
        }
        for (lvl, m) in sc.events.drain(..) {
            sh.log(lvl, m);
        }
    }

    pub fn defence_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| { ui.label(RichText::new("Planetary defence exercise").strong()); crate::provenance::chip(ui, "defence"); });
        let mut sr = self.search_share * 100.0;
        ui.add(egui::Slider::new(&mut sr, 0.0..=50.0).text("% of fleet held for NEO search"))
            .on_hover_text("Telescopes the science programme may not use; they keep surveying and can discover new objects.");
        self.search_share = sr / 100.0;
        let presets = crate::defence::presets();
        if let Some(sc) = &self.defence {
            let t = (sc.impact_jd - self.jd) * 86_400.0;
            ui.label(RichText::new(format!("{} · {} · {:.1} Mt", sc.designation, sc.cfg.name, sc.energy_mt)).small());
            ui.label(RichText::new(if sc.impacted { "Impact has occurred.".to_string() } else { format!("Impact in {} · V = {:.1} · {:.0} km away", crate::defence::fmt_dur(t), sc.now.v_mag, sc.now.dist_km) }).small());
            for s in sc.systems() {
                ui.label(RichText::new(format!(
                    "{}: {} · IP {}",
                    s.name,
                    s.warning_s.map(|w| format!("found {} before impact", crate::defence::fmt_dur(w))).unwrap_or_else(|| "not found".into()),
                    s.ip.map(|p| format!("{:.3} %", p * 100.0)).unwrap_or_else(|| "-".into())
                )).small());
            }
            if ui.button("End exercise").clicked() {
                if let (Some(id), Some(w)) = (sc.campaign, &self.web) {
                    let mut sh = w.lock().unwrap();
                    if let Some(c) = sh.campaigns.iter_mut().find(|c| c.id == id) {
                        c.active = false;
                    }
                    sh.log("info", "Planetary defence exercise ended".to_string());
                }
                self.defence = None;
            }
            return;
        }
        egui::ComboBox::from_id_source("pd_preset")
            .selected_text(presets[self.pd_preset.min(presets.len() - 1)].name.clone())
            .width(300.0)
            .show_ui(ui, |ui| {
                for (i, p) in presets.iter().enumerate() {
                    if ui.selectable_label(self.pd_preset == i, p.name.clone()).clicked() {
                        self.pd_preset = i;
                        self.pd_cfg = p.clone();
                    }
                }
            });
        let c = &mut self.pd_cfg;
        ui.add(egui::Slider::new(&mut c.diameter_m, 5.0..=1000.0).logarithmic(true).text("diameter (m)"));
        ui.add(egui::Slider::new(&mut c.days, 0.5..=120.0).logarithmic(true).text("days to impact"));
        ui.add(egui::Slider::new(&mut c.v_inf_kms, 3.0..=40.0).text("v-infinity (km/s)"));
        ui.add(egui::Slider::new(&mut c.radiant_elong_deg, 20.0..=180.0).text("radiant elongation from Sun (deg)"))
            .on_hover_text("Below ~60° it comes out of the daytime sky, where ground surveys cannot look.");
        ui.add(egui::Slider::new(&mut c.eyes, 1..=500).logarithmic(true).text("follow-up eyes"));
        if ui.button("Start exercise").clicked() {
            let mut cfg = c.clone();
            cfg.name = if presets.iter().any(|p| p.name == cfg.name && p.diameter_m == cfg.diameter_m && p.days == cfg.days) { cfg.name } else { "Custom scenario".into() };
            self.start_defence(cfg);
        }
    }

    pub fn start_defence(&mut self, cfg: crate::defence::Config) {
        let sc = crate::defence::Scenario::new(cfg, self.jd);
        // About two and a half minutes of wall time for the whole exercise.
        self.rate = (sc.cfg.days * 86_400.0 / 150.0).clamp(1.0, 86_400.0).round();
        self.playing = true;
        if let Some(w) = &self.web {
            let mut sh = w.lock().unwrap();
            sh.log(
                "warn",
                format!(
                    "PLANETARY DEFENCE EXERCISE started: undiscovered {:.0} m object on an impact course (simulated). Impact {}.",
                    sc.cfg.diameter_m,
                    crate::astro::format_utc(sc.impact_jd)
                ),
            );
        }
        self.defence = Some(sc);
    }

    /// Space traffic: propagate the other populations, sample streaks,
    /// screen conjunctions, and pass notices to the event log.
    pub fn traffic_step(&mut self, dt: f64) {
        let o = &self.obs_params;
        let n = self.shell.len();
        let bs: Vec<Option<V3>> = (0..n)
            .map(|i| {
                let a = &self.obs.att[i];
                (a.job != 0 && !self.obs.hold.get(i).copied().unwrap_or(false) && matches!(a.phase(), crate::attitude::Phase::Locked)).then_some(a.dir)
            })
            .collect();
        let f = crate::traffic::FacetState {
            jd: self.jd,
            dt_s: dt,
            sun: self.bodies[0].dir,
            alt_km: self.shell.params.alt_km,
            inc: self.shell.params.inc_deg.to_radians(),
            raan: &self.shell.raan_now,
            u: &self.shell.u_now,
            pos: &self.shell.pos,
            boresight: &bs,
            fov_deg: o.fov_deg,
            sub_s: o.exp_s.min(o.sub_max_s),
        };
        self.traffic.step(&f);
        self.traffic.dtg_sample(self.jd, self.shell.params.alt_km, f.inc, &self.shell.raan_now, &self.shell.u_now);
        if let Some(w) = &self.web {
            let mut sh = w.lock().unwrap();
            for (l, m) in self.traffic.events.drain(..) {
                sh.log(l, m);
            }
        } else {
            self.traffic.events.clear();
        }
    }

    pub fn traffic_panel(&mut self, ui: &mut egui::Ui) {
        use crate::traffic::Product;
        ui.label(RichText::new("Space traffic & data path").strong());
        let c = &mut self.traffic.cfg;
        ui.checkbox(&mut c.show, "Show other satellites and debris");
        ui.add(egui::Slider::new(&mut c.starlink_n, 0..=40_000).logarithmic(true).text("Starlink satellites"));
        ui.add(egui::Slider::new(&mut c.starlink_alt_km, 300.0..=600.0).text("Starlink altitude (km)"));
        ui.checkbox(&mut c.oneweb, "OneWeb (648 at 1,200 km)");
        ui.add(egui::Slider::new(&mut c.other_n, 0..=10_000).logarithmic(true).text("other objects near FACET"))
            .on_hover_text("Debris, other operators and small satellites within ± the band of FACET's altitude (assumed population).");
        ui.checkbox(&mut c.streak_avoidance, "Streak avoidance (gate exposures around predicted crossings)");
        ui.checkbox(&mut c.relay, "Starlink optical relay");
        ui.add(egui::Slider::new(&mut c.relay_gbps, 0.1..=100.0).logarithmic(true).text("relay terminal Gb/s"));
        ui.add(egui::Slider::new(&mut c.stations, 1..=24).text("ground stations"));
        ui.add(egui::Slider::new(&mut c.station_gbps, 0.1..=10.0).logarithmic(true).text("station link Gb/s"));
        egui::ComboBox::from_label("downlinked product").selected_text(c.product.label()).width(260.0).show_ui(ui, |ui| {
            for p in Product::ALL {
                ui.selectable_value(&mut c.product, p, p.label());
            }
        });
        let raw = self.science.bytes / 1e12 / (self.science.elapsed_s / 86_400.0).max(1e-9);
        let j = self.traffic.json(raw, self.shell.len());
        let d = &j["data"];
        let f = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0);
        crate::provenance::line(ui, "downlink", format!(
            "Demand {:.1} TB/day ({}) · relay {:.0} TB/day · ground {:.0} TB/day",
            f(&d["demand_tb_day"]), self.traffic.cfg.product.label(), f(&d["relay_cap_tb_day"]), f(&d["dtg_cap_tb_day"])
        ));
        let s = &j["streaks"];
        crate::provenance::line(ui, "streaks", format!(
            "Streaked frames without avoidance {:.2} % · avoidance costs {:.3} % of integration",
            f(&s["streaked_pct"]), f(&s["avoidance_cost_pct"])
        ));
        let cj = &j["conjunctions"]["per_day"];
        crate::provenance::line(ui, "conjunctions", format!(
            "Notices {:.1}/day · high-risk {:.2}/day · manoeuvres {:.2}/day",
            f(&cj["notices"]), f(&cj["high_risk"]), f(&cj["manoeuvres"])
        ));
    }

    pub fn science_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Science programme (simulated investigators)").strong());
        ui.checkbox(&mut self.gen.on, "Investigators submit requests");
        ui.add(egui::Slider::new(&mut self.gen.load, 0.1..=3.0).text("demand × fleet capacity"))
            .on_hover_text("1.0 = work arrives as fast as the whole fleet could integrate it. Above 1 the facility is oversubscribed.");
        let mut ninv = self.gen.investigators.len();
        if ui.add(egui::Slider::new(&mut ninv, 1..=500).logarithmic(true).text("investigator teams")).changed() {
            self.gen.set_investigators(ninv);
        }
        if matches!(self.mode, Mode::Survey | Mode::NeoSweep | Mode::AllSky) {
            ui.label(RichText::new("Paused: Survey, NEO-sweep and all-sky modes dedicate the fleet. Use Science operations.").small().color(egui::Color32::from_rgb(255, 180, 60)));
        }
        let s = self.science.summary(&self.gen.investigators);
        let e = &s["efficiency"];
        let d = &s["per_day"];
        let f = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0);
        crate::provenance::line(ui, "duty", format!(
            "Open shutter {:.1} % ({:.1} % pointed science) · overhead {:.1} % · idle {:.1} %",
            f(&e["integrating_pct"]), f(&e["pointed_science_pct"]), f(&e["overhead_pct"]), f(&e["idle_pct"])
        ));
        crate::provenance::line(ui, "science", format!(
            "Per day: {:.0} requests done · {:.0} telescope-h · {:.0} m²·deg²·h étendue-time · {:.1} TB raw",
            f(&d["completed"]), f(&d["tel_h"]), f(&d["etendue_m2deg2h"]), f(&d["tb"])
        ));
        crate::provenance::line(ui, "science", format!(
            "On-time {} · {} done, {} expired · {}/{} teams served",
            s["on_time_pct"].as_f64().map(|p| format!("{p:.1} %")).unwrap_or_else(|| "-".into()),
            s["completed"], s["expired"], s["investigators_served"], s["investigators"]
        ));
    }

    pub fn publish_status(&mut self, now: Instant) {
        let Some(web) = self.web.clone() else { return };
        if self.last_publish.map_or(false, |t| now.duration_since(t).as_secs_f64() < 0.25) {
            return;
        }
        self.last_publish = Some(now);
        let sp = &self.shell_params;
        let o = &self.obs_params;
        let n = self.shell.len();
        // Footprints at the ACTUAL boresight: 0 locked & eclipsed, 1 locked & sunlit, 2 slewing/settling.
        let tasked = self.obs.att.iter().filter(|a| a.job != 0).count();
        let stride = (tasked / 1500).max(1);
        let mut fps = Vec::new();
        let mut seen = 0usize;
        for (i, a) in self.obs.att.iter().enumerate() {
            if a.job == 0 {
                continue;
            }
            let locked = a.phase() == crate::attitude::Phase::Locked;
            if self.mode == Mode::Stack && locked && fps.iter().any(|f: &serde_json::Value| f[2] != 2) {
                continue;
            }
            if seen % stride == 0 {
                let (ra, dec) = a.dir.to_radec();
                let code = if !locked { 2 } else { self.obs.lit[i] as u8 };
                fps.push(serde_json::json!([r2(deg(ra)), r2(deg(dec)), code]));
            }
            seen += 1;
        }
        let bodies: Vec<serde_json::Value> = self
            .bodies
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let (ra, dec) = b.dir.to_radec();
                serde_json::json!({ "name": BODY_NAMES[i], "ra": r2(deg(ra)), "dec": r2(deg(dec)) })
            })
            .collect();
        let target = self.target_dir().map(|t| {
            let (ra, dec) = t.to_radec();
            serde_json::json!({ "name": self.target_name(), "ra": r2(deg(ra)), "dec": r2(deg(dec)),
                                "visible": self.obs.n_target_visible, "locked": self.obs.n_locked_target,
                                "blocked": self.obs.target_blocked_by })
        });
        let m = obs::metrics(o, self.obs.n_locked_target, self.obs.max_baseline_km, 0);
        let ag = crate::attitude::Agility::from_obs(o);
        let science = {
            let mut s = self.science.summary(&self.gen.investigators);
            let sh = web.lock().unwrap();
            if let Some(cs) = s["cases"].as_array_mut() {
                for c in cs {
                    let key = c["key"].as_str().unwrap_or("").to_string();
                    let open = sh.requests.iter().filter(|r| r.state.is_open() && r.programme == Some(key.as_str()) ).count();
                    c["open"] = serde_json::json!(open);
                }
            }
            s
        };
        let traffic_json = {
            let raw = self.science.bytes / 1e12 / (self.science.elapsed_s / 86_400.0).max(1e-9);
            self.traffic.json(raw, n)
        };
        let costs_json = {
            let r = self.cost_result();
            serde_json::json!({
                "scenario": self.cost_model.name, "fleet": r.fleet, "total": r.total,
                "first_year": self.cost_model.first_year, "horizon_years": self.cost_model.horizon_years,
                "first_science_year": r.first_science_year, "full_fleet_year": r.full_fleet_year,
                "steady_annual": r.steady_annual, "avg_unit_cost": r.avg_unit_cost,
                "per_tel_hour": r.cost_per_tel_hour, "per_observation": r.cost_per_observation,
                "per_etendue_hour": r.cost_per_etendue_hour, "per_tb": r.cost_per_tb,
            })
        };
        let status = serde_json::json!({
            "costs": costs_json,
            "resilience": self.resilience_json(),
            "environment": self.env.json(self.shell.params.alt_km),
            "allsky": self.allsky_json(),
            "survey": self.survey_json(),
            "neos": self.neo_json(),
            "public": self.public_json(),
            "interferometry": self.ii_json(),
            "fuel": self.fuel_budget(),
            "provenance": crate::provenance::json(),
            "photometry": crate::imaging::precision_json(&self.obs_params),
            "fleet_optics": crate::optics::fleet_spread(&self.obs_params),
            "optics": {
                "budget": crate::optics::blur_budget(&self.obs_params, self.obs_params.exp_s.min(self.obs_params.sub_max_s)),
                "focal_mm": self.obs_params.focal_mm,
                "plate_scale": self.obs_params.plate_scale(),
                "field_deg": self.obs_params.field_deg(),
                "sensor": crate::obs::SENSORS.get(self.obs_params.sensor_preset).map(|s| s.name),
                "design": crate::obs::DESIGNS.get(self.obs_params.design_preset).map(|d| d.name),
                "pointing": crate::obs::POINTING.get(self.obs_params.pointing_preset).map(|p| p.1),
            },
            "ground": self.resil.ground,
            "utc": astro::format_utc(self.jd),
            "jd": self.jd,
            "playing": self.playing,
            "rate": self.rate,
            "mode": self.mode.key(),
            "public_share": self.public_share,
            "defence": self.defence.as_ref().map(|d| d.json(self.jd)),
            "traffic": traffic_json,
            "programme": {
                "search_share": self.search_share,
                "on": self.gen.on, "load": self.gen.load, "investigators": self.gen.investigators.len(),
                "active": self.gen.on && !matches!(self.mode, Mode::Survey | Mode::NeoSweep | Mode::AllSky),
            },
            "science": science,
            "mode_label": self.mode.label(),
            "shell": {
                "n": n, "planes": sp.effective_planes(), "planes_setting": sp.planes, "phasing": sp.phasing,
                "alt_km": sp.alt_km, "inc_deg": r2(sp.inc_deg), "sun_excl_deg": self.obs_params.sun_excl_deg, "sso": self.sun_sync,
                "raan_span_deg": sp.raan_span_deg,
                "period_min": r2(sp.period_s() / 60.0),
                "node_drift_deg_day": r2(self.shell.raan_drift_deg_per_day()),
                "links": self.shell.links.len(), "links_blocked": self.obs.links_blocked, "links_down": self.resil.links_down,
            },
            "telescope": {
                "aperture_cm": o.aperture_cm, "fov_deg": o.fov_deg, "exp_s": o.exp_s,
                "overhead_s": o.overhead_s,
                "jitter_arcsec": o.jitter_arcsec, "read_noise_e": o.read_noise_e, "sub_max_s": o.sub_max_s,
                "wfe_nm": o.wfe_nm, "spider_vanes": o.spider_vanes,
                "strehl": r2(crate::imaging::Hardware::from_obs(o).psf(60.0).strehl),
                "inertia_kgm2": o.inertia_kgm2, "wheel_torque_nm": o.wheel_torque_nm, "wheel_momentum_nms": o.wheel_momentum_nms,
                "rate_limit_dps": o.rate_limit_dps, "settle_tau_s": o.settle_tau_s, "acquire_s": o.acquire_s,
                "coarse_jitter_arcsec": o.coarse_jitter_arcsec, "guide_mag": o.guide_mag,
                "alpha_dps2": r2(ag.alpha.to_degrees()), "omega_dps": r2(ag.omega.to_degrees()),
                "repoint_10_s": r1(ag.repoint_time(10f64.to_radians())), "repoint_90_s": r1(ag.repoint_time(90f64.to_radians())),
                "lim_1tel_60s": r2(crate::imaging::point_lim_mag(&crate::imaging::Hardware::from_obs(o), 60.0, 60.0f64.min(o.sub_max_s), 22.0)),
                "sun_excl_deg": o.sun_excl_deg, "moon_excl_deg": o.moon_excl_deg,
                "limb_margin_deg": o.limb_margin_deg, "neo_elong_deg": o.neo_elong_deg, "neo_band_deg": o.neo_band_deg,
            },
            "stats": {
                "lit": self.obs.n_lit, "eclipsed": n - self.obs.n_lit, "active": self.obs.n_active,
                "locked": self.obs.n_locked, "slewing": self.obs.n_slewing, "settling": self.obs.n_settling,
                "locked_on_target": self.obs.n_locked_target,
                "earth_cone_deg": r2(self.obs.earth_cone_deg),
                "sky_reachable": r2(self.cov_frac * 100.0), "sky_allowed": r2(self.cov_sky_allowed * 100.0),
                "mean_per_point": r2(self.cov_mean),
                "stack_area_m2": r2(m.area_m2), "stack_d_eq_m": r2(m.d_eq_m), "stack_lim_mag": r2(m.lim_mag),
            },
            "target": target,
            "bodies": bodies,
            "footprints": fps,
            "footprint_stride": stride,
        });
        let fleet_due = self.last_fleet.map_or(true, |t| now.duration_since(t).as_secs_f64() >= 1.0);
        if fleet_due {
            self.traffic.relay_sample(&self.shell.pos);
        }
        let fleet = if fleet_due {
            self.last_fleet = Some(now);
            Some(self.fleet_json())
        } else {
            None
        };
        let inspect_req = web.lock().unwrap().inspect_req;
        let inspect = inspect_req.map(|i| self.inspect_json(i, &web));
        let mut sh = web.lock().unwrap();
        sh.status_json = status.to_string();
        sh.jd_now = self.jd;
        if let Some(f) = fleet {
            sh.fleet_json = f;
        }
        if let Some(j) = inspect {
            sh.inspect_json = j;
        }
        if self.logged_fleet_n != n {
            if self.logged_fleet_n != 0 {
                sh.log("warn", format!("Fleet roster changed: {} satellites in {} planes", n, sp.effective_planes()));
            } else {
                sh.log("info", format!("Operations console online · fleet of {} satellites in {} planes", n, sp.effective_planes()));
            }
            self.logged_fleet_n = n;
        }
        sh.utc_now = astro::format_utc(self.jd);
    }

    /// Sub-satellite points and states for the ground-track map.
    fn fleet_json(&self) -> String {
        use crate::attitude::Phase;
        let gm = astro::gmst(self.jd);
        let ll = |p: V3| {
            let e = p.rot_z(-gm);
            let lon = deg(e.y.atan2(e.x));
            let lat = deg((e.z / e.norm()).clamp(-1.0, 1.0).asin());
            [r1(lon), r1(lat)]
        };
        let sats: Vec<[f64; 3]> = self
            .shell
            .pos
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let [lon, lat] = ll(p);
                let code = if crate::resilience::failed_code(self, i) {
                    6.0
                } else if self.obs.hold.get(i).copied().unwrap_or(false) {
                    5.0
                } else {
                    match self.obs.att.get(i).map(|a| a.phase()) {
                        Some(Phase::Slewing) => 2.0,
                        Some(Phase::Settling) | Some(Phase::Acquiring) => 3.0,
                        Some(Phase::Locked) => 4.0,
                        _ => {
                            if self.obs.lit.get(i).copied().unwrap_or(true) {
                                0.0
                            } else {
                                1.0
                            }
                        }
                    }
                };
                [lon, lat, code]
            })
            .collect();
        let sub = ll(self.bodies[0].dir * 1.0e8);
        let traffic: Vec<[f32; 3]> = if self.traffic.cfg.show {
            self.traffic.map_points(self.jd).into_iter().map(|(a, b, c)| [(a * 10.0).round() / 10.0, (b * 10.0).round() / 10.0, c as f32]).collect()
        } else {
            Vec::new()
        };
        serde_json::json!({ "utc": astro::format_utc(self.jd), "subsolar": sub, "sats": sats, "traffic": traffic }).to_string()
    }

    /// Detail record for one satellite (the console's inspector).
    fn inspect_json(&self, i: usize, web: &crate::server::SharedRef) -> String {
        use crate::attitude::Phase;
        if i >= self.shell.len() {
            return serde_json::json!({ "index": i, "error": "no such satellite" }).to_string();
        }
        let ag = crate::attitude::Agility::from_obs(&self.obs_params);
        let p = self.shell.pos[i];
        let gm = astro::gmst(self.jd);
        let e = p.rot_z(-gm);
        let sat = &self.shell.sats[i];
        let a = &self.obs.att[i];
        let hold = self.obs.hold.get(i).copied().unwrap_or(false);
        let phase = if hold {
            "SAFE HOLD"
        } else {
            match a.phase() {
                Phase::Idle => "idle",
                Phase::Slewing => "slewing",
                Phase::Settling => "settling",
                Phase::Acquiring => "acquiring guide star",
                Phase::Locked => "locked · integrating",
            }
        };
        let j = self.obs.job.get(i).copied().unwrap_or(0);
        let task = match obs::job_tag(j) {
            0 => if hold { "none (safe hold)".to_string() } else { "unassigned".to_string() },
            obs::TAG_SURVEY => "Programme: survey (zenith stare)".into(),
            obs::TAG_ALLSKY => format!("All-sky survey: tile {}", obs::job_val(j)),
            obs::TAG_STACK => format!("Programme: stack on {}", self.target_name()),
            obs::TAG_MOSAIC => format!("Programme: mosaic of {}, tile {}", self.target_name(), obs::job_val(j) >> 20),
            obs::TAG_NEO => "Programme: NEO sweep".into(),
            obs::TAG_QUEUE => {
                let id = obs::job_val(j);
                let name = web
                    .lock()
                    .unwrap()
                    .requests
                    .iter()
                    .find(|r| r.id == id)
                    .map(|r| format!("{} for {}", r.target_name, r.submitter))
                    .unwrap_or_default();
                format!("Request #{id}: {name}")
            }
            _ => "?".into(),
        };
        let (bra, bdec) = a.dir.to_radec();
        let tgt = (a.job != 0).then(|| {
            let (r, d) = a.target.to_radec();
            serde_json::json!({ "ra": r2(deg(r)), "dec": r2(deg(d)) })
        });
        let (mut up, mut blocked) = (0, 0);
        for (k, &(x, y)) in self.shell.links.iter().enumerate() {
            if x as usize == i || y as usize == i {
                if self.obs.link_blocked.get(k).copied().unwrap_or(false) {
                    blocked += 1;
                } else {
                    up += 1;
                }
            }
        }
        let v = (crate::astro::MU_EARTH / p.norm()).sqrt();
        serde_json::json!({
            "index": i,
            "name": format!("FACET-{:05}", i + 1),
            "plane": sat.plane + 1,
            "slot": sat.slot + 1,
            "alt_km": r1(p.norm() - crate::astro::EARTH_RADIUS_KM),
            "lat": r2(deg((e.z / e.norm()).asin())),
            "lon": r2(deg(e.y.atan2(e.x))),
            "speed_km_s": r2(v),
            "sunlit": self.obs.lit.get(i).copied().unwrap_or(true),
            "hold": hold,
            "failed": crate::resilience::failed_code(self, i),
            "phase": phase,
            "task": task,
            "boresight": { "ra": r2(deg(bra)), "dec": r2(deg(bdec)) },
            "target": tgt,
            "pointing_error_arcsec": r2(a.error_arcsec(&ag, 0.8)),
            "guidance": if a.job == 0 { "—" } else if a.fine { "fine (guide star)" } else { "star tracker only" },
            "jitter_arcsec": if a.phase() == Phase::Locked { Some(r2(a.jitter(&ag))) } else { None },
            "links_up": up,
            "links_blocked": blocked,
        })
        .to_string()
    }

    pub fn web_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Web access").strong());
        let Some(web) = self.web.clone() else {
            ui.label(RichText::new("disabled (--no-web)").weak());
            return;
        };
        let mut sh = web.lock().unwrap();
        for e in &sh.errors {
            ui.label(RichText::new(e).color(egui::Color32::from_rgb(255, 120, 90)).small());
        }
        if let Some(u) = sh.control_url.clone() {
            ui.horizontal(|ui| {
                ui.label("Director:");
                if ui.small_button("copy URL").clicked() {
                    ui.output_mut(|o| o.copied_text = u.clone());
                }
                ui.hyperlink_to("open", &u);
            });
        }
        if let Some(u) = sh.public_url.clone() {
            ui.horizontal(|ui| {
                ui.label("Public:");
                if ui.small_button("copy URL").clicked() {
                    ui.output_mut(|o| o.copied_text = u.clone());
                }
                ui.hyperlink_to(u.clone(), &u);
            });
        }
        ui.checkbox(&mut sh.auto_approve, "auto-approve public requests");
        let open: Vec<usize> = (0..sh.requests.len()).filter(|&i| sh.requests[i].state.is_open()).collect();
        let done = sh.requests.iter().filter(|r| r.state == ReqState::Done).count();
        ui.label(
            RichText::new(format!("{} open requests · {} completed", open.len(), done)).small(),
        );
        if self.mode == Mode::Queue {
            ui.label(RichText::new("Queue mode: every telescope serves the request queue.").small().weak());
        } else {
            ui.add(egui::Slider::new(&mut self.public_share, 0.0..=1.0).text("public time share"))
                .on_hover_text("Fraction of the constellation that serves public requests while the director runs another mode.");
        }
        for &i in open.iter().take(8) {
            let r = &mut sh.requests[i];
            ui.horizontal(|ui| {
                let pct = 100.0 * r.progress_s / r.required_s().max(1e-9);
                ui.label(
                    RichText::new(format!(
                        "#{} {} · {} · {}×{:.0}s · {:?} {:.0}%",
                        r.id, r.submitter, r.target_name, r.telescopes, r.exposure_s, r.state, pct
                    ))
                    .small(),
                );
                if r.state == ReqState::Pending {
                    if ui.small_button("✔").on_hover_text("approve").clicked() {
                        r.state = ReqState::Approved;
                        r.approved_jd = Some(self.jd);
                    }
                    if ui.small_button("✖").on_hover_text("reject").clicked() {
                        r.state = ReqState::Rejected;
                    }
                }
            });
        }
        // STARE campaigns (desktop shortcut; normally run from the ops console)
        ui.add_space(4.0);
        ui.label(RichText::new("STARE campaigns (Sustained Tracking And Rotating Eyes)").small().strong());
        let shown: Vec<usize> = (0..sh.campaigns.len()).filter(|&i| sh.campaigns[i].active || sh.campaigns[i].armed).collect();
        let mut activate = Vec::new();
        for &i in &shown {
            let c = &mut sh.campaigns[i];
            ui.horizontal(|ui| {
                if c.active {
                    let state = if c.paused {
                        format!("PAUSED ({})", c.blocked.clone().unwrap_or_default())
                    } else if c.eligible_s > 0.0 {
                        format!("{:.2} % full", 100.0 * c.full_s / c.eligible_s)
                    } else {
                        "acquiring".into()
                    };
                    ui.label(
                        RichText::new(format!(
                            "● #{} {} · {}/{} locked · {} · gaps {} · handovers {}",
                            c.id, c.target_name, c.locked_now, c.eyes, state, c.gaps, c.handovers
                        ))
                        .small()
                        .color(if c.paused { egui::Color32::from_rgb(255, 180, 60) } else { egui::Color32::from_rgb(90, 255, 150) }),
                    );
                    if ui.small_button("stop").clicked() {
                        c.active = false;
                        c.paused = false;
                        c.armed = c.preset;
                        c.assigned_now = 0;
                        c.locked_now = 0;
                    }
                } else {
                    ui.label(
                        RichText::new(format!(
                            "○ #{} {} · ready{}",
                            c.id,
                            c.target_name,
                            c.blocked.as_ref().map(|b| format!(" (in {b}: would start paused)")).unwrap_or_default()
                        ))
                        .small(),
                    );
                    if ui.small_button("start").clicked() {
                        activate.push(c.id);
                    }
                }
            });
        }
        if shown.iter().any(|&i| sh.campaigns[i].armed) && ui.small_button("Start all ready").clicked() {
            activate.extend(shown.iter().filter(|&&i| sh.campaigns[i].armed).map(|&i| sh.campaigns[i].id));
        }
        for id in activate {
            if let Err(e) = crate::server::activate_campaign(&mut sh, id) {
                sh.errors.push(format!("STARE: {e}"));
            }
        }
        let tgt = match &self.target {
            Some(crate::app::Target::Body(i)) => Some(BODY_NAMES[*i].to_string()),
            Some(crate::app::Target::Fixed { name, .. }) => Some(name.clone()),
            None => None,
        };
        if let Some(t) = tgt {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut self.stare_eyes).clamp_range(1..=5000).suffix(" eyes"));
                if ui.small_button(format!("STARE at {t}")).clicked() {
                    let dir = self.target_dir();
                    let res = match (&self.target, dir) {
                        (Some(crate::app::Target::Body(_)), _) => crate::server::start_campaign(&mut sh, Some(&t), None, None, Some(self.stare_eyes), None, None),
                        (_, Some(d)) => {
                            let (ra, dec) = d.to_radec();
                            crate::server::start_campaign(&mut sh, None, Some(deg(ra)), Some(deg(dec)), Some(self.stare_eyes), None, None)
                        }
                        _ => Err("no target"),
                    };
                    if let Err(e) = res {
                        sh.errors.push(format!("STARE: {e}"));
                    }
                }
            });
        }
        if ui.small_button("Queue mode").clicked() {
            self.mode = Mode::Queue;
        }
    }
}

fn sh_utc(jd: f64) -> String {
    crate::astro::format_utc(jd)
}
