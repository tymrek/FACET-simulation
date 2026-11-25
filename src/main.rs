//! FACET simulator — an Earth-centred planetarium with a scalable shell of
//! small space telescopes inside it.

// Unsafe only in `windows_console` below (one Win32 call).
#![deny(unsafe_code)]
#![recursion_limit = "256"]
// A GUI program on Windows: no console window of its own. Output still reaches
// a terminal it was started from (see `windows_console`).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

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

/// Started from a terminal (cmd, PowerShell), attach to its console so
/// `--help`, warnings and FACET_BENCH/FACET_PROFILE output appear there.
/// Started from Explorer there is no console and nothing is shown.
#[cfg(windows)]
#[allow(unsafe_code)]
fn windows_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    // SAFETY: AttachConsole takes a process id by value and touches no Rust
    // memory; failure (no parent console) is harmless and ignored. Rust's
    // standard streams look up the console handles on each write.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// The FACET compound-eye icon for the window and taskbar.
fn window_icon() -> Option<egui::IconData> {
    let mut dec = png::Decoder::new(&include_bytes!("../assets/icon/facet-256.png")[..]);
    dec.set_transformations(png::Transformations::EXPAND);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).ok()?;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()].chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        _ => return None,
    };
    Some(egui::IconData { rgba, width: info.width, height: info.height })
}

fn main() -> eframe::Result<()> {
    #[cfg(windows)]
    windows_console();
    let mut viewport = egui::ViewportBuilder::default();
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport: viewport
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

#[cfg(test)]
mod tests {
    #[test]
    fn window_icon_decodes() {
        let icon = super::window_icon().expect("icon");
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }
}
