//! Application state, simulation clock and the control panel.

use crate::astro::{self, Body, BODY_NAMES};
use crate::catalog::{self, NamedStar, Star};
use crate::constellation::{Shell, ShellParams};
use crate::math::{deg, rad, V3};
use crate::obs::{self, Mode, ObsParams, ObsState};
use egui::{Color32, RichText};
use std::time::Instant;

#[derive(Clone, Debug)]
pub enum Target {
    Body(usize),
    Fixed { name: String, dir: V3 },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Orbit,
    Sky,
    Split,
    Unified,
}

pub struct OrbitCam {
    pub yaw: f64,
    pub pitch: f64,
    pub dist_km: f64,
    /// Vertical field of view, degrees.
    pub fov_deg: f64,
    /// Rotate the camera with the Earth (Earth-fixed view).
    pub earth_fixed: bool,
}

pub struct SkyCam {
    pub ra: f64,
    pub dec: f64,
    pub fov_deg: f64,
}

pub struct Show {
    pub links: bool,
    pub rays: bool,
    pub coverage: bool,
    pub names: bool,
    pub grid: bool,
    pub coast: bool,
    pub footprints: bool,
    pub orbit_stars: bool,
}

pub struct App {
    pub stars: Vec<Star>,
    pub names: Vec<NamedStar>,
    pub coast: Vec<Vec<(f32, f32)>>,

    pub shell_params: ShellParams,
    pub sun_sync: bool,
    pub shell: Shell,
    pub obs_params: ObsParams,
    pub mode: Mode,
    pub obs: ObsState,

    pub jd: f64,
    pub playing: bool,
    pub rate: f64,
    last_tick: Instant,

    pub bodies: [Body; 9],
    pub target: Option<Target>,
    /// Clicking the sky sets the target (off = clicks only inspect/pan).
    pub click_target: bool,
    pub view: View,
    pub orbit_cam: OrbitCam,
    pub sky_cam: SkyCam,
    pub show: Show,

    pub grid: obs::SkyGrid,
    pub cov: Vec<u32>,
    last_cov: Option<Instant>,
    pub cov_frac: f64,
    pub cov_mean: f64,
    pub cov_sky_allowed: f64,

    frame_ms: f32,
    pub export_msg: Option<String>,

    // Web access
    pub web: Option<crate::server::SharedRef>,
    pub web_control_addr: Option<String>,
    pub web_public_addr: Option<String>,
    pub web_token: Option<String>,
    pub last_publish: Option<Instant>,
    /// Fraction of telescopes serving the public queue outside Queue mode.
    pub public_share: f64,
    pub last_fleet: Option<Instant>,
    pub logged_fleet_n: usize,
    pub stare_eyes: u32,
    /// Synthetic investigators submitting work, and the science-return ledger.
    pub gen: crate::programme::Generator,
    /// Planetary-defence exercise, its editable configuration, and the share
    /// of the fleet held back from the programme to keep searching.
    pub defence: Option<crate::defence::Scenario>,
    pub pd_cfg: crate::defence::Config,
    pub pd_preset: usize,
    pub pending_defence: bool,
    pub search_share: f64,
    pub science: crate::programme::Science,
    /// Campaign targets given on the command line, started once web is up.
    pub pending_stares: Vec<String>,
}

pub const PRESET_NS: [usize; 8] = [2, 10, 50, 100, 500, 1000, 5000, 10_000];

/// Named deep-sky / reference targets (J2000 RA, Dec in degrees).
pub const FIXED_TARGETS: [(&str, f64, f64); 6] = [
    ("Orion Nebula (M42)", 83.822, -5.391),
    ("Andromeda (M31)", 10.685, 41.269),
    ("Galactic Centre", 266.405, -28.936),
    ("Pleiades (M45)", 56.75, 24.117),
    ("LMC", 80.894, -69.756),
    ("Ecliptic pole N", 270.0, 66.561),
];

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        let mut vis = egui::Visuals::dark();
        vis.panel_fill = Color32::from_rgb(14, 16, 22);
        cc.egui_ctx.set_visuals(vis);

        let jd = astro::jd_now();
        let shell_params = ShellParams { n: 1000, ..ShellParams::default() };
        let shell = Shell::new(shell_params.clone(), jd);
        let bodies = astro::bodies(jd);
        let (sra, sdec) = bodies[0].dir.to_radec();
        let mut app = App {
            stars: catalog::load_stars(),
            names: catalog::load_names(),
            coast: catalog::load_coast(),
            shell_params,
            sun_sync: false,
            shell,
            obs_params: ObsParams::default(),
            mode: Mode::Queue,
            obs: ObsState::default(),
            jd,
            playing: true,
            rate: 1.0,
            last_tick: Instant::now(),
            bodies,
            target: Some(Target::Body(5)), // Jupiter
            click_target: false,
            view: View::Split,
            orbit_cam: OrbitCam {
                yaw: rad(30.0),
                pitch: rad(25.0),
                dist_km: 32_000.0,
                fov_deg: 45.0,
                earth_fixed: false,
            },
            // Start looking at the anti-Sun side of the sky (night sky).
            sky_cam: SkyCam {
                ra: sra + std::f64::consts::PI,
                dec: -sdec,
                fov_deg: 110.0,
            },
            show: Show {
                links: true,
                rays: true,
                coverage: false,
                names: true,
                grid: true,
                coast: true,
                footprints: true,
                orbit_stars: true,
            },
            grid: obs::SkyGrid::new(4.0),
            cov: Vec::new(),
            last_cov: None,
            cov_frac: 0.0,
            cov_mean: 0.0,
            cov_sky_allowed: 0.0,
            frame_ms: 0.0,
            export_msg: None,
            web: None,
            web_control_addr: Some("127.0.0.1:8780".into()),
            web_public_addr: Some("127.0.0.1:8781".into()),
            web_token: None,
            last_publish: None,
            public_share: 0.25,
            last_fleet: None,
            logged_fleet_n: 0,
            stare_eyes: 10,
            gen: crate::programme::Generator::new(120, 0x5EED_FACE_7),
            defence: None,
            pd_cfg: crate::defence::presets()[0].clone(),
            pd_preset: 0,
            pending_defence: false,
            search_share: 0.10,
            science: crate::programme::Science::default(),
            pending_stares: Vec::new(),
        };
        app.apply_args(std::env::args().skip(1).collect());
        app.start_web();
        if let Some(web) = app.web.clone() {
            let mut sh = web.lock().unwrap();
            sh.jd_now = app.jd;
            crate::server::arm_presets(&mut sh, app.stare_eyes);
            for t in std::mem::take(&mut app.pending_stares) {
                if let Err(e) = crate::server::start_campaign(&mut sh, Some(&t), None, None, Some(app.stare_eyes), None, None) {
                    eprintln!("facet-sim: --stare {t}: {e}");
                }
            }
        }
        if app.pending_defence {
            app.pending_defence = false;
            let cfg = app.pd_cfg.clone();
            app.start_defence(cfg);
        }
        app
    }

    /// Command-line start-up state, e.g.
    /// `facet-sim --n 1000 --mode neo --view sky --coverage --target Saturn --look`.
    fn apply_args(&mut self, args: Vec<String>) {
        let mut it = args.into_iter();
        while let Some(a) = it.next() {
            let mut val = || it.next().unwrap_or_default();
            match a.as_str() {
                "--n" => self.shell_params.n = val().parse().unwrap_or(100).clamp(2, 10_000),
                "--planes" => self.shell_params.planes = val().parse().unwrap_or(0),
                "--alt" => self.shell_params.alt_km = val().parse().unwrap_or(550.0),
                "--inc" => self.shell_params.inc_deg = val().parse().unwrap_or(53.0),
                "--sso" => self.sun_sync = true,
                "--star" => self.shell_params.raan_span_deg = 180.0,
                "--aperture" => self.obs_params.aperture_cm = val().parse().unwrap_or(20.0),
                "--rate" => self.rate = val().parse().unwrap_or(1.0),
                "--paused" => self.playing = false,
                "--coverage" => self.show.coverage = true,
                "--fov" => self.sky_cam.fov_deg = val().parse().unwrap_or(110.0),
                "--zoom" => self.orbit_cam.dist_km = val().parse().unwrap_or(32_000.0),
                "--camfov" => self.orbit_cam.fov_deg = val().parse().unwrap_or(45.0),
                "--mode" => {
                    self.mode = match val().as_str() {
                        "survey" => Mode::Survey,
                        "mosaic" => Mode::Mosaic,
                        "neo" => Mode::NeoSweep,
                        _ => Mode::Stack,
                    }
                }
                "--view" => {
                    self.view = match val().as_str() {
                        "orbit" => View::Orbit,
                        "sky" => View::Sky,
                        "unified" => View::Unified,
                        _ => View::Split,
                    }
                }
                "--target" => {
                    let v = val();
                    if let Some(i) = BODY_NAMES.iter().position(|b| b.eq_ignore_ascii_case(&v)) {
                        self.target = Some(Target::Body(i));
                    } else if let Some(s) = self.names.iter().find(|s| s.name.eq_ignore_ascii_case(&v)) {
                        self.target = Some(Target::Fixed { name: s.name.clone(), dir: s.dir });
                    }
                }
                "--look" => self.look_at_target(),
                "--no-target" => self.target = None,
                "--target-eyes" => self.obs_params.target_eyes = val().parse().unwrap_or(25),
                "--click-target" => self.click_target = true,
                "--no-programme" => self.gen.on = false,
                "--search-share" => self.search_share = (val().parse::<f64>().unwrap_or(10.0) / 100.0).clamp(0.0, 1.0),
                "--defence" | "--defense" => {
                    let p = crate::defence::presets();
                    let i = val().parse::<usize>().unwrap_or(1).clamp(1, p.len()) - 1;
                    self.pd_preset = i;
                    self.pd_cfg = p[i].clone();
                    self.pending_defence = true;
                }
                "--load" => self.gen.load = val().parse::<f64>().unwrap_or(1.5).clamp(0.0, 10.0),
                "--teams" => {
                    let n = val().parse().unwrap_or(120);
                    self.gen.set_investigators(n);
                }
                "--look-sun" => self.look_at(astro::bodies(self.jd)[0].dir),
                "--control-addr" => self.web_control_addr = Some(val()),
                "--public-addr" => self.web_public_addr = Some(val()),
                "--no-control" => self.web_control_addr = None,
                "--no-public" => self.web_public_addr = None,
                "--no-web" => {
                    self.web_control_addr = None;
                    self.web_public_addr = None;
                }
                "--token" => self.web_token = Some(val()),
                "--stare" => self.pending_stares.push(val()),
                "--stare-eyes" => self.stare_eyes = val().parse().unwrap_or(10),
                "--public-share" => self.public_share = val().parse::<f64>().unwrap_or(0.25).clamp(0.0, 1.0),
                _ => eprintln!("facet-sim: ignoring unknown argument {a}"),
            }
        }
    }

    fn look_at_target(&mut self) {
        if let Some(t) = self.target_dir() {
            self.look_at(t);
        }
    }

    /// Point both the sky camera and the 3D camera at a sky direction (the 3D
    /// camera moves to the far side of Earth so the direction is behind it).
    pub fn look_at(&mut self, d: V3) {
        let (ra, dec) = d.to_radec();
        self.sky_cam.ra = ra;
        self.sky_cam.dec = dec;
        // Aim the 3D camera a little to one side of `d` so the direction is
        // visible just past Earth's limb instead of hidden behind the globe.
        let earth_ang = (astro::EARTH_RADIUS_KM / self.orbit_cam.dist_km).asin();
        let half_fov = rad(self.orbit_cam.fov_deg) / 2.0;
        let off = (earth_ang * 1.35 + rad(4.0)).min(half_fov * 0.85);
        let side = V3::Z.cross(d);
        let side = if side.norm() < 1e-6 { V3::new(1.0, 0.0, 0.0) } else { side.unit() };
        let axis = d.rot_axis(side, off); // camera looks along `axis`
        let earth_rot = if self.orbit_cam.earth_fixed { astro::gmst(self.jd) } else { 0.0 };
        self.orbit_cam.yaw = (-axis.y).atan2(-axis.x) - earth_rot;
        self.orbit_cam.pitch = (-axis.z).clamp(-1.0, 1.0).asin().clamp(-1.55, 1.55);
    }

    pub fn target_dir(&self) -> Option<V3> {
        match &self.target {
            Some(Target::Body(i)) => Some(self.bodies[*i].dir),
            Some(Target::Fixed { dir, .. }) => Some(*dir),
            None => None,
        }
    }

    pub fn target_name(&self) -> String {
        match &self.target {
            Some(Target::Body(i)) => BODY_NAMES[*i].to_string(),
            Some(Target::Fixed { name, .. }) => name.clone(),
            None => "none".into(),
        }
    }

    fn step(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64().min(0.25);
        self.frame_ms = self.frame_ms * 0.9 + (dt * 1000.0) as f32 * 0.1;
        if std::env::var_os("FACET_BENCH").is_some() && self.last_cov.map_or(false, |t| now.duration_since(t).as_secs_f64() > 0.49) {
            eprintln!("frame {:.1} ms", self.frame_ms);
        }
        self.last_tick = now;
        let dt_sim = if self.playing { dt * self.rate } else { 0.0 };
        self.jd += dt_sim / 86_400.0;
        self.apply_web_commands();

        if self.sun_sync {
            if let Some(i) = ShellParams::sun_sync_inc_deg(self.shell_params.alt_km) {
                self.shell_params.inc_deg = (i * 100.0).round() / 100.0;
            }
        }
        if self.shell_params != self.shell.params {
            self.shell = Shell::new(self.shell_params.clone(), self.shell.epoch_jd);
        }
        self.shell.propagate(self.jd);
        self.bodies = astro::bodies(self.jd);
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let target = self.target_dir();
        obs::update(
            &mut self.obs,
            &self.obs_params,
            self.mode,
            &self.shell.pos,
            &self.shell.links,
            sun,
            moon,
            target,
            match self.target {
                Some(Target::Body(i)) => Some(i),
                _ => None,
            },
        );
        // STARE campaigns first (director commitments), then the public queue
        // on its share of the shell.
        self.programme_tick(dt_sim);
        self.stare_assign();
        self.queue_assign();
        let ag = crate::attitude::Agility::from_obs(&self.obs_params);
        obs::advance_attitudes(&mut self.obs, &ag, dt_sim, target);
        self.queue_credit();
        self.science_account(dt_sim);
        self.defence_step(dt_sim);
        self.stare_credit(dt_sim);
        self.publish_status(now);

        let due = self
            .last_cov
            .map_or(true, |t| now.duration_since(t).as_secs_f64() > 0.5);
        if due {
            obs::coverage(
                &self.grid.pts,
                &self.obs.rhat,
                self.obs.cos_lim,
                &self.obs_params,
                sun,
                moon,
                &mut self.cov,
            );
            let g = &self.grid;
            let mut allowed = 0.0;
            let mut covered = 0.0;
            let mut mean = 0.0;
            for (k, &p) in g.pts.iter().enumerate() {
                let w = g.weight[k];
                if obs::sky_blocked(&self.obs_params, p, sun, moon).is_none() {
                    allowed += w;
                }
                if self.cov[k] > 0 {
                    covered += w;
                }
                mean += w * self.cov[k] as f64;
            }
            self.cov_sky_allowed = allowed;
            self.cov_frac = covered;
            self.cov_mean = mean;
            self.last_cov = Some(now);
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(RichText::new("FACET simulator").size(18.0).strong());
        ui.label(RichText::new("an open astronomical survey shell").italics().weak());
        ui.separator();

        // ---------------- Time
        ui.label(RichText::new("Time").strong());
        ui.monospace(astro::format_utc(self.jd));
        ui.horizontal_wrapped(|ui| {
            if ui.button(if self.playing { "⏸ Pause" } else { "▶ Play" }).clicked() {
                self.playing = !self.playing;
            }
            if ui.button("Now").clicked() {
                self.jd = astro::jd_now();
            }
            for (lbl, r) in [("1×", 1.0), ("60×", 60.0), ("600×", 600.0), ("1 h/s", 3600.0), ("1 d/s", 86_400.0)] {
                if ui.selectable_label(self.rate == r, lbl).clicked() {
                    self.rate = r;
                    self.playing = true;
                }
            }
        });
        ui.separator();

        // ---------------- Constellation
        ui.label(RichText::new("Constellation").strong());
        ui.add(
            egui::Slider::new(&mut self.shell_params.n, 2..=10_000)
                .logarithmic(true)
                .text("satellites"),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Stages:");
            for n in PRESET_NS {
                if ui.selectable_label(self.shell_params.n == n, n.to_string()).clicked() {
                    self.shell_params.n = n;
                }
            }
        });
        let p_eff = self.shell_params.effective_planes();
        let mut auto = self.shell_params.planes == 0;
        ui.horizontal(|ui| {
            if ui.checkbox(&mut auto, "auto planes").changed() {
                self.shell_params.planes = if auto { 0 } else { p_eff };
            }
            ui.label(format!("P = {p_eff}, ~{} per plane", (self.shell_params.n + p_eff - 1) / p_eff));
        });
        if !auto {
            let maxp = self.shell_params.n.min(400);
            ui.add(egui::Slider::new(&mut self.shell_params.planes, 1..=maxp).text("planes"));
        }
        let maxf = p_eff.saturating_sub(1);
        self.shell_params.phasing = self.shell_params.phasing.min(maxf);
        ui.add(egui::Slider::new(&mut self.shell_params.phasing, 0..=maxf.max(1)).text("phasing F"));
        ui.add(egui::Slider::new(&mut self.shell_params.alt_km, 300.0..=2000.0).text("altitude km"));
        ui.add_enabled(
            !self.sun_sync,
            egui::Slider::new(&mut self.shell_params.inc_deg, 0.0..=180.0).text("inclination °"),
        );
        ui.checkbox(&mut self.sun_sync, "sun-synchronous inclination");
        ui.horizontal(|ui| {
            ui.label("Node spread:");
            ui.selectable_value(&mut self.shell_params.raan_span_deg, 360.0, "Walker delta 360°");
            ui.selectable_value(&mut self.shell_params.raan_span_deg, 180.0, "Walker star 180°");
        });
        ui.add(egui::Slider::new(&mut self.shell_params.raan0_deg, 0.0..=360.0).text("RAAN of plane 0 °"));
        ui.label(
            RichText::new(format!(
                "Period {:.1} min · node drift {:+.2}°/day · {} laser links ({} through atmosphere)",
                self.shell_params.period_s() / 60.0,
                self.shell.raan_drift_deg_per_day(),
                self.shell.links.len(),
                self.obs.links_blocked
            ))
            .small()
            .weak(),
        );
        if ui
            .button("Export TLEs (for Stellarium)")
            .on_hover_text("Writes the shell at the current sim time as a TLE file in the working directory")
            .clicked()
        {
            let path = format!("facet_{}_sats.tle", self.shell.len());
            let txt = crate::tle::shell_to_tle(&self.shell, self.jd);
            self.export_msg = Some(match std::fs::write(&path, txt) {
                Ok(()) => format!("Wrote {}", std::fs::canonicalize(&path).map(|p| p.display().to_string()).unwrap_or(path)),
                Err(e) => format!("Export failed: {e}"),
            });
        }
        if let Some(m) = &self.export_msg {
            ui.label(RichText::new(m).small());
        }
        ui.separator();

        // ---------------- Telescope
        ui.label(RichText::new("Telescope (per satellite)").strong());
        let o = &mut self.obs_params;
        ui.add(egui::Slider::new(&mut o.aperture_cm, 5.0..=100.0).text("aperture cm"));
        ui.add(egui::Slider::new(&mut o.fov_deg, 0.1..=5.0).logarithmic(true).text("FOV ° (square)"));
        ui.add(egui::Slider::new(&mut o.exp_s, 1.0..=3600.0).logarithmic(true).text("exposure s"));
        ui.add(egui::Slider::new(&mut o.jitter_arcsec, 0.1..=5.0).text("pointing jitter arcsec RMS"))
            .on_hover_text("ASSUMPTION: bus pointing stability over a sub-frame. Small-sat buses: ~0.5–2 arcsec.");
        ui.add(egui::Slider::new(&mut o.wfe_nm, 0.0..=200.0).text("wavefront error nm RMS"))
            .on_hover_text("ASSUMPTION: total figure + alignment + thermal wavefront error. λ/10 at 633 nm ≈ 63 nm.");
        ui.horizontal(|ui| {
            ui.label("spider:");
            for v in [0u32, 3, 4] {
                ui.selectable_value(&mut o.spider_vanes, v, if v == 0 { "none (off-axis)".to_string() } else { format!("{v} vanes") });
            }
        });
        ui.add_enabled(o.spider_vanes > 0, egui::Slider::new(&mut o.spider_mm, 0.5..=10.0).text("vane width mm"));
        ui.add(egui::Slider::new(&mut o.read_noise_e, 0.5..=10.0).text("read noise e-"));
        ui.add(egui::Slider::new(&mut o.sub_max_s, 10.0..=1200.0).logarithmic(true).text("max sub-frame s"));
        {
            let hw = crate::imaging::Hardware::from_obs(o);
            let lim = crate::imaging::point_lim_mag(&hw, o.exp_s, o.exp_s.min(o.sub_max_s), 22.0);
            ui.label(
                RichText::new(format!(
                    "f/{:.1}, {:.0} mm, {:.2}\"/px, Strehl {:.2}, FWHM {:.2}\" · 1 telescope, {:.0} s: V≈{:.1} (5σ, dark sky)",
                    hw.f_ratio(),
                    hw.focal_mm(),
                    hw.pixel_scale(),
                    hw.psf(o.exp_s.min(o.sub_max_s)).strehl,
                    hw.fwhm(o.exp_s.min(o.sub_max_s)),
                    o.exp_s,
                    lim
                ))
                .small()
                .weak(),
            );
        }
        ui.add(egui::Slider::new(&mut o.overhead_s, 0.0..=120.0).text("slew+settle overhead s"))
            .on_hover_text("ASSUMPTION: dead time per pointing (slew, settle, readout).");
        ui.separator();

        // ---------------- Agility
        ui.label(RichText::new("Attitude control (per satellite)").strong());
        let o = &mut self.obs_params;
        ui.add(egui::Slider::new(&mut o.inertia_kgm2, 2.0..=200.0).logarithmic(true).text("inertia kg·m²"))
            .on_hover_text("ASSUMPTION: ~150 kg, 0.6 m bus + 1 m optical tube → ≈17–20 kg·m² about a transverse axis.");
        ui.add(egui::Slider::new(&mut o.wheel_torque_nm, 0.01..=1.0).logarithmic(true).text("wheel torque N·m"))
            .on_hover_text("ASSUMPTION: usable reaction-wheel torque about the slew axis (small-sat wheels: 0.05–0.25 N·m).");
        ui.add(egui::Slider::new(&mut o.rate_limit_dps, 0.1..=10.0).logarithmic(true).text("slew-rate limit °/s"));
        ui.add(egui::Slider::new(&mut o.settle_tau_s, 0.2..=30.0).logarithmic(true).text("settling τ s"));
        ui.add(egui::Slider::new(&mut o.acquire_s, 0.0..=60.0).text("guide-star acquisition s"));
        ui.add(egui::Slider::new(&mut o.coarse_jitter_arcsec, 0.5..=60.0).logarithmic(true).text("star-tracker-only jitter arcsec"));
        ui.add(egui::Slider::new(&mut o.guide_mag, 6.0..=18.0).text("guide-star limit V"));
        {
            let ag = crate::attitude::Agility::from_obs(o);
            let t = |d: f64| ag.repoint_time(d.to_radians());
            ui.label(
                RichText::new(format!(
                    "α {:.2}°/s², ω {:.1}°/s · repoint (slew+settle+lock): 1° {:.0} s, 10° {:.0} s, 45° {:.0} s, 90° {:.0} s, 180° {:.0} s",
                    ag.alpha.to_degrees(),
                    ag.omega.to_degrees(),
                    t(1.0),
                    t(10.0),
                    t(45.0),
                    t(90.0),
                    t(180.0)
                ))
                .small()
                .weak(),
            );
        }
        ui.separator();

        // ---------------- Constraints
        ui.label(RichText::new("Keep-out constraints").strong());
        ui.add(egui::Slider::new(&mut o.sun_excl_deg, 5.0..=120.0).text("Sun exclusion °"));
        ui.add(egui::Slider::new(&mut o.moon_excl_deg, 0.0..=45.0).text("Moon exclusion °"));
        ui.add(egui::Slider::new(&mut o.limb_margin_deg, 0.0..=40.0).text("Earth-limb margin °"));
        ui.separator();

        // ---------------- Mode & target
        ui.label(RichText::new("Operations").strong());
        egui::ComboBox::from_id_source("mode")
            .selected_text(self.mode.label())
            .width(260.0)
            .show_ui(ui, |ui| {
                for m in Mode::ALL {
                    ui.selectable_value(&mut self.mode, m, m.label());
                }
            });
        if self.mode == Mode::NeoSweep {
            let o = &mut self.obs_params;
            ui.add(egui::Slider::new(&mut o.neo_elong_deg, 5.0..=120.0).text("elongation °"));
            ui.add(egui::Slider::new(&mut o.neo_band_deg, 0.0..=60.0).text("band width °"));
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Target:");
            ui.label(RichText::new(self.target_name()).color(Color32::from_rgb(120, 230, 160)));
            if self.target.is_some() && ui.small_button("Clear").on_hover_text("No target: every telescope surveys (Esc)").clicked() {
                self.target = None;
            }
        });
        ui.checkbox(&mut self.click_target, "Click on the sky sets the target");
        if matches!(self.mode, Mode::Stack | Mode::Mosaic) {
            let mut v = self.obs_params.target_eyes;
            ui.add(egui::Slider::new(&mut v, 0..=1000).logarithmic(true).text("max eyes on target (0 = all)"))
                .on_hover_text("Telescopes beyond this keep surveying; STARE and queue work is unaffected");
            self.obs_params.target_eyes = v;
        }
        ui.horizontal_wrapped(|ui| {
            for i in 1..9 {
                if ui.small_button(BODY_NAMES[i]).clicked() {
                    self.target = Some(Target::Body(i));
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (name, ra, dec) in FIXED_TARGETS {
                if ui.small_button(name).clicked() {
                    self.target = Some(Target::Fixed {
                        name: name.into(),
                        dir: V3::from_radec(rad(ra), rad(dec)),
                    });
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.small_button("Look at target").clicked() {
                self.look_at_target();
                self.sky_cam.fov_deg = self.sky_cam.fov_deg.min(60.0);
            }
            if ui.small_button("Look at Sun").clicked() {
                self.look_at(self.bodies[0].dir);
            }
        });
        ui.separator();

        self.stats(ui);
        ui.separator();

        self.defence_panel(ui);
        ui.separator();
        self.science_panel(ui);
        ui.separator();
        self.web_panel(ui);
        ui.separator();

        // ---------------- Display
        ui.label(RichText::new("Display").strong());
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.view, View::Orbit, "Orbit");
            ui.selectable_value(&mut self.view, View::Sky, "Sky");
            ui.selectable_value(&mut self.view, View::Split, "Split");
            ui.selectable_value(&mut self.view, View::Unified, "Unified");
        });
        ui.add(egui::Slider::new(&mut self.orbit_cam.fov_deg, 5.0..=120.0).text("3D camera FOV °"));
        let s = &mut self.show;
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut s.links, "laser links");
            ui.checkbox(&mut s.rays, "pointing rays");
            ui.checkbox(&mut s.footprints, "FOV footprints");
            ui.checkbox(&mut s.coverage, "stacking-depth map");
            ui.checkbox(&mut s.names, "labels");
            ui.checkbox(&mut s.grid, "RA/Dec grid");
            ui.checkbox(&mut s.coast, "coastlines");
            ui.checkbox(&mut s.orbit_stars, "stars in orbit view");
            ui.checkbox(&mut self.orbit_cam.earth_fixed, "Earth-fixed camera");
        });
        ui.label(RichText::new(format!("{:.1} ms/frame", self.frame_ms)).small().weak());
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "All performance numbers are unchecked soft estimates from idealised scaling \
                 (equal telescopes, background-limited stacking, perfect registration).",
            )
            .small()
            .weak(),
        );
        ui.label(RichText::new("Stars: HYG v4.0 (CC BY-SA 4.0). Coastlines: Natural Earth.").small().weak());
    }

    fn stats(&mut self, ui: &mut egui::Ui) {
        let n = self.shell.len();
        ui.label(RichText::new("Now").strong());
        egui::Grid::new("stats").num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
            let row = |ui: &mut egui::Ui, k: &str, v: String| {
                ui.label(RichText::new(k).weak());
                ui.label(v);
                ui.end_row();
            };
            row(ui, "Sunlit / eclipsed", format!("{} / {}", self.obs.n_lit, n - self.obs.n_lit));
            row(ui, "Earth keep-out cone", format!("{:.1}° + {:.0}° margin", self.obs.earth_cone_deg, self.obs_params.limb_margin_deg));
            row(ui, "Tasked telescopes", format!("{} of {}", self.obs.n_active, n));
            row(
                ui,
                "Locked / slewing / settling",
                format!("{} / {} / {}", self.obs.n_locked, self.obs.n_slewing, self.obs.n_settling),
            );
            row(
                ui,
                "Sky reachable now",
                format!(
                    "{:.1}% (of {:.1}% outside Sun/Moon keep-out)",
                    self.cov_frac * 100.0,
                    self.cov_sky_allowed * 100.0
                ),
            );
            row(ui, "Mean telescopes per sky point", format!("{:.1}", self.cov_mean));
            // Survey throughput: how much sky one exposure round images.
            let full_sky = 41_252.96;
            let per_round = self.obs.n_active as f64 * self.obs_params.fov_deg.powi(2);
            row(
                ui,
                "Sky imaged per exposure",
                format!("{:.0} deg² ({:.2}% of sky)", per_round, per_round / full_sky * 100.0),
            );
            if per_round > 0.0 {
                let rounds = (full_sky * self.cov_sky_allowed / per_round).max(1.0);
                let secs = rounds * (self.obs_params.exp_s + self.obs_params.overhead_s);
                row(ui, "Accessible-sky revisit (est.)", fmt_duration(secs));
            }
        });

        if matches!(self.mode, Mode::Stack | Mode::Mosaic) {
            ui.add_space(4.0);
            ui.label(RichText::new(format!("On target: {}", self.target_name())).strong());
            if let Some(why) = self.obs.target_blocked_by {
                ui.label(RichText::new(format!("Blocked: {why}")).color(Color32::from_rgb(255, 120, 90)));
            }
            if let Some(t) = self.target_dir() {
                let sun_sep = deg(t.angle(self.bodies[0].dir));
                ui.label(RichText::new(format!("Solar elongation {sun_sep:.1}°")).weak());
            }
            let nv = self.obs.n_target_visible;
            let tiles = if self.mode == Mode::Mosaic { self.obs.n_active } else { 0 };
            let nl = if self.mode == Mode::Stack { self.obs.n_locked_target } else { nv };
            let m = obs::metrics(&self.obs_params, nl, self.obs.max_baseline_km, tiles);
            egui::Grid::new("metrics").num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
                let row = |ui: &mut egui::Ui, k: &str, v: String| {
                    ui.label(RichText::new(k).weak());
                    ui.label(v);
                    ui.end_row();
                };
                row(ui, "Telescopes with clear view", format!("{nv}"));
                if self.mode == Mode::Stack {
                    row(ui, "Locked on (integrating)", format!("{}", self.obs.n_locked_target));
                }
                if self.mode == Mode::Stack {
                    row(ui, "Collecting area", format!("{:.2} m²", m.area_m2));
                    row(ui, "Equivalent aperture", format!("{:.2} m", m.d_eq_m));
                    row(ui, "Stacked 5σ limit (est.)", if nv > 0 { format!("V ≈ {:.1}", m.lim_mag) } else { "—".into() });
                    row(ui, "Diffraction limit (each)", format!("{:.2}\"", m.diffraction_arcsec));
                    row(
                        ui,
                        "Max baseline (projected)",
                        format!("{:.0} km", self.obs.max_baseline_km),
                    );
                    if let Some(u) = m.interf_uas {
                        row(ui, "λ/B if phased (theory)", format!("{u:.2} µas"));
                    }
                } else {
                    row(ui, "Mosaic tiles placed", format!("{}", self.obs.n_active));
                    row(ui, "Mosaic area", format!("{:.0} deg²", m.mosaic_deg2));
                    row(ui, "Depth per tile (est.)", format!("V ≈ {:.1}", obs::metrics(&self.obs_params, 1, 0.0, 0).lim_mag));
                }
            });
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.step();
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.wants_keyboard_input() {
            self.target = None;
        }

        egui::SidePanel::left("controls")
            .resizable(true)
            .default_width(340.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.controls(ui));
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(2, 3, 8)))
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                match self.view {
                    View::Orbit => self.orbit_view(ui, rect, false),
                    View::Unified => self.orbit_view(ui, rect, true),
                    View::Sky => self.sky_view(ui, rect),
                    View::Split => {
                        let mid = rect.center().x;
                        let left = egui::Rect::from_min_max(rect.min, egui::pos2(mid - 1.0, rect.max.y));
                        let right = egui::Rect::from_min_max(egui::pos2(mid + 1.0, rect.min.y), rect.max);
                        self.orbit_view(ui, left, false);
                        self.sky_view(ui, right);
                        ui.painter().line_segment(
                            [egui::pos2(mid, rect.min.y), egui::pos2(mid, rect.max.y)],
                            egui::Stroke::new(1.0_f32, Color32::from_gray(40)),
                        );
                    }
                }
            });

        ctx.request_repaint();
    }
}

pub fn fmt_duration(s: f64) -> String {
    if s < 120.0 {
        format!("{s:.0} s")
    } else if s < 7200.0 {
        format!("{:.1} min", s / 60.0)
    } else if s < 3.0 * 86_400.0 {
        format!("{:.1} h", s / 3600.0)
    } else {
        format!("{:.1} days", s / 86_400.0)
    }
}
