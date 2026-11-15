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
                Command::Shell { n, planes, phasing, alt_km, inc_deg, sso, raan_span_deg } => {
                    let p = &mut self.shell_params;
                    if let Some(v) = n {
                        p.n = v.clamp(2, 10_000);
                    }
                    if let Some(v) = planes {
                        p.planes = v.min(p.n.min(400));
                    }
                    if let Some(v) = phasing {
                        p.phasing = v;
                    }
                    if let Some(v) = alt_km {
                        p.alt_km = v.clamp(300.0, 2000.0);
                    }
                    if let Some(v) = inc_deg {
                        p.inc_deg = v.clamp(0.0, 180.0);
                        self.sun_sync = false;
                    }
                    if let Some(v) = sso {
                        self.sun_sync = v;
                    }
                    if let Some(v) = raan_span_deg {
                        p.raan_span_deg = if v < 270.0 { 180.0 } else { 360.0 };
                    }
                    p.phasing = p.phasing.min(p.effective_planes().saturating_sub(1));
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
                Command::Time { rate, playing, now } => {
                    if let Some(r) = rate {
                        self.rate = r.clamp(0.0, 864_000.0);
                    }
                    if let Some(p) = playing {
                        self.playing = p;
                    }
                    if now == Some(true) {
                        self.jd = astro::jd_now();
                    }
                }
                Command::Telescope {
                    aperture_cm,
                    fov_deg,
                    exp_s,
                    jitter_arcsec,
                    read_noise_e,
                    sub_max_s,
                    overhead_s,
                    sun_excl_deg,
                    moon_excl_deg,
                    limb_margin_deg,
                    neo_elong_deg,
                    neo_band_deg,
                } => {
                    let o = &mut self.obs_params;
                    let set = |dst: &mut f64, v: Option<f64>, lo: f64, hi: f64| {
                        if let Some(v) = v.filter(|v| v.is_finite()) {
                            *dst = v.clamp(lo, hi);
                        }
                    };
                    set(&mut o.aperture_cm, aperture_cm, 5.0, 100.0);
                    set(&mut o.fov_deg, fov_deg, 0.1, 5.0);
                    set(&mut o.exp_s, exp_s, 1.0, 3600.0);
                    set(&mut o.jitter_arcsec, jitter_arcsec, 0.1, 5.0);
                    set(&mut o.read_noise_e, read_noise_e, 0.5, 10.0);
                    set(&mut o.sub_max_s, sub_max_s, 10.0, 1200.0);
                    set(&mut o.overhead_s, overhead_s, 0.0, 120.0);
                    set(&mut o.sun_excl_deg, sun_excl_deg, 5.0, 120.0);
                    set(&mut o.moon_excl_deg, moon_excl_deg, 0.0, 45.0);
                    set(&mut o.limb_margin_deg, limb_margin_deg, 0.0, 40.0);
                    set(&mut o.neo_elong_deg, neo_elong_deg, 5.0, 120.0);
                    set(&mut o.neo_band_deg, neo_band_deg, 0.0, 60.0);
                }
                // Handled in the server thread.
                Command::Request { .. } | Command::AutoApprove { .. } => {}
            }
        }
    }

    /// Allocate telescopes to approved requests and integrate their exposure.
    pub fn run_queue(&mut self, dt_sim: f64) {
        let Some(web) = self.web.clone() else { return };
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let mut sh = web.lock().unwrap();
        let utc = astro::format_utc(self.jd);
        let mut order: Vec<usize> = (0..sh.requests.len())
            .filter(|&i| sh.requests[i].state.is_schedulable())
            .collect();
        order.sort_by_key(|&i| (std::cmp::Reverse(sh.requests[i].priority), sh.requests[i].id));
        order.truncate(300);
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
                QueueItem { id: r.id, dir: r.dir, want: r.telescopes as usize }
            })
            .collect();
        let results = obs::assign_queue(&mut self.obs, &self.obs_params, &items, sun, moon);
        for (res, &i) in results.iter().zip(&order) {
            let r = &mut sh.requests[i];
            debug_assert_eq!(r.id, res.id);
            r.assigned_now = res.assigned as u32;
            r.blocked = res.blocked.map(String::from);
            if res.assigned > 0 {
                if r.hw.is_none() {
                    r.hw = Some(crate::imaging::Hardware::from_obs(&self.obs_params));
                }
                r.obs_jd = self.jd;
                r.state = ReqState::Observing;
                r.progress_s += res.assigned as f64 * dt_sim;
                if r.progress_s >= r.required_s() {
                    r.progress_s = r.required_s();
                    r.state = ReqState::Done;
                    r.assigned_now = 0;
                    r.completed_utc = Some(utc.clone());
                }
            } else if r.state == ReqState::Observing {
                r.state = ReqState::Approved;
            }
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
        let stride = (self.obs.n_active / 1500).max(1);
        let mut fps = Vec::new();
        let mut seen = 0usize;
        for (i, p) in self.obs.pointing.iter().enumerate() {
            if let Some(d) = p {
                if self.mode == Mode::Stack && !fps.is_empty() {
                    break;
                }
                if seen % stride == 0 {
                    let (ra, dec) = d.to_radec();
                    fps.push(serde_json::json!([r2(deg(ra)), r2(deg(dec)), self.obs.lit[i] as u8]));
                }
                seen += 1;
            }
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
                                "visible": self.obs.n_target_visible, "blocked": self.obs.target_blocked_by })
        });
        let m = obs::metrics(o, self.obs.n_target_visible, self.obs.max_baseline_km, 0);
        let status = serde_json::json!({
            "utc": astro::format_utc(self.jd),
            "jd": self.jd,
            "playing": self.playing,
            "rate": self.rate,
            "mode": self.mode.key(),
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
                "lim_1tel_60s": r2(crate::imaging::point_lim_mag(&crate::imaging::Hardware::from_obs(o), 60.0, 60.0f64.min(o.sub_max_s), 22.0)),
                "sun_excl_deg": o.sun_excl_deg, "moon_excl_deg": o.moon_excl_deg,
                "limb_margin_deg": o.limb_margin_deg, "neo_elong_deg": o.neo_elong_deg, "neo_band_deg": o.neo_band_deg,
            },
            "stats": {
                "lit": self.obs.n_lit, "eclipsed": n - self.obs.n_lit, "active": self.obs.n_active,
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
        let mut sh = web.lock().unwrap();
        sh.status_json = status.to_string();
        sh.utc_now = astro::format_utc(self.jd);
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
        if self.mode != Mode::Queue && !open.is_empty() {
            ui.label(RichText::new("Requests are served only in Queue mode.").small().weak());
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
                    }
                    if ui.small_button("✖").on_hover_text("reject").clicked() {
                        r.state = ReqState::Rejected;
                    }
                }
            });
        }
        if ui.small_button("Queue mode").clicked() {
            self.mode = Mode::Queue;
        }
    }
}
