//! The native window's layout: a headline strip across the top, and a side
//! panel with a fixed header (time, scenario, target search) above tabs that
//! group the controls.

use crate::app::{App, Target, FIXED_TARGETS};
use crate::astro::BODY_NAMES;
use crate::constellation::ShellParams;
use crate::math::{rad, V3};
use crate::obs::{Mode, ObsParams};
use egui::{Color32, RichText};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Tab {
    #[default]
    Operations,
    Fleet,
    Telescope,
    Science,
    Public,
    Environment,
    Interferometry,
    Display,
}

const TABS: [(Tab, &str, &str); 8] = [
    (Tab::Operations, "Operations", "Mode, target, what the fleet is doing now, all-sky survey, web servers"),
    (Tab::Fleet, "Fleet", "Constellation, attitude control, keep-out constraints, failures, traffic"),
    (Tab::Telescope, "Telescope", "Optics, detector, pointing, fleet variation, photometric precision"),
    (Tab::Science, "Science", "Science programme, near-Earth asteroids, planetary defence"),
    (Tab::Public, "Public", "Public engagement"),
    (Tab::Environment, "Environment", "SAA, Earthshine, stray light, orbital lifetime"),
    (Tab::Interferometry, "Interferometry & fuel", "Intensity-interferometry clusters and the propellant budget"),
    (Tab::Display, "Display", "Views, overlays, credits"),
];

/// One search result.
#[derive(Clone)]
pub struct Hit {
    pub label: String,
    pub kind: &'static str,
    pub target: Target,
}

/// Values for the headline strip, refreshed about once a second.
#[derive(Default, Clone)]
pub struct Headlines {
    pub at: f64,
    pub items: Vec<(String, String, &'static str)>, // label, value, provenance key
}

/// A saved set of settings. Missing fields take their defaults, so older
/// files keep loading as settings are added.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Scenario {
    pub name: String,
    pub note: String,
    pub shell: ShellParams,
    pub sun_sync: bool,
    pub obs: ObsParams,
    pub mode: String,
    pub rate: f64,
    pub programme_on: bool,
    pub programme_load: f64,
    pub public_on: bool,
    pub public_users: usize,
    pub saa_policy: String,
    pub handover: bool,
}

impl Default for Scenario {
    fn default() -> Self {
        Scenario {
            name: "Default".into(),
            note: String::new(),
            shell: ShellParams::default(),
            sun_sync: false,
            obs: ObsParams::default(),
            mode: "queue".into(),
            rate: 60.0,
            programme_on: true,
            programme_load: 1.5,
            public_on: true,
            public_users: 20_000,
            saa_policy: "pause".into(),
            handover: true,
        }
    }
}

fn mode_key(m: Mode) -> &'static str {
    match m {
        Mode::Survey => "survey",
        Mode::Stack => "stack",
        Mode::Mosaic => "mosaic",
        Mode::NeoSweep => "neo",
        Mode::Queue => "queue",
        Mode::AllSky => "allsky",
    }
}

fn mode_from(s: &str) -> Mode {
    match s {
        "survey" => Mode::Survey,
        "stack" => Mode::Stack,
        "mosaic" => Mode::Mosaic,
        "neo" => Mode::NeoSweep,
        "allsky" => Mode::AllSky,
        _ => Mode::Queue,
    }
}

/// Built-in scenarios. The conservative and optimistic sets bracket the
/// ASSUMED engineering inputs; none is a validated design point.
pub fn builtin_scenarios() -> Vec<Scenario> {
    let base = Scenario::default();
    let mut s1k = base.clone();
    s1k.name = "1k stage (defaults)".into();
    s1k.note = "1,000 satellites, default assumptions.".into();
    s1k.shell.n = 1000;

    let mut pilot = base.clone();
    pilot.name = "100-satellite pilot".into();
    pilot.note = "First deployment stage.".into();
    pilot.shell.n = 100;

    let mut cons = s1k.clone();
    cons.name = "1k stage, conservative".into();
    cons.note = "Worse optics and pointing, higher photometric floor, brighter stray light.".into();
    cons.obs.wfe_nm = 90.0;
    cons.obs.jitter_arcsec = 1.0;
    cons.obs.sys_floor_ppm = 1000.0;
    cons.obs.wfe_spread = 0.45;
    cons.obs.stray_sb45 -= 1.0;
    cons.programme_load = 2.0;

    let mut opt = base.clone();
    opt.name = "10k stage, optimistic".into();
    opt.note = "10,000 satellites, fine steering mirror, good optics, low photometric floor.".into();
    opt.shell.n = 10_000;
    opt.obs.apply_pointing(2);
    opt.obs.wfe_nm = 40.0;
    opt.obs.sys_floor_ppm = 200.0;
    opt.obs.wfe_spread = 0.2;
    vec![s1k, pilot, cons, opt]
}

fn scenario_dir() -> std::path::PathBuf {
    std::path::PathBuf::from("scenarios")
}

/// Scenarios saved as JSON in ./scenarios.
pub fn saved_scenarios() -> Vec<Scenario> {
    let mut v: Vec<Scenario> = std::fs::read_dir(scenario_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().map_or(false, |x| x == "json"))
        .filter_map(|e| serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// Find a scenario by name (built-in or saved) or load it from a path.
pub fn find_scenario(q: &str) -> Option<Scenario> {
    if let Ok(t) = std::fs::read_to_string(q) {
        return serde_json::from_str(&t).ok();
    }
    let ql = q.to_lowercase();
    builtin_scenarios().into_iter().chain(saved_scenarios()).find(|s| s.name.to_lowercase() == ql || s.name.to_lowercase().starts_with(&ql))
}

impl App {
    pub fn capture_scenario(&self, name: &str) -> Scenario {
        Scenario {
            name: name.into(),
            note: String::new(),
            shell: self.shell_params.clone(),
            sun_sync: self.sun_sync,
            obs: self.obs_params.clone(),
            mode: mode_key(self.mode).into(),
            rate: self.rate,
            programme_on: self.gen.on,
            programme_load: self.gen.load,
            public_on: self.public.cfg.on,
            public_users: self.public.cfg.population,
            saa_policy: match self.env.cfg.saa_policy {
                crate::environment::SaaPolicy::Pause => "pause",
                crate::environment::SaaPolicy::Flag => "flag",
                crate::environment::SaaPolicy::Off => "off",
            }
            .into(),
            handover: self.handover,
        }
    }

    pub fn apply_scenario(&mut self, s: &Scenario) {
        use crate::environment::SaaPolicy;
        self.shell_params = s.shell.clone();
        self.sun_sync = s.sun_sync;
        self.obs_params = s.obs.clone();
        self.obs_params.derive();
        self.mode = mode_from(&s.mode);
        self.rate = s.rate;
        self.gen.on = s.programme_on;
        self.gen.load = s.programme_load;
        self.public.cfg.on = s.public_on;
        self.public.cfg.population = s.public_users;
        self.env.cfg.saa_policy = match s.saa_policy.as_str() {
            "flag" => SaaPolicy::Flag,
            "off" => SaaPolicy::Off,
            _ => SaaPolicy::Pause,
        };
        self.handover = s.handover;
        self.scenario_name = s.name.clone();
    }

    pub fn save_scenario(&mut self, name: &str) -> Result<std::path::PathBuf, String> {
        let s = self.capture_scenario(name);
        std::fs::create_dir_all(scenario_dir()).map_err(|e| e.to_string())?;
        let slug: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        let path = scenario_dir().join(format!("{}.json", slug.trim_matches('-')));
        std::fs::write(&path, serde_json::to_string_pretty(&s).unwrap()).map_err(|e| e.to_string())?;
        self.scenario_name = name.into();
        Ok(path)
    }

    // ------------------------------------------------------------ search

    pub fn search_targets(&self, q: &str) -> Vec<Hit> {
        let q = q.trim();
        let mut out = Vec::new();
        if q.len() < 2 {
            return out;
        }
        // Coordinates: "RA Dec" in decimal degrees.
        let nums: Vec<f64> = q.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect();
        if nums.len() == 2 && (0.0..360.0).contains(&nums[0]) && (-90.0..=90.0).contains(&nums[1]) {
            out.push(Hit {
                label: format!("RA {:.3}°, Dec {:+.3}°", nums[0], nums[1]),
                kind: "coordinates",
                target: Target::Fixed { name: format!("RA {:.2} Dec {:+.2}", nums[0], nums[1]), dir: V3::from_radec(rad(nums[0]), rad(nums[1])) },
            });
            return out;
        }
        let ql = q.to_lowercase();
        for (i, b) in BODY_NAMES.iter().enumerate() {
            if b.to_lowercase().starts_with(&ql) {
                out.push(Hit { label: (*b).into(), kind: "Solar System", target: Target::Body(i) });
            }
        }
        for (name, ra, dec) in FIXED_TARGETS {
            if name.to_lowercase().contains(&ql) {
                out.push(Hit { label: name.into(), kind: "target", target: Target::Fixed { name: name.into(), dir: V3::from_radec(rad(ra), rad(dec)) } });
            }
        }
        for s in &self.names {
            if out.len() >= 30 {
                break;
            }
            if s.name.to_lowercase().starts_with(&ql) {
                out.push(Hit { label: format!("{} (V {:.1})", s.name, s.mag), kind: "star", target: Target::Fixed { name: s.name.clone(), dir: s.dir } });
            }
        }
        let mut n_dso = 0;
        for d in crate::catalog::dso() {
            if n_dso >= 10 {
                break;
            }
            if d.matches(q) {
                n_dso += 1;
                out.push(Hit { label: d.name(), kind: "deep sky", target: Target::Fixed { name: d.name(), dir: d.dir } });
            }
        }
        let neos = crate::neo::all();
        let mut n_neo = 0;
        for (i, n) in neos.iter().enumerate() {
            if n_neo >= 8 {
                break;
            }
            if n.name.to_lowercase().contains(&ql) {
                if let Some(&dir) = self.neo.dir.get(i) {
                    n_neo += 1;
                    let v = self.neo.vmag.get(i).copied().unwrap_or(f32::NAN);
                    out.push(Hit {
                        label: format!("{} (V {:.1} now)", n.name, v),
                        kind: "NEO",
                        target: Target::Fixed { name: n.name.to_string(), dir },
                    });
                }
            }
        }
        out.truncate(24);
        out
    }

    // ------------------------------------------------------------ headlines

    fn refresh_headlines(&mut self, now: f64) {
        if now - self.headlines.at < 1.0 && !self.headlines.items.is_empty() {
            return;
        }
        let mut it: Vec<(String, String, &'static str)> = Vec::new();
        let n = self.shell.len();
        let failed = self.resil.n_failed();
        it.push(("Fleet".into(), if failed > 0 { format!("{} sats ({failed} failed)", n) } else { format!("{n} satellites") }, "build_out"));
        match self.science.duty() {
            Some(d) => it.push(("Observing".into(), format!("{:.0} % of the time", 100.0 * d), "duty")),
            None => it.push(("Observing".into(), "measuring…".into(), "duty")),
        }
        let days = (self.science.elapsed_s / 86_400.0).max(1e-9);
        let done: u32 = self.science.cases.iter().map(|c| c.completed).sum();
        let on: u32 = self.science.cases.iter().map(|c| c.on_time).sum();
        let fin: u32 = self.science.cases.iter().map(|c| c.completed + c.expired + c.failed).sum();
        if self.science.elapsed_s > 600.0 && fin > 0 {
            it.push(("Science".into(), format!("{:.0}/day, {:.0} % met", done as f64 / days, 100.0 * on as f64 / fin as f64), "science"));
        }
        let hw = crate::imaging::Hardware::from_obs(&self.obs_params);
        let nominal = crate::imaging::point_lim_mag(&hw, 60.0, 60.0, 22.0);
        match crate::optics::fleet_spread(&self.obs_params) {
            Some(f) => {
                it.push(("60 s depth".into(), format!("V {:.1}–{:.1}", f.depth_v60.0, f.depth_v60.2), "fleet_optics"));
                it.push(("Image size".into(), format!("{:.2}\" median", f.fwhm.1), "fleet_optics"));
            }
            None => it.push(("60 s depth".into(), format!("V {nominal:.1}"), "depth")),
        }
        if let Some(p) = self.survey.history.last() {
            it.push(("All-sky pass".into(), crate::allsky::fmt_span(p.seconds), "allsky_measured"));
        } else if let Some(p) = self.allsky().passes.first() {
            it.push(("All-sky pass (est.)".into(), crate::allsky::fmt_span(p.seconds), "allsky_estimate"));
        }
        if self.public.cfg.on {
            let pj = self.public_json();
            if let Some(y) = pj["images_per_year"].as_f64().filter(|y| *y > 0.0) {
                let v = if y >= 1e6 { format!("{:.1} M images/yr", y / 1e6) } else { format!("{:.0}k images/yr", y / 1e3) };
                it.push(("Public".into(), v, "public_service"));
            }
        }
        let pj = crate::imaging::precision_json(&self.obs_params);
        if let Some(t) = pj["rows"][1]["transit_5sigma_ppm"].as_f64() {
            it.push(("Transit 5σ (V 10, 4 tel.)".into(), format!("{:.0} ppm", t), "photometry"));
        }
        self.headlines = Headlines { at: now, items: it };
    }

    pub fn headline_strip(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.refresh_headlines(now);
        egui::TopBottomPanel::top("headlines").frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(10.0, 5.0))).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (k, (label, value, key)) in self.headlines.items.iter().enumerate() {
                    if k > 0 {
                        ui.separator();
                    }
                    ui.label(RichText::new(label).small().weak());
                    ui.label(RichText::new(value).strong());
                    crate::provenance::chip(ui, key);
                }
            });
        });
    }

    // ------------------------------------------------------------ side panel

    pub fn side_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("controls").resizable(true).default_width(370.0).min_width(300.0).show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("FACET simulator").size(17.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Costs").on_hover_text("Programme costs in a separate window").clicked() {
                        self.cost_window_open = true;
                    }
                    if ui.small_button("Present").on_hover_text("Narrated, scripted tours for an audience (full screen)").clicked() {
                        self.present_open = true;
                    }
                });
            });
            self.sec_time(ui);
            self.scenario_row(ui);
            self.search_row(ui);
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                for (t, label, hover) in TABS {
                    let sel = self.tab == t;
                    let txt = if sel { RichText::new(label).strong().color(Color32::WHITE) } else { RichText::new(label) };
                    if ui.add(egui::SelectableLabel::new(sel, txt)).on_hover_text(hover).clicked() {
                        self.tab = t;
                    }
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().id_source(format!("tab{:?}", self.tab)).auto_shrink([false, false]).show(ui, |ui| {
                match self.tab {
                    Tab::Operations => {
                        self.sec_mode_target(ui);
                        ui.separator();
                        self.stats(ui);
                        ui.separator();
                        self.survey_panel(ui);
                        ui.separator();
                        self.web_panel(ui);
                    }
                    Tab::Fleet => {
                        self.sec_constellation(ui);
                        ui.separator();
                        self.sec_attitude(ui);
                        ui.separator();
                        self.sec_constraints(ui);
                        ui.separator();
                        self.resilience_panel(ui);
                        ui.separator();
                        self.traffic_panel(ui);
                    }
                    Tab::Telescope => self.sec_telescope(ui),
                    Tab::Science => {
                        self.science_panel(ui);
                        ui.separator();
                        self.neo_panel(ui);
                        ui.separator();
                        self.defence_panel(ui);
                    }
                    Tab::Public => self.public_panel(ui),
                    Tab::Environment => self.environment_panel(ui),
                    Tab::Interferometry => {
                        self.ii_panel(ui);
                        ui.separator();
                        self.fuel_panel(ui);
                    }
                    Tab::Display => self.sec_display(ui),
                }
                ui.add_space(12.0);
            });
        });
    }

    fn scenario_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Scenario").weak());
            let mut pick: Option<Scenario> = None;
            egui::ComboBox::from_id_source("scenario")
                .selected_text(if self.scenario_name.is_empty() { "custom".to_string() } else { self.scenario_name.clone() })
                .width(190.0)
                .show_ui(ui, |ui| {
                    ui.label(RichText::new("Built in").small().weak());
                    for s in builtin_scenarios() {
                        if ui.selectable_label(self.scenario_name == s.name, &s.name).on_hover_text(&s.note).clicked() {
                            pick = Some(s);
                        }
                    }
                    let saved = saved_scenarios();
                    if !saved.is_empty() {
                        ui.separator();
                        ui.label(RichText::new("Saved").small().weak());
                        for s in saved {
                            if ui.selectable_label(self.scenario_name == s.name, &s.name).clicked() {
                                pick = Some(s);
                            }
                        }
                    }
                });
            if let Some(s) = pick {
                self.apply_scenario(&s);
                self.scenario_msg = Some(format!("Loaded “{}”", s.name));
            }
            if ui.small_button("Save…").on_hover_text("Save the current settings as a named scenario (./scenarios/*.json)").clicked() {
                self.scenario_saving = true;
                if self.scenario_save_name.is_empty() {
                    self.scenario_save_name = "My scenario".into();
                }
            }
        });
        if self.scenario_saving {
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.scenario_save_name).desired_width(170.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.small_button("Save").clicked() || enter {
                    let name = self.scenario_save_name.trim().to_string();
                    if !name.is_empty() {
                        self.scenario_msg = Some(match self.save_scenario(&name) {
                            Ok(p) => format!("Saved {}", p.display()),
                            Err(e) => format!("Save failed: {e}"),
                        });
                        self.scenario_saving = false;
                    }
                }
                if ui.small_button("Cancel").clicked() {
                    self.scenario_saving = false;
                }
            });
        }
        if let Some(m) = &self.scenario_msg {
            ui.label(RichText::new(m).small().weak());
        }
    }

    fn search_row(&mut self, ui: &mut egui::Ui) {
        let r = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Find a target: Vega, M31, Mars, Apophis, or RA Dec in degrees")
                .desired_width(f32::INFINITY),
        );
        if r.changed() {
            self.scenario_msg = None;
            self.search_hits = self.search_targets(&self.search);
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if self.search.trim().len() >= 2 {
            let mut chosen: Option<Hit> = None;
            if enter {
                chosen = self.search_hits.first().cloned();
            }
            egui::Frame::group(ui.style()).inner_margin(4.0).show(ui, |ui| {
                if self.search_hits.is_empty() {
                    ui.label(RichText::new("No match").small().weak());
                }
                egui::ScrollArea::vertical().id_source("hits").max_height(180.0).show(ui, |ui| {
                    for h in &self.search_hits {
                        ui.horizontal(|ui| {
                            if ui.selectable_label(false, &h.label).clicked() {
                                chosen = Some(h.clone());
                            }
                            ui.label(RichText::new(h.kind).small().weak());
                        });
                    }
                });
            });
            if let Some(h) = chosen {
                if h.kind == "NEO" {
                    self.scenario_msg = Some(format!("{}: position at selection; NEOs move, so re-select to update", h.label));
                }
                self.target = Some(h.target);
                self.look_at_target();
                self.sky_cam.fov_deg = self.sky_cam.fov_deg.min(40.0);
                if matches!(self.view, crate::app::View::Orbit) {
                    self.view = crate::app::View::Split;
                }
                self.search.clear();
                self.search_hits.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenarios_round_trip() {
        for s in builtin_scenarios() {
            let j = serde_json::to_string(&s).unwrap();
            let back: Scenario = serde_json::from_str(&j).unwrap();
            assert_eq!(back.shell, s.shell);
            assert_eq!(back.obs, s.obs);
        }
        // Old files with missing fields still load.
        let partial: Scenario = serde_json::from_str(r#"{"name":"x","shell":{"n":42}}"#).unwrap();
        assert_eq!(partial.shell.n, 42);
        assert_eq!(partial.obs, ObsParams::default());
    }
}
