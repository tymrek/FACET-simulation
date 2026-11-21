//! Where each headline number comes from.
//!
//! Every figure FACET puts in front of people is tagged with one of four
//! bases, so a reader can tell a result of running the simulated fleet from
//! a formula, an input someone chose, or real external data:
//!
//! * **simulated**: measured from the simulated fleet as it runs (scheduling,
//!   slews, eclipses, the SAA, queues). It inherits every model and
//!   assumption underneath it; it is not a measurement of hardware.
//! * **modelled**: computed from a physical or engineering model with stated
//!   inputs, not from running the fleet.
//! * **assumed**: an input or scenario someone chose (adoption, demand mix,
//!   build-out, prices). Changing it changes the answer.
//! * **data**: taken from real external data (MPC orbits, star catalogues).
//!
//! The registry below is served in `/api/status` as `provenance` and drawn as
//! a chip beside each headline in the native panels and the web pages.

use serde_json::{json, Value};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Basis {
    Simulated,
    Modelled,
    Assumed,
    Data,
}

impl Basis {
    pub fn word(self) -> &'static str {
        match self {
            Basis::Simulated => "simulated",
            Basis::Modelled => "modelled",
            Basis::Assumed => "assumed",
            Basis::Data => "data",
        }
    }
    pub fn meaning(self) -> &'static str {
        match self {
            Basis::Simulated => "Measured from the simulated fleet as it runs; inherits every model and assumption beneath it.",
            Basis::Modelled => "Computed from a physical or engineering model with stated inputs, not from running the fleet.",
            Basis::Assumed => "An input or scenario chosen for the simulation; change it and the answer changes.",
            Basis::Data => "Taken from real external data.",
        }
    }
    pub fn colour(self) -> egui::Color32 {
        match self {
            Basis::Simulated => egui::Color32::from_rgb(90, 200, 140),
            Basis::Modelled => egui::Color32::from_rgb(110, 170, 255),
            Basis::Assumed => egui::Color32::from_rgb(255, 170, 70),
            Basis::Data => egui::Color32::from_rgb(200, 200, 200),
        }
    }
}

use Basis::*;

/// (key, basis, what the figure rests on)
pub const HEADLINES: &[(&str, Basis, &str)] = &[
    // Imaging
    ("depth", Modelled, "Fourier-optics PSF, sensor QE and read noise, zodiacal + Earthshine + stray-light background; noise-limited only, no photometric systematics; nominal telescope (see fleet spread for the variation)."),
    ("fwhm", Modelled, "Diffraction with obstruction, spiders and an imposed 60 nm RMS wavefront error, plus pointing jitter at the full RMS figure for every frame length."),
    ("fleet_optics", Modelled, "Per-telescope random Zernike patterns, lognormal wavefront-error scatter and throughput scatter (all assumed distributions), each through the full Fourier-optics PSF."),
    ("photometry", Modelled, "Photon, sky, dark and read noise plus an assumed systematic floor (flat-field, intrapixel, pointing-jitter flux scatter); not validated against flight data."),
    // Duty and environment
    ("duty", Simulated, "Fraction of satellite time spent integrating after slews, settling, eclipses-by-target, SAA holds and Earthshine losses in this run."),
    ("saa", Simulated, "Time the fleet spends inside a two-lobe Gaussian proton-flux model of the South Atlantic Anomaly (modelled flux, simulated exposure)."),
    ("earthshine", Simulated, "Dark-sky-equivalent time lost to a limb-brightness model (19.0 mag/arcsec² at 10°, falling 6 mag per decade), accumulated over this run."),
    ("stray", Modelled, "Parametric sunward stray-light law (elongation⁻³, tapered 60–100°); needs ray tracing and baffle tests to validate."),
    ("stray_loss", Simulated, "Dark-sky-equivalent time the fleet loses to the parametric sunward stray-light law, accumulated over this run."),
    ("keepout", Modelled, "Sun keep-out trade computed from the simulation's ephemeris; planet visibility is geometry, the stray-light cost is the parametric model."),
    ("lifetime", Modelled, "CIRA-72 densities scaled by an assumed solar-cycle ratio, with an assumed mass, area and drag coefficient."),
    ("ground_equiv", Modelled, "Simulated FACET duty against a modelled ground duty (night, weather, airmass, seeing statistics for good sites)."),
    // All-sky
    ("allsky_estimate", Modelled, "Fields × (exposure + overhead) ÷ (telescopes × duty); duty is simulated once the run has measured it."),
    ("allsky_measured", Simulated, "Wall-clock time for the simulated fleet in survey mode to tile the whole unexcluded sky, including slews, settling and SAA losses."),
    ("allsky_data", Modelled, "Pixels × bytes per pixel × fields; uncompressed raw frames."),
    // Science programme
    ("science", Simulated, "Completions, response times and on-time rates for requests from a synthetic programme (assumed investigators, targets and demand)."),
    ("science_demand", Assumed, "The research programme's investigators, targets and request rates are synthetic."),
    ("stare", Simulated, "Coverage of STARE campaigns by the simulated fleet, with drift-corrected handover prediction."),
    ("defence", Simulated, "Warning time and impact probability from a scripted impactor scenario run through the simulated fleet and a modelled ground network."),
    // Public
    ("public_users", Assumed, "Monthly user population and the casual / amateur / classroom / citizen-science mix and habits."),
    ("public_service", Simulated, "Waits and deliveries for public requests scheduled and observed by the simulated fleet; the demand behind them is assumed."),
    ("public_stages", Modelled, "Users served at each build-out stage, scaled from this run at the same demand mix."),
    ("public_live", Data, "Requests from real visitors to the public site."),
    // NEOs
    ("neo_catalogue", Data, "IAU Minor Planet Center list of known near-Earth asteroids."),
    ("neo_visible", Modelled, "Two-body positions from MPC orbits, H,G magnitudes and the modelled depth limit."),
    ("neo_approaches", Modelled, "Two-body propagation without planetary perturbations; dates typically within hours, distances within 10–20 %."),
    // Interferometry and propulsion
    ("ii_snr", Modelled, "Hanbury Brown–Twiss SNR with assumed timing jitter, synchronisation, throughput and channel count."),
    ("ii_fit", Simulated, "Angular diameter fitted to |V|² samples the simulated cluster accumulates, with noise from the modelled SNR."),
    ("twin", Simulated, "Angular diameter fitted to |V|² samples twin-telescope satellites accumulate as they observe; SNR from the intensity-interferometry model with an assumed boom, metrology and published approximate diameters."),
    ("fuel", Modelled, "Rocket equation over drag make-up, an assumed collision-avoidance allowance, disposal and cluster keeping, with a 25 % margin."),
    // Operations
    ("resilience", Simulated, "Fleet output with random failures and replacement at an assumed rate and delay, run through the simulated scheduler."),
    ("downlink", Modelled, "Data volume from the science ledger against assumed relay and ground-station capacities; a capacity graph, not a link budget."),
    ("streaks", Simulated, "Sampled exposures crossed by catalogued satellites propagated alongside the fleet (modelled populations)."),
    ("conjunctions", Modelled, "Screening against modelled traffic populations with assumed covariance and Pc thresholds."),
    // Build-out
    ("build_out", Assumed, "Fleet size, launch cadence and deployment stages."),
    ("costs", Assumed, "Unit, launch and operations prices. Twin-telescope satellites are priced as ordinary satellites (their extra hardware, mass and propellant are not costed)."),
];

pub fn get(key: &str) -> Option<(Basis, &'static str)> {
    HEADLINES.iter().find(|(k, _, _)| *k == key).map(|&(_, b, r)| (b, r))
}

pub fn json() -> Value {
    let mut m = serde_json::Map::new();
    for (k, b, r) in HEADLINES {
        m.insert((*k).into(), json!({ "basis": b.word(), "rests_on": r }));
    }
    let bases: Vec<Value> = [Simulated, Modelled, Assumed, Data]
        .iter()
        .map(|b| json!({"basis": b.word(), "meaning": b.meaning()}))
        .collect();
    json!({ "items": Value::Object(m), "bases": bases })
}

/// A small coloured chip naming the basis of `key`, with what it rests on on hover.
pub fn chip(ui: &mut egui::Ui, key: &str) {
    let Some((b, rests)) = get(key) else { return };
    let t = egui::RichText::new(b.word()).small().color(b.colour());
    ui.label(t).on_hover_text(format!("{}\n\n{}", b.meaning(), rests));
}

/// A headline line led by its basis chip.
pub fn line(ui: &mut egui::Ui, key: &str, text: impl Into<String>) {
    ui.horizontal_wrapped(|ui| {
        chip(ui, key);
        ui.label(egui::RichText::new(text.into()).small());
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn keys_are_unique_and_described() {
        let mut ks: Vec<_> = super::HEADLINES.iter().map(|h| h.0).collect();
        ks.sort();
        let n = ks.len();
        ks.dedup();
        assert_eq!(n, ks.len(), "duplicate provenance key");
        assert!(super::HEADLINES.iter().all(|h| h.2.len() > 20));
    }
}
