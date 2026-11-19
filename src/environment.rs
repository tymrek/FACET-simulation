//! The near-Earth environment: South Atlantic Anomaly radiation, Earthshine
//! stray light and atmospheric-drag orbital lifetime.
//!
//! All three are deliberately simple engineering models, sized to give the
//! right magnitudes and trends for a pitch-level simulator. None replaces
//! AP-9/AE-9, a ray-traced baffle analysis or NRLMSISE-00 with a solar-flux
//! forecast; the assumptions are listed beside each model.
//!
//! ## South Atlantic Anomaly
//! Earth's dipole is offset ~500 km from the geocentre and tilted, so the
//! inner proton belt reaches down into LEO over the South Atlantic. The model
//! is the >10 MeV trapped-proton flux as two Gaussian lobes in geographic
//! latitude/longitude (the main anomaly over South America and the weaker
//! south-west-African lobe that is slowly separating from it), drifting west
//! at ~0.3°/yr, with a peak flux that grows exponentially with altitude
//! (e-folding ~60 km).
//! ASSUMED calibration: inside the 10 p/cm²/s contour detectors are safed;
//! a Hubble-like orbit (540 km, 28.5°) then spends ~11 % of its time in the
//! anomaly, an ISS-like orbit (420 km, 51.6°) ~5 %, FACET (550 km, 53°) ~9 %.
//!
//! ## Earthshine
//! Sunlight scattered off the Earth's limb into the telescope. Surface
//! brightness falls with angle above the limb and scales with how sunlit the
//! limb is; ASSUMED V ≈ 19.0 mag/arcsec² at 10° above a fully lit limb,
//! fading 6 mag per decade of limb angle (≈20.8 at 20°, 21.9 at 30°). The
//! penalty is applied as background-limited equivalent exposure time.
//!
//! ## Orbital lifetime
//! Circular-orbit drag decay da/dt = −ρ·(C_D·A/m)·√(μa) with the CIRA-72
//! exponential density table (Vallado, mean solar activity), modulated over an
//! 11-year solar cycle by an altitude-dependent max/min ratio.

use crate::astro::{EARTH_RADIUS_KM, MU_EARTH};
use crate::math::{deg, rad, V3};
use serde::Serialize;

// ---------------------------------------------------------------- SAA

/// ASSUMED geographic centre of the main SAA lobe in 2025 (deg) and its drift.
pub const SAA_LAT0: f64 = -26.0;
pub const SAA_LON0: f64 = -52.0;
pub const SAA_DRIFT_DEG_YR: f64 = -0.28;
/// Lobes: (Δlat, Δlon from main centre, σlat, σlon, relative amplitude).
const LOBES: [(f64, f64, f64, f64, f64); 2] = [(0.0, 0.0, 6.0, 13.0, 1.0), (-6.0, 50.0, 7.0, 13.0, 0.30)];
/// Peak >10 MeV proton flux at 500 km (p/cm²/s) and its altitude e-folding.
const SAA_PEAK_500: f64 = 450.0;
const SAA_SCALE_KM: f64 = 60.0;
/// ASSUMED galactic-cosmic-ray flux at the sensor outside the SAA (p/cm²/s).
pub const GCR_FLUX: f64 = 2.0;

/// >10 MeV trapped-proton flux (p/cm²/s) at a geographic point and altitude.
pub fn saa_flux(lat_deg: f64, lon_deg: f64, alt_km: f64, year: f64) -> f64 {
    let lon_c = SAA_LON0 + SAA_DRIFT_DEG_YR * (year - 2025.0);
    let peak = SAA_PEAK_500 * ((alt_km - 500.0) / SAA_SCALE_KM).exp();
    let mut f = 0.0;
    for &(dlat, dlon, slat, slon, amp) in &LOBES {
        let y = lat_deg - (SAA_LAT0 + dlat);
        let x = (lon_deg - (lon_c + dlon) + 540.0).rem_euclid(360.0) - 180.0;
        f += amp * (-0.5 * ((y / slat).powi(2) + (x / slon).powi(2))).exp();
    }
    peak * f
}

/// Geodetic-enough latitude/longitude (deg) of an ECI position at `gmst`.
pub fn latlon(p: V3, gmst: f64) -> (f64, f64) {
    let e = p.rot_z(-gmst);
    let r = e.norm().max(1e-9);
    (deg((e.z / r).clamp(-1.0, 1.0).asin()), deg(e.y.atan2(e.x)))
}

/// Contour of the SAA at `alt_km` where flux = `thr`, as (lat, lon) points
/// around the main lobe centre (ray-marched; the two-lobe shape is star-shaped
/// from there). Empty if the anomaly is weaker than `thr` at that altitude.
pub fn saa_contour(alt_km: f64, year: f64, thr: f64, steps: usize) -> Vec<(f64, f64)> {
    let lon_c = SAA_LON0 + SAA_DRIFT_DEG_YR * (year - 2025.0);
    if saa_flux(SAA_LAT0, lon_c, alt_km, year) < thr {
        return Vec::new();
    }
    (0..=steps)
        .map(|k| {
            let a = k as f64 / steps as f64 * std::f64::consts::TAU;
            let (dy, dx) = (a.sin() * 0.45, a.cos());
            let (mut lo, mut hi) = (0.0, 120.0);
            for _ in 0..30 {
                let m = 0.5 * (lo + hi);
                if saa_flux(SAA_LAT0 + dy * m, lon_c + dx * m, alt_km, year) > thr {
                    lo = m;
                } else {
                    hi = m;
                }
            }
            (SAA_LAT0 + dy * lo, lon_c + dx * lo)
        })
        .collect()
}

/// Fraction of time a circular orbit spends inside the `thr` contour, averaged
/// over the ground track (argument of latitude × Earth-fixed node longitude).
pub fn saa_time_fraction(alt_km: f64, inc_deg: f64, year: f64, thr: f64) -> f64 {
    let (nu, nl) = (720, 180);
    let i = rad(inc_deg);
    let mut hit = 0usize;
    for a in 0..nu {
        let u = (a as f64 + 0.5) / nu as f64 * std::f64::consts::TAU;
        let lat = (i.sin() * u.sin()).asin();
        let dl = (i.cos() * u.sin()).atan2(u.cos());
        for b in 0..nl {
            let lon = (b as f64 + 0.5) / nl as f64 * 360.0 - 180.0 + deg(dl);
            if saa_flux(deg(lat), lon, alt_km, year) > thr {
                hit += 1;
            }
        }
    }
    hit as f64 / (nu * nl) as f64
}

// ---------------------------------------------------------- Earthshine

/// ASSUMED Earthshine surface brightness at 10° above a fully sunlit limb, and
/// its fall-off in magnitudes per decade of limb angle.
pub const ES_SB_10DEG: f64 = 19.0;
pub const ES_MAG_PER_DECADE: f64 = 6.0;

/// Earthshine (V mag/arcsec²) toward `d` from a satellite at `p`, or `None`
/// when negligible (> 90° above the limb). `limb_r` is the radius treated as
/// the bright limb (Earth + atmosphere), km.
pub fn earthshine_sb(p: V3, d: V3, sun: V3, limb_r: f64) -> Option<f64> {
    let r = p.norm();
    if r <= limb_r {
        return None;
    }
    let n = -p.unit();
    let half = (limb_r / r).asin();
    let alpha = d.angle(n);
    let theta = deg(alpha - half);
    if theta > 90.0 {
        return None;
    }
    // Limb point in the plane of nadir and the pointing.
    let w = d - n * d.dot(n);
    let w = if w.norm() < 1e-9 { n.any_perp() } else { w.unit() };
    let ld = n * half.cos() + w * half.sin();
    let l = p + ld * (r * r - limb_r * limb_r).sqrt();
    let illum = ((l.unit().dot(sun) + 0.1) / 1.1).clamp(0.0, 1.0);
    if illum < 1e-4 {
        return None;
    }
    Some(ES_SB_10DEG + ES_MAG_PER_DECADE * (theta.max(2.0) / 10.0).log10() - 2.5 * illum.log10())
}

/// Background-limited efficiency: dark-sky-equivalent time per second of
/// exposure when Earthshine `es` adds to sky `sky` (both mag/arcsec²).
#[cfg_attr(not(test), allow(dead_code))]
pub fn bg_efficiency(sky: f64, es: Option<f64>) -> f64 {
    match es {
        None => 1.0,
        Some(e) => {
            let b0 = 10f64.powf(-0.4 * sky);
            b0 / (b0 + 10f64.powf(-0.4 * e))
        }
    }
}

// ------------------------------------------------------------ Lifetime

/// CIRA-72 exponential atmosphere (Vallado table 8-4): base km, ρ0 kg/m³, H km.
const ATM: [(f64, f64, f64); 15] = [
    (150.0, 2.070e-9, 22.523),
    (180.0, 5.464e-10, 29.740),
    (200.0, 2.789e-10, 37.105),
    (250.0, 7.248e-11, 45.546),
    (300.0, 2.418e-11, 53.628),
    (350.0, 9.518e-12, 53.298),
    (400.0, 3.725e-12, 58.515),
    (450.0, 1.585e-12, 60.828),
    (500.0, 6.967e-13, 63.822),
    (600.0, 1.454e-13, 71.835),
    (700.0, 3.614e-14, 88.667),
    (800.0, 1.170e-14, 124.64),
    (900.0, 5.245e-15, 181.05),
    (1000.0, 3.019e-15, 268.00),
    (1100.0, 1.0e-15, 300.0),
];

/// Mean-activity density, kg/m³.
pub fn density_mean(h: f64) -> f64 {
    let mut k = 0;
    for (i, b) in ATM.iter().enumerate() {
        if h >= b.0 {
            k = i;
        }
    }
    let (h0, r0, s) = ATM[k];
    r0 * (-(h - h0) / s).exp()
}

/// Solar max/mean density ratio (= mean/min), ASSUMED: ~1 at 150 km rising to
/// ~3.5 above 600 km (thermospheric heating dominates high up).
pub fn solar_ratio(h: f64) -> f64 {
    1.0 + 2.5 * ((h - 150.0) / 450.0).clamp(0.0, 1.0)
}

/// ASSUMED solar-cycle maximum (decimal year) and period.
pub const SOLAR_MAX_YEAR: f64 = 2024.8;
pub const SOLAR_PERIOD_YR: f64 = 11.0;

/// Density at altitude and decimal year (`None` = hold mean activity).
pub fn density(h: f64, year: Option<f64>) -> f64 {
    match year {
        None => density_mean(h),
        Some(y) => {
            let ph = (y - SOLAR_MAX_YEAR) / SOLAR_PERIOD_YR * std::f64::consts::TAU;
            density_mean(h) * solar_ratio(h).powf(ph.cos())
        }
    }
}

#[derive(Clone, Copy, Serialize)]
pub enum Solar {
    Mean,
    Min,
    Max,
    /// Follow the solar cycle from the given start year.
    Cycle(f64),
}

/// Passive orbital lifetime (years) from a circular orbit at `alt_km` for a
/// ballistic coefficient `bc` = C_D·A/m (m²/kg). Capped at `cap_yr`.
pub fn lifetime_years(alt_km: f64, bc: f64, solar: Solar, cap_yr: f64) -> f64 {
    let mu = MU_EARTH * 1e9; // m³/s²
    let mut h = alt_km;
    let mut t = 0.0; // s
    let cap = cap_yr * 365.25 * 86_400.0;
    while h > 150.0 && t < cap {
        let a = (EARTH_RADIUS_KM + h) * 1e3;
        let rho = match solar {
            Solar::Mean => density_mean(h),
            Solar::Min => density_mean(h) / solar_ratio(h),
            Solar::Max => density_mean(h) * solar_ratio(h),
            Solar::Cycle(y0) => density(h, Some(y0 + t / (365.25 * 86_400.0))),
        };
        let dadt = rho * bc * (mu * a).sqrt(); // m/s
        // Step so altitude drops ≤ 0.5 km, between 1 hour and 30 days.
        let dt = (500.0 / dadt.max(1e-12)).clamp(3600.0, 30.0 * 86_400.0);
        h -= dadt * dt / 1e3;
        t += dt;
    }
    t / (365.25 * 86_400.0)
}

/// Δv (m/s) to drop perigee from a circular orbit at `alt_km` to `perigee_km`.
pub fn deorbit_dv(alt_km: f64, perigee_km: f64) -> f64 {
    let ra = EARTH_RADIUS_KM + alt_km;
    let rp = EARTH_RADIUS_KM + perigee_km;
    let vc = (MU_EARTH / ra).sqrt();
    let va = (MU_EARTH * (2.0 / ra - 2.0 / (ra + rp))).sqrt();
    (vc - va) * 1e3
}

// ----------------------------------------------------- Sun keep-out trade

/// One row of the Sun keep-out trade: what a given keep-out angle buys and costs.
#[derive(Clone, Serialize)]
pub struct KeepRow {
    pub keepout_deg: f64,
    /// Share of days over the next three years each inner planet is outside it.
    pub venus_frac: f64,
    pub mercury_frac: f64,
    /// Sky outside the keep-out, deg².
    pub open_deg2: f64,
    /// Background at the keep-out edge on the ecliptic, V mag/arcsec²
    /// (zodiacal + stray), and the stray light alone.
    pub edge_sb: f64,
    pub edge_stray_sb: f64,
    /// Depth at the edge versus a dark sky (ecliptic pole, 22.3), background-
    /// limited, mag; and the part of that due to stray light alone.
    pub edge_depth_loss: f64,
    pub edge_stray_loss: f64,
}

/// Keep-out trade over 20°–60°, inner-planet visibility from the sim's own
/// ephemeris sampled daily for three years from `jd`.
pub fn keepout_trade(jd: f64, o: &crate::obs::ObsParams) -> Vec<KeepRow> {
    let days = 3 * 365;
    let el: Vec<(f64, f64)> = (0..days)
        .map(|k| {
            let b = crate::astro::bodies(jd + k as f64);
            (deg(b[3].dir.angle(b[0].dir)), deg(b[2].dir.angle(b[0].dir)))
        })
        .collect();
    let sun = V3::new(1.0, 0.0, 0.0).rot_x(rad(crate::astro::OBLIQUITY_J2000_DEG));
    let mut ks: Vec<f64> = (0..9).map(|k| 20.0 + 5.0 * k as f64).collect();
    if !ks.iter().any(|&k| (k - o.sun_excl_deg).abs() < 0.5) {
        ks.push(o.sun_excl_deg);
        ks.sort_by(|a, b| a.partial_cmp(b).unwrap());
    }
    ks.iter()
        .map(|&k| {
            // A point on the ecliptic `k` from the Sun.
            let ecl_pole = V3::Z.rot_x(rad(crate::astro::OBLIQUITY_J2000_DEG));
            let d = sun.rot_axis(ecl_pole, rad(k));
            let z = crate::imaging::zodi_sb(d, sun);
            let st = crate::imaging::sun_stray_sb(d, sun, o.stray_sb45, o.stray_slope);
            let tot = if o.stray_on { crate::imaging::add_sb(z, st) } else { z };
            KeepRow {
                keepout_deg: k,
                venus_frac: el.iter().filter(|e| e.0 > k).count() as f64 / days as f64,
                mercury_frac: el.iter().filter(|e| e.1 > k).count() as f64 / days as f64,
                open_deg2: crate::allsky::SKY_DEG2 * (1.0 + rad(k).cos()) / 2.0,
                edge_sb: tot,
                edge_stray_sb: st,
                edge_depth_loss: 1.25 * (10f64.powf(-0.4 * tot) / 10f64.powf(-0.4 * 22.3)).log10(),
                edge_stray_loss: 1.25 * (10f64.powf(-0.4 * tot) / 10f64.powf(-0.4 * z)).log10(),
            }
        })
        .collect()
}

// --------------------------------------------------------------- State

#[derive(Clone, Copy, PartialEq, Serialize)]
pub enum SaaPolicy {
    /// Detectors safed inside the anomaly (what Hubble does).
    Pause,
    /// Keep exposing; frames are flagged and particle hits must be rejected.
    Flag,
    /// Ignore the anomaly (for comparison).
    Off,
}

#[derive(Clone, Serialize)]
pub struct EnvCfg {
    pub saa_policy: SaaPolicy,
    /// Safe-mode threshold, >10 MeV p/cm²/s.
    pub saa_threshold: f64,
    pub earthshine: bool,
    /// Spacecraft mass (kg), mean tumbling cross-section (m²), drag coefficient.
    pub mass_kg: f64,
    pub area_m2: f64,
    pub cd: f64,
    /// Perigee for a controlled disposal burn, km.
    pub disposal_perigee_km: f64,
    /// ASSUMED fraction of particle hits that look like a point source in a
    /// single frame (after shape cuts) — candidate false transients.
    pub starlike_frac: f64,
}

impl Default for EnvCfg {
    fn default() -> Self {
        EnvCfg {
            saa_policy: SaaPolicy::Pause,
            saa_threshold: 10.0,
            earthshine: true,
            mass_kg: 150.0,
            area_m2: 1.2,
            cd: 2.2,
            disposal_perigee_km: 250.0,
            starlike_frac: 0.01,
        }
    }
}

impl EnvCfg {
    pub fn ballistic(&self) -> f64 {
        self.cd * self.area_m2 / self.mass_kg.max(1.0)
    }
}

/// One row of the altitude trade study.
#[derive(Clone, Serialize)]
pub struct TradeRow {
    pub alt_km: f64,
    pub saa_frac: f64,
    pub peak_flux: f64,
    pub life_mean_yr: f64,
    pub life_min_yr: f64,
    pub life_max_yr: f64,
    pub disposal_dv: f64,
}

#[derive(Default)]
pub struct Environment {
    pub cfg: EnvCfg,
    /// Satellite inside the SAA contour this step.
    pub in_saa: Vec<bool>,
    pub n_in_saa: usize,
    /// Dark-sky-equivalent efficiency of the current exposure (1 = no Earthshine).
    pub shine: Vec<f32>,
    pub n_shine: usize,
    // accumulators (satellite-seconds unless noted)
    pub saa_s: f64,
    pub saa_lost_s: f64,
    pub flagged_s: f64,
    /// Exposure seconds lost to Earthshine (raw − dark-sky equivalent).
    pub earthshine_lost_s: f64,
    /// Exposure seconds lost to sunward stray light.
    pub sunlight_lost_s: f64,
    pub n_sun: usize,
    pub keepout: Vec<KeepRow>,
    pub keepout_key: (i64, i64, i64, i64),
    pub exposed_s: f64,
    /// Fleet proton fluence through the sensors, p/cm² summed over satellites.
    pub fluence: f64,
    pub hits: f64,
    pub candidates: f64,
    /// Expected chance coincidences of candidates between telescopes on the same field.
    pub coincidences: f64,
    pub coincidences3: f64,
    pub elapsed_s: f64,
    pub sat_s: f64,
    // cached trade study and lifetime
    pub trade: Vec<TradeRow>,
    pub trade_key: (i64, i64, i64, i64),
    pub life: Option<(f64, f64, f64, f64)>, // mean, min, max, cycle (yr)
    pub contour: Vec<(f64, f64)>,
    pub contour_key: (i64, i64, i64),
}

impl Default for SaaPolicy {
    fn default() -> Self {
        SaaPolicy::Pause
    }
}

impl Environment {
    pub fn reset_stats(&mut self) {
        let cfg = self.cfg.clone();
        let (trade, tk, life, contour, ck) =
            (std::mem::take(&mut self.trade), self.trade_key, self.life, std::mem::take(&mut self.contour), self.contour_key);
        let (keepout, kk) = (std::mem::take(&mut self.keepout), self.keepout_key);
        *self = Environment { cfg, trade, trade_key: tk, life, contour, contour_key: ck, keepout, keepout_key: kk, ..Default::default() };
    }

    /// Refresh the altitude trade study and lifetime when inputs change.
    pub fn refresh_cached(&mut self, alt_km: f64, inc_deg: f64, year: f64) {
        let bc = self.cfg.ballistic();
        let key = ((alt_km * 10.0) as i64, (inc_deg * 10.0) as i64, (bc * 1e5) as i64, (self.cfg.saa_threshold * 10.0) as i64);
        if key != self.trade_key || self.trade.is_empty() {
            self.trade_key = key;
            let mut alts: Vec<f64> = (0..9).map(|k| 400.0 + 50.0 * k as f64).collect();
            if !alts.iter().any(|&a| (a - alt_km).abs() < 1.0) {
                alts.push(alt_km);
                alts.sort_by(|a, b| a.partial_cmp(b).unwrap());
            }
            let thr = self.cfg.saa_threshold;
            let pd = self.cfg.disposal_perigee_km;
            use rayon::prelude::*;
            self.trade = alts
                .par_iter()
                .map(|&a| TradeRow {
                    alt_km: a,
                    saa_frac: saa_time_fraction(a, inc_deg, year, thr),
                    peak_flux: saa_flux(SAA_LAT0, SAA_LON0 + SAA_DRIFT_DEG_YR * (year - 2025.0), a, year),
                    life_mean_yr: lifetime_years(a, bc, Solar::Mean, 300.0),
                    life_min_yr: lifetime_years(a, bc, Solar::Max, 300.0),
                    life_max_yr: lifetime_years(a, bc, Solar::Min, 300.0),
                    disposal_dv: if a > pd { deorbit_dv(a, pd) } else { 0.0 },
                })
                .collect();
            self.life = Some((
                lifetime_years(alt_km, bc, Solar::Mean, 300.0),
                lifetime_years(alt_km, bc, Solar::Max, 300.0),
                lifetime_years(alt_km, bc, Solar::Min, 300.0),
                lifetime_years(alt_km, bc, Solar::Cycle(year), 300.0),
            ));
        }
        let ck = ((alt_km * 10.0) as i64, (year * 10.0) as i64, (self.cfg.saa_threshold * 10.0) as i64);
        if ck != self.contour_key {
            self.contour_key = ck;
            self.contour = saa_contour(alt_km, year, self.cfg.saa_threshold, 96);
        }
    }

    pub fn json(&self, alt_km: f64) -> serde_json::Value {
        let sat = self.sat_s.max(1e-9);
        let days = (self.elapsed_s / 86_400.0).max(1e-9);
        let row = self.trade.iter().find(|r| (r.alt_km - alt_km).abs() < 1.0);
        serde_json::json!({
            "policy": match self.cfg.saa_policy { SaaPolicy::Pause => "pause", SaaPolicy::Flag => "flag", SaaPolicy::Off => "off" },
            "threshold": self.cfg.saa_threshold,
            "in_saa_now": self.n_in_saa,
            "saa_time_pct": 100.0 * self.saa_s / sat,
            "saa_lost_pct": 100.0 * self.saa_lost_s / sat,
            "orbit_saa_pct": row.map(|r| 100.0 * r.saa_frac),
            "earthshine_on": self.cfg.earthshine,
            "earthshine_now": self.n_shine,
            "earthshine_lost_pct": if self.exposed_s > 0.0 { 100.0 * self.earthshine_lost_s / self.exposed_s } else { 0.0 },
            "sunlight_now": self.n_sun,
            "sunlight_lost_pct": if self.exposed_s > 0.0 { 100.0 * self.sunlight_lost_s / self.exposed_s } else { 0.0 },
            "keepout": self.keepout,
            "fluence_per_sat_yr": self.fluence / sat * 365.25 * 86_400.0,
            "hits_per_day": self.hits / days,
            "candidates_per_day": self.candidates / days,
            "coincidences_per_day": self.coincidences / days,
            "coincidences3_per_day": self.coincidences3 / days,
            "lifetime": self.life.map(|(m, lo, hi, cy)| serde_json::json!({"mean_yr": m, "solar_max_yr": lo, "solar_min_yr": hi, "cycle_yr": cy})),
            "ballistic_m2_kg": self.cfg.ballistic(),
            "disposal_dv_ms": row.map(|r| r.disposal_dv),
            "trade": self.trade,
        })
    }
}

pub fn fmt_years(y: f64) -> String {
    if y >= 299.0 {
        "> 300 yr".into()
    } else if y >= 20.0 {
        format!("{:.0} yr", y)
    } else if y >= 1.0 {
        format!("{:.1} yr", y)
    } else {
        format!("{:.0} days", y * 365.25)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saa_calibration() {
        let hst = saa_time_fraction(540.0, 28.5, 2026.0, 10.0);
        let iss = saa_time_fraction(420.0, 51.6, 2026.0, 10.0);
        let facet = saa_time_fraction(550.0, 53.0, 2026.0, 10.0);
        let hi = saa_time_fraction(800.0, 53.0, 2026.0, 10.0);
        eprintln!("SAA time: HST-like {:.3} ISS-like {:.3} FACET {:.3} 800km {:.3}", hst, iss, facet, hi);
        assert!(hst > 0.07 && hst < 0.14, "hst {hst}");
        assert!(iss > 0.03 && iss < 0.08, "iss {iss}");
        assert!(hi > facet && facet > iss);
        assert!(!saa_contour(550.0, 2026.0, 10.0, 32).is_empty());
    }

    #[test]
    fn lifetimes() {
        let bc = EnvCfg::default().ballistic();
        let l = |h| lifetime_years(h, bc, Solar::Mean, 300.0);
        let (a, b, c, d) = (l(400.0), l(550.0), l(700.0), l(800.0));
        eprintln!("lifetime mean: 400 {a:.2} 550 {b:.2} 700 {c:.1} 800 {d:.1} yr");
        assert!(a < 2.0 && a > 0.2);
        assert!(b > 2.0 && b < 15.0);
        assert!(c > 20.0 && d > c);
        assert!(lifetime_years(550.0, bc, Solar::Max, 300.0) < b && lifetime_years(550.0, bc, Solar::Min, 300.0) > b);
        let dv = deorbit_dv(550.0, 250.0);
        assert!(dv > 70.0 && dv < 95.0, "dv {dv}");
    }

    #[test]
    fn keepout() {
        let o = crate::obs::ObsParams::default();
        let jd = 2_461_360.5; // 2026-11
        let t = keepout_trade(jd, &o);
        let at = |k: f64| t.iter().find(|r| (r.keepout_deg - k).abs() < 0.1).unwrap().clone();
        let (k20, k30, k45, k60) = (at(20.0), at(30.0), at(45.0), at(60.0));
        eprintln!(
            "venus 20 {:.2} 30 {:.2} 45 {:.2} 60 {:.2} · mercury 20 {:.2} 30 {:.2} · edge loss 20 {:.2} 45 {:.2} 60 {:.2}",
            k20.venus_frac, k30.venus_frac, k45.venus_frac, k60.venus_frac, k20.mercury_frac, k30.mercury_frac,
            k20.edge_stray_loss, k45.edge_stray_loss, k60.edge_stray_loss
        );
        assert!(k45.venus_frac > 0.03 && k45.venus_frac < 0.25);
        assert!(k30.venus_frac > k45.venus_frac && k60.venus_frac == 0.0);
        assert!(k30.mercury_frac == 0.0 && k20.mercury_frac > 0.0);
        assert!(k20.edge_depth_loss > k45.edge_depth_loss + 1.0 && k60.edge_depth_loss > 0.0);
        assert!(k20.edge_stray_loss > k60.edge_stray_loss && k60.edge_stray_loss > 0.0);
    }

    #[test]
    fn earthshine() {
        let p = V3::new(EARTH_RADIUS_KM + 550.0, 0.0, 0.0);
        let sun = V3::new(1.0, 0.0, 0.0);
        let limb = EARTH_RADIUS_KM + 100.0;
        let half = (limb / p.norm()).asin();
        let at = |t: f64| {
            let a = half + rad(t);
            V3::new(-a.cos(), a.sin(), 0.0)
        };
        let s10 = earthshine_sb(p, at(10.0), sun, limb).unwrap();
        let s30 = earthshine_sb(p, at(30.0), sun, limb).unwrap();
        assert!((s10 - 19.0).abs() < 0.6 && s30 > s10 + 2.0, "{s10} {s30}");
        assert!(earthshine_sb(p, V3::new(1.0, 0.0, 0.0), sun, limb).is_none()); // zenith
        assert!(earthshine_sb(p, at(10.0), -sun, limb).is_none()); // night limb
        assert!(bg_efficiency(22.0, Some(s10)) < 0.1 && bg_efficiency(22.0, Some(s30)) > 0.3);
    }
}

// ------------------------------------------------------- App integration

use crate::app::App;
use egui::RichText;

/// Decimal year of a Julian date.
pub fn year_of(jd: f64) -> f64 {
    2000.0 + (jd - crate::astro::JD_J2000) / 365.25
}

impl App {
    /// Apply the radiation and stray-light environment to this step's
    /// exposures (after attitudes advance, before anything is credited).
    pub fn environment_step(&mut self, dt: f64) {
        let year = year_of(self.jd);
        let (alt, inc) = (self.shell.params.alt_km, self.shell.params.inc_deg);
        self.env.refresh_cached(alt, inc, year);
        {
            let o = &self.obs_params;
            let key = ((self.jd / 30.0) as i64, (o.stray_sb45 * 10.0) as i64, (o.stray_slope * 10.0 + if o.stray_on { 0.0 } else { 1e4 }) as i64, (o.sun_excl_deg * 2.0).round() as i64);
            if key != self.env.keepout_key {
                self.env.keepout_key = key;
                self.env.keepout = keepout_trade(self.jd, o);
            }
        }
        let n = self.shell.pos.len();
        let gm = crate::astro::gmst(self.jd);
        let sun = self.bodies[0].dir;
        let limb = EARTH_RADIUS_KM + self.obs_params.atmo_km;
        let o = &self.obs_params;
        let cfg = self.env.cfg.clone();
        use rayon::prelude::*;
        let pos = &self.shell.pos;
        let att = &self.obs.att;
        let obs_dt = &self.obs.obs_dt;
        // Per satellite: (proton flux, dark-sky efficiency, share of the loss
        // due to sunlight rather than Earthshine).
        let per: Vec<(f64, f64, f64)> = (0..n)
            .into_par_iter()
            .map(|i| {
                let p = pos[i];
                let (lat, lon) = latlon(p, gm);
                let f = saa_flux(lat, lon, p.norm() - EARTH_RADIUS_KM, year);
                if obs_dt.get(i).copied().unwrap_or(0.0) <= 0.0 {
                    return (f, 1.0, 0.0);
                }
                let d = att[i].dir;
                let b0 = 10f64.powf(-0.4 * crate::imaging::zodi_sb(d, sun));
                let be = if cfg.earthshine { earthshine_sb(p, d, sun, limb).map(|m| 10f64.powf(-0.4 * m)).unwrap_or(0.0) } else { 0.0 };
                let bs = if o.stray_on {
                    10f64.powf(-0.4 * crate::imaging::sun_stray_sb(d, sun, o.stray_sb45, o.stray_slope))
                } else {
                    0.0
                };
                let extra = be + bs;
                if extra <= 0.0 {
                    (f, 1.0, 0.0)
                } else {
                    (f, b0 / (b0 + extra), bs / extra)
                }
            })
            .collect();
        let e = &mut self.env;
        e.in_saa.clear();
        e.in_saa.extend(per.iter().map(|x| x.0 > cfg.saa_threshold));
        e.shine.clear();
        e.shine.resize(n, 1.0);
        e.n_in_saa = 0;
        e.n_shine = 0;
        e.n_sun = 0;
        if dt <= 0.0 {
            e.n_in_saa = e.in_saa.iter().filter(|&&b| b).count();
            return;
        }
        let area_cm2 = (o.sensor_px * o.pixel_um * 1e-4) * (o.sensor_py * o.pixel_um * 1e-4);
        let fov_as2 = (o.fov_deg * 3600.0).powi(2);
        let sub = o.exp_s.min(o.sub_max_s).max(1.0);
        e.elapsed_s += dt;
        for i in 0..n {
            if self.obs.hold.get(i).copied().unwrap_or(false) {
                continue;
            }
            let (flux, eff, sun_share) = per[i];
            let in_saa = e.in_saa[i];
            e.sat_s += dt;
            e.fluence += (flux + GCR_FLUX) * dt;
            if in_saa {
                e.n_in_saa += 1;
                e.saa_s += dt;
            }
            let raw = self.obs.obs_dt[i];
            if raw <= 0.0 {
                continue;
            }
            if in_saa {
                match cfg.saa_policy {
                    SaaPolicy::Pause => {
                        e.saa_lost_s += raw;
                        self.obs.obs_dt[i] = 0.0;
                        continue;
                    }
                    SaaPolicy::Flag => e.flagged_s += raw,
                    SaaPolicy::Off => {}
                }
            }
            // Particle hits on the sensor while exposing, and the ones that
            // would pass a single-frame point-source cut.
            let rate = (GCR_FLUX + if in_saa { flux } else { 0.0 }) * area_cm2;
            let hits = rate * raw;
            let cands = hits * cfg.starlike_frac;
            e.hits += hits;
            e.candidates += cands;
            // Chance that another telescope on the same field records a
            // candidate within 2″ in the same sub-frame.
            let density = rate * cfg.starlike_frac * sub / fov_as2;
            let p = density * std::f64::consts::PI * 4.0;
            e.coincidences += cands * p;
            e.coincidences3 += cands * p * p;
            e.exposed_s += raw;
            if eff < 1.0 {
                e.shine[i] = eff as f32;
                if eff < 0.7 {
                    e.n_shine += 1;
                }
                e.earthshine_lost_s += raw * (1.0 - eff) * (1.0 - sun_share);
                e.sunlight_lost_s += raw * (1.0 - eff) * sun_share;
                if eff < 0.7 && sun_share > 0.5 {
                    e.n_sun += 1;
                }
                self.obs.obs_dt[i] = raw * eff;
            }
        }
    }

    pub fn environment_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Space environment").strong());
        let alt = self.shell.params.alt_km;
        let inc = self.shell.params.inc_deg;
        let n = self.shell.len().max(1);
        let e = &mut self.env;
        ui.label(RichText::new("South Atlantic Anomaly").small().strong());
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut e.cfg.saa_policy, SaaPolicy::Pause, "Safe detectors")
                .on_hover_text("Stop exposures inside the anomaly, as Hubble does.");
            ui.selectable_value(&mut e.cfg.saa_policy, SaaPolicy::Flag, "Keep exposing")
                .on_hover_text("Expose through it; frames are flagged and particle hits must be rejected.");
            ui.selectable_value(&mut e.cfg.saa_policy, SaaPolicy::Off, "Ignore");
        });
        ui.add(egui::Slider::new(&mut e.cfg.saa_threshold, 1.0..=200.0).logarithmic(true).text("safe-mode flux (>10 MeV p/cm²/s)"));
        ui.checkbox(&mut self.show.saa, "show anomaly in orbit view");
        let e = &self.env;
        let sat = e.sat_s.max(1e-9);
        let row = e.trade.iter().find(|r| (r.alt_km - alt).abs() < 1.0).cloned();
        crate::provenance::line(ui, "saa", format!(
            "In the anomaly now: {} of {} ({:.1} %) · orbit average at {:.0} km / {:.1}°: {}",
            e.n_in_saa,
            n,
            100.0 * e.n_in_saa as f64 / n as f64,
            alt,
            inc,
            row.as_ref().map(|r| format!("{:.1} %", 100.0 * r.saa_frac)).unwrap_or("-".into())
        ));
        crate::provenance::line(ui, "saa", format!(
            "This run: {:.1} % of satellite time inside; {:.1} % of fleet time lost to safed detectors",
            100.0 * e.saa_s / sat,
            100.0 * e.saa_lost_s / sat
        ));
        let o = &self.obs_params;
        let area = (o.sensor_px * o.pixel_um * 1e-4) * (o.sensor_py * o.pixel_um * 1e-4);
        let peak = row.as_ref().map(|r| r.peak_flux).unwrap_or(0.0);
        let px = o.sensor_px * o.sensor_py;
        let frame = o.exp_s.min(o.sub_max_s).max(1.0);
        let frac = |f: f64| 100.0 * f * area * frame * 10.0 / px;
        crate::provenance::line(ui, "saa", format!(
            "Particle hits per {:.0} s frame ({:.1} cm² sensor): ~{:.0} outside the anomaly ({:.3} % of pixels), ~{:.0} at its centre ({:.1} %)",
            frame,
            area,
            GCR_FLUX * area * frame,
            frac(GCR_FLUX),
            (GCR_FLUX + peak) * area * frame,
            frac(GCR_FLUX + peak)
        ));
        let days = (e.elapsed_s / 86_400.0).max(1e-9);
        if e.elapsed_s > 0.0 {
            crate::provenance::line(ui, "saa", format!(
                "Single-frame false transients from particle hits: {:.0}/day fleet-wide; chance matches between two telescopes: {:.0}/day, between three: {:.1e}/day",
                e.candidates / days,
                e.coincidences / days,
                e.coincidences3 / days
            ));
            crate::provenance::line(ui, "saa", format!("Proton fluence per satellite: {:.1e} p/cm²/yr", e.fluence / sat * 365.25 * 86_400.0));
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Earthshine").small().strong());
        ui.checkbox(&mut self.env.cfg.earthshine, "model sunlit-limb stray light");
        let e = &self.env;
        if e.cfg.earthshine && e.exposed_s > 0.0 {
            crate::provenance::line(ui, "earthshine", format!(
                "{} telescopes now lose > 30 % to Earthshine · {:.1} % of all exposure time lost (dark-sky equivalent)",
                e.n_shine,
                100.0 * e.earthshine_lost_s / e.exposed_s
            ));
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Sunward stray light and the Sun keep-out").small().strong());
        {
            let o = &mut self.obs_params;
            ui.checkbox(&mut o.stray_on, "model sunlight scattered past the baffle");
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("baffle:").small());
                for (name, v, tip) in [
                    ("basic tube", 19.5, "a plain light-shield tube"),
                    ("good baffle", 21.0, "vaned baffle and blackened tube (default)"),
                    ("sunshade", 22.5, "deployable sunshade beyond the baffle"),
                ] {
                    if ui.selectable_label((o.stray_sb45 - v).abs() < 0.05, name).on_hover_text(tip).clicked() {
                        o.stray_sb45 = v;
                    }
                }
            });
            ui.add(egui::Slider::new(&mut o.stray_sb45, 17.0..=24.0).text("stray light at 45° (V mag/arcsec²)"));
            ui.add(egui::Slider::new(&mut o.sun_excl_deg, 5.0..=120.0).text("Sun exclusion °"));
        }
        let e = &self.env;
        if self.obs_params.stray_on && e.exposed_s > 0.0 {
            crate::provenance::line(ui, "stray_loss", format!(
                "{} telescopes now lose > 30 % to sunlight · {:.1} % of all exposure time lost (dark-sky equivalent)",
                e.n_sun,
                100.0 * e.sunlight_lost_s / e.exposed_s
            ));
        }
        let cur = self.obs_params.sun_excl_deg;
        egui::CollapsingHeader::new(RichText::new("Keep-out trade").small()).id_source("keep_trade").default_open(true).show(ui, |ui| {
            egui::Grid::new("keepout").striped(true).show(ui, |ui| {
                for h in ["keep-out", "Venus visible", "Mercury visible", "open sky", "sky at edge", "depth vs dark sky", "of which stray light"] {
                    ui.label(RichText::new(h).small().strong());
                }
                ui.end_row();
                for r in &e.keepout {
                    let me = (r.keepout_deg - cur).abs() < 0.5;
                    let t = |s: String| if me { RichText::new(s).small().strong() } else { RichText::new(s).small() };
                    ui.label(t(format!("{:.0}°", r.keepout_deg)));
                    ui.label(t(format!("{:.0} %", 100.0 * r.venus_frac)));
                    ui.label(t(format!("{:.0} %", 100.0 * r.mercury_frac)));
                    ui.label(t(format!("{:.0} deg²", r.open_deg2)));
                    ui.label(t(format!("{:.1}", r.edge_sb)));
                    ui.label(t(format!("−{:.1} mag", r.edge_depth_loss)));
                    ui.label(t(format!("−{:.2} mag", r.edge_stray_loss)));
                    ui.end_row();
                }
            });
            ui.label(RichText::new("Venus and Mercury: share of days over the next three years outside the keep-out (this ephemeris). Edge figures: on the ecliptic, at the keep-out angle, zodiacal light plus stray light.").small().weak());
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Orbital lifetime and disposal").small().strong());
        let c = &mut self.env.cfg;
        ui.add(egui::Slider::new(&mut c.mass_kg, 20.0..=500.0).text("mass kg"));
        ui.add(egui::Slider::new(&mut c.area_m2, 0.1..=5.0).text("mean drag area m²"));
        ui.add(egui::Slider::new(&mut c.disposal_perigee_km, 150.0..=350.0).text("disposal perigee km"));
        let e = &self.env;
        if let Some((m, lo, hi, cy)) = e.life {
            crate::provenance::line(ui, "lifetime", format!(
                "A dead satellite at {:.0} km re-enters in {} (mean sun; {} at solar max, {} at solar min; {} following the cycle from now)",
                alt,
                fmt_years(m),
                fmt_years(lo),
                fmt_years(hi),
                fmt_years(cy)
            ));
            let ok = cy <= 5.0;
            ui.label(
                RichText::new(if ok {
                    "Meets a 5-year post-mission disposal rule passively: a failed satellite cleans itself up.".to_string()
                } else {
                    format!(
                        "Does not meet a 5-year rule passively: needs a disposal burn of {:.0} m/s to a {:.0} km perigee while still working.",
                        row.as_ref().map(|r| r.disposal_dv).unwrap_or(0.0),
                        e.cfg.disposal_perigee_km
                    )
                })
                .small()
                .color(if ok { egui::Color32::from_rgb(120, 220, 140) } else { egui::Color32::from_rgb(240, 180, 90) }),
            );
            ui.label(RichText::new(format!("Ballistic coefficient C_D·A/m = {:.4} m²/kg", e.cfg.ballistic())).small().weak());
        }
        egui::CollapsingHeader::new(RichText::new("Altitude trade").small()).id_source("env_trade").show(ui, |ui| {
            egui::Grid::new("trade").striped(true).show(ui, |ui| {
                for h in ["alt km", "in SAA", "peak flux", "life (mean)", "solar max–min", "disposal Δv"] {
                    ui.label(RichText::new(h).small().strong());
                }
                ui.end_row();
                for r in &e.trade {
                    let me = (r.alt_km - alt).abs() < 1.0;
                    let t = |s: String| if me { RichText::new(s).small().strong() } else { RichText::new(s).small() };
                    ui.label(t(format!("{:.0}", r.alt_km)));
                    ui.label(t(format!("{:.1} %", 100.0 * r.saa_frac)));
                    ui.label(t(format!("{:.0}", r.peak_flux)));
                    ui.label(t(fmt_years(r.life_mean_yr)));
                    ui.label(t(format!("{} – {}", fmt_years(r.life_min_yr), fmt_years(r.life_max_yr))));
                    ui.label(t(format!("{:.0} m/s", r.disposal_dv)));
                    ui.end_row();
                }
            });
            ui.label(RichText::new("Higher shells: longer-lived and less drag, but more time in the anomaly and slower natural clean-up.").small().weak());
        });
    }
}
