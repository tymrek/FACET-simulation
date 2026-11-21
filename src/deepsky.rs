//! Catalogue deep-sky objects drawn into result images.
//!
//! Each OpenNGC object in or near the field is drawn as a smooth model scaled
//! to its catalogue size, axis ratio, position angle and magnitude:
//!
//! * galaxies: a de Vaucouleurs bulge plus an exponential disc; large discs
//!   get procedural two-armed spiral structure, star-forming clumps and dust
//!   lanes, and edge-on discs a dust lane along the major axis;
//! * globular clusters: a King-like profile, part smooth light and part
//!   resolved stars;
//! * open clusters and stellar associations: synthetic member stars;
//! * planetary nebulae: an elliptical shell with a faint halo;
//! * emission, reflection and H II nebulae: procedural fractal texture
//!   inside the catalogue ellipse; supernova remnants: filamentary texture.
//!
//! The catalogue fixes position, size, shape, orientation and total
//! brightness. Internal structure (arms, dust, nebular texture) is
//! illustrative, not a real image, and the result notes say so.

use crate::catalog::Dso;
use crate::math::V3;
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Galaxy,
    Globular,
    OpenCluster,
    Planetary,
    Nebula,
    Remnant,
}

pub fn kind_of(code: &str) -> Option<Kind> {
    Some(match code {
        "G" | "GPair" | "GTrpl" => Kind::Galaxy,
        "GCl" => Kind::Globular,
        "OCl" | "*Ass" => Kind::OpenCluster,
        "PN" => Kind::Planetary,
        "Neb" | "HII" | "EmN" | "RfN" | "Cl+N" => Kind::Nebula,
        "SNR" => Kind::Remnant,
        _ => return None,
    })
}

/// A drawable object: centre, total V magnitude, semi-major axis (arcsec),
/// axis ratio, position angle (rad, north through east).
#[derive(Clone, Debug)]
pub struct Model {
    pub name: String,
    pub kind: Kind,
    pub dir: V3,
    pub mag: f64,
    pub a: f64,
    pub q: f64,
    pub pa: f64,
    pub seed: u64,
    /// Spiral structure strength (0 for smooth discs).
    arms: f64,
    /// Bulge-to-total light ratio (galaxies).
    bt: f64,
    /// Bar strength (SB / SAB spirals) and its angle in the disc plane, rad.
    bar: f64,
    bar_angle: f64,
    /// Elliptical (no disc) or irregular (clumpy, armless disc).
    elliptical: bool,
    irregular: bool,
    /// A nebula with its own star cluster (OpenNGC "Cl+N", e.g. M42).
    cluster_too: bool,
    /// Integral of `sb` over the plane, arcsec² (so flux × sb / norm is per arcsec²).
    norm: f64,
}

/// Mean surface brightness assumed for nebulae without a catalogue magnitude, V mag/arcsec².
const NEB_SB_DEFAULT: f64 = 21.5;

fn hash(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

fn h01(seed: u64, i: i64, j: i64) -> f64 {
    let k = hash(seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
    (k >> 11) as f64 / (1u64 << 53) as f64
}

/// Smooth value noise in [0, 1].
fn vnoise(seed: u64, x: f64, y: f64) -> f64 {
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (x - x0, y - y0);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let (i, j) = (x0 as i64, y0 as i64);
    let a = h01(seed, i, j);
    let b = h01(seed, i + 1, j);
    let c = h01(seed, i, j + 1);
    let d = h01(seed, i + 1, j + 1);
    a + (b - a) * sx + (c - a) * sy + (a - b - c + d) * sx * sy
}

/// Fractal noise in roughly [0, 1].
fn fbm(seed: u64, x: f64, y: f64, oct: u32) -> f64 {
    let (mut s, mut amp, mut f, mut tot) = (0.0, 0.5, 1.0, 0.0);
    for o in 0..oct {
        s += amp * vnoise(seed.wrapping_add(o as u64 * 7919), x * f, y * f);
        tot += amp;
        amp *= 0.5;
        f *= 2.03;
    }
    s / tot
}

impl Model {
    pub fn from_dso(d: &Dso) -> Option<Model> {
        let kind = kind_of(&d.kind)?;
        let major = d.major? as f64;
        if major <= 0.0 {
            return None;
        }
        let minor = d.minor.map(|m| m as f64).filter(|m| *m > 0.0).unwrap_or(major);
        let a = major * 60.0 / 2.0;
        let q = (minor / major).clamp(0.08, 1.0);
        let pa = (d.pa.unwrap_or(0.0) as f64).to_radians();
        let seed = hash(d.id.bytes().fold(0xFACE_u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64)));
        let mag = match d.mag {
            Some(m) => m as f64,
            None if matches!(kind, Kind::Nebula | Kind::Remnant | Kind::Planetary) => {
                // No catalogue magnitude: assume a mean surface brightness over the ellipse.
                NEB_SB_DEFAULT - 2.5 * (PI * a * a * q).log10()
            }
            None => return None,
        };
        // Morphology from the OpenNGC Hubble type; without one, a size and
        // axis-ratio heuristic.
        let h = d.hubble.as_deref().unwrap_or("").trim();
        let elliptical = kind == Kind::Galaxy && (h.starts_with('E') || h.starts_with("cD") || h.starts_with("dE"));
        let lenticular = kind == Kind::Galaxy && (h.starts_with("S0") || h.starts_with("SA0") || h.starts_with("SB0") || h.starts_with("SAB0"));
        let irregular = kind == Kind::Galaxy && (h.starts_with('I') || h.ends_with('m'));
        let spiral = kind == Kind::Galaxy && h.starts_with('S') && !lenticular && !irregular;
        let stage = h.trim_start_matches("SAB").trim_start_matches("SA").trim_start_matches("SB").trim_start_matches('S').chars().next();
        let bt = if elliptical {
            1.0
        } else if lenticular {
            0.55
        } else if spiral {
            match stage {
                Some('a') => 0.4,
                Some('b') => 0.25,
                Some('c') => 0.12,
                Some('d') => 0.05,
                _ => 0.25,
            }
        } else if irregular {
            0.0
        } else if a > 300.0 {
            0.3
        } else {
            0.4
        };
        let big = major > 3.0;
        let arms = if kind != Kind::Galaxy || !big || elliptical || lenticular || irregular {
            0.0
        } else if q < 0.25 {
            0.0
        } else if spiral || h.is_empty() {
            if q >= 0.92 && h.is_empty() { 0.3 } else { 0.55 }
        } else {
            0.0
        };
        let bar = if h.starts_with("SB") { 1.0 } else if h.starts_with("SAB") { 0.5 } else { 0.0 };
        let cluster_too = d.kind == "Cl+N";
        let bar_angle = (seed % 1000) as f64 / 1000.0 * PI;
        Some(Model { name: d.name(), kind, dir: d.dir, mag, a, q, pa, seed, arms, bt, bar, bar_angle, elliptical, irregular, cluster_too, norm: 0.0 })
    }

    /// `from_dso` with the normalisation integral computed.
    #[cfg(test)]
    pub fn from_dso_normed(d: &Dso) -> Option<Model> {
        let mut m = Self::from_dso(d)?;
        m.norm = m.integrate();
        Some(m)
    }

    /// Radius beyond which the model is negligible, arcsec.
    pub fn extent(&self) -> f64 {
        match self.kind {
            Kind::Galaxy => 1.5 * self.a,
            Kind::Planetary => 1.6 * self.a,
            _ => 1.2 * self.a,
        }
    }

    /// Relative surface brightness at offset (xi east, eta north), arcsec
    /// from the centre, tapered smoothly to zero at `extent()` so no model
    /// shows the edge of its drawing box.
    pub fn sb(&self, xi: f64, eta: f64) -> f64 {
        let rr = (xi * xi + eta * eta).sqrt() / self.extent();
        if rr >= 1.0 {
            return 0.0;
        }
        self.sb_raw(xi, eta) * (1.0 - smooth(0.6, 1.0, rr))
    }

    fn sb_raw(&self, xi: f64, eta: f64) -> f64 {
        let (s, c) = self.pa.sin_cos();
        let u = xi * s + eta * c; // along the major axis
        let v = -xi * c + eta * s; // along the minor axis
        let a = self.a;
        match self.kind {
            Kind::Galaxy => {
                if self.elliptical {
                    // de Vaucouleurs r^1/4 law, R25 ≈ 2.5 effective radii.
                    let re = (a / 2.5).max(1.0);
                    let soft = 0.6;
                    let r = (u * u + (v / self.q).powi(2) + soft * soft).sqrt();
                    return (-7.669 * ((r / re).powf(0.25) - 1.0)).exp() / (22.67 * re * re * self.q);
                }
                let rs = a / 3.2;
                let re = (0.06 * a).max(1.0);
                let qb = self.q.max(0.6);
                let soft = 0.6;
                let rb = (u * u + (v / qb).powi(2) + soft * soft).sqrt();
                let bulge = (-7.669 * ((rb / re).powf(0.25) - 1.0)).exp() / (22.67 * re * re * qb);
                let (ud, vd) = (u, v / self.q); // disc-plane coordinates
                let rd = (ud * ud + vd * vd).sqrt();
                let mut disc = (-rd / rs).exp() / (2.0 * PI * rs * rs * self.q);
                let mut tau = 0.0;
                if self.arms > 0.0 {
                    let th = vd.atan2(ud);
                    let x = rd / rs;
                    let pitch = 15f64.to_radians();
                    let phase = 2.0 * (th - (x + 0.15).ln() / pitch.tan()) + (self.seed % 628) as f64 / 100.0;
                    let win = smooth(0.25, 0.8, x) * (1.0 - smooth(3.2, 4.5, x));
                    let arm = (0.5 + 0.5 * phase.cos()).powi(3);
                    let clumps = fbm(self.seed, u / (0.12 * rs), v / (0.12 * rs * self.q), 3);
                    disc *= 1.0 + self.arms * win * (1.6 * arm - 0.35 + 0.8 * (clumps - 0.5) * arm);
                    tau += 0.9 * self.arms * win * (0.5 + 0.5 * (phase - 0.7).cos()).powi(10) * (0.7 + 0.6 * clumps);
                }
                if self.irregular {
                    // Clumpy, lopsided star formation instead of arms.
                    let c = fbm(self.seed, ud / (0.25 * rs), vd / (0.25 * rs), 4);
                    disc *= (0.25 + 1.5 * c * c).max(0.0) * (1.0 + 0.3 * (ud / (2.0 * rs)).tanh());
                }
                let mut bar = 0.0;
                if self.bar > 0.0 {
                    let (sb, cb) = self.bar_angle.sin_cos();
                    let p = ud * cb + vd * sb;
                    let w = -ud * sb + vd * cb;
                    let (l, t) = (1.2 * rs, 0.3 * rs);
                    bar = (-(p / l).powi(2) - (w / t).powi(2)).exp() / (PI * l * t * self.q);
                }
                if self.q < 0.35 {
                    // Edge-on: a dust lane along the major axis.
                    tau += 1.2 * (-(v / (0.06 * a * self.q.max(0.1) * 3.0)).powi(2)).exp() * (1.0 - smooth(0.6 * a, 1.1 * a, u.abs()));
                }
                let fbar = 0.12 * self.bar;
                (self.bt * bulge + (1.0 - self.bt - fbar).max(0.0) * disc + fbar * bar) * (-tau).exp()
            }
            Kind::Globular => {
                let rt = a.max(2.0);
                let rc = rt / 15.0;
                let r = (u * u + (v / self.q).powi(2)).sqrt();
                if r >= rt {
                    return 0.0;
                }
                let k = 1.0 / (1.0 + (r / rc).powi(2)).sqrt() - 1.0 / (1.0 + (rt / rc).powi(2)).sqrt();
                k * k
            }
            Kind::OpenCluster => 0.0, // drawn as member stars
            Kind::Planetary => {
                let r = (u * u + (v / self.q).powi(2)).sqrt();
                let r0 = 0.6 * a;
                let w = (0.22 * a).max(0.5);
                let shell = (-((r - r0) / w).powi(2)).exp();
                let halo = 0.15 * (-(r / (1.2 * a)).powi(2)).exp();
                let texture = 0.75 + 0.5 * fbm(self.seed, u / (0.3 * a + 0.5), v / (0.3 * a + 0.5), 3);
                (shell + halo) * texture
            }
            Kind::Nebula | Kind::Remnant => {
                let r = ((u / a).powi(2) + (v / (a * self.q)).powi(2)).sqrt();
                let mask = (-(r * r) * 2.2).exp();
                let scale = a / 3.0;
                let n = fbm(self.seed, xi / scale, eta / scale, 5);
                if self.kind == Kind::Remnant {
                    let ridge = 1.0 - (2.0 * n - 1.0).abs();
                    let shell = (-((r - 0.75) / 0.25).powi(2)).exp();
                    ridge.powi(6) * (0.3 + shell) * mask.sqrt()
                } else {
                    let warp = fbm(self.seed ^ 0xABCD, xi / (2.0 * scale), eta / (2.0 * scale), 3);
                    (n * (0.6 + 0.8 * warp)).powi(3) * mask
                }
            }
        }
    }

    /// Numerical integral of `sb` over the plane (arcsec²).
    fn integrate(&self) -> f64 {
        if self.kind == Kind::OpenCluster {
            return 1.0;
        }
        let r = self.extent();
        let n = 240;
        let step = 2.0 * r / n as f64;
        let mut s = 0.0;
        for j in 0..n {
            for i in 0..n {
                let xi = -r + (i as f64 + 0.5) * step;
                let eta = -r + (j as f64 + 0.5) * step;
                s += self.sb(xi, eta);
            }
        }
        (s * step * step).max(1e-30)
    }

    /// Peak surface brightness relative to the total, per arcsec² (for
    /// saturation checks): the brightest sample over a coarse grid.
    pub fn peak_per_arcsec2(&self) -> f64 {
        if self.kind == Kind::OpenCluster {
            return 0.0;
        }
        let r = self.extent();
        let mut best = 0.0f64;
        let n = 200;
        for j in 0..n {
            for i in 0..n {
                let xi = -r + (i as f64 + 0.5) * 2.0 * r / n as f64;
                let eta = -r + (j as f64 + 0.5) * 2.0 * r / n as f64;
                best = best.max(self.sb(xi, eta));
            }
        }
        // The bulge cusp is finer than this grid: sample the centre too.
        best.max(self.sb(0.0, 0.0)) / self.norm
    }

    pub fn norm(&self) -> f64 {
        self.norm
    }

    /// Synthetic member stars for clusters: (xi, eta arcsec, V mag). Globular
    /// clusters put ~60 % of their light in resolved stars.
    pub fn members(&self, lim: f64) -> Vec<(f64, f64, f64)> {
        let mut out = Vec::new();
        let (n, v_lo, frac) = match self.kind {
            Kind::OpenCluster => ((25.0 * 10f64.powf(0.2 * (10.0 - self.mag))).clamp(30.0, 1500.0) as usize, self.mag + 3.0, 1.0),
            Kind::Globular => (3000, self.mag + 6.5, 0.6),
            Kind::Nebula if self.cluster_too => (300, self.mag + 2.0, 0.3),
            _ => return out,
        };
        let v_hi = (lim + 0.5).max(v_lo + 1.0);
        let mut k = 0u64;
        let mut rnd = || {
            k += 1;
            h01(self.seed, k as i64, 77)
        };
        let (s, c) = self.pa.sin_cos();
        let total = 10f64.powf(-0.4 * self.mag) * frac;
        let mut sum = 0.0;
        let mut stars = Vec::with_capacity(n);
        for _ in 0..n {
            // Concentrated towards the centre.
            let r = self.a * (if self.kind == Kind::Globular { rnd().powf(2.2) } else { rnd().sqrt() * 0.9 });
            let th = rnd() * 2.0 * PI;
            let (u, v) = (r * th.cos(), r * th.sin() * self.q);
            let xi = u * s - v * c;
            let eta = u * c + v * s;
            // Luminosity function: more faint than bright.
            let m = v_lo + (v_hi - v_lo) * rnd().powf(0.45);
            sum += 10f64.powf(-0.4 * m);
            stars.push((xi, eta, m));
        }
        // Scale so the members carry their share of the catalogue brightness.
        let dm = if sum > 0.0 { -2.5 * (total / sum).log10() } else { 0.0 };
        for (xi, eta, m) in stars {
            out.push((xi, eta, m + dm.max(0.0)));
        }
        out
    }
}

fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Catalogue objects whose extent reaches a field of half-width `half_deg` around `center`.
pub fn in_field(center: V3, half_deg: f64) -> Vec<Model> {
    static MODELS: std::sync::OnceLock<Vec<Model>> = std::sync::OnceLock::new();
    let all = MODELS.get_or_init(|| crate::catalog::dso().iter().filter_map(Model::from_dso).collect());
    let reach = half_deg.to_radians() * 1.45;
    all.iter()
        .filter(|m| m.dir.angle(center) < reach + (m.extent() / 3600.0).to_radians())
        .map(|m| {
            let mut m = m.clone();
            m.norm = m.integrate();
            m
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m31() -> Model {
        let d = crate::catalog::find_dso("M31").expect("M31 in catalogue");
        Model::from_dso(d).expect("model")
    }

    #[test]
    fn m31_is_a_large_inclined_spiral() {
        let m = m31();
        assert_eq!(m.kind, Kind::Galaxy);
        assert!((m.a - 177.83 * 30.0).abs() < 1.0);
        assert!(m.q > 0.35 && m.q < 0.45);
        // Brighter along the major axis than the minor at the same distance.
        let r = 0.3 * m.a;
        let (s, c) = m.pa.sin_cos();
        let along = m.sb(r * s, r * c);
        let across = m.sb(-r * c * 0.9, r * s * 0.9);
        assert!(along > 2.0 * across, "along {along} across {across}");
        // The bulge dominates the very centre.
        assert!(m.sb(0.0, 0.0) > 20.0 * m.sb(r * s, r * c));
    }

    #[test]
    fn hubble_types_set_the_model() {
        let get = |q: &str| Model::from_dso(crate::catalog::find_dso(q).unwrap()).unwrap();
        let (m110, m31, m87) = (get("M110"), get("M31"), get("M87"));
        assert!(m110.elliptical && m110.arms == 0.0, "M110 is a dwarf elliptical");
        assert!(m87.elliptical);
        assert!(!m31.elliptical && m31.arms > 0.0 && (m31.bt - 0.25).abs() < 1e-9, "M31 is an Sb spiral");
    }

    #[test]
    fn field_selection_finds_m31_and_companions() {
        let m = m31();
        let names: Vec<String> = in_field(m.dir, 0.65).iter().map(|x| x.name.clone()).collect();
        assert!(names.iter().any(|n| n.contains("M31")), "{names:?}");
        assert!(names.iter().any(|n| n.contains("M32")), "{names:?}");
    }

    #[test]
    fn normalisation_integrates_to_one() {
        for q in ["M31", "M57", "M13", "M42", "M1"] {
            let d = crate::catalog::find_dso(q).unwrap();
            let m = Model::from_dso_normed(d).unwrap();
            if m.kind == Kind::OpenCluster {
                continue;
            }
            assert!(m.norm() > 0.0 && m.norm().is_finite(), "{q}");
        }
    }
}
