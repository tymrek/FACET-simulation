//! How long the fleet would take to image the whole observable sky.
//!
//! An estimate from fleet throughput, not a scheduled simulation:
//!
//!   pass time = fields × (exposure + overhead) / (working telescopes × duty)
//!
//! * observable sky = the celestial sphere minus the Sun and Moon keep-outs
//!   right now (the Earth blocks each telescope's view but not the fleet's:
//!   every patch of sky is above the limb for thousands of satellites);
//! * fields = observable area / field area × 1.1 (10 % tile overlap);
//! * duty = measured dark-sky-equivalent integration per working
//!   satellite-second, so slews, the South Atlantic Anomaly and Earthshine
//!   are already charged (0.75 is used until the run has measured it);
//! * overhead = TILE_OVERHEAD_S per field, calibrated against the measured
//!   all-sky survey (survey.rs): with it the estimate for 1,000 telescopes
//!   is ~77 min against 73-82 min measured, and for 10,000 ~8 min, the
//!   measured time to 95 % (the last few tiles near the keep-out edges take
//!   the full pass to ~11 min). The earlier 15 s overhead read ~1.2-1.8x fast.

use crate::app::App;

pub const SKY_DEG2: f64 = 41_252.96;
const OVERLAP: f64 = 1.1;
/// Telescope time per field beyond the exposure when tiling (slew between
/// neighbouring tiles, settle, acquire), calibrated to the measured survey.
pub const TILE_OVERHEAD_S: f64 = 30.0;
/// Share of the fleet assumed free for surveying in normal operations.
pub const SURVEY_SHARE: f64 = 0.2;

#[derive(Clone, Copy)]
pub struct Pass {
    pub exp_s: f64,
    pub seconds: f64,
    pub depth_v: f64,
}

pub struct AllSky {
    pub available_deg2: f64,
    pub fields: f64,
    pub eyes: usize,
    pub duty: f64,
    pub duty_measured: bool,
    pub overhead_s: f64,
    pub passes: Vec<Pass>,
    pub per_day_60s: f64,
    pub survey_share_60s: f64,
    pub tb_per_pass_60s: f64,
    pub whole_sphere_days: f64,
    /// Build-out stages: (telescopes, 60 s pass, 5 min pass, 1 h pass) in seconds.
    pub stages: Vec<(usize, f64, f64, f64)>,
}

impl App {
    pub fn allsky(&self) -> AllSky {
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let o = &self.obs_params;
        let g = &self.grid;
        let allowed: f64 = g
            .pts
            .iter()
            .zip(&g.weight)
            .filter(|(p, _)| crate::obs::sky_blocked(o, **p, sun, moon).is_none())
            .map(|(_, w)| *w)
            .sum();
        let available = allowed * SKY_DEG2;
        let fields = OVERLAP * available / (o.fov_deg * o.fov_deg).max(1e-6);
        let eyes = self.shell.len().saturating_sub(self.resil.n_failed()).max(1);
        let measured = self.science.elapsed_s > 1800.0;
        let duty = if measured { self.science.duty().unwrap_or(0.75).clamp(0.05, 1.0) } else { 0.75 };
        let hw = crate::imaging::Hardware::from_obs(o);
        let pass = |exp: f64| Pass {
            exp_s: exp,
            seconds: fields * (exp + TILE_OVERHEAD_S) / (eyes as f64 * duty),
            depth_v: crate::imaging::point_lim_mag(&hw, exp, exp.min(o.sub_max_s).max(1.0), 22.0),
        };
        let passes: Vec<Pass> = [60.0, 300.0, 3600.0].iter().map(|&e| pass(e)).collect();
        let p60 = passes[0].seconds;
        let frame_bytes = o.sensor_px * o.sensor_py * 2.0;
        AllSky {
            available_deg2: available,
            fields,
            eyes,
            duty,
            duty_measured: measured,
            overhead_s: TILE_OVERHEAD_S,
            passes,
            per_day_60s: 86_400.0 / p60,
            survey_share_60s: p60 / SURVEY_SHARE,
            tb_per_pass_60s: fields * (60.0 / o.sub_max_s.min(60.0).max(1.0)).ceil() * frame_bytes / 1e12,
            // An ecliptic point stays inside the Sun keep-out for 2 × radius / (0.9856°/day).
            whole_sphere_days: 2.0 * o.sun_excl_deg / 0.9856,
            stages: [100usize, 500, 1000, 5000, 10_000]
                .iter()
                .map(|&n| {
                    let t = |e: f64| fields * (e + TILE_OVERHEAD_S) / (n as f64 * duty);
                    (n, t(60.0), t(300.0), t(3600.0))
                })
                .collect(),
        }
    }

    pub fn allsky_json(&self) -> serde_json::Value {
        let a = self.allsky();
        serde_json::json!({
            "sky_deg2": SKY_DEG2,
            "available_deg2": a.available_deg2,
            "fields": a.fields,
            "eyes": a.eyes,
            "duty": a.duty,
            "duty_measured": a.duty_measured,
            "overhead_s": a.overhead_s,
            "fov_deg": self.obs_params.fov_deg,
            "passes": a.passes.iter().map(|p| serde_json::json!({"exp_s": p.exp_s, "seconds": p.seconds, "depth_v": p.depth_v})).collect::<Vec<_>>(),
            "per_day_60s": a.per_day_60s,
            "survey_share": SURVEY_SHARE,
            "survey_share_60s": a.survey_share_60s,
            "tb_per_pass_60s": a.tb_per_pass_60s,
            "whole_sphere_days": a.whole_sphere_days,
            "stages": a.stages.iter().map(|s| serde_json::json!({"n": s.0, "s60": s.1, "s300": s.2, "s3600": s.3})).collect::<Vec<_>>(),
        })
    }
}

pub fn fmt_span(s: f64) -> String {
    if s < 90.0 {
        format!("{:.0} s", s)
    } else if s < 1200.0 {
        // Under 20 min, whole minutes hide differences that matter
        // (8.0 and 8.7 min both read "9 min").
        format!("{:.1} min", s / 60.0)
    } else if s < 5400.0 {
        format!("{:.0} min", s / 60.0)
    } else if s < 2.0 * 86_400.0 {
        format!("{:.1} h", s / 3600.0)
    } else {
        format!("{:.1} days", s / 86_400.0)
    }
}
