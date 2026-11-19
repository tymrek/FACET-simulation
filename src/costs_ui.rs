//! The cost-model window: a separate native window (egui viewport) where line
//! items, trends and programme assumptions are edited on the fly; results
//! update live from the running simulation.

use crate::app::App;
use crate::costs::{self, fmt_money, LaunchMode, Model, SciencePerSat};
use egui::{Color32, RichText};

const FILE: &str = "facet-costs.json";

const CATS: [(&str, Color32); 7] = [
    ("Development", Color32::from_rgb(150, 120, 230)),
    ("Satellites", Color32::from_rgb(79, 163, 255)),
    ("Launch", Color32::from_rgb(255, 159, 110)),
    ("Insurance", Color32::from_rgb(200, 90, 90)),
    ("Programme", Color32::from_rgb(140, 140, 160)),
    ("Operations", Color32::from_rgb(95, 224, 160)),
    ("Data", Color32::from_rgb(255, 209, 102)),
];

impl App {
    /// Science per satellite-day from the running simulation (with sensible
    /// fallbacks before the ledger has data).
    pub fn science_per_sat(&self) -> SciencePerSat {
        let s = &self.science;
        let o = &self.obs_params;
        let n = self.shell.len().max(1) as f64;
        let sat_days = s.sat_s / 86_400.0;
        let area = std::f64::consts::PI * (o.aperture_cm / 200.0).powi(2) * (1.0 - o.obstruction * o.obstruction);
        let raw_per_sat_day = if s.elapsed_s > 0.0 { s.bytes / 1e12 / (s.elapsed_s / 86_400.0) / n } else { 0.15 };
        let tb = raw_per_sat_day * self.traffic.cfg.product.factor();
        // Until six simulated hours have passed, use typical figures: early
        // ledger values are dominated by start-up transients.
        if s.elapsed_s < 6.0 * 3600.0 || sat_days <= 0.0 {
            return SciencePerSat {
                tel_h: 0.9 * 24.0,
                completed: 8.0,
                etendue_m2deg2h: area * o.fov_deg * o.fov_deg * 0.9 * 24.0,
                tb_downlinked: tb,
                public_frac: self.public_share * 0.1,
                planes: self.shell.params.effective_planes(),
            };
        }
        let done: u32 = s.cases.iter().map(|c| c.completed).sum();
        SciencePerSat {
            tel_h: s.integrating_s / 3600.0 / sat_days,
            completed: done as f64 / sat_days,
            etendue_m2deg2h: s.etendue_s / 3600.0 / sat_days,
            tb_downlinked: tb,
            public_frac: if s.integrating_s > 0.0 { s.by_use.public / s.integrating_s } else { 0.0 },
            planes: self.shell.params.effective_planes(),
        }
    }

    pub fn cost_fleet(&self) -> usize {
        if self.cost_model.follow_sim {
            self.shell.len()
        } else {
            self.cost_model.fleet
        }
    }

    pub fn cost_result(&self) -> costs::Result {
        costs::evaluate(&self.cost_model, self.cost_fleet(), &self.science_per_sat())
    }

    /// Show the cost window if it is open.
    pub fn cost_window(&mut self, ctx: &egui::Context) {
        if !self.cost_window_open {
            return;
        }
        let id = egui::ViewportId::from_hash_of("facet_costs");
        let builder = egui::ViewportBuilder::default()
            .with_title("FACET cost model")
            .with_inner_size([1250.0, 860.0])
            .with_min_inner_size([800.0, 500.0]);
        ctx.show_viewport_immediate(id, builder, |ctx, class| {
            if class == egui::ViewportClass::Embedded {
                // Backend without multiple native windows: fall back to an in-app window.
                let mut open = true;
                egui::Window::new("FACET cost model").open(&mut open).default_size([1100.0, 760.0]).show(ctx, |ui| self.cost_ui(ui));
                if !open {
                    self.cost_window_open = false;
                }
            } else {
                egui::CentralPanel::default().show(ctx, |ui| self.cost_ui(ui));
                if ctx.input(|i| i.viewport().close_requested()) {
                    self.cost_window_open = false;
                }
            }
        });
    }

    fn cost_ui(&mut self, ui: &mut egui::Ui) {
        let sci = self.science_per_sat();
        let fleet_sim = self.shell.len();
        // ---- top bar
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Scenario").strong());
            for (name, f) in [("Baseline", Model::baseline as fn() -> Model), ("Optimistic", Model::optimistic), ("Pessimistic", Model::pessimistic)] {
                if ui.selectable_label(self.cost_model.name == name, name).clicked() {
                    let keep = (self.cost_model.follow_sim, self.cost_model.fleet);
                    self.cost_model = f();
                    (self.cost_model.follow_sim, self.cost_model.fleet) = keep;
                }
            }
            ui.separator();
            if ui.button("Save").on_hover_text(format!("Write {FILE} in the working folder")).clicked() {
                self.cost_msg = Some(match serde_json::to_string_pretty(&self.cost_model).map(|s| std::fs::write(FILE, s)) {
                    Ok(Ok(())) => format!("Saved {FILE}"),
                    Ok(Err(e)) => format!("Save failed: {e}"),
                    Err(e) => format!("Save failed: {e}"),
                });
            }
            if ui.button("Load").clicked() {
                self.cost_msg = Some(match std::fs::read_to_string(FILE).map(|s| serde_json::from_str::<Model>(&s)) {
                    Ok(Ok(m)) => {
                        self.cost_model = m;
                        format!("Loaded {FILE}")
                    }
                    Ok(Err(e)) => format!("Load failed: {e}"),
                    Err(e) => format!("Load failed: {e}"),
                });
            }
            if let Some(m) = &self.cost_msg {
                ui.label(RichText::new(m).small().weak());
            }
        });
        ui.horizontal_wrapped(|ui| {
            let m = &mut self.cost_model;
            ui.checkbox(&mut m.follow_sim, format!("Fleet follows the simulator ({fleet_sim})"));
            if !m.follow_sim {
                ui.add(egui::DragValue::new(&mut m.fleet).clamp_range(2..=100_000).speed(10).prefix("fleet "));
            }
            ui.add(egui::DragValue::new(&mut m.horizon_years).clamp_range(3..=30).prefix("horizon ").suffix(" yr"));
            ui.add(egui::DragValue::new(&mut m.first_year).clamp_range(2025..=2050).prefix("start "));
            ui.separator();
            ui.radio_value(&mut m.launch_mode, LaunchMode::Dedicated, "Dedicated launches");
            ui.radio_value(&mut m.launch_mode, LaunchMode::Rideshare, "Rideshare by mass");
            if m.launch_mode == LaunchMode::Dedicated {
                ui.add(egui::DragValue::new(&mut m.planes_per_launch).clamp_range(1.0..=8.0).speed(0.05).prefix("planes/launch "))
                    .on_hover_text("Above 1 relies on differential nodal drift after deployment (months per plane).");
            }
        });
        ui.separator();
        let r = costs::evaluate(&self.cost_model, self.cost_fleet(), &sci);

        egui::SidePanel::left("cost_items").resizable(true).default_width(560.0).show_inside(ui, |ui| {
            ui.label(RichText::new("Line items").strong());
            ui.label(RichText::new("Value now · annual trend (negative = getting cheaper). Hover a row for its source or assumption.").small().weak());
            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut group = String::new();
                let items = &mut self.cost_model.items;
                egui::Grid::new("cost_grid").num_columns(4).striped(true).spacing([8.0, 4.0]).show(ui, |ui| {
                    for it in items.iter_mut() {
                        if it.group != group {
                            group = it.group.clone();
                            ui.label(RichText::new(&group).strong().color(Color32::from_rgb(150, 190, 255)));
                            ui.end_row();
                        }
                        ui.label(&it.label).on_hover_text(&it.note);
                        let speed = (it.value.abs() * 0.005).max(0.01);
                        let dv = egui::DragValue::new(&mut it.value).speed(speed).clamp_range(0.0..=f64::MAX);
                        let dv = if it.unit == "$" { dv.custom_formatter(|v, _| fmt_money(v)) } else { dv.suffix(format!(" {}", it.unit)) };
                        ui.add(dv).on_hover_text(&it.note);
                        let tr = egui::DragValue::new(&mut it.trend_pct).speed(0.1).clamp_range(-50.0..=50.0).suffix(" %/yr");
                        let resp = ui.add(tr);
                        let col = if it.trend_pct > 0.0 { Color32::from_rgb(255, 140, 120) } else if it.trend_pct < 0.0 { Color32::from_rgb(120, 230, 160) } else { Color32::GRAY };
                        ui.label(RichText::new(if it.trend_pct > 0.0 { "rising" } else if it.trend_pct < 0.0 { "falling" } else { "flat" }).small().color(col));
                        let _ = resp;
                        ui.end_row();
                    }
                });
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.horizontal(|ui| { ui.label(RichText::new(format!("{} · {} satellites · {} – {}", self.cost_model.name, r.fleet, self.cost_model.first_year, self.cost_model.first_year + self.cost_model.horizon_years as i32 - 1)).strong()); crate::provenance::chip(ui, "costs"); });
                let opt = |x: Option<f64>| x.map(fmt_money).unwrap_or_else(|| "–".into());
                let tiles: Vec<(String, String)> = vec![
                    (format!("{}-year programme cost", self.cost_model.horizon_years), fmt_money(r.total)),
                    ("first science".into(), r.first_science_year.map(|y| y.to_string()).unwrap_or("–".into())),
                    ("full fleet".into(), r.full_fleet_year.map(|y| y.to_string()).unwrap_or("not reached".into())),
                    ("annual cost, final year".into(), fmt_money(r.steady_annual)),
                    ("average satellite (hardware)".into(), fmt_money(r.avg_unit_cost)),
                    ("per telescope-hour".into(), opt(r.cost_per_tel_hour)),
                    ("per completed observation".into(), opt(r.cost_per_observation)),
                    ("per m²·deg²·h étendue".into(), opt(r.cost_per_etendue_hour)),
                    ("per TB delivered".into(), opt(r.cost_per_tb)),
                    ("per public telescope-hour".into(), opt(r.cost_per_public_hour)),
                ];
                let tile_w = 160.0;
                let cols = ((ui.available_width() / (tile_w + 12.0)).floor() as usize).max(1);
                egui::Grid::new("cost_tiles").num_columns(cols).spacing([8.0, 8.0]).show(ui, |ui| {
                    for (k, (label, v)) in tiles.iter().enumerate() {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_width(tile_w);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(v).size(18.0).strong());
                                ui.label(RichText::new(label).small().weak());
                            });
                        });
                        if (k + 1) % cols == 0 {
                            ui.end_row();
                        }
                    }
                });
                ui.label(RichText::new(format!(
                    "Science per satellite-day from the running simulation: {:.1} telescope-h, {:.1} observations, {:.2} m²·deg²·h, {:.3} TB downlinked ({}).",
                    sci.tel_h, sci.completed, sci.etendue_m2deg2h, sci.tb_downlinked, self.traffic.cfg.product.label()
                )).small().weak());
                for n in &r.notes {
                    ui.label(RichText::new(format!("Note: {n}")).small().color(Color32::from_rgb(255, 196, 107)));
                }
                ui.add_space(6.0);
                cost_chart(ui, &r);
                ui.add_space(6.0);
                ui.label(RichText::new("Year by year").strong());
                egui::ScrollArea::horizontal().id_source("years_h").show(ui, |ui| {
                egui::Grid::new("cost_years").striped(true).num_columns(12).spacing([10.0, 3.0]).show(ui, |ui| {
                    for h in ["Year", "Fleet", "Built", "Launches", "Development", "Satellites", "Launch", "Insurance", "Programme", "Operations", "Data", "Total", "Cumulative"] {
                        ui.label(RichText::new(h).small().strong());
                    }
                    ui.end_row();
                    for y in &r.years {
                        ui.label(y.year.to_string());
                        ui.label(format!("{:.0}", y.fleet_end));
                        ui.label(format!("{:.0}", y.built));
                        ui.label(format!("{:.0}", y.launches));
                        for v in [y.development, y.satellites, y.launch, y.insurance, y.programme, y.operations, y.data, y.total, y.cumulative] {
                            ui.label(RichText::new(fmt_money(v)).monospace());
                        }
                        ui.end_row();
                    }
                });
                });
                ui.add_space(6.0);
                ui.label(RichText::new("Funding tranches: what a budget buys").strong());
                egui::Grid::new("tranches").striped(true).show(ui, |ui| {
                    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
                    for (t, date, fleet) in &r.tranches {
                        ui.label(fmt_money(*t));
                        ui.label(match date {
                            Some(d) => {
                                let y = d.floor();
                                let m = ((d - y) * 12.0).floor().clamp(0.0, 11.0) as usize;
                                format!("lasts until about {} {}, with {:.0} satellites in orbit", months[m], y as i32, fleet)
                            }
                            None => format!("covers the whole {}-year plan ({:.0} satellites)", self.cost_model.horizon_years, fleet),
                        });
                        ui.end_row();
                    }
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Launch prices are 2026 public list prices; other line items are assumptions to be replaced with quotes.").small().weak());
            });
        });
    }
}

fn cost_chart(ui: &mut egui::Ui, r: &costs::Result) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.max_rect().width().clamp(400.0, 900.0), 220.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, Color32::from_rgb(10, 13, 20));
    if r.years.is_empty() {
        return;
    }
    let maxv = r.years.iter().map(|y| y.total).fold(1.0, f64::max);
    let maxf = r.years.iter().map(|y| y.fleet_end).fold(1.0, f64::max);
    let (l, rr, t, b) = (rect.left() + 60.0, rect.right() - 50.0, rect.top() + 10.0, rect.bottom() - 34.0);
    let w = (rr - l) / r.years.len() as f32;
    let ys = |v: f64| b - (v / maxv) as f32 * (b - t);
    for k in 0..=4 {
        let v = maxv * k as f64 / 4.0;
        let y = ys(v);
        p.line_segment([egui::pos2(l, y), egui::pos2(rr, y)], egui::Stroke::new(0.5_f32, Color32::from_gray(40)));
        p.text(egui::pos2(l - 4.0, y), egui::Align2::RIGHT_CENTER, fmt_money(v), egui::FontId::proportional(10.0), Color32::GRAY);
    }
    let mut fleet_pts = Vec::new();
    for (k, y) in r.years.iter().enumerate() {
        let x0 = l + w * k as f32 + w * 0.15;
        let x1 = l + w * (k as f32 + 1.0) - w * 0.15;
        let mut acc = 0.0;
        for (v, (_, col)) in [y.development, y.satellites, y.launch, y.insurance, y.programme, y.operations, y.data].iter().zip(CATS) {
            let (y0, y1) = (ys(acc), ys(acc + v));
            p.rect_filled(egui::Rect::from_min_max(egui::pos2(x0, y1), egui::pos2(x1, y0)), 0.0, col);
            acc += v;
        }
        p.text(egui::pos2(0.5 * (x0 + x1), b + 4.0), egui::Align2::CENTER_TOP, y.year.to_string(), egui::FontId::proportional(10.0), Color32::GRAY);
        fleet_pts.push(egui::pos2(0.5 * (x0 + x1), b - (y.fleet_end / maxf) as f32 * (b - t)));
    }
    p.add(egui::Shape::line(fleet_pts, egui::Stroke::new(2.0_f32, Color32::WHITE)));
    p.text(egui::pos2(rr + 4.0, t), egui::Align2::LEFT_TOP, format!("{maxf:.0} sats"), egui::FontId::proportional(10.0), Color32::WHITE);
    let mut x = l;
    for (name, col) in CATS {
        p.rect_filled(egui::Rect::from_min_size(egui::pos2(x, rect.bottom() - 14.0), egui::vec2(9.0, 9.0)), 1.0, col);
        x += 12.0;
        let g = p.text(egui::pos2(x, rect.bottom() - 15.0), egui::Align2::LEFT_TOP, name, egui::FontId::proportional(10.0), Color32::GRAY);
        x = g.right() + 12.0;
    }
    p.text(egui::pos2(x, rect.bottom() - 15.0), egui::Align2::LEFT_TOP, "— fleet in orbit", egui::FontId::proportional(10.0), Color32::WHITE);
}
