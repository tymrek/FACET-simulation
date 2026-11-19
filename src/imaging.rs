//! Simulated observations: what a request would actually return, given the
//! hardware, the integration achieved, and the sky.
//!
//! Model (all SOFT ESTIMATES — every input is a labelled assumption):
//!   * Optics: circular aperture D with central obstruction ε; PSF = Gaussian
//!     with FWHM = √(diffraction² + pointing-jitter²); diffraction FWHM ≈ 1.03 λ/D.
//!     Sub-second sub-frames are assumed registered ("lucky imaging"), which
//!     freezes most of the jitter.
//!   * Detector: IMX455-class back-illuminated CMOS (3.76 µm pixels, 9576 px
//!     across, QE, read noise, dark current, full well). Pixel scale follows from
//!     the field of view; focal length / f-ratio are derived.
//!   * Photometry: broad clear band, V-referenced zero point
//!     (≈3×10⁶ photons s⁻¹ cm⁻² for V = 0 over 400–700 nm) × throughput × QE.
//!   * Sky: zodiacal light only (no atmosphere), interpolated from the Leinert
//!     et al. (1998) shape vs solar elongation and ecliptic latitude.
//!   * Stacking: N telescopes × t seconds are combined incoherently — signal and
//!     variance add, so depth grows with total telescope-seconds; resolution
//!     stays that of ONE aperture.
//!   * Scene: real catalogue stars to V 6.5, plus a SYNTHETIC statistical
//!     population of fainter stars (latitude-dependent counts) and galaxies,
//!     plus planets/Moon rendered with correct size, phase, pole orientation
//!     and Saturn's ring tilt, but PROCEDURAL surface markings.
//!   * Noise: Gaussian approximation to Poisson + read noise per sub-frame.

use crate::astro::{self, AU_KM, BODY_NAMES};
use crate::catalog::{self, Star};
use crate::math::{deg, rad, V3};
use crate::obs::{tangent_basis, ObsParams};
use serde::Serialize;
use std::f64::consts::PI;
use std::sync::OnceLock;

const ARCSEC: f64 = 206_264.806;

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
pub struct Hardware {
    pub aperture_cm: f64,
    pub obstruction: f64,
    pub fov_deg: f64,
    pub sensor_px: f64,
    pub pixel_um: f64,
    pub qe: f64,
    pub throughput: f64,
    pub read_noise_e: f64,
    pub dark_e_s: f64,
    pub full_well_e: f64,
    pub jitter_arcsec: f64,
    pub sub_max_s: f64,
    pub wavelength_nm: f64,
    /// Photons s⁻¹ cm⁻² in the band for a V = 0 source.
    pub ph0: f64,
    pub wfe_nm: f64,
    pub spider_vanes: u32,
    pub spider_mm: f64,
}

impl Hardware {
    pub fn from_obs(o: &ObsParams) -> Hardware {
        Hardware {
            aperture_cm: o.aperture_cm,
            obstruction: o.obstruction,
            fov_deg: o.fov_deg,
            sensor_px: o.sensor_px,
            pixel_um: o.pixel_um,
            qe: o.qe,
            throughput: o.throughput,
            read_noise_e: o.read_noise_e,
            dark_e_s: o.dark_e_s,
            full_well_e: o.full_well_e,
            jitter_arcsec: o.jitter_arcsec,
            sub_max_s: o.sub_max_s,
            wavelength_nm: o.wavelength_nm,
            ph0: 3.0e6,
            wfe_nm: o.wfe_nm,
            spider_vanes: o.spider_vanes,
            spider_mm: o.spider_mm,
        }
    }
    pub fn area_cm2(&self) -> f64 {
        PI * (self.aperture_cm / 2.0).powi(2) * (1.0 - self.obstruction.powi(2))
    }
    /// Detected electrons per second from a V = 0 point source.
    pub fn zp_e_s(&self) -> f64 {
        self.ph0 * self.area_cm2() * self.throughput * self.qe
    }
    pub fn pixel_scale(&self) -> f64 {
        self.fov_deg * 3600.0 / self.sensor_px
    }
    pub fn focal_mm(&self) -> f64 {
        self.pixel_um * 1e-3 * ARCSEC / self.pixel_scale()
    }
    pub fn f_ratio(&self) -> f64 {
        self.focal_mm() / (self.aperture_cm * 10.0)
    }
    pub fn diffraction_fwhm(&self) -> f64 {
        1.03 * self.wavelength_nm * 1e-9 / (self.aperture_cm / 100.0) * ARCSEC
    }
    /// Pointing jitter (RMS, arcsec) that applies for a given sub-frame
    /// length: short registered frames freeze most of it.
    pub fn jitter_for(&self, sub_s: f64) -> f64 {
        if sub_s < 1.0 {
            self.jitter_arcsec.min(0.25)
        } else {
            self.jitter_arcsec
        }
    }
    /// The full Fourier-optics PSF (incl. jitter) for a sub-frame length.
    pub fn psf(&self, sub_s: f64) -> std::sync::Arc<crate::optics::Psf> {
        crate::optics::psf(self, self.jitter_for(sub_s))
    }
    /// Delivered FWHM (arcsec), measured from the Fourier-optics PSF.
    pub fn fwhm(&self, sub_s: f64) -> f64 {
        self.psf(sub_s).fwhm
    }
    /// Sky + dark electrons s⁻¹ per native pixel.
    pub fn bg_e_s(&self, sky_sb: f64) -> f64 {
        self.zp_e_s() * 10f64.powf(-0.4 * sky_sb) * self.pixel_scale().powi(2) + self.dark_e_s
    }
}

/// 5σ point-source limiting magnitude for `total_s` telescope-seconds split in
/// sub-frames of `sub_s`, against sky surface brightness `sky_sb` (mag/arcsec²).
pub fn point_lim_mag(hw: &Hardware, total_s: f64, sub_s: f64, sky_sb: f64) -> f64 {
    if total_s <= 0.0 {
        return f64::NAN;
    }
    let ps = hw.pixel_scale();
    let psf = hw.psf(sub_s);
    // Photometric aperture: the EE50 radius scaled up a little (≈ optimal for
    // background-limited point sources), at least one pixel; encircled energy
    // measured from the real PSF.
    let r_as = (1.2 * psf.ee50_r).max(0.6 * ps);
    let ee = psf.ee_at(r_as);
    let r = r_as / ps;
    let npix = (PI * r * r).max(1.0);
    let nframes = (total_s / sub_s.max(1e-3)).max(1.0);
    let b = npix * (hw.bg_e_s(sky_sb) * total_s + nframes * hw.read_noise_e.powi(2));
    let a = ee * total_s;
    let s_tot = (25.0 + (625.0 + 100.0 * b).sqrt()) / 2.0; // electrons in aperture at SNR 5
    let rate = s_tot / a;
    -2.5 * (rate / hw.zp_e_s()).log10()
}

/// 1σ surface-brightness limit (mag/arcsec²) over a 1″×1″ box.
pub fn sb_lim(hw: &Hardware, total_s: f64, sub_s: f64, sky_sb: f64) -> f64 {
    let ps = hw.pixel_scale();
    let npix = (1.0 / (ps * ps)).max(1.0);
    let nframes = (total_s / sub_s.max(1e-3)).max(1.0);
    let sigma = (npix * (hw.bg_e_s(sky_sb) * total_s + nframes * hw.read_noise_e.powi(2))).sqrt();
    let rate_per_arcsec2 = sigma / total_s / (npix * ps * ps);
    -2.5 * (rate_per_arcsec2 / hw.zp_e_s()).log10()
}

/// Zodiacal-light surface brightness (V mag/arcsec²) toward `d`.
/// Sunlight scattered off the baffle and tube into the field, V mag/arcsec²
/// (ASSUMED parametric point-source-transmittance law: brightness falls as
/// elongation^-3, i.e. 7.5 mag per decade, from `sb45` at 45°, then tapers
/// to nothing between 60° and 100° as the Sun drops behind the aperture
/// plane and the sunshade and can no longer light the baffle interior).
pub fn sun_stray_sb(d: V3, sun: V3, sb45: f64, slope: f64) -> f64 {
    let e = deg(d.angle(sun)).max(5.0);
    let taper = ((100.0 - e) / 40.0).clamp(0.0, 1.0).powi(2);
    if taper <= 1e-6 {
        return 40.0;
    }
    sb45 + slope * (e / 45.0).log10() - 2.5 * taper.log10()
}

/// Sum of two surface brightnesses (mag/arcsec²).
pub fn add_sb(a: f64, b: f64) -> f64 {
    -2.5 * (10f64.powf(-0.4 * a) + 10f64.powf(-0.4 * b)).log10()
}

/// Sky background a telescope actually sees: zodiacal light plus sunward
/// stray light (Earthshine is applied separately, per satellite).
pub fn sky_sb(d: V3, sun: V3, o: &crate::obs::ObsParams) -> f64 {
    let z = zodi_sb(d, sun);
    if o.stray_on {
        add_sb(z, sun_stray_sb(d, sun, o.stray_sb45, o.stray_slope))
    } else {
        z
    }
}

pub fn zodi_sb(d: V3, sun: V3) -> f64 {
    const ECL: [(f64, f64); 11] = [
        (10.0, 15.8),
        (15.0, 16.9),
        (20.0, 17.5),
        (30.0, 18.7),
        (45.0, 19.8),
        (60.0, 20.5),
        (75.0, 20.9),
        (90.0, 21.2),
        (120.0, 21.6),
        (150.0, 21.7),
        (180.0, 21.4),
    ];
    const POLE: f64 = 22.3;
    let elong = deg(d.angle(sun)).clamp(10.0, 180.0);
    let mut sb = ECL[ECL.len() - 1].1;
    for w in ECL.windows(2) {
        if elong <= w[1].0 {
            let t = (elong - w[0].0) / (w[1].0 - w[0].0);
            sb = w[0].1 + t * (w[1].1 - w[0].1);
            break;
        }
    }
    let ecl_pole = V3::Z.rot_x(rad(astro::OBLIQUITY_J2000_DEG));
    let cb = (1.0 - d.dot(ecl_pole).powi(2)).max(0.0); // cos² β
    let f_pole = 10f64.powf(-0.4 * POLE);
    let f_ecl = 10f64.powf(-0.4 * sb);
    // Near the Sun the glow is bright at every latitude; elsewhere it fades to the pole value.
    let near_sun = ((60.0 - elong) / 50.0).clamp(0.0, 1.0);
    let w = cb + (1.0 - cb) * near_sun;
    -2.5 * (f_pole + (f_ecl - f_pole).max(0.0) * w).log10()
}

/// Is there a usable guide star (V ≤ guide_mag) in a square field of side
/// `fov_deg` centred on `d`? Poisson draw from the star-count model, seeded by
/// sky position so the answer is stable for a given field.
pub fn guide_star_available(d: V3, fov_deg: f64, guide_mag: f64) -> bool {
    let expect = star_counts(guide_mag, galactic_lat_deg(d)) * fov_deg * fov_deg;
    let p = 1.0 - (-expect).exp();
    if p > 0.9999 {
        return true;
    }
    let (ra, dec) = d.to_radec();
    let cell = ((deg(ra) / fov_deg.max(0.05)).floor() as i64, (deg(dec) / fov_deg.max(0.05)).floor() as i64);
    hash3(cell.0, cell.1, 17, 0x6A1D) < p
}

fn galactic_lat_deg(d: V3) -> f64 {
    let ngp = V3::from_radec(rad(192.859_48), rad(27.128_25));
    deg(d.dot(ngp).asin())
}

// ---------------------------------------------------------------- randomness

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn gauss(&mut self) -> f64 {
        let u1 = self.f().max(1e-300);
        let u2 = self.f();
        (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
    }
}

fn hash3(x: i64, y: i64, z: i64, seed: u64) -> f64 {
    let mut r = Rng(seed ^ (x as u64).wrapping_mul(0x9E37_79B9) ^ (y as u64).wrapping_mul(0x85EB_CA6B) ^ (z as u64).wrapping_mul(0xC2B2_AE35));
    r.next();
    r.f()
}

/// Smooth 3-D value noise in [0, 1].
fn value_noise(p: V3, seed: u64) -> f64 {
    let (fx, fy, fz) = (p.x.floor(), p.y.floor(), p.z.floor());
    let (tx, ty, tz) = (p.x - fx, p.y - fy, p.z - fz);
    let s = |t: f64| t * t * (3.0 - 2.0 * t);
    let (sx, sy, sz) = (s(tx), s(ty), s(tz));
    let (ix, iy, iz) = (fx as i64, fy as i64, fz as i64);
    let c = |dx, dy, dz| hash3(ix + dx, iy + dy, iz + dz, seed);
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    lerp(
        lerp(lerp(c(0, 0, 0), c(1, 0, 0), sx), lerp(c(0, 1, 0), c(1, 1, 0), sx), sy),
        lerp(lerp(c(0, 0, 1), c(1, 0, 1), sx), lerp(c(0, 1, 1), c(1, 1, 1), sx), sy),
        sz,
    )
}

fn fbm(p: V3, seed: u64, octaves: u32) -> f64 {
    let (mut a, mut f, mut sum, mut norm) = (1.0, 1.0, 0.0, 0.0);
    for o in 0..octaves {
        sum += a * value_noise(p * f, seed + o as u64);
        norm += a;
        a *= 0.5;
        f *= 2.03;
    }
    sum / norm
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ------------------------------------------------------------------- images

struct Frame {
    w: usize,
    h: usize,
    /// arcsec per output pixel
    scale: f64,
    /// native pixels per output pixel (area)
    bin2: f64,
    /// electrons per second per output pixel
    rate: Vec<f64>,
    center: V3,
    e: V3,
    n: V3,
}

impl Frame {
    fn new(w: usize, h: usize, scale: f64, native: f64, center: V3) -> Frame {
        let (e, n) = tangent_basis(center);
        Frame { w, h, scale, bin2: (scale / native).powi(2), rate: vec![0.0; w * h], center, e, n }
    }
    /// Sky direction → pixel (x right, y down; north up, east left).
    fn project(&self, d: V3) -> Option<(f64, f64)> {
        let z = d.dot(self.center);
        if z <= 0.0 {
            return None;
        }
        let xi = d.dot(self.e) / z * ARCSEC / self.scale;
        let eta = d.dot(self.n) / z * ARCSEC / self.scale;
        Some((self.w as f64 / 2.0 - xi, self.h as f64 / 2.0 - eta))
    }
    /// Add an ideal point source of total `flux` e⁻/s, split bilinearly
    /// between the four nearest pixels (sub-pixel position preserved). The
    /// optics are applied afterwards by convolving the whole scene with the
    /// Fourier-optics PSF. (`_sigma` is unused; kept for call-site clarity.)
    fn add_psf(&mut self, x: f64, y: f64, flux: f64, _sigma: f64) {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        for (dx, dy, w) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
            let px = x0 as i64 + dx;
            let py = y0 as i64 + dy;
            if px >= 0 && py >= 0 && (px as usize) < self.w && (py as usize) < self.h {
                self.rate[py as usize * self.w + px as usize] += flux * w;
            }
        }
    }
    /// Exponential-disc galaxy (scale length `rs` arcsec, axis ratio q, angle pa).
    fn add_galaxy(&mut self, x: f64, y: f64, flux: f64, rs: f64, q: f64, pa: f64, sigma: f64) {
        let rs_px = rs / self.scale;
        if rs_px < 0.7 {
            // Unresolved at this scale: a point (the PSF is applied later).
            return self.add_psf(x, y, flux, sigma);
        }
        let r = (6.0 * rs_px).ceil() as i64;
        let (c, s) = (pa.cos(), pa.sin());
        let mut stamp = Vec::with_capacity(((2 * r + 1) * (2 * r + 1)) as usize);
        let mut sum = 0.0;
        for j in -r..=r {
            for i in -r..=r {
                let dx = (x.floor() + i as f64 + 0.5) - x;
                let dy = (y.floor() + j as f64 + 0.5) - y;
                let u = dx * c + dy * s;
                let v = (-dx * s + dy * c) / q;
                let v_ = (-(u * u + v * v).sqrt() / rs_px).exp();
                stamp.push(v_);
                sum += v_;
            }
        }
        if sum <= 0.0 {
            return;
        }
        let (cx, cy) = (x.floor() as i64, y.floor() as i64);
        let mut k = 0;
        for j in -r..=r {
            for i in -r..=r {
                let (px, py) = (cx + i, cy + j);
                if px >= 0 && py >= 0 && (px as usize) < self.w && (py as usize) < self.h {
                    self.rate[py as usize * self.w + px as usize] += flux * stamp[k] / sum;
                }
                k += 1;
            }
        }
    }
}

// ------------------------------------------------------------------ planets

/// IAU north-pole directions (J2000 RA, Dec), body index as astro::bodies.
const POLES: [(f64, f64); 9] = [
    (286.13, 63.87),
    (266.86, 65.64),
    (281.01, 61.45),
    (272.76, 67.16),
    (317.68, 52.89),
    (268.06, 64.50),
    (40.59, 83.54),
    (257.31, -15.18),
    (299.36, 43.46),
];

/// Apparent V magnitude and phase angle (deg) of a body. Simplified Mallama/Hilton fits.
pub fn body_mag(i: usize, b: &[astro::Body; 9]) -> (f64, f64) {
    let sun = b[0].dir * b[0].dist_km;
    let p = b[i].dir * b[i].dist_km;
    let r = (p - sun).norm() / AU_KM;
    let delta = b[i].dist_km / AU_KM;
    let alpha = deg((sun - p).angle(-p));
    let m = match i {
        1 => -12.73 + 0.026 * alpha + 4e-9 * alpha.powi(4) + 5.0 * (b[1].dist_km / 384_400.0).log10(),
        2 => -0.60 + 0.038 * alpha - 0.000_273 * alpha.powi(2) + 2e-6 * alpha.powi(3) + 5.0 * (r * delta).log10(),
        3 => -4.40 + 0.000_9 * alpha + 0.000_239 * alpha.powi(2) - 6.5e-7 * alpha.powi(3) + 5.0 * (r * delta).log10(),
        4 => -1.52 + 0.016 * alpha + 5.0 * (r * delta).log10(),
        5 => -9.40 + 0.005 * alpha + 5.0 * (r * delta).log10(),
        6 => -8.88 + 0.044 * alpha + 5.0 * (r * delta).log10(),
        7 => -7.19 + 5.0 * (r * delta).log10(),
        _ => -6.87 + 5.0 * (r * delta).log10(),
    };
    (m, alpha)
}

struct PlanetView {
    i: usize,
    /// Tangent-plane basis at the planet (east, north) and toward-observer.
    e: V3,
    n: V3,
    w: V3,
    pole: V3,
    q1: V3,
    q2: V3,
    sun: V3,
    k: f64,
    seed: u64,
    rings: bool,
}

impl PlanetView {
    fn new(i: usize, b: &[astro::Body; 9]) -> PlanetView {
        let d = b[i].dir;
        let (e, n) = tangent_basis(d);
        let pole = V3::from_radec(rad(POLES[i].0), rad(POLES[i].1));
        let q1 = pole.any_perp();
        let q2 = pole.cross(q1);
        let sunpos = b[0].dir * b[0].dist_km;
        let ppos = d * b[i].dist_km;
        PlanetView {
            i,
            e,
            n,
            w: -d,
            pole,
            q1,
            q2,
            sun: (sunpos - ppos).unit(),
            k: match i {
                1 | 2 => 0.55,
                3 => 0.9,
                _ => 0.8,
            },
            seed: 1000 + i as u64,
            rings: i == 6,
        }
    }

    fn albedo(&self, nrm: V3) -> f64 {
        let lat = deg(nrm.dot(self.pole).clamp(-1.0, 1.0).asin());
        let lon = deg(nrm.dot(self.q2).atan2(nrm.dot(self.q1)));
        let body = V3::new(nrm.dot(self.q1), nrm.dot(self.q2), nrm.dot(self.pole));
        let a = lat.abs();
        match self.i {
            1 => {
                // Moon: low-frequency maria + cratered texture
                let maria = smoothstep(0.52, 0.62, fbm(body * 2.2, self.seed, 4));
                let craters = fbm(body * 18.0, self.seed + 50, 3);
                (0.95 - 0.38 * maria) * (0.85 + 0.3 * craters)
            }
            2 => 0.75 + 0.35 * fbm(body * 14.0, self.seed, 4),
            3 => 1.0 + 0.05 * (fbm(body * 3.0, self.seed, 3) - 0.5),
            4 => {
                let cap = smoothstep(72.0, 80.0, a);
                (0.55 + 0.4 * fbm(body * 3.5, self.seed, 5)) * (1.0 - cap) + 1.4 * cap
            }
            5 => {
                let belt = |lo: f64, hi: f64, depth: f64| depth * smoothstep(lo - 1.5, lo + 1.5, a) * (1.0 - smoothstep(hi - 1.5, hi + 1.5, a));
                let mut v = 1.0 - belt(7.0, 18.0, 0.28) - belt(24.0, 31.0, 0.15) - 0.25 * smoothstep(50.0, 75.0, a);
                v *= 0.95 + 0.1 * fbm(V3::new(lat * 0.6, lon * 0.05, 0.0), self.seed, 4);
                // Great Red Spot (darker in a clear band)
                let dl = (lat + 22.0) / 5.0;
                let dn = ((lon - 40.0 + 540.0) % 360.0 - 180.0) / 9.0;
                v - 0.18 * (-(dl * dl + dn * dn)).exp()
            }
            6 => 0.95 - 0.09 * smoothstep(15.0, 20.0, a) * (1.0 - smoothstep(32.0, 38.0, a)) - 0.15 * smoothstep(65.0, 80.0, a),
            _ => 1.0,
        }
    }

    /// Unnormalised brightness at tangent-plane offset (u east, v north) in planet radii.
    fn sample(&self, u: f64, v: f64) -> f64 {
        let rr = u * u + v * v;
        let mut out = 0.0;
        let zs = if rr < 1.0 { (1.0 - rr).sqrt() } else { f64::NEG_INFINITY };
        let mut ring_val = 0.0;
        let mut ring_t = f64::NEG_INFINITY;
        if self.rings {
            let wp = self.w.dot(self.pole);
            if wp.abs() > 0.004 {
                let t = -(u * self.e.dot(self.pole) + v * self.n.dot(self.pole)) / wp;
                let p = self.e * u + self.n * v + self.w * t;
                let r = p.norm();
                let opacity = if (1.24..1.53).contains(&r) {
                    0.25
                } else if (1.53..1.95).contains(&r) {
                    0.95
                } else if (2.03..2.27).contains(&r) {
                    0.6
                } else {
                    0.0
                };
                if opacity > 0.0 {
                    // Lit face brightness rises with the Sun's elevation over the rings.
                    let lit = self.sun.dot(self.pole).abs().powf(0.3);
                    // In the planet's shadow?
                    let along = p.dot(self.sun);
                    let shadow = along < 0.0 && (p - self.sun * along).norm() < 1.0;
                    ring_val = if shadow { 0.0 } else { 0.9 * opacity * lit };
                    ring_t = t;
                }
            }
        }
        if rr < 1.0 {
            let nrm = (self.e * u + self.n * v + self.w * zs).unit();
            let ci = nrm.dot(self.sun);
            let ce = zs;
            if ci > 0.0 {
                out = self.albedo(nrm) * ci.powf(self.k) * ce.powf(self.k - 1.0);
                // Ring shadow on the globe
                if self.rings {
                    let sp = self.sun.dot(self.pole);
                    if sp.abs() > 1e-3 {
                        let t = -nrm.dot(self.pole) / sp;
                        if t > 0.0 {
                            let hit = (nrm + self.sun * t).norm();
                            if (1.53..1.95).contains(&hit) {
                                out *= 0.15;
                            } else if (1.24..2.27).contains(&hit) && !(1.95..2.03).contains(&hit) {
                                out *= 0.5;
                            }
                        }
                    }
                }
            }
            if ring_t > zs {
                out = out * (1.0 - 0.9) + ring_val;
            }
        } else if ring_val > 0.0 {
            out = ring_val;
        }
        out
    }
}

/// Disc normalisation: (e⁻/s per (planet radius)² per unit brightness, max brightness).
fn planet_norm(pv: &PlanetView, flux: f64) -> Option<(f64, f64)> {
    let ext = if pv.rings { 2.3 } else { 1.0 };
    let ng = 256;
    let mut sum = 0.0;
    let mut maxb: f64 = 0.0;
    for j in 0..ng {
        for k in 0..ng {
            let u = ((k as f64 + 0.5) / ng as f64 * 2.0 - 1.0) * ext;
            let v = ((j as f64 + 0.5) / ng as f64 * 2.0 - 1.0) * ext;
            let b = pv.sample(u, v);
            sum += b;
            maxb = maxb.max(b);
        }
    }
    let cell_area = (2.0 * ext / ng as f64).powi(2);
    if sum <= 0.0 {
        None
    } else {
        Some((flux / (sum * cell_area), maxb))
    }
}

// ------------------------------------------------------------------ request

#[derive(Clone, Debug)]
pub struct Spec {
    pub id: u64,
    pub target_name: String,
    pub dir: V3,
    pub body: Option<usize>,
    pub jd: f64,
    pub total_s: f64,
    pub required_s: f64,
    pub telescopes: u32,
    pub exposure_s: f64,
    pub hw: Hardware,
    pub done: bool,
    /// Fraction of integration taken under fine guidance.
    pub fine_frac: f64,
    /// Approval → first locked photon, seconds.
    pub response_s: Option<f64>,
    /// Approval → all requested telescopes locked, seconds.
    pub full_lock_s: Option<f64>,
    pub slews: u32,
    pub mean_slew_deg: f64,
    pub agility: Option<crate::attitude::Agility>,
}

pub struct Output {
    pub report: serde_json::Value,
    pub full_png: Vec<u8>,
    pub crop_png: Vec<u8>,
    /// The effective PSF (log stretch), for the report.
    pub psf_png: Vec<u8>,
}

static STARS: OnceLock<Vec<Star>> = OnceLock::new();
static NAMED: OnceLock<Vec<catalog::NamedStar>> = OnceLock::new();

/// Cumulative star counts per deg² brighter than V at galactic latitude b.
fn star_counts(v: f64, b_deg: f64) -> f64 {
    let pole = 10f64.powf(0.32 * (v - 20.0) + 2.9);
    let f = (1.0 / rad(b_deg.abs().max(2.0)).sin()).powf(1.5).min(30.0);
    pole * f
}
fn galaxy_counts(v: f64) -> f64 {
    10f64.powf(0.45 * (v - 20.0) + 2.6)
}

fn encode_png(w: usize, h: usize, px: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        if let Ok(mut wr) = enc.write_header() {
            let _ = wr.write_image_data(px);
        }
    }
    out
}

struct Exposed {
    img: Vec<f64>,
    sigma_bg: f64,
    saturated: usize,
}

/// Turn a rate map into a stacked, sky-subtracted electron image with noise.
fn expose(f: &Frame, hw: &Hardware, total_s: f64, sub_s: f64, bg_native: f64, rng: &mut Rng) -> Exposed {
    let nframes = (total_s / sub_s).max(1.0);
    let bg = bg_native * f.bin2;
    let rn2 = hw.read_noise_e.powi(2) * nframes * f.bin2;
    let sat_rate = hw.full_well_e / sub_s * f.bin2.max(1.0);
    let mut saturated = 0;
    let img = f
        .rate
        .iter()
        .map(|&r| {
            let mut s = r * total_s;
            if r > sat_rate {
                s = sat_rate * total_s;
                saturated += 1;
            }
            let var = s + bg * total_s + rn2;
            s + var.sqrt() * rng.gauss()
        })
        .collect();
    Exposed { img, sigma_bg: (bg * total_s + rn2).sqrt(), saturated }
}

/// asinh stretch to 8-bit.
fn stretch(img: &[f64], sigma: f64, planet: bool) -> Vec<u8> {
    let mut sorted: Vec<f64> = img.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pct = |q: f64| sorted[((sorted.len() - 1) as f64 * q) as usize];
    let (lo, soft, hi) = if planet {
        // Near-linear: keep albedo contrast on bright discs.
        let hi = pct(0.9999).max(10.0 * sigma);
        (-sigma, hi * 0.6, hi)
    } else {
        let hi = pct(0.9995).max(40.0 * sigma);
        (-1.5 * sigma, 2.5 * sigma, hi)
    };
    let norm = ((hi - lo) / soft).asinh();
    img.iter()
        .map(|&v| ((((v - lo).max(0.0) / soft).asinh() / norm).clamp(0.0, 1.0) * 255.0) as u8)
        .collect()
}

pub fn render(spec: &Spec) -> Output {
    let hw = &spec.hw;
    let stars = STARS.get_or_init(catalog::load_stars);
    let named = NAMED.get_or_init(catalog::load_names);
    let bodies = astro::bodies(spec.jd);
    let sun = bodies[0].dir;
    let center = match spec.body {
        Some(i) => bodies[i].dir,
        None => spec.dir,
    };
    let sky_sb = zodi_sb(center, sun);
    let ps = hw.pixel_scale();
    let zp = hw.zp_e_s();
    let bg_native = hw.bg_e_s(sky_sb);
    let gal_b = galactic_lat_deg(center);
    let mut notes: Vec<String> = Vec::new();

    // ---- Sub-frame length: avoid saturating the brightest thing in the field.
    let mut peak_rate_native: f64 = 0.0; // e-/s in the brightest native pixel
    let fwhm_long = hw.fwhm(hw.sub_max_s.min(spec.exposure_s));
    let sig_px = |fwhm: f64| (fwhm / 2.355 / ps).max(0.3);
    let half_fov = rad(hw.fov_deg / 2.0 * 1.42);
    let in_fov_bodies: Vec<usize> = (1..9).filter(|&i| bodies[i].dir.angle(center) < half_fov).collect();
    for &i in &in_fov_bodies {
        let (m, _) = body_mag(i, &bodies);
        let ang_r = (bodies[i].radius_km / bodies[i].dist_km).asin() * ARCSEC;
        let flux = zp * 10f64.powf(-0.4 * m);
        let disk_px = PI * (ang_r / ps).powi(2);
        let peak = if disk_px > 4.0 {
            // Peak surface brightness from the actual shaded disc.
            match planet_norm(&PlanetView::new(i, &bodies), flux) {
                Some((per_area, maxb)) => per_area * maxb * (ps / ang_r).powi(2),
                None => 0.0,
            }
        } else {
            flux / (2.0 * PI * sig_px(fwhm_long).powi(2))
        };
        peak_rate_native = peak_rate_native.max(peak);
    }
    if spec.body.is_none() {
        // Brightest catalogue star in the field (catalogue is magnitude-sorted).
        // Deep stacks accept saturated bright stars, as surveys do.
        if let Some(s) = stars.iter().take_while(|s| s.mag < 6.5).find(|s| s.dir.angle(center) < half_fov) {
            notes.push(format!("Brightest catalogue star in field: V = {:.1} (saturated core).", s.mag));
        }
    }
    let mut sub_s = hw.sub_max_s.min(spec.exposure_s).max(0.001);
    if spec.body.is_some() && peak_rate_native > 0.0 {
        let t_sat = 0.7 * hw.full_well_e / peak_rate_native;
        if t_sat < sub_s {
            sub_s = (t_sat * 1e4).floor().max(1.0) / 1e4;
            notes.push(format!(
                "Bright target: sub-frames shortened to {} to stay below 70 % of full well; frames are registered and stacked (lucky-imaging style).",
                fmt_s(sub_s)
            ));
        }
    }
    let fwhm = hw.fwhm(sub_s);
    let total_s = spec.total_s.max(1e-6);
    let nframes = total_s / sub_s;
    let lim = point_lim_mag(hw, total_s, sub_s, sky_sb);
    let sbl = sb_lim(hw, total_s, sub_s, sky_sb);

    // ---- Frames: full field (binned) and native-resolution centre crop.
    let full_w = 720usize;
    let full_scale = hw.fov_deg * 3600.0 / full_w as f64;
    let mut full = Frame::new(full_w, full_w, full_scale, ps, center);
    let crop_w = 512usize;
    let mut crop_scale = ps;
    let mut planet_crop = false;
    if let Some(i) = spec.body {
        // Frame the planet: native scale, or coarser if the disc doesn't fit.
        let ang_r = (bodies[i].radius_km / bodies[i].dist_km).asin() * ARCSEC;
        // Fill ~85 % of the crop; allow up to 3× resampling below the native
        // pixel so the detail the PSF does deliver is visible.
        let need = ang_r * (if i == 6 { 2.4 } else { 1.18 }) * 2.0 / crop_w as f64;
        crop_scale = (ps / 3.0).max(need);
        planet_crop = true;
    }
    let mut crop = Frame::new(crop_w, crop_w, crop_scale, ps, center);

    // ---- Seeded RNG: same patch of sky → same synthetic field.
    let (ra, dec) = center.to_radec();
    let cell = ((deg(ra) * 4.0).round() as i64, (deg(dec) * 4.0).round() as i64);
    let field_seed = 0x5EED ^ (cell.0 as u64).wrapping_mul(1_000_003) ^ (cell.1 as u64).wrapping_mul(7_919);
    let mut rng = Rng(field_seed);

    let mut detected = 0usize;
    let area_deg2 = hw.fov_deg * hw.fov_deg;

    // Real catalogue stars
    let hfov_rad = rad(hw.fov_deg / 2.0);
    for s in stars.iter() {
        let ang = s.dir.angle(center);
        if ang > hfov_rad * 1.5 {
            continue;
        }
        let flux = zp * 10f64.powf(-0.4 * s.mag as f64);
        if let Some((x, y)) = full.project(s.dir) {
            if x >= 0.0 && y >= 0.0 && x < full.w as f64 && y < full.h as f64 {
                detected += 1;
            }
            full.add_psf(x, y, flux, sig_px(fwhm) * ps / full.scale);
        }
        if let Some((x, y)) = crop.project(s.dir) {
            crop.add_psf(x, y, flux, (fwhm / 2.355 / crop.scale).max(0.3));
        }
    }

    // Synthetic faint stars
    let v_min = 6.5;
    let mut v_max = (lim + 1.0).clamp(8.0, 27.0);
    while star_counts(v_max, gal_b) * area_deg2 > 300_000.0 && v_max > v_min + 1.0 {
        v_max -= 0.25;
    }
    let n_stars = ((star_counts(v_max, gal_b) - star_counts(v_min, gal_b)).max(0.0) * area_deg2) as usize;
    let (a0, a1) = (10f64.powf(0.32 * v_min), 10f64.powf(0.32 * v_max));
    let half = hw.fov_deg / 2.0;
    for _ in 0..n_stars {
        let v = (a0 + rng.f() * (a1 - a0)).log10() / 0.32;
        let xi = rad((rng.f() * 2.0 - 1.0) * half);
        let eta = rad((rng.f() * 2.0 - 1.0) * half);
        let d = (center + full.e * xi + full.n * eta).unit();
        let flux = zp * 10f64.powf(-0.4 * v);
        if v < lim {
            detected += 1;
        }
        if let Some((x, y)) = full.project(d) {
            full.add_psf(x, y, flux, sig_px(fwhm) * ps / full.scale);
        }
        if let Some((x, y)) = crop.project(d) {
            if x > -2.0 && y > -2.0 && x < crop.w as f64 + 2.0 && y < crop.h as f64 + 2.0 {
                crop.add_psf(x, y, flux, (fwhm / 2.355 / crop.scale).max(0.3));
            }
        }
    }

    // Synthetic galaxies
    let g_max = (lim + 0.5).clamp(12.0, 27.0);
    let mut g_lim = g_max;
    while galaxy_counts(g_lim) * area_deg2 > 150_000.0 && g_lim > 12.0 {
        g_lim -= 0.25;
    }
    let n_gal = (galaxy_counts(g_lim) * area_deg2) as usize;
    let gal_extinct = (1.0 / rad(gal_b.abs().max(1.0)).sin()).min(20.0); // crude: fewer galaxies near the plane
    let n_gal = (n_gal as f64 / gal_extinct.max(1.0).powf(0.7)) as usize;
    let (g0, g1) = (10f64.powf(0.45 * 12.0), 10f64.powf(0.45 * g_lim));
    let mut n_gal_det = 0;
    for _ in 0..n_gal {
        let v = (g0 + rng.f() * (g1 - g0)).log10() / 0.45;
        let xi = rad((rng.f() * 2.0 - 1.0) * half);
        let eta = rad((rng.f() * 2.0 - 1.0) * half);
        let d = (center + full.e * xi + full.n * eta).unit();
        let flux = zp * 10f64.powf(-0.4 * v);
        let rs = 2.5 * 10f64.powf(-0.12 * (v - 18.0)) * (0.6 + 0.8 * rng.f());
        let q = 0.25 + 0.75 * rng.f();
        let pa = rng.f() * PI;
        if v < lim - 0.5 {
            n_gal_det += 1;
        }
        if let Some((x, y)) = full.project(d) {
            full.add_galaxy(x, y, flux, rs, q, pa, sig_px(fwhm) * ps / full.scale);
        }
        if let Some((x, y)) = crop.project(d) {
            if x > -60.0 && y > -60.0 && x < crop.w as f64 + 60.0 && y < crop.h as f64 + 60.0 {
                crop.add_galaxy(x, y, flux, rs, q, pa, (fwhm / 2.355 / crop.scale).max(0.3));
            }
        }
    }

    // Solar-system bodies in the field
    let mut target_mag = None;
    let mut target_snr = None;
    let mut phase = None;
    let mut ring_tilt = None;
    let mut ang_diam = None;
    for &i in &in_fov_bodies {
        let pv = PlanetView::new(i, &bodies);
        let (m, alpha) = body_mag(i, &bodies);
        let flux = zp * 10f64.powf(-0.4 * m);
        let ang_r = (bodies[i].radius_km / bodies[i].dist_km).asin() * ARCSEC;
        if Some(i) == spec.body {
            target_mag = Some(m);
            phase = Some(alpha);
            ang_diam = Some(2.0 * ang_r);
            if i == 6 {
                ring_tilt = Some(deg(pv.w.dot(pv.pole).asin()));
            }
        }
        let ext = if pv.rings { 2.3 } else { 1.0 };
        let Some((per_unit_area, _)) = planet_norm(&pv, flux) else {
            notes.push(format!("{} is too thin a crescent to image (phase angle {:.0}°).", BODY_NAMES[i], alpha));
            continue;
        };
        for fr in [&mut full, &mut crop] {
            let Some((cx, cy)) = fr.project(bodies[i].dir) else { continue };
            let r_px = ang_r / fr.scale;
            if r_px < 1.5 {
                fr.add_psf(cx, cy, flux, (fwhm / 2.355 / fr.scale).max(0.3));
                continue;
            }
            let mut layer = vec![0.0; fr.w * fr.h];
            let x0 = ((cx - r_px * ext - 2.0).floor().max(0.0)) as usize;
            let x1 = ((cx + r_px * ext + 2.0).ceil().min(fr.w as f64)) as usize;
            let y0 = ((cy - r_px * ext - 2.0).floor().max(0.0)) as usize;
            let y1 = ((cy + r_px * ext + 2.0).ceil().min(fr.h as f64)) as usize;
            let pix_area = (1.0 / r_px).powi(2);
            let ss = 3; // supersampling
            for y in y0..y1 {
                for x in x0..x1 {
                    let mut acc = 0.0;
                    for sy in 0..ss {
                        for sx in 0..ss {
                            let px = x as f64 + (sx as f64 + 0.5) / ss as f64;
                            let py = y as f64 + (sy as f64 + 0.5) / ss as f64;
                            let u = -(px - cx) / r_px; // east is left
                            let v = -(py - cy) / r_px;
                            acc += pv.sample(u, v);
                        }
                    }
                    layer[y * fr.w + x] = acc / (ss * ss) as f64 * per_unit_area * pix_area;
                }
            }
            for (a, l) in fr.rate.iter_mut().zip(&layer) {
                *a += l;
            }
        }
    }

    // ---- Optics: convolve the ideal scene with the Fourier-optics PSF
    // (diffraction from aperture, obstruction and spiders; wavefront error;
    // 7-wavelength polychromatic; pointing jitter), pixel-integrated.
    let psf = hw.psf(sub_s);
    for fr in [&mut full, &mut crop] {
        let (k, ker) = psf.kernel(fr.scale);
        fr.rate = crate::optics::convolve(&fr.rate, fr.w, fr.h, &ker, k);
    }
    let psf_png = psf.png();
    let optics_only = crate::optics::psf(hw, 0.0);

    // ---- Expose & noise
    let mut nrng = Rng(field_seed ^ spec.id.wrapping_mul(0xA24B_AED4_963E_E407) ^ (total_s as u64));
    let fe = expose(&full, hw, total_s, sub_s, bg_native, &mut nrng);
    let ce = expose(&crop, hw, total_s, sub_s, bg_native, &mut nrng);
    if spec.body.is_some() {
        if let Some(i) = spec.body {
            if spec.body == Some(i) {
                // Brightest resolution element on the disc
                let s = crop.rate.iter().cloned().fold(0.0, f64::max) * total_s;
                target_snr = Some(s / (s + ce.sigma_bg.powi(2)).sqrt());
            }
        }
    }
    let full_png = encode_png(full.w, full.h, &stretch(&fe.img, fe.sigma_bg, false));
    let crop_png = encode_png(crop.w, crop.h, &stretch(&ce.img, ce.sigma_bg, planet_crop));

    if spec.body.is_none() {
        // Named-star target?
        if let Some(s) = named.iter().find(|s| s.dir.angle(center) < rad(0.01)) {
            target_mag = Some(s.mag as f64);
        }
    }
    if matches!(spec.target_name.as_str(), "Orion Nebula (M42)" | "Andromeda (M31)" | "LMC" | "Pleiades (M45)" | "Galactic Centre") {
        notes.push("Extended deep-sky objects (nebulae, galaxies, clusters) are not modelled yet: the field shows its stars and a synthetic background only.".into());
    }
    if spec.fine_frac < 0.999 {
        notes.push(format!(
            "No guide star brighter than the fine-guidance limit for {:.0} % of the integration: those frames used star-tracker pointing only, which blurs the stack.",
            100.0 * (1.0 - spec.fine_frac)
        ));
    }
    if spec.body == Some(1) {
        notes.push("Lunar parallax from low Earth orbit (up to ~1°) is not modelled: the Moon is placed at its geocentric position.".into());
    }
    if fe.saturated > 0 || ce.saturated > 0 {
        notes.push(format!("{} pixels saturated in the full field, {} in the crop.", fe.saturated, ce.saturated));
    }
    notes.push(format!(
        "Stars fainter than V 6.5 and all galaxies are a synthetic statistical population for galactic latitude {:+.0}°, not real objects. Planet surface markings are procedural.",
        gal_b
    ));
    notes.push("Every figure is an unchecked soft estimate from the idealised model described in imaging.rs.".into());
    if !spec.done {
        notes.push(format!(
            "Live stack: {:.0} % of the requested integration so far; depth will improve as more frames arrive.",
            100.0 * spec.total_s / spec.required_s.max(1e-9)
        ));
    }

    let equiv_d = hw.aperture_cm * (spec.total_s / spec.exposure_s.max(1e-9)).sqrt();
    let nyq = fwhm / ps;
    let report = serde_json::json!({
        "id": spec.id,
        "target": spec.target_name,
        "final": spec.done,
        "utc": astro::format_utc(spec.jd),
        "hardware": {
            "telescopes_requested": spec.telescopes,
            "aperture_cm": hw.aperture_cm,
            "obstruction": hw.obstruction,
            "focal_mm": r1(hw.focal_mm()),
            "f_ratio": r1(hw.f_ratio()),
            "sensor": format!("{:.0} px across, {} µm pixels (IMX455-class CMOS)", hw.sensor_px, hw.pixel_um),
            "pixel_scale_arcsec": r3(ps),
            "fov_deg": hw.fov_deg,
            "qe": hw.qe, "throughput": hw.throughput,
            "read_noise_e": hw.read_noise_e, "dark_e_s": hw.dark_e_s, "full_well_e": hw.full_well_e,
            "jitter_arcsec_rms": hw.jitter_arcsec,
            "zero_point_e_s": r1(zp),
        },
        "exposure": {
            "per_telescope_requested_s": spec.exposure_s,
            "telescope_seconds": r1(spec.total_s),
            "telescope_seconds_required": r1(spec.required_s),
            "single_scope_equivalent_h": r2(spec.total_s / 3600.0),
            "equivalent_aperture_cm_if_simultaneous": r1(equiv_d),
            "sub_frame_s": sub_s,
            "frames": nframes.round(),
        },
        "sky": {
            "zodiacal_mag_arcsec2": r2(sky_sb),
            "solar_elongation_deg": r1(deg(center.angle(sun))),
            "galactic_latitude_deg": r1(gal_b),
            "background_e_s_px": r3(bg_native),
        },
        "results": {
            "fwhm_arcsec": r2(fwhm),
            "diffraction_fwhm_arcsec": r2(hw.diffraction_fwhm()),
            "strehl": r2(psf.strehl),
            "fwhm_px": r2(nyq),
            "sampling": if nyq < 2.0 { "undersampled (FWHM < 2 px)" } else { "Nyquist-sampled" },
            "lim_mag_5sigma": r2(lim),
            "sb_lim_1sigma_mag_arcsec2": r2(sbl),
            "sources_above_5sigma": detected,
            "galaxies_above_5sigma": n_gal_det,
            "target_mag": target_mag.map(r2),
            "target_peak_snr": target_snr.map(r1),
            "phase_angle_deg": phase.map(r1),
            "angular_diameter_arcsec": ang_diam.map(r2),
            "saturn_ring_tilt_deg": ring_tilt.map(r2),
        },
        "optics": {
            "model": format!(
                "Fourier optics: {:.0} cm aperture, {:.0} % central obstruction, {} spider vane{} × {} mm, {:.0} nm RMS wavefront error, 7 wavelengths 420–690 nm, ⊗ {:.2}″ RMS jitter, pixel-integrated",
                hw.aperture_cm, hw.obstruction * 100.0, hw.spider_vanes, if hw.spider_vanes == 1 { "" } else { "s" },
                hw.spider_mm, hw.wfe_nm, hw.jitter_for(sub_s)
            ),
            "strehl": r2(psf.strehl),
            "fwhm_optics_arcsec": r2(optics_only.fwhm),
            "fwhm_delivered_arcsec": r2(psf.fwhm),
            "ee50_radius_arcsec": r2(psf.ee50_r),
            "ee80_radius_arcsec": r2(psf.ee80_r),
            "psf_field_arcsec": r1(psf.scale * psf.n as f64),
        },
        "pointing": {
            "effective_jitter_arcsec_rms": r2(hw.jitter_arcsec),
            "fine_guided_fraction": r2(spec.fine_frac),
            "repoints": spec.slews,
            "mean_slew_deg": r1(spec.mean_slew_deg),
            "response_s": spec.response_s.map(r1),
            "full_lock_s": spec.full_lock_s.map(r1),
            "agility": spec.agility.map(|a| serde_json::json!({
                "alpha_dps2": r2(a.alpha.to_degrees()), "omega_dps": r2(a.omega.to_degrees()),
                "settle_tau_s": a.settle_tau_s, "capture_arcsec": a.capture_arcsec, "acquire_s": a.acquire_s,
                "slew_s": r1(a.slew_time(spec.mean_slew_deg.to_radians())),
                "settle_s": r1(a.settle_time(spec.mean_slew_deg.to_radians())),
                "repoint_s": r1(a.repoint_time(spec.mean_slew_deg.to_radians())),
            })),
            "trace": spec.agility.map(|a| {
                crate::attitude::trace(&a, spec.mean_slew_deg.max(0.5).to_radians(), spec.fine_frac >= 0.5, spec.id + 1)
                    .iter().map(|&(t, e)| [r1(t), r2(e)]).collect::<Vec<_>>()
            }),
        },
        "images": {
            "full": { "px": full.w, "arcsec_per_px": r3(full.scale), "bin": r1(full.scale / ps) },
            "crop": { "px": crop.w, "arcsec_per_px": r3(crop.scale), "width_arcmin": r2(crop.w as f64 * crop.scale / 60.0),
                      "resampled": if crop.scale < ps * 0.99 { format!("{:.1}× finer than the detector pixel", ps / crop.scale) } else { "native".into() } },
            "orientation": "north up, east left",
            "stretch": "asinh",
        },
        "notes": notes,
    });
    Output { report, full_png, crop_png, psf_png }
}

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}
fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
fn r3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}
fn fmt_s(s: f64) -> String {
    if s < 1.0 {
        format!("{:.1} ms", s * 1000.0)
    } else {
        format!("{s:.0} s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw() -> Hardware {
        Hardware::from_obs(&ObsParams::default())
    }

    #[test]
    fn plate_scale_and_focal() {
        let h = hw();
        assert!((h.pixel_scale() - 0.376).abs() < 0.001);
        assert!((h.focal_mm() - 2063.0).abs() < 5.0, "{}", h.focal_mm());
        assert!((h.diffraction_fwhm() - 0.584).abs() < 0.01);
    }

    #[test]
    fn limits_are_sane() {
        let h = hw();
        let one = point_lim_mag(&h, 60.0, 60.0, 22.0);
        // A 20 cm telescope in space for 60 s: roughly V 20 (ground 20 cm gets ~18–19).
        assert!(one > 19.0 && one < 21.5, "{one}");
        // 100 telescopes × 60 s stacked: ~+2.5 mag when sky-limited
        let hundred = point_lim_mag(&h, 6000.0, 60.0, 22.0);
        assert!((hundred - one - 2.5).abs() < 0.4, "{}", hundred - one);
        // Brighter sky → shallower
        assert!(point_lim_mag(&h, 60.0, 60.0, 19.0) < one);
    }

    #[test]
    fn zodi_shape() {
        let sun = V3::new(1.0, 0.0, 0.0);
        let anti = zodi_sb(V3::new(-1.0, 0.0, 0.0), sun);
        let near = zodi_sb(V3::new(0.7, 0.714, 0.0), sun);
        let pole = zodi_sb(V3::Z.rot_x(rad(astro::OBLIQUITY_J2000_DEG)), sun);
        assert!(near < anti && anti < pole, "{near} {anti} {pole}");
        assert!((pole - 22.3).abs() < 0.2);
    }

    #[test]
    fn renders_star_field_and_planet() {
        let jd = 2_461_358.5;
        let b = astro::bodies(jd);
        for (body, dir) in [(None, V3::from_radec(rad(150.0), rad(60.0))), (Some(5usize), b[5].dir), (Some(6usize), b[6].dir)] {
            let spec = Spec {
                id: 1,
                target_name: "test".into(),
                dir,
                body,
                jd,
                total_s: 3000.0,
                required_s: 3000.0,
                telescopes: 50,
                exposure_s: 60.0,
                hw: hw(),
                done: true,
                fine_frac: 1.0,
                response_s: None,
                full_lock_s: None,
                slews: 0,
                mean_slew_deg: 0.0,
                agility: None,
            };
            let out = render(&spec);
            assert!(out.full_png.len() > 1000 && out.crop_png.len() > 1000);
            assert!(out.report["results"]["lim_mag_5sigma"].as_f64().unwrap() > 18.0);
        }
    }
}

#[cfg(test)]
mod gallery {
    use super::*;
    #[test]
    #[ignore]
    fn write_gallery() {
        let jd = 2_461_358.6;
        let b = astro::bodies(jd);
        let cases: Vec<(&str, Option<usize>, V3, f64, u32, f64)> = vec![
            ("jupiter", Some(5), b[5].dir, 100.0 * 60.0, 100, 60.0),
            ("saturn", Some(6), b[6].dir, 100.0 * 60.0, 100, 60.0),
            ("moon", Some(1), b[1].dir, 10.0 * 10.0, 10, 10.0),
            ("mars", Some(4), b[4].dir, 50.0 * 60.0, 50, 60.0),
            ("deep_ngp", None, V3::from_radec(rad(192.86), rad(27.13)), 300.0 * 300.0, 300, 300.0),
            ("single_60s", None, V3::from_radec(rad(192.86), rad(27.13)), 60.0, 1, 60.0),
            ("milkyway", None, V3::from_radec(rad(266.4), rad(-28.9)), 40.0 * 120.0, 40, 120.0),
            ("vega", None, V3::from_radec(rad(279.2347 + 0.004), rad(38.7837 + 0.003)), 20.0 * 60.0, 20, 60.0),
        ];
        for (name, body, dir, total, n, exp) in cases {
            let spec = Spec { id: 7, target_name: name.into(), dir, body, jd, total_s: total, required_s: total,
                              telescopes: n, exposure_s: exp, hw: Hardware::from_obs(&ObsParams::default()), done: true,
                              fine_frac: 1.0, response_s: None, full_lock_s: None, slews: 0, mean_slew_deg: 0.0, agility: None };
            let t = std::time::Instant::now();
            let out = render(&spec);
            std::fs::write(format!("/tmp/g_{name}_full.png"), &out.full_png).unwrap();
            std::fs::write(format!("/tmp/g_{name}_crop.png"), &out.crop_png).unwrap();
            std::fs::write(format!("/tmp/g_{name}_psf.png"), &out.psf_png).unwrap();
            println!("== {name} ({:.2}s)\n{}", t.elapsed().as_secs_f64(), serde_json::to_string(&out.report["results"]).unwrap());
            println!("{}", serde_json::to_string(&out.report["exposure"]).unwrap());
            println!("{}", serde_json::to_string(&out.report["sky"]).unwrap());
        }
    }
}
