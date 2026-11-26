//! Observation model: pointing constraints, per-satellite pointing for each
//! operating mode, and the derived performance numbers.
//!
//! Every performance figure here is a SOFT ESTIMATE from idealised scaling
//! laws (equal telescopes, background-limited incoherent stacking, perfect
//! registration, no read-noise floor). They are for building intuition about
//! how capability scales with N, not for the proposal's error budget.

use crate::astro::EARTH_RADIUS_KM;
use crate::math::{rad, V3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Every telescope stares at its local zenith: a free-running survey belt.
    Survey,
    /// Every telescope that can see the target points at it; images are stacked.
    Stack,
    /// Telescopes that can see the target tile a mosaic around it.
    Mosaic,
    /// Planetary-defence sweep: a ring of pointings at fixed solar elongation.
    NeoSweep,
    /// Serve the observation-request queue (public / director requests).
    Queue,
    /// Tile the whole observable sky and time each complete pass.
    AllSky,
}

impl Mode {
    pub const ALL: [Mode; 6] = [Mode::Survey, Mode::Stack, Mode::Mosaic, Mode::NeoSweep, Mode::Queue, Mode::AllSky];
    pub fn key(self) -> &'static str {
        match self {
            Mode::Survey => "survey",
            Mode::Stack => "stack",
            Mode::Mosaic => "mosaic",
            Mode::NeoSweep => "neo",
            Mode::Queue => "queue",
            Mode::AllSky => "allsky",
        }
    }
    pub fn from_key(k: &str) -> Option<Mode> {
        Mode::ALL.iter().copied().find(|m| m.key() == k)
    }
    pub fn label(self) -> &'static str {
        match self {
            Mode::Survey => "Survey (zenith stare)",
            Mode::Stack => "Target: stack",
            Mode::Mosaic => "Target: mosaic",
            Mode::NeoSweep => "NEO sweep (solar elongation ring)",
            Mode::Queue => "Science operations (request queue)",
            Mode::AllSky => "All-sky survey (tile the whole sky)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ObsParams {
    pub aperture_cm: f64,
    /// Square field of view, side, degrees.
    pub fov_deg: f64,
    pub sun_excl_deg: f64,
    pub moon_excl_deg: f64,
    /// Extra keep-out above the limb (stray light), degrees.
    pub limb_margin_deg: f64,
    /// Height of the atmosphere treated as opaque/bright limb, km.
    pub atmo_km: f64,
    pub exp_s: f64,
    pub neo_elong_deg: f64,
    pub neo_band_deg: f64,
    pub wavelength_nm: f64,
    /// ASSUMED slew + settle + readout overhead per pointing, s.
    pub overhead_s: f64,
    // --- Detector / optics assumptions (see imaging.rs)
    pub obstruction: f64,
    pub sensor_px: f64,
    pub pixel_um: f64,
    pub qe: f64,
    pub throughput: f64,
    pub read_noise_e: f64,
    pub dark_e_s: f64,
    pub full_well_e: f64,
    /// Pointing jitter, arcsec RMS per axis over a sub-frame.
    pub jitter_arcsec: f64,
    /// Longest sub-frame (cosmic rays, drift), s.
    pub sub_max_s: f64,
    /// Optical wavefront error, nm RMS (figure + alignment + thermal).
    pub wfe_nm: f64,
    /// Secondary-mirror spider: number of vanes and vane width (mm).
    pub spider_vanes: u32,
    pub spider_mm: f64,
    // --- Attitude / agility assumptions (see attitude.rs)
    /// Transverse moment of inertia, kg·m² (~150 kg: 0.6 m bus + 1 m optical tube).
    pub inertia_kgm2: f64,
    /// Usable reaction-wheel torque about a slew axis, N·m.
    pub wheel_torque_nm: f64,
    /// Usable wheel momentum about a slew axis, N·m·s.
    pub wheel_momentum_nms: f64,
    /// Software slew-rate limit, °/s.
    pub rate_limit_dps: f64,
    /// Settling decay time constant, s.
    pub settle_tau_s: f64,
    /// Post-slew residual as a fraction of the slew angle.
    pub overshoot_frac: f64,
    /// Fine-guidance capture radius, arcsec.
    pub capture_arcsec: f64,
    /// Fine-guidance acquisition time, s.
    pub acquire_s: f64,
    /// Star-tracker-only pointing jitter when no guide star is available, arcsec RMS.
    pub coarse_jitter_arcsec: f64,
    /// Faintest usable guide star (V) in the science field.
    pub guide_mag: f64,
    /// Most telescopes the director's target may take in Stack/Mosaic
    /// (0 = every telescope that can see it). The rest keep surveying.
    pub target_eyes: u32,
    /// Sunlight scattered into the telescope (baffle point-source
    /// transmittance): surface brightness at 45° solar elongation,
    /// V mag/arcsec², and its fall-off in mag per decade of elongation.
    pub stray_sb45: f64,
    pub stray_slope: f64,
    /// Model sunward stray light at all (off = perfect baffle).
    pub stray_on: bool,
    /// Systematic photometric floor per telescope over one hour, ppm (ASSUMED).
    pub sys_floor_ppm: f64,
    /// Fleet optical variation (ASSUMED): lognormal σ of each telescope's
    /// wavefront error about the nominal, and 1σ throughput scatter.
    pub wfe_spread: f64,
    pub throughput_spread: f64,
    /// Telescope focal length, mm (design input: with the sensor it sets the
    /// plate scale and field of view).
    pub focal_mm: f64,
    /// Sensor rows (sensor_px is columns).
    pub sensor_py: f64,
    /// Corrected-field diameter of the optical design, deg (ASSUMED per design).
    pub corrected_field_deg: f64,
    /// Selected presets (indices into SENSORS / DESIGNS / POINTING; usize::MAX = custom).
    pub sensor_preset: usize,
    pub design_preset: usize,
    pub pointing_preset: usize,
}

/// Detector presets: name, columns, rows, pixel µm, QE, read noise e⁻,
/// dark e⁻/s/px (cooled), full well e⁻. Typical datasheet-level figures;
/// treat as ASSUMED for a flight build.
pub struct SensorSpec {
    pub key: &'static str,
    pub name: &'static str,
    pub cols: f64,
    pub rows: f64,
    pub pixel_um: f64,
    pub qe: f64,
    pub read_noise_e: f64,
    pub dark_e_s: f64,
    pub full_well_e: f64,
}
pub const SENSORS: [SensorSpec; 5] = [
    SensorSpec { key: "imx455", name: "Sony IMX455 · full frame, 61 MP, 3.76 µm", cols: 9576.0, rows: 6388.0, pixel_um: 3.76, qe: 0.80, read_noise_e: 1.5, dark_e_s: 0.003, full_well_e: 51_000.0 },
    SensorSpec { key: "imx571", name: "Sony IMX571 · APS-C, 26 MP, 3.76 µm", cols: 6252.0, rows: 4176.0, pixel_um: 3.76, qe: 0.80, read_noise_e: 1.5, dark_e_s: 0.003, full_well_e: 51_000.0 },
    SensorSpec { key: "imx461", name: "Sony IMX461 · medium format, 102 MP, 3.76 µm", cols: 11664.0, rows: 8750.0, pixel_um: 3.76, qe: 0.80, read_noise_e: 1.5, dark_e_s: 0.003, full_well_e: 51_000.0 },
    SensorSpec { key: "gsense4040", name: "Gpixel GSENSE4040 · 4k × 4k, 16.8 MP, 9 µm", cols: 4096.0, rows: 4096.0, pixel_um: 9.0, qe: 0.72, read_noise_e: 3.7, dark_e_s: 0.04, full_well_e: 70_000.0 },
    SensorSpec { key: "kaf16803", name: "onsemi KAF-16803 CCD · 4k × 4k, 16.8 MP, 9 µm", cols: 4096.0, rows: 4096.0, pixel_um: 9.0, qe: 0.60, read_noise_e: 9.0, dark_e_s: 0.02, full_well_e: 100_000.0 },
];

/// Optical design presets: central obstruction, spider vanes, corrected
/// field diameter (deg) and a representative as-built wavefront error.
/// ASSUMED typical values for a 20 cm class space telescope.
pub struct DesignSpec {
    pub key: &'static str,
    pub name: &'static str,
    pub obstruction: f64,
    pub vanes: u32,
    pub corrected_field_deg: f64,
    pub wfe_nm: f64,
    pub f_ratio: f64,
}
pub const DESIGNS: [DesignSpec; 4] = [
    DesignSpec { key: "rc", name: "Ritchey–Chrétien + field flattener", obstruction: 0.38, vanes: 4, corrected_field_deg: 1.6, wfe_nm: 60.0, f_ratio: 8.0 },
    DesignSpec { key: "dk", name: "Corrected Dall–Kirkham", obstruction: 0.33, vanes: 4, corrected_field_deg: 1.0, wfe_nm: 60.0, f_ratio: 10.0 },
    DesignSpec { key: "wide", name: "Fast wide-field Cassegrain + corrector", obstruction: 0.42, vanes: 4, corrected_field_deg: 3.0, wfe_nm: 70.0, f_ratio: 4.0 },
    DesignSpec { key: "tma", name: "Off-axis three-mirror anastigmat (unobstructed)", obstruction: 0.0, vanes: 0, corrected_field_deg: 3.0, wfe_nm: 50.0, f_ratio: 6.0 },
];

/// Pointing-stability presets: name and RMS jitter over a sub-frame, arcsec.
pub const POINTING: [(&str, &str, f64); 3] = [
    ("body", "Body pointing only (star trackers, reaction wheels)", 1.5),
    ("fine", "Fine guidance on the science sensor", 0.5),
    ("fsm", "Fine guidance + fast steering mirror", 0.15),
];

impl ObsParams {
    pub fn apply_sensor(&mut self, k: usize) {
        let s = &SENSORS[k];
        self.sensor_px = s.cols;
        self.sensor_py = s.rows;
        self.pixel_um = s.pixel_um;
        self.qe = s.qe;
        self.read_noise_e = s.read_noise_e;
        self.dark_e_s = s.dark_e_s;
        self.full_well_e = s.full_well_e;
        self.sensor_preset = k;
        self.derive();
    }
    pub fn apply_design(&mut self, k: usize) {
        let d = &DESIGNS[k];
        self.obstruction = d.obstruction;
        self.spider_vanes = d.vanes;
        self.corrected_field_deg = d.corrected_field_deg;
        self.wfe_nm = d.wfe_nm;
        self.focal_mm = d.f_ratio * self.aperture_cm * 10.0;
        self.design_preset = k;
        self.derive();
    }
    pub fn apply_pointing(&mut self, k: usize) {
        self.jitter_arcsec = POINTING[k].2;
        self.pointing_preset = k;
    }
    /// Plate scale, arcsec per pixel.
    pub fn plate_scale(&self) -> f64 {
        self.pixel_um * 1e-3 / self.focal_mm.max(1.0) * 206_264.806
    }
    /// Field of view, deg (width, height, diagonal).
    pub fn field_deg(&self) -> (f64, f64, f64) {
        let s = self.plate_scale() / 3600.0;
        let (w, h) = (self.sensor_px * s, self.sensor_py * s);
        (w, h, w.hypot(h))
    }
    /// Recompute the derived field: `fov_deg` is the side of the square of
    /// equal area, used wherever the sim needs one footprint size.
    pub fn derive(&mut self) {
        let (w, h, _) = self.field_deg();
        self.fov_deg = (w * h).sqrt();
    }
}


impl Default for ObsParams {
    fn default() -> Self {
        let mut o = ObsParams {
            aperture_cm: 20.0,
            fov_deg: 1.0,
            sun_excl_deg: 45.0,
            moon_excl_deg: 15.0,
            limb_margin_deg: 10.0,
            atmo_km: 100.0,
            exp_s: 60.0,
            neo_elong_deg: 55.0,
            neo_band_deg: 20.0,
            wavelength_nm: 550.0,
            overhead_s: 15.0,
            obstruction: 0.38,
            sensor_px: 9576.0,
            pixel_um: 3.76,
            qe: 0.8,
            throughput: 0.75,
            read_noise_e: 1.5,
            dark_e_s: 0.003,
            full_well_e: 51_000.0,
            jitter_arcsec: 0.5,
            sub_max_s: 300.0,
            wfe_nm: 60.0,
            spider_vanes: 4,
            spider_mm: 2.0,
            inertia_kgm2: 20.0,
            wheel_torque_nm: 0.15,
            wheel_momentum_nms: 2.0,
            rate_limit_dps: 2.0,
            settle_tau_s: 3.0,
            overshoot_frac: 0.0005,
            capture_arcsec: 10.0,
            acquire_s: 3.0,
            coarse_jitter_arcsec: 6.0,
            guide_mag: 13.0,
            target_eyes: 25,
            stray_sb45: 21.0,
            stray_slope: 7.5,
            stray_on: true,
            sys_floor_ppm: 500.0,
            wfe_spread: 0.30,
            throughput_spread: 0.04,
            focal_mm: 1600.0,
            sensor_py: 6388.0,
            corrected_field_deg: 1.6,
            sensor_preset: 0,
            design_preset: 0,
            pointing_preset: 1,
        };
        o.derive();
        o
    }
}

/// Per-frame observation state.
#[derive(Default)]
pub struct ObsState {
    /// Seconds until each satellite enters the SAA and safes its detector
    /// (0 = inside now, +∞ = not within the look-ahead, or SAA not safing).
    /// Handover planning treats SAA entry like losing the target.
    pub saa_eta: Vec<f64>,
    /// Serving an open event window (transit, occultation) on the queue:
    /// STARE recruits these only when nothing else can see its target.
    pub protected: Vec<bool>,
    /// Unit radial (zenith) vector of each satellite.
    pub rhat: Vec<V3>,
    pub lit: Vec<bool>,
    /// Pointing direction per satellite (None = idle / constrained).
    pub pointing: Vec<Option<V3>>,
    pub n_lit: usize,
    pub n_active: usize,
    /// Satellites with an unobstructed line of sight to the target right now.
    pub n_target_visible: usize,
    pub target_blocked_by: Option<&'static str>,
    /// Earth keep-out cone half-angle seen from the shell, degrees.
    pub earth_cone_deg: f64,
    /// dot(target, rhat) must exceed this for the Earth not to block.
    pub cos_lim: f64,
    /// Longest baseline among stacking telescopes projected ⟂ to target, km.
    pub max_baseline_km: f64,
    pub links_blocked: usize,
    pub link_blocked: Vec<bool>,

    // --- Task and attitude state (persists between frames)
    /// Task id per satellite (0 = idle). See `job_*` helpers.
    pub job: Vec<u64>,
    pub prev_job: Vec<u64>,
    /// Fine guidance available for the commanded pointing.
    pub fine: Vec<bool>,
    pub att: Vec<crate::attitude::Att>,
    /// Locked integration seconds per satellite in the last step.
    pub obs_dt: Vec<f64>,
    pub n_slewing: usize,
    pub n_settling: usize,
    pub n_locked: usize,
    /// Locked on the director's target (stack/mosaic).
    pub n_locked_target: usize,
    /// Repoints started in the last step: (satellite, job, angle rad).
    pub new_slews: Vec<(usize, u64, f64)>,
    /// Satellites in safe hold (excluded from all tasking).
    pub hold: Vec<bool>,
    /// Satellites reserved for their own programme (twin-telescope
    /// interferometers), and where they point (None: idle at zenith).
    pub reserved: Vec<bool>,
    pub twin_dir: Option<V3>,
    /// hold || reserved: not available to the shared schedulers.
    pub off: Vec<bool>,
}

pub const TAG_SURVEY: u64 = 1;
pub const TAG_STACK: u64 = 2;
pub const TAG_MOSAIC: u64 = 3;
pub const TAG_NEO: u64 = 4;
pub const TAG_QUEUE: u64 = 5;
/// STARE campaign (Sustained Tracking And Rotating Eyes).
pub const TAG_STARE: u64 = 6;
/// All-sky survey tile.
pub const TAG_ALLSKY: u64 = 7;
/// Twin-telescope satellites on their interferometry target.
pub const TAG_TWIN: u64 = 8;
pub fn job(tag: u64, v: u64) -> u64 {
    (tag << 56) | (v & ((1 << 56) - 1))
}
pub fn job_tag(j: u64) -> u64 {
    j >> 56
}
pub fn job_val(j: u64) -> u64 {
    j & ((1 << 56) - 1)
}
/// Stable 20-bit key for a sky direction (0.05° cells).
fn dir_key(d: V3) -> u64 {
    let (ra, dec) = d.to_radec();
    let a = (ra.to_degrees() * 20.0) as u64;
    let b = ((dec.to_degrees() + 90.0) * 20.0) as u64;
    (a.wrapping_mul(7_919) ^ b.wrapping_mul(104_729)) & 0xF_FFFF
}

/// Spiral tile offsets (col, row) outward from the centre.
fn spiral(n: usize) -> &'static [(i32, i32)] {
    use std::sync::OnceLock;
    static S: OnceLock<Vec<(i32, i32)>> = OnceLock::new();
    let v = S.get_or_init(|| {
        let r = 60i32;
        let mut v: Vec<(i32, i32)> = (-r..=r).flat_map(|y| (-r..=r).map(move |x| (x, y))).collect();
        v.sort_by(|a, b| {
            let ka = (a.0.abs().max(a.1.abs()), (a.1 as f64).atan2(a.0 as f64));
            let kb = (b.0.abs().max(b.1.abs()), (b.1 as f64).atan2(b.0 as f64));
            ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
        });
        v
    });
    &v[..n.min(v.len())]
}

pub struct Metrics {
    pub area_m2: f64,
    pub d_eq_m: f64,
    pub lim_mag: f64,
    pub diffraction_arcsec: f64,
    pub interf_uas: Option<f64>,
    pub mosaic_deg2: f64,
}

pub fn metrics(o: &ObsParams, n: usize, max_baseline_km: f64, mosaic_tiles: usize) -> Metrics {
    let d = o.aperture_cm / 100.0;
    let lam = o.wavelength_nm * 1e-9;
    let nf = n.max(1) as f64;
    Metrics {
        area_m2: n as f64 * std::f64::consts::PI * (d / 2.0).powi(2),
        d_eq_m: d * nf.sqrt(),
        // Physical model, typical dark sky (zodiacal V 22.0 mag/arcsec²).
        lim_mag: {
            let hw = crate::imaging::Hardware::from_obs(o);
            crate::imaging::point_lim_mag(&hw, nf * o.exp_s, o.exp_s.min(o.sub_max_s), 22.0)
        },
        diffraction_arcsec: 1.22 * lam / d * 206_264.806,
        interf_uas: if n >= 2 && max_baseline_km > 0.0 {
            Some(lam / (max_baseline_km * 1000.0) * 206_264.806e6)
        } else {
            None
        },
        mosaic_deg2: mosaic_tiles as f64 * o.fov_deg * o.fov_deg,
    }
}

/// Is the shell in Earth's (cylindrical) shadow?
fn in_shadow(r: V3, sun: V3) -> bool {
    let along = r.dot(sun);
    along < 0.0 && (r - sun * along).norm() < EARTH_RADIUS_KM
}

/// Celestial keep-outs that are the same for every satellite (Sun/Moon parallax
/// from LEO is < 1°, ignored).
pub fn sky_blocked(o: &ObsParams, dir: V3, sun: V3, moon: V3) -> Option<&'static str> {
    if dir.angle(sun) < rad(o.sun_excl_deg) {
        Some("Sun exclusion")
    } else if dir.angle(moon) < rad(o.moon_excl_deg) && dir.angle(moon) > rad(1.0) {
        // The Moon keep-out protects against scattered moonlight; it does not
        // apply when the Moon itself is the target (within ~1°, allowing for
        // parallax from LEO).
        Some("Moon exclusion")
    } else {
        None
    }
}

/// Tangent-plane basis (east, north) at a sky direction.
/// The telescope frame as (east-west, north-south) extent, degrees: tiled
/// fields (all-sky survey, mosaics, recovery searches) are rolled so the
/// long side of the sensor runs east-west.
pub fn frame_ew_ns(o: &ObsParams) -> (f64, f64) {
    let (w, h, _) = o.field_deg();
    (w.max(h), w.min(h))
}

pub fn tangent_basis(t: V3) -> (V3, V3) {
    let e = V3::Z.cross(t);
    let e = if e.norm() < 1e-9 { V3::new(0.0, 1.0, 0.0) } else { e.unit() };
    (e, t.cross(e).unit())
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    st: &mut ObsState,
    o: &ObsParams,
    mode: Mode,
    pos: &[V3],
    links: &[(u32, u32)],
    sun: V3,
    moon: V3,
    target: Option<V3>,
    target_body: Option<usize>,
) {
    use rayon::prelude::*;
    let n = pos.len();
    st.rhat.clear();
    st.rhat.par_extend(pos.par_iter().map(|p| p.unit()));
    if st.att.len() != n {
        // Shell rebuilt: everyone starts idle, pointing at zenith.
        st.att = st.rhat.iter().map(|&z| crate::attitude::Att::new(z)).collect();
        st.job = vec![0; n];
        st.hold = vec![false; n];
    }
    std::mem::swap(&mut st.prev_job, &mut st.job);
    st.prev_job.resize(n, 0);
    st.job.clear();
    st.job.resize(n, 0);
    st.fine.clear();
    st.fine.resize(n, true);
    let guide = |d: V3| crate::imaging::guide_star_available(d, o.fov_deg, o.guide_mag);
    st.lit.clear();
    st.lit.par_extend(pos.par_iter().map(|&p| !in_shadow(p, sun)));
    st.n_lit = st.lit.iter().filter(|&&l| l).count();

    let r = pos.first().map(|p| p.norm()).unwrap_or(EARTH_RADIUS_KM + 550.0);
    let earth_half = ((EARTH_RADIUS_KM + o.atmo_km) / r).clamp(-1.0, 1.0).asin();
    st.earth_cone_deg = earth_half.to_degrees();
    st.cos_lim = -(earth_half + rad(o.limb_margin_deg)).cos();
    let cos_lim = st.cos_lim;

    // Laser links grazing the atmosphere are unusable.
    let r_min = EARTH_RADIUS_KM + o.atmo_km;
    st.link_blocked.clear();
    st.link_blocked.par_extend(links.par_iter().map(|&(a, b)| {
        let (pa, pb) = (pos[a as usize], pos[b as usize]);
        let d = pb - pa;
        let t = (-pa.dot(d) / d.dot(d).max(1e-9)).clamp(0.0, 1.0);
        (pa + d * t).norm() < r_min
    }));
    st.links_blocked = st.link_blocked.iter().filter(|&&b| b).count();

    st.pointing.clear();
    st.pointing.resize(n, None);
    st.n_target_visible = 0;
    st.reserved.resize(n, false);
    st.off.clear();
    st.off.extend((0..n).map(|i| st.hold.get(i).copied().unwrap_or(false) || st.reserved[i]));
    // Twin-telescope satellites first: their own target, nothing else.
    if let Some(d) = st.twin_dir {
        if sky_blocked(o, d, sun, moon).is_none() {
            for i in 0..n {
                if st.reserved[i] && !st.hold.get(i).copied().unwrap_or(false) && d.dot(st.rhat[i]) > cos_lim {
                    st.pointing[i] = Some(d);
                    st.job[i] = job(TAG_TWIN, 0);
                    st.fine[i] = true; // a bright star is its own guide source
                }
            }
        }
    }
    st.max_baseline_km = 0.0;
    st.target_blocked_by = None;

    let ok = |d: V3| sky_blocked(o, d, sun, moon).is_none();

    match mode {
        Mode::Survey => {
            for i in 0..n {
                let z = st.rhat[i];
                if !st.off[i] && ok(z) {
                    st.pointing[i] = Some(z);
                    st.job[i] = job(TAG_SURVEY, 0);
                    st.fine[i] = guide(z);
                }
            }
        }
        Mode::Stack | Mode::Mosaic => {
            if let Some(t) = target {
                st.target_blocked_by = sky_blocked(o, t, sun, moon);
                let mut visible: Vec<usize> = if st.target_blocked_by.is_none() {
                    (0..n).filter(|&i| !st.off[i] && t.dot(st.rhat[i]) > cos_lim).collect()
                } else {
                    Vec::new()
                };
                st.n_target_visible = visible.len();
                // Bright solar-system targets are their own guide source.
                // Each solar-system body gets its own key so retargeting from
                // one planet to another is a new task (a real slew), while a
                // planet's own slow motion is tracked under the same task.
                let tkey = match target_body {
                    Some(b) => 0xF_FF00 | b as u64,
                    None => dir_key(t),
                };
                // Cap the number of eyes the target may take: current members
                // first (no needless handovers), then the cheapest slews.
                let cap = o.target_eyes as usize;
                if cap > 0 && visible.len() > cap {
                    let member = |i: usize| {
                        let pj = st.prev_job[i];
                        (job_tag(pj) == TAG_STACK || job_tag(pj) == TAG_MOSAIC) && (job_val(pj) & 0xF_FFFF) == tkey
                    };
                    visible.sort_by(|&a, &b| {
                        member(b).cmp(&member(a)).then(
                            st.att[a].dir.angle(t).partial_cmp(&st.att[b].dir.angle(t)).unwrap_or(std::cmp::Ordering::Equal),
                        )
                    });
                    visible.truncate(cap);
                }
                let target_is_body = target_body.is_some();
                if mode == Mode::Stack {
                    let fine = target_is_body || guide(t);
                    for &i in &visible {
                        st.pointing[i] = Some(t);
                        st.job[i] = job(TAG_STACK, tkey);
                        st.fine[i] = fine;
                    }
                    st.max_baseline_km = projected_max_baseline(pos, &visible, t);
                } else {
                    // Sticky spiral mosaic: satellites keep their tile while they
                    // can see it; free tiles (inner first) go to the satellites
                    // with the shortest slew.
                    let (e, nn) = tangent_basis(t);
                    let step = rad(o.fov_deg);
                    let tiles = spiral(visible.len());
                    let tile_dir = |k: usize| {
                        let (c, r) = tiles[k];
                        (t + e * (c as f64 * step).tan() + nn * (r as f64 * step).tan()).unit()
                    };
                    let mut used = vec![false; tiles.len()];
                    let mut free_sats = Vec::new();
                    for &i in &visible {
                        let pj = st.prev_job[i];
                        let k = (job_val(pj) >> 20) as usize;
                        if job_tag(pj) == TAG_MOSAIC && (job_val(pj) & 0xF_FFFF) == tkey && k < tiles.len() && !used[k] {
                            let d = tile_dir(k);
                            if ok(d) && d.dot(st.rhat[i]) > cos_lim {
                                used[k] = true;
                                st.pointing[i] = Some(d);
                                st.job[i] = job(TAG_MOSAIC, ((k as u64) << 20) | tkey);
                                st.fine[i] = guide(d);
                                continue;
                            }
                        }
                        free_sats.push(i);
                    }
                    free_sats.sort_by(|&a, &b| {
                        st.att[a].dir.angle(t).partial_cmp(&st.att[b].dir.angle(t)).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    let mut fs = free_sats.into_iter();
                    for k in 0..tiles.len() {
                        if used[k] {
                            continue;
                        }
                        let d = tile_dir(k);
                        if !ok(d) {
                            continue;
                        }
                        let Some(i) = fs.next() else { break };
                        if d.dot(st.rhat[i]) > cos_lim {
                            used[k] = true;
                            st.pointing[i] = Some(d);
                            st.job[i] = job(TAG_MOSAIC, ((k as u64) << 20) | tkey);
                            st.fine[i] = guide(d);
                        }
                    }
                }
            }
        }
        Mode::NeoSweep => {
            let elong = o.neo_elong_deg.max(o.sun_excl_deg);
            let perp = sun.any_perp();
            for i in 0..n {
                // Low-discrepancy spread of azimuths and elongations.
                let az = (i as f64 * 0.618_033_988_75).fract() * std::f64::consts::TAU;
                let band = ((i as f64 * 0.754_877_666_2).fract() - 0.5) * o.neo_band_deg;
                let el = rad((elong + band).max(o.sun_excl_deg));
                let d = sun.rot_axis(perp, el).rot_axis(sun, az).unit();
                if !st.off[i] && ok(d) && d.dot(st.rhat[i]) > cos_lim {
                    st.pointing[i] = Some(d);
                    st.job[i] = job(TAG_NEO, i as u64);
                    st.fine[i] = guide(d);
                }
            }
        }
        Mode::Queue => {} // pointing assigned by `assign_queue`; idle eyes survey
        Mode::AllSky => {} // tiles assigned by `App::survey_assign` from the free pool
    }
    // Stack/Mosaic/Queue: telescopes with nothing else to do (over the target
    // cap, no line of sight, no queued work they can see) run the zenith survey.
    // Queue work and STARE campaigns override this below.
    if matches!(mode, Mode::Stack | Mode::Mosaic | Mode::Queue | Mode::AllSky) {
        for i in 0..n {
            let z = st.rhat[i];
            if st.pointing[i].is_none() && !st.off[i] && ok(z) {
                st.pointing[i] = Some(z);
                st.job[i] = job(TAG_SURVEY, 0);
                st.fine[i] = guide(z);
            } else if mode == Mode::AllSky && st.pointing[i].is_none() && !st.off[i] {
                // Zenith inside the Sun or Moon keep-out: no zenith stare, but
                // the all-sky planner can still give it a dark tile elsewhere
                // in its sky (it checks every tile against the keep-outs).
                st.job[i] = job(TAG_SURVEY, 0);
            }
        }
    }
    st.n_active = st.pointing.iter().filter(|p| p.is_some()).count();
}

pub struct QueueItem {
    pub id: u64,
    pub dir: V3,
    pub want: usize,
    /// Fine guidance possible (bright body target, or a guide star in the field).
    pub fine: bool,
    /// Needs unbroken coverage now (an open event window): staff it like a
    /// STARE campaign, recruiting replacements before members lose the target.
    pub continuous: bool,
    /// Geocentric distance of a Solar System target, km (0 = at infinity).
    /// Each telescope then aims from its own position, correcting parallax.
    pub range_km: f64,
    /// Look for more telescopes this call. When false, the request keeps the
    /// members it has and the fleet-wide scan is skipped (background science
    /// is topped up on a stagger; see `queue_assign`).
    pub rescan: bool,
}

/// Direction from satellite `i` (ECI position `sat_km`) to a target at
/// geocentric direction `dir` and distance `range_km` (0 = at infinity).
pub fn aim(dir: V3, range_km: f64, sat_km: V3) -> V3 {
    if range_km > 0.0 {
        (dir * range_km - sat_km).unit()
    } else {
        dir
    }
}

/// When satellite `i` stops being useful on `dir`: whichever comes first of
/// the target setting behind the Earth and the satellite safing in the SAA.
fn loss_time(st: &ObsState, shell: &crate::constellation::Shell, i: usize, dir: V3) -> Option<f64> {
    let saa = st.saa_eta.get(i).copied().unwrap_or(f64::INFINITY);
    if saa <= 0.0 {
        return None;
    }
    shell.time_to_set(i, dir, st.cos_lim).map(|t| t.min(saa))
}

/// `loss_time` with the closed-form (frozen-node) set time, no Newton
/// refinement: cheap enough to screen the whole fleet.
fn loss_time_fast(st: &ObsState, shell: &crate::constellation::Shell, i: usize, dir: V3) -> Option<f64> {
    let saa = st.saa_eta.get(i).copied().unwrap_or(f64::INFINITY);
    if saa <= 0.0 {
        return None;
    }
    shell.time_to_set_frozen(i, dir, st.cos_lim).map(|t| t.min(saa))
}

pub struct QueueResult {
    #[allow(dead_code)]
    pub id: u64,
    pub assigned: usize,
    pub blocked: Option<&'static str>,
    /// The scan was skipped this call: keep the previous blocked reason.
    pub skipped: bool,
}

/// Slew-aware greedy allocator. Items in priority order each take up to
/// `want` telescopes with a clear line of sight: first those already working
/// on this request (so nobody repoints needlessly), then idle telescopes with
/// the shortest slew, and only then telescopes pre-empted from lower-priority work.
///
/// `budget` caps how many telescopes the queue may use in total (the public
/// time share); the queue runs after the director's mode and pre-empts its
/// assignments up to that cap.
///
/// Call it more than once per step for separate pools (public share, then the
/// science programme): telescopes already given queue work this step are
/// skipped. With `steal` false only idle or surveying telescopes are used.
pub fn assign_queue(
    st: &mut ObsState,
    o: &ObsParams,
    items: &[QueueItem],
    sun: V3,
    moon: V3,
    budget: usize,
    steal: bool,
    pool: &'static str,
    shell: &crate::constellation::Shell,
    lead: f64,
) -> Vec<QueueResult> {
    let n = st.rhat.len();
    let mut out = Vec::with_capacity(items.len());
    let mut taken: Vec<bool> = (0..n)
        .map(|i| {
            let t = job_tag(st.job[i]);
            t == TAG_QUEUE || (!steal && t != 0 && t != TAG_SURVEY)
        })
        .collect();
    let mut used = 0usize;
    let mut free = taken.iter().filter(|t| !**t).count();
    // Telescopes this pool may use at all (not taken, held or on STARE).
    let mut pool_idx: Vec<usize> = (0..n).filter(|&i| !taken[i] && !st.off[i] && job_tag(st.job[i]) != TAG_STARE).collect();
    let mut by_job: std::collections::HashMap<u64, Vec<usize>> = std::collections::HashMap::new();
    for (i, &pj) in st.prev_job.iter().enumerate() {
        if job_tag(pj) == TAG_QUEUE {
            by_job.entry(pj).or_default().push(i);
        }
    }
    for it in items {
        if budget == 0 || used >= budget || free == 0 {
            out.push(QueueResult { id: it.id, assigned: 0, blocked: Some(if free == 0 { "no free telescopes" } else { pool }), skipped: false });
            continue;
        }
        if let Some(why) = sky_blocked(o, it.dir, sun, moon) {
            out.push(QueueResult { id: it.id, assigned: 0, blocked: Some(why), skipped: false });
            continue;
        }
        let me = job(TAG_QUEUE, it.id);
        let mut got = 0;
        let mut cands: Vec<(f64, usize)> = Vec::new();
        // Fast path: keep current members; if that fully staffs the request,
        // no fleet-wide scan is needed.
        // For continuous items, `healthy` counts members that keep the target
        // longer than `lead`; outgoing members stay on until they lose it.
        let mut healthy = 0;
        let mut assigned = 0;
        if let Some(members) = by_job.get(&me) {
            for &i in members {
                // Event windows keep every member that still sees the target:
                // an outgoing member stays locked until its replacement (which
                // already counts as healthy while it slews) has locked on.
                let keep = healthy < it.want || (it.continuous && st.att[i].phase() == crate::attitude::Phase::Locked);
                if keep && used < budget && !taken[i] && !st.off[i] && job_tag(st.job[i]) != TAG_STARE && it.dir.dot(st.rhat[i]) > st.cos_lim {
                    taken[i] = true;
                    st.pointing[i] = Some(aim(it.dir, it.range_km, shell.pos[i]));
                    st.job[i] = me;
                    st.fine[i] = it.fine;
                    assigned += 1;
                    used += 1;
                    free -= 1;
                    if !it.continuous || loss_time(st, shell, i, it.dir).map_or(false, |t| t > lead) {
                        healthy += 1;
                    }
                }
            }
        }
        got += healthy;
        let outgoing = assigned - healthy;
        if got >= it.want || used >= budget {
            out.push(QueueResult { id: it.id, assigned, blocked: None, skipped: false });
            continue;
        }
        if !it.rescan {
            out.push(QueueResult { id: it.id, assigned, blocked: None, skipped: true });
            continue;
        }
        // Eligibility and slew cost over the free telescopes only.
        // Drop taken telescopes once enough have piled up (each sweep removes
        // at least 256, so the sweeps cost O(n) in all per call).
        if pool_idx.len() > free + 256 {
            pool_idx.retain(|&i| !taken[i]);
        }
        // Time until telescope `i` loses the target, as a slew-cost penalty:
        // 0 beyond 3 × lead, 1 beyond 1.5 × lead, else unusable (None).
        // Closed form first; the exact (J2-drift) set time only where the
        // estimate is near a threshold.
        let keep_score = |st_ref: &ObsState, i: usize| -> Option<f64> {
            let near = |t: f64, x: f64| (t - x).abs() < 0.2 * x;
            let t = match loss_time_fast(st_ref, shell, i, it.dir) {
                Some(t) if near(t, 3.0 * lead) || near(t, 1.5 * lead) => loss_time(st_ref, shell, i, it.dir),
                t => t,
            };
            match t {
                Some(t) if t > 3.0 * lead => Some(0.0),
                Some(t) if t > 1.5 * lead => Some(1.0),
                _ => None,
            }
        };
        let scan: Vec<(usize, bool, f64)> = {
            use rayon::prelude::*;
            let (taken, st_ref) = (&taken, &*st);
            let f = |&i: &usize| -> Option<(usize, bool, f64)> {
                if taken[i] || it.dir.dot(st_ref.rhat[i]) <= st_ref.cos_lim {
                    return None;
                }
                let mine = st_ref.prev_job[i] == me;
                let pt = job_tag(st_ref.prev_job[i]);
                let busy = pt != 0 && pt != TAG_SURVEY && pt != TAG_ALLSKY && !mine;
                // Continuous items: members we already hold are checked for
                // an imminent loss of the target here; other candidates are
                // checked after the scan, cheapest slew first (see below).
                let mut extra = 0.0;
                if it.continuous && mine {
                    match keep_score(st_ref, i) {
                        Some(x) => extra = x,
                        None => return None,
                    }
                }
                // Mid-way through an all-sky tile: take it only if no truly idle
                // telescope is about as close (0.3 in 1 − cos ≈ 45° of slew).
                let tiling = if pt == TAG_ALLSKY && !mine { 0.3 } else { 0.0 };
                // 1 − cos: same ordering as the slew angle, without acos.
                Some((i, mine, 1.0 - st_ref.att[i].dir.dot(it.dir) + extra + tiling + if busy { 10.0 } else { 0.0 }))
            };
            if pool_idx.len() > 4096 {
                pool_idx.par_iter().filter_map(f).collect()
            } else {
                pool_idx.iter().filter_map(f).collect()
            }
        };
        for (i, mine, cost) in scan {
            if mine && got < it.want && used < budget {
                used += 1;
                free -= 1;
                taken[i] = true;
                st.pointing[i] = Some(aim(it.dir, it.range_km, shell.pos[i]));
                st.job[i] = me;
                st.fine[i] = it.fine;
                got += 1;
            } else {
                cands.push((cost, i));
            }
        }
        let need = it.want.saturating_sub(got).min(budget - used);
        if it.continuous && need > 0 && !cands.is_empty() {
            // The loss-time penalty only ever adds to the slew cost, so
            // visit candidates in slew-cost order and stop once the next
            // one cannot beat the `need` best found so far. Usually a few
            // dozen loss-time evaluations instead of one per visible
            // telescope.
            let mut best: Vec<(f64, usize)> = Vec::with_capacity(need + 1);
            let batch = (4 * need).max(32);
            let mut rest = std::mem::take(&mut cands);
            'outer: while !rest.is_empty() {
                let mut chunk = if rest.len() > batch {
                    rest.select_nth_unstable_by(batch - 1, |a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    let tail = rest.split_off(batch);
                    std::mem::replace(&mut rest, tail)
                } else {
                    std::mem::take(&mut rest)
                };
                chunk.sort_unstable_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                for (c, i) in chunk {
                    if best.len() >= need && c >= best[need - 1].0 {
                        break 'outer;
                    }
                    if let Some(x) = keep_score(st, i) {
                        let v = (c + x, i);
                        let at = best.partition_point(|b| b.0 <= v.0);
                        best.insert(at, v);
                        best.truncate(need);
                    }
                }
            }
            cands = best;
        }
        if need > 0 && !cands.is_empty() {
            let k = need.min(cands.len());
            if k < cands.len() {
                cands.select_nth_unstable_by(k - 1, |a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            }
            for &(_, i) in &cands[..k] {
                taken[i] = true;
                st.pointing[i] = Some(aim(it.dir, it.range_km, shell.pos[i]));
                st.job[i] = me;
                st.fine[i] = it.fine;
                got += 1;
                used += 1;
                free -= 1;
            }
        }
        let blocked = if got == 0 {
            Some(if used >= budget {
                pool
            } else if free == 0 {
                "no free telescopes"
            } else {
                "Earth blocks every telescope able to take it"
            })
        } else {
            None
        };
        out.push(QueueResult { id: it.id, assigned: got + outgoing, blocked, skipped: false });
    }
    st.protected.clear();
    st.protected.resize(n, false);
    let cont: std::collections::HashSet<u64> = items.iter().filter(|it| it.continuous).map(|it| job(TAG_QUEUE, it.id)).collect();
    if !cont.is_empty() {
        for i in 0..n {
            if cont.contains(&st.job[i]) {
                st.protected[i] = true;
            }
        }
    }
    st.n_active = st.pointing.iter().filter(|p| p.is_some()).count();
    out
}

pub struct StareItem {
    pub id: u64,
    pub dir: V3,
    pub eyes: usize,
    pub fine: bool,
    /// Geocentric distance, km (0 = at infinity); see QueueItem.
    pub range_km: f64,
}

pub struct StareResult {
    #[allow(dead_code)]
    pub id: u64,
    /// Telescopes tasked (healthy + outgoing).
    pub assigned: usize,
    /// New recruits this step.
    #[cfg_attr(not(test), allow(dead_code))]
    pub recruited: usize,
    pub blocked: Option<&'static str>,
}

/// STARE scheduler (Sustained Tracking And Rotating Eyes).
///
/// Every telescope's line of sight to a target ends at a predictable time —
/// when its orbit carries the target into the Earth keep-out cone. Members
/// whose remaining window is shorter than `lead` (a worst-case repoint plus
/// margin) are "outgoing": they keep integrating until they lose the target,
/// but a replacement is recruited NOW so it is slewed, settled and locked
/// before they drop. Recruits are the free telescopes with the LONGEST
/// remaining window among those that can repoint in time, which minimises
/// the handover rate. STARE eyes pre-empt programme work and are never
/// taken by the public queue.
pub fn assign_stare(
    st: &mut ObsState,
    o: &ObsParams,
    shell: &crate::constellation::Shell,
    items: &[StareItem],
    sun: V3,
    moon: V3,
    ag: &crate::attitude::Agility,
) -> Vec<StareResult> {
    let n = st.rhat.len().min(shell.len());
    let lead = ag.repoint_time(60f64.to_radians()) + 20.0;
    let dwell = 120.0;
    let mut taken = vec![false; n];
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        if let Some(why) = sky_blocked(o, it.dir, sun, moon) {
            out.push(StareResult { id: it.id, assigned: 0, recruited: 0, blocked: Some(why) });
            continue;
        }
        let me = job(TAG_STARE, it.id);
        let mut healthy = 0;
        let mut assigned = 0;
        let take = |st: &mut ObsState, taken: &mut Vec<bool>, i: usize| {
            taken[i] = true;
            st.pointing[i] = Some(aim(it.dir, it.range_km, shell.pos[i]));
            st.job[i] = me;
            st.fine[i] = it.fine;
        };
        // 1. Keep current members while they can still see the target.
        for i in 0..n {
            if st.prev_job[i] != me || taken[i] || st.off[i] {
                continue;
            }
            if let Some(tts) = loss_time(st, shell, i, it.dir) {
                take(st, &mut taken, i);
                assigned += 1;
                if tts > lead {
                    healthy += 1;
                }
            }
        }
        // 2. Recruit replacements so `eyes` healthy members exist.
        let need = it.eyes.saturating_sub(healthy);
        let mut recruited = 0;
        if need > 0 {
            let mut cands: Vec<(bool, bool, f64, usize)> = {
                use rayon::prelude::*;
                let (taken, st_ref) = (&taken, &*st);
                (0..n)
                    .into_par_iter()
                    .filter_map(|i| {
                        if taken[i] || st_ref.off[i] {
                            return None;
                        }
                        let pj = st_ref.prev_job[i];
                        if job_tag(pj) == TAG_STARE && pj != me {
                            return None; // another campaign's eye
                        }
                        // Screen with the closed form; the best few are
                        // refined below.
                        let tts = loss_time_fast(st_ref, shell, i, it.dir)?;
                        let rt = ag.repoint_time(st_ref.att[i].dir.angle(it.dir));
                        if tts <= 0.8 * (rt + dwell) {
                            return None;
                        }
                        // Leave telescopes alone that are mid-way through an
                        // event window; then prefer telescopes that can lock in
                        // time and, among them, the longest window.
                        let prot = st_ref.protected.get(i).copied().unwrap_or(false);
                        Some((prot, rt > lead - 10.0, -tts.min(1e7), i))
                    })
                    .collect()
            };
            cands.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            // Refine the leading candidates with the exact set time (J2
            // drift), then re-rank: the same choice at a fraction of the cost.
            cands.truncate((3 * need).max(need + 8));
            let mut cands: Vec<(bool, bool, f64, usize)> = cands
                .into_iter()
                .filter_map(|(p, late, _, i)| {
                    let tts = loss_time(st, shell, i, it.dir)?;
                    let rt = ag.repoint_time(st.att[i].dir.angle(it.dir));
                    (tts > rt + dwell).then_some((p, late, -tts.min(1e7), i))
                })
                .collect();
            cands.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for &(_, _, _, i) in cands.iter().take(need) {
                take(st, &mut taken, i);
                assigned += 1;
                recruited += 1;
            }
        }
        let blocked = (assigned == 0).then_some("no telescope can see the target");
        out.push(StareResult { id: it.id, assigned, recruited, blocked });
    }
    st.n_active = st.pointing.iter().filter(|p| p.is_some()).count();
    out
}

/// Telescopes locked and integrating on a given job.
#[cfg_attr(not(test), allow(dead_code))]
pub fn locked_on(st: &ObsState, j: u64) -> usize {
    st.att
        .iter()
        .zip(&st.job)
        .filter(|(a, &jj)| jj == j && a.phase() == crate::attitude::Phase::Locked)
        .count()
}

/// Advance every satellite's attitude by `dt` seconds toward its command.
pub fn advance_attitudes(st: &mut ObsState, ag: &crate::attitude::Agility, dt: f64, target: Option<V3>) {
    use crate::attitude::Phase;
    let n = st.att.len();
    st.obs_dt.clear();
    st.obs_dt.resize(n, 0.0);
    st.new_slews.clear();
    let (mut sl, mut se, mut lk, mut lt) = (0, 0, 0, 0);
    // Each satellite's attitude evolves independently: step them in parallel.
    {
        use rayon::prelude::*;
        let (pointing, job, fine) = (&st.pointing, &st.job, &st.fine);
        let res: Vec<(f64, Option<f64>)> = st
            .att
            .par_iter_mut()
            .enumerate()
            .map(|(i, a)| {
                let cmd = pointing[i].map(|d| (d, job[i], fine[i]));
                let r = a.step(cmd, dt, ag);
                (r.observed_s, r.new_slew)
            })
            .collect();
        for (i, (obs_s, ns)) in res.into_iter().enumerate() {
            st.obs_dt[i] = obs_s;
            if let Some(a) = ns {
                st.new_slews.push((i, st.job[i], a));
            }
        }
    }
    for i in 0..n {
        match st.att[i].phase() {
            Phase::Slewing => sl += 1,
            Phase::Settling | Phase::Acquiring => se += 1,
            Phase::Locked => {
                lk += 1;
                if let Some(t) = target {
                    if matches!(job_tag(st.job[i]), TAG_STACK) && st.att[i].dir.angle(t) < 1e-6 {
                        lt += 1;
                    }
                }
            }
            Phase::Idle => {}
        }
    }
    st.n_slewing = sl;
    st.n_settling = se;
    st.n_locked = lk;
    st.n_locked_target = lt;
}

/// Two-pass farthest-point heuristic on positions projected onto the plane ⟂ t.
/// Exact for collinear sets; within a few percent otherwise — fine for a readout.
fn projected_max_baseline(pos: &[V3], idx: &[usize], t: V3) -> f64 {
    if idx.len() < 2 {
        return 0.0;
    }
    let proj = |i: usize| {
        let p = pos[i];
        p - t * p.dot(t)
    };
    let far = |from: V3| idx.iter().map(|&j| (proj(j) - from).norm()).fold(0.0f64, f64::max);
    // Find the point farthest from an arbitrary start, then farthest from that.
    let p0 = proj(idx[0]);
    let mut best_i = idx[0];
    let mut best = 0.0;
    for &j in idx {
        let d = (proj(j) - p0).norm();
        if d > best {
            best = d;
            best_i = j;
        }
    }
    far(proj(best_i))
}

/// Regular RA/Dec vertex grid used for the coverage heat map.
pub struct SkyGrid {
    pub rows: usize,
    pub cols: usize,
    pub pts: Vec<V3>,
    /// Solid-angle weight of each vertex (∝ cos dec), normalised to sum 1.
    pub weight: Vec<f64>,
}

impl SkyGrid {
    pub fn new(step_deg: f64) -> SkyGrid {
        let rows = (180.0 / step_deg).round() as usize + 1;
        let cols = (360.0 / step_deg).round() as usize + 1; // last column duplicates RA 0
        let mut pts = Vec::with_capacity(rows * cols);
        let mut weight = Vec::with_capacity(rows * cols);
        for i in 0..rows {
            let dec = -90.0 + step_deg * i as f64;
            for j in 0..cols {
                let ra = step_deg * j as f64;
                pts.push(V3::from_radec(rad(ra), rad(dec)));
                let w = if j + 1 == cols { 0.0 } else { rad(dec).cos().max(1e-4) };
                weight.push(w);
            }
        }
        let sum: f64 = weight.iter().sum();
        weight.iter_mut().for_each(|w| *w /= sum);
        SkyGrid { rows, cols, pts, weight }
    }
    pub fn idx(&self, i: usize, j: usize) -> usize {
        i * self.cols + j
    }
}

/// For each sky grid point, how many telescopes could point there right now.
pub fn coverage(
    grid: &[V3],
    rhat: &[V3],
    cos_lim: f64,
    o: &ObsParams,
    sun: V3,
    moon: V3,
    out: &mut Vec<u32>,
) {
    use rayon::prelude::*;
    out.clear();
    out.par_extend(grid.par_iter().map(|&g| {
        if sky_blocked(o, g, sun, moon).is_some() {
            0
        } else {
            rhat.iter().filter(|r| g.dot(**r) > cos_lim).count() as u32
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacking_gain() {
        let o = ObsParams::default();
        let m1 = metrics(&o, 1, 0.0, 0);
        let m100 = metrics(&o, 100, 0.0, 0);
        // ~2.5 mag for 100× the light when background-limited
        assert!((m100.lim_mag - m1.lim_mag - 2.5).abs() < 0.4, "{}", m100.lim_mag - m1.lim_mag);
        assert!((m100.d_eq_m - 2.0).abs() < 1e-9); // 10 × 20 cm
        assert!((m1.diffraction_arcsec - 0.692).abs() < 0.01);
    }

    #[test]
    fn earth_cone_550km() {
        // asin((6378+100)/6928) ≈ 69.2°
        let mut st = ObsState::default();
        let pos = vec![V3::new(6928.137, 0.0, 0.0)];
        update(&mut st, &ObsParams::default(), Mode::Survey, &pos, &[], V3::new(0.0, 1.0, 0.0), V3::Z, None, None);
        assert!((st.earth_cone_deg - 69.2).abs() < 0.2, "{}", st.earth_cone_deg);
    }

    #[test]
    fn opposite_pair_link_blocked() {
        let pos = vec![V3::new(6928.0, 0.0, 0.0), V3::new(-6928.0, 0.0, 0.0)];
        let mut st = ObsState::default();
        update(&mut st, &ObsParams::default(), Mode::Survey, &pos, &[(0, 1)], V3::new(0.0, 1.0, 0.0), V3::Z, None, None);
        assert_eq!(st.links_blocked, 1);
    }
}

#[cfg(test)]
mod stare_tests {
    use super::*;
    use crate::astro::{self, BODY_NAMES};
    use crate::attitude::Agility;
    use crate::constellation::{Shell, ShellParams};

    struct Cov {
        eligible_s: f64,
        excluded_s: f64,
        full_s: f64,
        any_s: f64,
        max_gap_s: f64,
        cur_gap: f64,
        recruits: usize,
        acquired: bool,
    }

    /// Fly the shell for `hours` with one STARE campaign of `eyes` on each
    /// planet (and the Moon); background telescopes run the zenith survey.
    fn fly(n_sats: usize, eyes: usize, hours: f64, dt: f64) -> Vec<(usize, Cov)> {
        let jd0 = 2_461_359.5; // 2026-11-15 00:00 UTC
        let o = ObsParams::default();
        let ag = Agility::from_obs(&o);
        let mut shell = Shell::new(ShellParams { n: n_sats, ..Default::default() }, jd0);
        let mut st = ObsState::default();
        let targets: Vec<usize> = (1..9).collect();
        let mut cov: Vec<(usize, Cov)> = targets
            .iter()
            .map(|&i| (i, Cov { eligible_s: 0.0, excluded_s: 0.0, full_s: 0.0, any_s: 0.0, max_gap_s: 0.0, cur_gap: 0.0, recruits: 0, acquired: false }))
            .collect();
        let steps = (hours * 3600.0 / dt) as usize;
        for k in 0..steps {
            let jd = jd0 + k as f64 * dt / 86_400.0;
            shell.propagate(jd);
            let b = astro::bodies(jd);
            let (sun, moon) = (b[0].dir, b[1].dir);
            update(&mut st, &o, Mode::Survey, &shell.pos, &[], sun, moon, None, None);
            let items: Vec<StareItem> = targets
                .iter()
                .map(|&i| StareItem { id: i as u64, dir: b[i].dir, eyes, fine: true, range_km: 0.0 })
                .collect();
            let res = assign_stare(&mut st, &o, &shell, &items, sun, moon, &ag);
            advance_attitudes(&mut st, &ag, dt, None);
            for ((_, c), r) in cov.iter_mut().zip(&res) {
                c.recruits += r.recruited;
                if r.blocked == Some("Sun exclusion") || r.blocked == Some("Moon exclusion") {
                    c.excluded_s += dt;
                    c.acquired = false;
                    c.cur_gap = 0.0;
                    continue;
                }
                let locked = locked_on(&st, job(TAG_STARE, r.id));
                if !c.acquired {
                    if locked > 0 {
                        c.acquired = true;
                    } else {
                        continue; // initial acquisition, not a gap
                    }
                }
                c.eligible_s += dt;
                if locked >= eyes {
                    c.full_s += dt;
                }
                if locked > 0 {
                    c.any_s += dt;
                    c.cur_gap = 0.0;
                } else {
                    c.cur_gap += dt;
                    c.max_gap_s = c.max_gap_s.max(c.cur_gap);
                }
            }
        }
        cov
    }

    #[test]
    fn retarget_between_planets_is_a_real_slew_even_at_high_time_rates() {
        let jd0 = 2_461_359.5;
        let o = ObsParams::default();
        let ag = Agility::from_obs(&o);
        let shell = Shell::new(ShellParams { n: 200, ..Default::default() }, jd0);
        let b = astro::bodies(jd0);
        let mut st = ObsState::default();
        let (sun, moon) = (b[0].dir, b[1].dir);
        // Lock on Jupiter
        for _ in 0..3 {
            update(&mut st, &o, Mode::Stack, &shell.pos, &[], sun, moon, Some(b[5].dir), Some(5));
            advance_attitudes(&mut st, &ag, 600.0, Some(b[5].dir));
        }
        assert!(st.n_locked_target > 0);
        // Retarget to Saturn with a 1-hour step: must be a new task, with a
        // slew whose time is subtracted from the integration.
        update(&mut st, &o, Mode::Stack, &shell.pos, &[], sun, moon, Some(b[6].dir), Some(6));
        advance_attitudes(&mut st, &ag, 3600.0, Some(b[6].dir));
        let slews = st.new_slews.len();
        assert!(slews > 0, "retarget should start slews");
        let on: Vec<f64> = (0..st.job.len()).filter(|&i| job_tag(st.job[i]) == TAG_STACK).map(|i| st.obs_dt[i]).collect();
        assert!(!on.is_empty() && on.len() <= o.target_eyes as usize, "target capped at {} eyes, got {}", o.target_eyes, on.len());
        assert!(on.iter().all(|&d| d < 3600.0 - 7.0), "integration must lose the repoint time");
        // Everyone else keeps surveying.
        assert!(st.job.iter().any(|&j| job_tag(j) == TAG_SURVEY));
    }

    #[test]
    fn thousand_sats_hold_ten_eyes_on_jupiter_and_saturn() {
        let cov = fly(1000, 10, 3.0, 2.0);
        for (i, c) in cov.iter().filter(|(i, _)| *i == 5 || *i == 6) {
            assert!(c.excluded_s == 0.0, "{} unexpectedly excluded", BODY_NAMES[*i]);
            let full = c.full_s / c.eligible_s;
            assert!(full > 0.995, "{}: full-strength coverage {:.4}", BODY_NAMES[*i], full);
            assert_eq!(c.max_gap_s, 0.0, "{}: gap {}", BODY_NAMES[*i], c.max_gap_s);
        }
    }

    /// cargo test --release stare_24h -- --ignored --nocapture
    #[test]
    #[ignore]
    fn stare_24h_report() {
        for (n, eyes) in [(1000usize, 10usize), (1000, 50), (100, 10)] {
            let t = std::time::Instant::now();
            let cov = fly(n, eyes, 24.0, 2.0);
            println!("\n== {n} satellites, {eyes} eyes per target, 24 h from 2026-11-15 ({:.0} s CPU)", t.elapsed().as_secs_f64());
            println!("{:8} {:>9} {:>11} {:>11} {:>9} {:>9}", "target", "excluded", "full-k %", "≥1 eye %", "max gap", "handovers/h");
            for (i, c) in &cov {
                if c.eligible_s == 0.0 {
                    println!("{:8} {:>8.1}h   (inside Sun/Moon exclusion all day)", BODY_NAMES[*i], c.excluded_s / 3600.0);
                    continue;
                }
                println!(
                    "{:8} {:>8.1}h {:>10.3}% {:>10.3}% {:>8.0}s {:>9.1}",
                    BODY_NAMES[*i],
                    c.excluded_s / 3600.0,
                    100.0 * c.full_s / c.eligible_s,
                    100.0 * c.any_s / c.eligible_s,
                    c.max_gap_s,
                    c.recruits as f64 / ((c.eligible_s + c.excluded_s) / 3600.0)
                );
            }
        }
    }
}
