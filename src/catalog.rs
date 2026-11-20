//! Embedded data: bright-star catalogue and world coastlines.
//!
//! Stars: HYG database v4.1 (astronexus, CC BY-SA 4.0), filtered to V ≤ 10
//! (108,071 stars, J2000 positions, sorted by magnitude), with 443 proper
//! names and 5,221 variable stars with magnitude ranges.
//! Deep sky: OpenNGC (Mattia Verga, CC BY-SA 4.0): 12,162 NGC/IC/Messier
//! galaxies, clusters and nebulae (duplicates, non-existent and star entries
//! removed).
//! Coastlines: Natural Earth 1:110m (public domain).

use crate::math::V3;

pub struct Star {
    pub dir: V3,
    pub mag: f32,
    /// B−V colour index.
    pub ci: f32,
}

pub struct NamedStar {
    pub name: String,
    pub dir: V3,
    pub mag: f32,
}

static STARS_BIN: &[u8] = include_bytes!("../assets/stars.bin");
static NAMES_TXT: &str = include_str!("../assets/star_names.txt");
static COAST_BIN: &[u8] = include_bytes!("../assets/coast.bin");
static DSO_TXT: &str = include_str!("../assets/dso.txt");
static VAR_TXT: &str = include_str!("../assets/variables.txt");

/// The star catalogue is treated as complete to this V magnitude; fainter
/// stars in simulated images come from the statistical population.
pub const REAL_STAR_LIMIT: f32 = 9.0;

pub struct Dso {
    /// Catalogue designation, e.g. "NGC 224".
    pub id: String,
    /// OpenNGC type code: G, GPair, OCl, GCl, PN, Neb, HII, Cl+N, RfN, SNR …
    pub kind: String,
    pub dir: V3,
    pub mag: Option<f32>,
    /// Major and minor axes, arcmin; position angle, deg.
    pub major: Option<f32>,
    pub minor: Option<f32>,
    pub pa: Option<f32>,
    pub messier: Option<String>,
    pub common: Option<String>,
}

impl Dso {
    /// Best display name: Messier number, then common name, then designation.
    pub fn name(&self) -> String {
        match (&self.messier, &self.common) {
            (Some(m), Some(c)) => format!("{} ({})", c, m),
            (Some(m), None) => format!("{} ({})", m, self.id),
            (None, Some(c)) => format!("{} ({})", c, self.id),
            (None, None) => self.id.clone(),
        }
    }
    pub fn short(&self) -> &str {
        self.messier.as_deref().unwrap_or(&self.id)
    }
    pub fn is_galaxy(&self) -> bool {
        self.kind.starts_with('G') && self.kind != "GCl"
    }
    pub fn is_cluster(&self) -> bool {
        matches!(self.kind.as_str(), "OCl" | "GCl" | "Cl+N" | "*Ass")
    }
    pub fn matches(&self, q: &str) -> bool {
        let q = q.trim().to_lowercase().replace(' ', "");
        let n = |s: &str| s.to_lowercase().replace(' ', "");
        n(&self.id) == q || self.messier.as_deref().map_or(false, |m| n(m) == q) || self.common.as_deref().map_or(false, |c| n(c) == q)
    }
}

pub struct VarStar {
    pub name: String,
    pub dir: V3,
    pub bright: f32,
    pub faint: f32,
}

pub fn dso() -> &'static [Dso] {
    use std::sync::OnceLock;
    static D: OnceLock<Vec<Dso>> = OnceLock::new();
    D.get_or_init(|| {
        DSO_TXT
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                if f.len() < 10 {
                    return None;
                }
                let num = |s: &str| s.parse::<f32>().ok();
                let opt = |s: &str| if s.is_empty() { None } else { Some(s.to_string()) };
                Some(Dso {
                    id: f[0].to_string(),
                    kind: f[1].to_string(),
                    dir: V3::from_radec(f[2].parse().ok()?, f[3].parse().ok()?),
                    mag: num(f[4]),
                    major: num(f[5]),
                    minor: num(f[6]),
                    pa: num(f[7]),
                    messier: opt(f[8]),
                    common: opt(f[9]),
                })
            })
            .collect()
    })
}

pub fn variables() -> &'static [VarStar] {
    use std::sync::OnceLock;
    static V: OnceLock<Vec<VarStar>> = OnceLock::new();
    V.get_or_init(|| {
        VAR_TXT
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                Some(VarStar {
                    name: f.first()?.to_string(),
                    dir: V3::from_radec(f.get(1)?.parse().ok()?, f.get(2)?.parse().ok()?),
                    bright: f.get(3)?.parse().ok()?,
                    faint: f.get(4)?.parse().ok()?,
                })
            })
            .collect()
    })
}

/// Find a deep-sky object by Messier number, designation or common name.
pub fn find_dso(q: &str) -> Option<&'static Dso> {
    if let Some(d) = dso().iter().find(|d| d.matches(q)) {
        return Some(d);
    }
    // Then common names containing the query ("Orion Nebula" →
    // "Great Orion Nebula"), brightest first.
    let ql = q.trim().to_lowercase();
    if ql.len() < 4 {
        return None;
    }
    dso()
        .iter()
        .filter(|d| d.common.as_deref().map_or(false, |c| c.to_lowercase().contains(&ql)))
        .min_by(|a, b| a.mag.unwrap_or(99.0).partial_cmp(&b.mag.unwrap_or(99.0)).unwrap_or(std::cmp::Ordering::Equal))
}

/// Well-known extragalactic deep survey fields (J2000 centres, deg).
pub const DEEP_FIELDS: [(&str, f64, f64); 10] = [
    ("Hubble Deep Field North (GOODS-N)", 189.206, 62.216),
    ("Hubble Ultra Deep Field (GOODS-S)", 53.162, -27.791),
    ("COSMOS", 150.119, 2.206),
    ("Extended Groth Strip (AEGIS)", 214.825, 52.825),
    ("UKIDSS Ultra Deep Survey (UDS)", 34.405, -5.200),
    ("Lockman Hole", 161.250, 58.000),
    ("ELAIS-N1", 242.750, 54.950),
    ("XMM-LSS", 35.708, -4.750),
    ("Subaru Deep Field", 201.163, 27.488),
    ("Euclid Deep Field North", 269.733, 66.018),
];

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub fn load_stars() -> Vec<Star> {
    STARS_BIN
        .chunks_exact(16)
        .map(|c| Star {
            dir: V3::from_radec(f32_at(c, 0) as f64, f32_at(c, 4) as f64),
            mag: f32_at(c, 8),
            ci: f32_at(c, 12),
        })
        .collect()
}

pub fn load_names() -> Vec<NamedStar> {
    NAMES_TXT
        .lines()
        .filter_map(|l| {
            let mut it = l.split('\t');
            let name = it.next()?.to_string();
            let ra: f64 = it.next()?.parse().ok()?;
            let dec: f64 = it.next()?.parse().ok()?;
            let mag: f32 = it.next()?.parse().ok()?;
            Some(NamedStar {
                name,
                dir: V3::from_radec(ra, dec),
                mag,
            })
        })
        .collect()
}

/// Coastline polylines as (lon, lat) degrees.
/// B−V of the catalogue star nearest `dir` (within 0.1°) whose magnitude is
/// within 0.5 of `mag`; 0 if none. For named stars, whose list has no colour.
pub fn bv_near(dir: V3, mag: f32) -> f32 {
    static STARS: std::sync::OnceLock<Vec<Star>> = std::sync::OnceLock::new();
    let stars = STARS.get_or_init(load_stars);
    let cmin = (0.1f64).to_radians().cos();
    stars
        .iter()
        .filter(|s| (s.mag - mag).abs() < 0.5 && s.dir.dot(dir) > cmin)
        .max_by(|a, b| a.dir.dot(dir).partial_cmp(&b.dir.dot(dir)).unwrap())
        .map(|s| s.ci)
        .unwrap_or(0.0)
}

/// Cached `bv_near` for a named star.
pub fn bv_named(name: &str, dir: V3, mag: f32) -> f32 {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, f32>>> = std::sync::OnceLock::new();
    let c = CACHE.get_or_init(Default::default);
    if let Some(&v) = c.lock().unwrap().get(name) {
        return v;
    }
    let v = bv_near(dir, mag);
    c.lock().unwrap().insert(name.to_string(), v);
    v
}

pub fn load_coast() -> Vec<Vec<(f32, f32)>> {
    let b = COAST_BIN;
    let n = u32_at(b, 0) as usize;
    let mut o = 4;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let m = u32_at(b, o) as usize;
        o += 4;
        let mut line = Vec::with_capacity(m);
        for _ in 0..m {
            line.push((f32_at(b, o), f32_at(b, o + 4)));
            o += 8;
        }
        out.push(line);
    }
    out
}

/// Approximate sRGB colour for a B−V index (Ballesteros-style ramp, hand-tuned).
pub fn bv_to_rgb(ci: f32) -> [u8; 3] {
    let stops: [(f32, [u8; 3]); 7] = [
        (-0.4, [155, 176, 255]),
        (0.0, [202, 215, 255]),
        (0.3, [248, 247, 255]),
        (0.6, [255, 244, 234]),
        (1.0, [255, 210, 161]),
        (1.5, [255, 180, 120]),
        (2.0, [255, 150, 90]),
    ];
    let c = ci.clamp(-0.4, 2.0);
    for w in stops.windows(2) {
        let (a, ca) = w[0];
        let (b, cb) = w[1];
        if c <= b {
            let t = (c - a) / (b - a);
            let mix = |i: usize| (ca[i] as f32 + (cb[i] as f32 - ca[i] as f32) * t) as u8;
            return [mix(0), mix(1), mix(2)];
        }
    }
    stops[6].1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogue_loads() {
        let s = load_stars();
        assert_eq!(s.len(), 108_071);
        assert!(s.windows(2).all(|w| w[0].mag <= w[1].mag));
        let m31 = find_dso("M31").expect("M31");
        assert_eq!(m31.id, "NGC 224");
        assert!(find_dso("ngc 7000").is_some() && find_dso("Orion Nebula").is_some());
        assert!(dso().len() > 12_000 && variables().len() > 5_000);
        assert!(s[0].mag < -1.0); // Sirius first (sorted by magnitude)
        assert!(load_names().iter().any(|n| n.name == "Vega"));
        assert!(load_coast().len() > 100);
    }
}
