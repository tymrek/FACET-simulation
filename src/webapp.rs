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
        let utc = sh.utc_now.clone();
        for m in ended {
            sh.log("good", m);
        }
        for c in sh.campaigns.iter_mut().filter(|c| !c.active && c.stopped_utc.is_none()) {
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
            if excluded {
                if c.acquired {
                    events.push(("warn", format!("STARE #{} on {}: target entered {} — paused", c.id, c.target_name, c.blocked.as_deref().unwrap_or("").to_lowercase())));
                }
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
    pub fn queue_assign(&mut self) {
        let Some(web) = self.web.clone() else { return };
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let mut sh = web.lock().unwrap();
        let mut order: Vec<usize> = (0..sh.requests.len())
            .filter(|&i| sh.requests[i].state.is_schedulable())
            .collect();
        order.sort_by_key(|&i| (std::cmp::Reverse(sh.requests[i].priority), sh.requests[i].id));
        order.truncate(300);
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
                QueueItem { id: r.id, dir: r.dir, want: r.telescopes as usize, fine }
            })
            .collect();
        let n = self.shell.len();
        let budget = if self.mode == Mode::Queue {
            n
        } else {
            (self.public_share * n as f64).round() as usize
        };
        let results = obs::assign_queue(&mut self.obs, &self.obs_params, &items, sun, moon, budget);
        for (res, &i) in results.iter().zip(&order) {
            let r = &mut sh.requests[i];
            r.assigned_now = res.assigned as u32;
            r.blocked = res.blocked.map(String::from);
            if res.assigned > 0 && r.hw.is_none() {
                r.hw = Some(crate::imaging::Hardware::from_obs(&self.obs_params));
                r.agility = Some(crate::attitude::Agility::from_obs(&self.obs_params));
            }
        }
    }

    /// After the attitude step: credit locked integration to requests, track
    /// slews, jitter and first light.
    pub fn queue_credit(&mut self) {
        let Some(web) = self.web.clone() else { return };
        let ag = crate::attitude::Agility::from_obs(&self.obs_params);
        let mut per: HashMap<u64, (f64, f64, f64, u32)> = HashMap::new(); // id -> (s, jit²·s, fine s, locked sats)
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
            if got > 0.0 {
                if r.first_light_jd.is_none() {
                    r.first_light_jd = Some(jd);
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
                r.obs_jd = jd;
                r.progress_s += got;
                r.jit2_s += jit2;
                r.fine_s += fine_s;
                if r.progress_s >= r.required_s() {
                    r.progress_s = r.required_s();
                    r.state = ReqState::Done;
                    r.assigned_now = 0;
                    r.locked_now = 0;
                    r.completed_utc = Some(utc.clone());
                    events.push((
                        "good",
                        format!("Request #{} complete: {} — {:.0} telescope-seconds, {} repoints", r.id, r.target_name, r.progress_s, r.slews),
                    ));
                    continue;
                }
            }
            if r.full_lock_jd.is_none() && r.first_light_jd.is_some() && locked >= r.telescopes {
                r.full_lock_jd = Some(jd);
                if let Some(t) = r.full_lock_s() {
                    events.push(("good", format!("Request #{} at full strength: {} telescopes locked (+{:.0} s)", r.id, locked, t)));
                }
            }
            r.state = if r.assigned_now > 0 { ReqState::Observing } else { ReqState::Approved };
        }
        for (lvl, m) in events {
            sh.log(lvl, m);
        }
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
        let status = serde_json::json!({
            "utc": astro::format_utc(self.jd),
            "jd": self.jd,
            "playing": self.playing,
            "rate": self.rate,
            "mode": self.mode.key(),
            "public_share": if self.mode == Mode::Queue { 1.0 } else { self.public_share },
            "mode_label": self.mode.label(),
            "shell": {
                "n": n, "planes": sp.effective_planes(), "planes_setting": sp.planes, "phasing": sp.phasing,
                "alt_km": sp.alt_km, "inc_deg": r2(sp.inc_deg), "sso": self.sun_sync,
                "raan_span_deg": sp.raan_span_deg,
                "period_min": r2(sp.period_s() / 60.0),
                "node_drift_deg_day": r2(self.shell.raan_drift_deg_per_day()),
                "links": self.shell.links.len(), "links_blocked": self.obs.links_blocked,
            },
            "telescope": {
                "aperture_cm": o.aperture_cm, "fov_deg": o.fov_deg, "exp_s": o.exp_s,
                "overhead_s": o.overhead_s,
                "jitter_arcsec": o.jitter_arcsec, "read_noise_e": o.read_noise_e, "sub_max_s": o.sub_max_s,
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
                let code = if self.obs.hold.get(i).copied().unwrap_or(false) {
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
        serde_json::json!({ "utc": astro::format_utc(self.jd), "subsolar": sub, "sats": sats }).to_string()
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
        let active: Vec<usize> = (0..sh.campaigns.len()).filter(|&i| sh.campaigns[i].active).collect();
        for &i in &active {
            let c = &mut sh.campaigns[i];
            ui.horizontal(|ui| {
                let cov = if c.eligible_s > 0.0 { format!("{:.2} %", 100.0 * c.full_s / c.eligible_s) } else { "acquiring".into() };
                ui.label(
                    RichText::new(format!(
                        "#{} {} · {}/{} locked · {} · gaps {} · handovers {}",
                        c.id, c.target_name, c.locked_now, c.eyes, c.blocked.clone().unwrap_or(cov), c.gaps, c.handovers
                    ))
                    .small(),
                );
                if ui.small_button("stop").clicked() {
                    c.active = false;
                    c.assigned_now = 0;
                    c.locked_now = 0;
                }
            });
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
