//! FACET compared with a ground-based network of the same telescopes.
//!
//! The comparison network has as many identical 20 cm telescopes as FACET has
//! satellites, spread evenly over a dozen good observatory sites. A target is
//! observable from a site when the Sun is below −12° (nautical twilight),
//! the target is above 30° altitude (airmass < 2) and more than 15° from the
//! Moon, and the night is clear. Weather is a per-site, per-night coin toss
//! with an ASSUMED clear fraction; results are averaged over many weather
//! realisations (Monte Carlo).
//!
//! Depth uses the same aperture with ASSUMED ground seeing, sky brightness and
//! extinction. Everything here is a generic model for comparison, not a
//! prediction for any particular observatory.

use crate::astro;
use crate::math::{rad, V3};
use serde::Serialize;

/// Generic good observatory sites: (lat, lon, assumed clear fraction).
pub const SITES: [(f64, f64, f64); 12] = [
    (-24.6, -70.4, 0.85),  // Atacama
    (-29.3, -70.7, 0.80),  // Atacama, south
    (28.8, -17.9, 0.70),   // Canary Islands
    (19.8, -155.5, 0.75),  // Hawaii
    (32.0, -111.6, 0.65),  // Arizona
    (31.0, -115.5, 0.70),  // Baja California
    (-32.4, 20.8, 0.60),   // South Africa
    (-23.3, 16.5, 0.75),   // Namibia
    (-31.3, 149.1, 0.55),  // Australia
    (37.2, -2.5, 0.60),    // Spain
    (38.7, 66.9, 0.60),    // Uzbekistan
    (32.8, 80.0, 0.65),    // Tibetan plateau
];

/// Ground observing conditions (ASSUMED, dark site, good night).
pub struct GroundConditions {
    pub seeing_arcsec: f64,
    pub sky_mag_arcsec2: f64,
    pub extinction_mag: f64,
    pub min_alt_deg: f64,
    pub sun_alt_deg: f64,
    pub moon_sep_deg: f64,
    pub uptime: f64,
}

pub const COND: GroundConditions = GroundConditions {
    seeing_arcsec: 1.8,
    sky_mag_arcsec2: 21.3,
    extinction_mag: 0.25,
    min_alt_deg: 30.0,
    sun_alt_deg: -12.0,
    moon_sep_deg: 15.0,
    uptime: 0.9,
};

#[derive(Clone, Default, Serialize)]
pub struct Comparison {
    pub computed_utc: String,
    pub sites: usize,
    pub telescopes: usize,
    /// Fraction of time a ground telescope is open on the sky (night × clear × uptime).
    pub ground_duty: f64,
    /// FACET's measured open-shutter fraction.
    pub facet_duty: f64,
    /// Sky observable right now by at least one site (expected, clear-weighted).
    pub ground_sky_now: f64,
    pub facet_sky_now: f64,
    /// Requests observable within 24 h (ground) and median wait.
    pub targets: usize,
    pub ground_within_24h: f64,
    /// Queue targets observable right now (expected over weather).
    pub ground_now: f64,
    pub facet_now: f64,
    pub ground_median_wait_s: Option<f64>,
    pub facet_within_24h: f64,
    pub facet_median_response_s: Option<f64>,
    /// 5σ point-source depth, one telescope, 60 s.
    pub facet_depth_60s: f64,
    pub ground_depth_60s: f64,
    /// Ground telescopes needed to match one FACET telescope's throughput at equal depth.
    pub equivalent_ground_telescopes: f64,
    pub notes: Vec<String>,
}

/// Run the comparison for a set of target directions (e.g. the open queue).
#[allow(clippy::too_many_arguments)]
pub fn compare(
    jd: f64,
    targets: &[V3],
    n_facet: usize,
    facet_duty: f64,
    facet_sky_now: f64,
    facet_sun_excl_deg: f64,
    facet_median_response_s: Option<f64>,
    hw: &crate::imaging::Hardware,
) -> Comparison {
    use rayon::prelude::*;
    let c = &COND;
    let ns = SITES.len();
    let step_s = 600.0;
    let steps = (48.0 * 3600.0 / step_s) as usize;
    let sin_sun = rad(c.sun_alt_deg).sin();
    let sin_alt = rad(c.min_alt_deg).sin();
    let cos_moon = rad(c.moon_sep_deg).cos();
    // Per time step: zenith of each site, Sun and Moon directions.
    let frames: Vec<(Vec<V3>, V3, V3, f64)> = (0..steps)
        .into_par_iter()
        .map(|k| {
            let t = jd + k as f64 * step_s / 86_400.0;
            let g = astro::gmst(t);
            let b = astro::bodies(t);
            let z = SITES.iter().map(|&(la, lo, _)| astro::ecef_to_eci(lo, la, 1.0, g)).collect();
            (z, b[0].dir, b[1].dir, t)
        })
        .collect();
    let night = |k: usize, s: usize| frames[k].0[s].dot(frames[k].1) < sin_sun;
    // Duty cycle: night fraction × clear × uptime, averaged over sites and 48 h.
    let mut duty = 0.0;
    for s in 0..ns {
        let nf = (0..steps).filter(|&k| night(k, s)).count() as f64 / steps as f64;
        duty += nf * SITES[s].2 * c.uptime;
    }
    let ground_duty = duty / ns as f64;
    // Local "night id" per site and step, for weather draws.
    let night_id = |k: usize, s: usize| ((frames[k].3 + 0.5 + SITES[s].1 / 360.0).floor() as i64 - (jd.floor() as i64) + 2) as usize;
    // Geometry masks: which sites can see direction d at step k.
    let mask = |d: V3, k: usize| -> u16 {
        let (z, _, moon, _) = &frames[k];
        let mut m = 0u16;
        for s in 0..ns {
            if night(k, s) && z[s].dot(d) > sin_alt && d.dot(*moon) < cos_moon {
                m |= 1 << s;
            }
        }
        m
    };
    // Sky now: grid of directions, expected probability at least one clear site sees it.
    let mut grid = Vec::new();
    for i in 0..36 {
        let dec = -85.0 + 170.0 * (i as f64 + 0.5) / 36.0;
        let nra = ((72.0 * rad(dec).cos()).round() as usize).max(1);
        for j in 0..nra {
            grid.push(V3::from_radec(rad(360.0 * j as f64 / nra as f64), rad(dec)));
        }
    }
    let sky_now = grid
        .par_iter()
        .map(|&d| {
            let m = mask(d, 0);
            1.0 - (0..ns).filter(|s| m & (1 << s) != 0).map(|s| 1.0 - SITES[s].2 * c.uptime).product::<f64>()
        })
        .sum::<f64>()
        / grid.len() as f64;
    // Targets: Monte Carlo over weather for the wait until first observable.
    let realisations = 40;
    let mut rng = 0x9E37_79B9_7F4A_7C15u64 ^ (jd * 1e5) as u64;
    let mut next = || {
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        (rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    };
    // clear[r][s][night]
    let clear: Vec<Vec<[bool; 6]>> = (0..realisations)
        .map(|_| (0..ns).map(|s| std::array::from_fn(|_| next() < SITES[s].2 * c.uptime)).collect())
        .collect();
    let waits: Vec<Vec<Option<f64>>> = targets
        .par_iter()
        .map(|&d| {
            let masks: Vec<u16> = (0..steps).map(|k| mask(d, k)).collect();
            (0..realisations)
                .map(|r| {
                    (0..steps).find(|&k| (0..ns).any(|s| masks[k] & (1 << s) != 0 && clear[r][s][night_id(k, s).min(5)])).map(|k| k as f64 * step_s)
                })
                .collect()
        })
        .collect();
    let mut all: Vec<f64> = Vec::new();
    let (mut within, mut total, mut now) = (0usize, 0usize, 0usize);
    for w in &waits {
        for x in w {
            total += 1;
            match x {
                Some(s) if *s <= 86_400.0 => {
                    within += 1;
                    if *s == 0.0 {
                        now += 1;
                    }
                    all.push(*s);
                }
                Some(s) => all.push(*s),
                None => all.push(f64::INFINITY),
            }
        }
    }
    all.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = all.get(all.len() / 2).copied().filter(|x| x.is_finite());
    // FACET: a target is observable unless inside the Sun keep-out.
    let sun0 = frames[0].1;
    let facet_ok = targets.iter().filter(|d| d.dot(sun0) < rad(facet_sun_excl_deg).cos()).count();
    // Depth: same aperture. Space: the Fourier-optics PSF and zodiacal sky;
    // ground: background-limited scaling with seeing, sky and extinction.
    let facet_sky = 22.8;
    let facet_depth = crate::imaging::point_lim_mag(hw, 60.0, 60.0f64.min(hw.sub_max_s), facet_sky);
    let fwhm_space = hw.fwhm(60.0).max(0.3);
    let dm = 2.5 * (c.seeing_arcsec / fwhm_space).log10() + 1.25 * 10f64.powf(0.4 * (facet_sky - c.sky_mag_arcsec2)).log10() + c.extinction_mag;
    let ground_depth = facet_depth - dm;
    // Time to reach equal depth scales as 10^(0.8 Δm) (background limited).
    let equiv = if ground_duty > 0.0 { facet_duty / ground_duty * 10f64.powf(0.8 * dm) } else { f64::INFINITY };
    let per_site = (n_facet as f64 / ns as f64).ceil() as usize;
    Comparison {
        computed_utc: astro::format_utc(jd),
        sites: ns,
        telescopes: per_site * ns,
        ground_duty,
        facet_duty,
        ground_sky_now: sky_now,
        facet_sky_now,
        targets: targets.len(),
        ground_within_24h: if total > 0 { within as f64 / total as f64 } else { 0.0 },
        ground_now: if total > 0 { now as f64 / total as f64 } else { 0.0 },
        facet_now: if targets.is_empty() { 0.0 } else { facet_ok as f64 / targets.len() as f64 },
        ground_median_wait_s: med,
        facet_within_24h: if targets.is_empty() { 0.0 } else { facet_ok as f64 / targets.len() as f64 },
        facet_median_response_s,
        facet_depth_60s: facet_depth,
        ground_depth_60s: ground_depth,
        equivalent_ground_telescopes: equiv,
        notes: vec![
            format!(
                "Ground: {} sites, Sun below {}°, targets above {}°, {}° from the Moon; seeing {}\", sky V {} mag/arcsec², extinction {} mag; clear fractions 55–85 % (assumed).",
                ns, c.sun_alt_deg, c.min_alt_deg, c.moon_sep_deg, c.seeing_arcsec, c.sky_mag_arcsec2, c.extinction_mag
            ),
            "Ground benefits not modelled: larger apertures are cheap on the ground, and it can be serviced and upgraded.".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_network_is_plausible() {
        let hw = crate::imaging::Hardware::from_obs(&crate::obs::ObsParams::default());
        let jd = 2_461_360.5;
        let targets: Vec<V3> = (0..40).map(|k| V3::from_radec(rad(9.0 * k as f64), rad(-30.0 + 1.5 * k as f64))).collect();
        let c = compare(jd, &targets, 1000, 0.9, 0.84, 45.0, Some(12.0), &hw);
        eprintln!(
            "duty ground {:.3} · sky now {:.3} · within 24h {:.2} · median wait {:?} · depth facet {:.2} ground {:.2} · equiv {:.1}",
            c.ground_duty, c.ground_sky_now, c.ground_within_24h, c.ground_median_wait_s, c.facet_depth_60s, c.ground_depth_60s, c.equivalent_ground_telescopes
        );
        assert!(c.ground_duty > 0.1 && c.ground_duty < 0.4, "duty {}", c.ground_duty);
        assert!(c.ground_sky_now > 0.05 && c.ground_sky_now < 0.6);
        assert!(c.ground_within_24h > 0.3 && c.ground_within_24h <= 1.0);
        assert!(c.ground_depth_60s < c.facet_depth_60s);
        assert!(c.equivalent_ground_telescopes > 2.0);
    }
}
