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
mod deepsky;
mod mosaic;
mod recovery;
mod provenance;
mod twin;
mod ui_shell;
mod public;
mod programme;
mod resilience;
mod server;
mod survey;
mod webapp;
mod tle;
mod traffic;
mod views;

#[cfg(not(target_arch = "wasm32"))]
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

/// Browser entry point: draws into the page's `<canvas id="facet_canvas">`.
#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    wasm_bindgen_futures::spawn_local(async {
        let r = eframe::WebRunner::new()
            .start("facet_canvas", eframe::WebOptions::default(), Box::new(|cc| Box::new(app::Gui::new(cc))))
            .await;
        if let Err(e) = r {
            web_sys::console::error_1(&e);
        }
    });
}
