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
}

impl Mode {
    pub const ALL: [Mode; 5] = [Mode::Survey, Mode::Stack, Mode::Mosaic, Mode::NeoSweep, Mode::Queue];
    pub fn key(self) -> &'static str {
        match self {
            Mode::Survey => "survey",
            Mode::Stack => "stack",
            Mode::Mosaic => "mosaic",
            Mode::NeoSweep => "neo",
            Mode::Queue => "queue",
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
            Mode::Queue => "Queue (observation requests)",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
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
}

impl Default for ObsParams {
    fn default() -> Self {
        ObsParams {
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
            obstruction: 0.33,
            sensor_px: 9576.0,
            pixel_um: 3.76,
            qe: 0.8,
            throughput: 0.75,
            read_noise_e: 1.5,
            dark_e_s: 0.003,
            full_well_e: 51_000.0,
            jitter_arcsec: 1.0,
            sub_max_s: 300.0,
        }
    }
}

/// Per-frame observation state.
#[derive(Default)]
pub struct ObsState {
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
    } else if dir.angle(moon) < rad(o.moon_excl_deg) {
        Some("Moon exclusion")
    } else {
        None
    }
}

/// Tangent-plane basis (east, north) at a sky direction.
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
) {
    let n = pos.len();
    st.rhat.clear();
    st.rhat.extend(pos.iter().map(|p| p.unit()));
    st.lit.clear();
    st.lit.extend(pos.iter().map(|&p| !in_shadow(p, sun)));
    st.n_lit = st.lit.iter().filter(|&&l| l).count();

    let r = pos.first().map(|p| p.norm()).unwrap_or(EARTH_RADIUS_KM + 550.0);
    let earth_half = ((EARTH_RADIUS_KM + o.atmo_km) / r).clamp(-1.0, 1.0).asin();
    st.earth_cone_deg = earth_half.to_degrees();
    st.cos_lim = -(earth_half + rad(o.limb_margin_deg)).cos();
    let cos_lim = st.cos_lim;

    // Laser links grazing the atmosphere are unusable.
    let r_min = EARTH_RADIUS_KM + o.atmo_km;
    st.link_blocked.clear();
    st.link_blocked.extend(links.iter().map(|&(a, b)| {
        let (pa, pb) = (pos[a as usize], pos[b as usize]);
        let d = pb - pa;
        let t = (-pa.dot(d) / d.dot(d).max(1e-9)).clamp(0.0, 1.0);
        (pa + d * t).norm() < r_min
    }));
    st.links_blocked = st.link_blocked.iter().filter(|&&b| b).count();

    st.pointing.clear();
    st.pointing.resize(n, None);
    st.n_target_visible = 0;
    st.max_baseline_km = 0.0;
    st.target_blocked_by = None;

    let ok = |d: V3| sky_blocked(o, d, sun, moon).is_none();

    match mode {
        Mode::Survey => {
            for i in 0..n {
                let z = st.rhat[i];
                if ok(z) {
                    st.pointing[i] = Some(z);
                }
            }
        }
        Mode::Stack | Mode::Mosaic => {
            if let Some(t) = target {
                st.target_blocked_by = sky_blocked(o, t, sun, moon);
                let visible: Vec<usize> = if st.target_blocked_by.is_none() {
                    (0..n).filter(|&i| t.dot(st.rhat[i]) > cos_lim).collect()
                } else {
                    Vec::new()
                };
                st.n_target_visible = visible.len();
                if mode == Mode::Stack {
                    for &i in &visible {
                        st.pointing[i] = Some(t);
                    }
                    st.max_baseline_km = projected_max_baseline(pos, &visible, t);
                } else {
                    let (e, nn) = tangent_basis(t);
                    let g = (visible.len() as f64).sqrt().ceil().max(1.0) as usize;
                    let step = rad(o.fov_deg);
                    let half = (g as f64 - 1.0) / 2.0;
                    // Assign tiles by sorting satellites along the tangent axes so
                    // neighbours on the sky are neighbours in orbit (short slews).
                    let mut order = visible.clone();
                    order.sort_by(|&a, &b| {
                        let ka = (pos[a].dot(nn), pos[a].dot(e));
                        let kb = (pos[b].dot(nn), pos[b].dot(e));
                        ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    for (k, &i) in order.iter().enumerate() {
                        let (row, col) = (k / g, k % g);
                        let x = (col as f64 - half) * step;
                        let y = (row as f64 - half) * step;
                        let d = (t + e * x.tan() + nn * y.tan()).unit();
                        if ok(d) && d.dot(st.rhat[i]) > cos_lim {
                            st.pointing[i] = Some(d);
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
                if ok(d) && d.dot(st.rhat[i]) > cos_lim {
                    st.pointing[i] = Some(d);
                }
            }
        }
        Mode::Queue => {} // pointing assigned by `assign_queue`
    }
    st.n_active = st.pointing.iter().filter(|p| p.is_some()).count();
}

pub struct QueueItem {
    pub id: u64,
    pub dir: V3,
    pub want: usize,
}

pub struct QueueResult {
    pub id: u64,
    pub assigned: usize,
    pub blocked: Option<&'static str>,
}

/// Greedy allocator: items in priority order each take up to `want` idle
/// telescopes that have a clear line of sight. Deliberately simple — a
/// baseline to compare smarter schedulers against (slew cost, handover,
/// fairness, airmass-free but limb-proximity weighting, …).
pub fn assign_queue(st: &mut ObsState, o: &ObsParams, items: &[QueueItem], sun: V3, moon: V3) -> Vec<QueueResult> {
    let n = st.rhat.len();
    let mut out = Vec::with_capacity(items.len());
    let mut idle = n;
    for it in items {
        if let Some(why) = sky_blocked(o, it.dir, sun, moon) {
            out.push(QueueResult { id: it.id, assigned: 0, blocked: Some(why) });
            continue;
        }
        let mut got = 0;
        if idle > 0 {
            for i in 0..n {
                if got >= it.want {
                    break;
                }
                if st.pointing[i].is_none() && it.dir.dot(st.rhat[i]) > st.cos_lim {
                    st.pointing[i] = Some(it.dir);
                    got += 1;
                    idle -= 1;
                }
            }
        }
        let blocked = if got == 0 {
            Some(if idle == 0 { "no free telescopes" } else { "Earth blocks every free telescope" })
        } else {
            None
        };
        out.push(QueueResult { id: it.id, assigned: got, blocked });
    }
    st.n_active = st.pointing.iter().filter(|p| p.is_some()).count();
    out
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
    out.clear();
    out.extend(grid.iter().map(|&g| {
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
        update(&mut st, &ObsParams::default(), Mode::Survey, &pos, &[], V3::new(0.0, 1.0, 0.0), V3::Z, None);
        assert!((st.earth_cone_deg - 69.2).abs() < 0.2, "{}", st.earth_cone_deg);
    }

    #[test]
    fn opposite_pair_link_blocked() {
        let pos = vec![V3::new(6928.0, 0.0, 0.0), V3::new(-6928.0, 0.0, 0.0)];
        let mut st = ObsState::default();
        update(&mut st, &ObsParams::default(), Mode::Survey, &pos, &[(0, 1)], V3::new(0.0, 1.0, 0.0), V3::Z, None);
        assert_eq!(st.links_blocked, 1);
    }
}
