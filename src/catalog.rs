//! Embedded data: bright-star catalogue and world coastlines.
//!
//! Stars: HYG database v4.0 (astronexus, CC BY-SA 4.0), filtered to V ≤ 6.5
//! (8,920 stars, J2000 positions). Coastlines: Natural Earth 1:110m (public domain).

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
        assert_eq!(s.len(), 8920);
        assert!(s[0].mag < -1.0); // Sirius first (sorted by magnitude)
        assert!(load_names().iter().any(|n| n.name == "Vega"));
        assert!(load_coast().len() > 100);
    }
}
