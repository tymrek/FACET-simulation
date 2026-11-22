//! Mosaics of catalogue objects larger than one field.
//!
//! A public request with `mosaic: true` on a deep-sky object is split into
//! tile requests on an RA/Dec-aligned grid of square fields with 10 %
//! overlap, covering the object's catalogue ellipse (OpenNGC major/minor
//! diameters and position angle). Grid cells that miss the ellipse are not
//! observed. The tiles are scheduled like any other requests; the stitched
//! overview (`/api/mosaic/{group}`) renders the whole footprint at once and
//! greys out any cell that was not observed.

use crate::math::V3;
use serde::Serialize;

/// Fractional overlap between neighbouring tiles.
pub const OVERLAP: f64 = 0.10;
/// Largest mosaic a public request may ask for.
pub const MAX_TILES: usize = 16;

/// Tile membership, carried by each tile request.
#[derive(Clone, Debug, Serialize)]
pub struct Mosaic {
    /// Id of the first tile request; shared by the whole mosaic.
    pub group: u64,
    pub index: u32,
    pub n: u32,
    pub cols: u32,
    pub rows: u32,
    pub col: u32,
    pub row: u32,
    /// Object name without the tile suffix.
    pub object: String,
    /// Mosaic centre (the object's catalogue position).
    pub ra_deg: f64,
    pub dec_deg: f64,
    /// Field side and grid step, degrees.
    pub field_deg: f64,
    pub step_deg: f64,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub cols: u32,
    pub rows: u32,
    pub step_deg: f64,
    /// Observed tiles: (col, row, pointing).
    pub tiles: Vec<(u32, u32, V3)>,
}

impl Plan {
    /// Side of the square overview that contains every grid cell, degrees.
    pub fn span_deg(cols: u32, rows: u32, step: f64, field: f64) -> f64 {
        (cols.max(rows) - 1) as f64 * step + field
    }
}

/// Tangent-plane offset (east, north; degrees) of grid cell (col, row).
pub fn cell_offset(col: u32, row: u32, cols: u32, rows: u32, step: f64) -> (f64, f64) {
    // Column 0 is the east-most (left on a north-up, east-left image); row 0 the north-most.
    let x = ((cols - 1) as f64 / 2.0 - col as f64) * step;
    let y = ((rows - 1) as f64 / 2.0 - row as f64) * step;
    (x, y)
}

/// Point at tangent-plane offset (xi east, eta north; degrees) from `c`.
pub fn offset_dir(c: V3, xi_deg: f64, eta_deg: f64) -> V3 {
    let (e, n) = crate::obs::tangent_basis(c);
    (c + e * xi_deg.to_radians().tan() + n * eta_deg.to_radians().tan()).unit()
}

/// Plan a mosaic over an ellipse: semi-axes `a_deg` ≥ `b_deg`, position angle
/// `pa_deg` (north through east), with square fields of side `field_deg`.
pub fn plan(center: V3, a_deg: f64, b_deg: f64, pa_deg: f64, field_deg: f64) -> Result<Plan, String> {
    let (s, c) = pa_deg.to_radians().sin_cos();
    let hx = ((a_deg * s).powi(2) + (b_deg * c).powi(2)).sqrt();
    let hy = ((a_deg * c).powi(2) + (b_deg * s).powi(2)).sqrt();
    if 2.0 * hx <= field_deg && 2.0 * hy <= field_deg {
        return Err(format!(
            "fits in one field ({:.0}′ × {:.0}′ against {:.0}′): no mosaic needed",
            2.0 * hx * 60.0,
            2.0 * hy * 60.0,
            field_deg * 60.0
        ));
    }
    plan_region(center, a_deg, b_deg, pa_deg, field_deg, MAX_TILES)
}

/// Tile any ellipse (one field if it fits), up to `max_tiles`.
pub fn plan_region(center: V3, a_deg: f64, b_deg: f64, pa_deg: f64, field_deg: f64, max_tiles: usize) -> Result<Plan, String> {
    let (s, c) = pa_deg.to_radians().sin_cos();
    // Half-extents of the rotated ellipse along east and north.
    let hx = ((a_deg * s).powi(2) + (b_deg * c).powi(2)).sqrt();
    let hy = ((a_deg * c).powi(2) + (b_deg * s).powi(2)).sqrt();
    let step = field_deg * (1.0 - OVERLAP);
    let count = |w: f64| if w <= field_deg { 1 } else { ((w - field_deg) / step).ceil() as u32 + 1 };
    let (cols, rows) = (count(2.0 * hx), count(2.0 * hy));
    let inside = |x: f64, y: f64| {
        let u = x * s + y * c;
        let v = -x * c + y * s;
        (u / a_deg).powi(2) + (v / b_deg).powi(2) <= 1.0
    };
    let mut tiles = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let (x0, y0) = cell_offset(col, row, cols, rows, step);
            let half = field_deg / 2.0;
            let k = 12;
            let hit = (0..=k).any(|i| {
                (0..=k).any(|j| inside(x0 - half + field_deg * i as f64 / k as f64, y0 - half + field_deg * j as f64 / k as f64))
            });
            if hit {
                tiles.push((col, row, offset_dir(center, x0, y0)));
            }
        }
    }
    if tiles.is_empty() {
        tiles.push((0, 0, center));
    }
    if tiles.len() > max_tiles {
        return Err(format!("needs {} tiles ({cols} × {rows} grid); the limit is {max_tiles}", tiles.len()));
    }
    Ok(Plan { cols, rows, step_deg: step, tiles })
}

/// Plan for a catalogue object.
pub fn plan_object(d: &crate::catalog::Dso, field_deg: f64) -> Result<Plan, String> {
    let major = d.major.ok_or("no catalogue size")? as f64 / 60.0;
    let minor = d.minor.map(|m| m as f64 / 60.0).filter(|m| *m > 0.0).unwrap_or(major);
    plan(d.dir, major / 2.0, minor / 2.0, d.pa.unwrap_or(0.0) as f64, field_deg)
}

/// Grey out the parts of an overview frame (side `w` px, `span_deg` across,
/// centred on the mosaic centre) that no observed tile covers.
pub fn mask_unobserved(px: &mut [u8], w: usize, span_deg: f64, m: &Mosaic, observed: &[(u32, u32)]) {
    let scale = span_deg / w as f64; // deg per pixel
    let half = m.field_deg / 2.0;
    let cells: Vec<(f64, f64)> = observed.iter().map(|&(c, r)| cell_offset(c, r, m.cols, m.rows, m.step_deg)).collect();
    for y in 0..w {
        let eta = (w as f64 / 2.0 - (y as f64 + 0.5)) * scale;
        for x in 0..w {
            let xi = (w as f64 / 2.0 - (x as f64 + 0.5)) * scale;
            let covered = cells.iter().any(|&(cx, cy)| (xi - cx).abs() <= half && (eta - cy).abs() <= half);
            if !covered {
                let p = &mut px[y * w + x];
                *p = (*p as u32 * 3 / 10 + 18) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m31_is_a_two_by_three_mosaic() {
        let d = crate::catalog::find_dso("M31").unwrap();
        let p = plan_object(d, 1.29).unwrap();
        assert_eq!((p.cols, p.rows), (2, 3), "{p:?}");
        assert!(p.tiles.len() >= 5 && p.tiles.len() <= 6);
        // Tiles sit within the object's extent.
        for (_, _, t) in &p.tiles {
            assert!(t.angle(d.dir).to_degrees() < 1.6);
        }
    }

    #[test]
    fn small_objects_need_no_mosaic() {
        let d = crate::catalog::find_dso("M57").unwrap();
        assert!(plan_object(d, 1.29).is_err());
    }

    #[test]
    fn neighbouring_tiles_overlap() {
        let d = crate::catalog::find_dso("M31").unwrap();
        let p = plan_object(d, 1.29).unwrap();
        let a = p.tiles.iter().find(|t| t.0 == 0 && t.1 == 1).unwrap().2;
        let b = p.tiles.iter().find(|t| t.0 == 1 && t.1 == 1).unwrap().2;
        let sep = a.angle(b).to_degrees();
        assert!(sep < 1.29 && sep > 1.0, "{sep}");
    }
}
