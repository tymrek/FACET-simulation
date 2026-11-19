//! FACET simulator — an Earth-centred planetarium with a scalable shell of
//! small space telescopes inside it.

#![forbid(unsafe_code)]
#![recursion_limit = "256"]

mod allsky;
mod app;
mod astro;
mod attitude;
mod catalog;
mod constellation;
mod costs;
mod costs_ui;
mod defence;
mod draw;
mod environment;
mod ground;
mod imaging;
mod intensity;
mod math;
mod neo;
mod obs;
mod optics;
mod present;
mod public;
mod programme;
mod resilience;
mod server;
mod survey;
mod webapp;
mod tle;
mod traffic;
mod views;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("FACET simulator")
            .with_inner_size([1500.0, 920.0])
            .with_min_inner_size([900.0, 600.0]),
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "FACET simulator",
        options,
        Box::new(|cc| Box::new(app::Gui::new(cc))),
    )
}
