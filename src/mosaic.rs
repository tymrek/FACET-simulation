//! Mosaics of catalogue objects larger than one field.
//!
//! A public request with `mosaic: true` on a deep-sky object is split into
//! tile requests on an RA/Dec-aligned grid of the telescope's rectangular
//! field (long side east-west) with 10 % overlap on each axis, covering the object's catalogue ellipse (OpenNGC major/minor
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
    /// Field (east-west × north-south) and grid steps, degrees.
    pub field_w_deg: f64,
    pub field_h_deg: f64,
    pub step_x_deg: f64,
    pub step_y_deg: f64,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub cols: u32,
    pub rows: u32,
    pub step_x_deg: f64,
    pub step_y_deg: f64,
    /// Observed tiles: (col, row, pointing).
    pub tiles: Vec<(u32, u32, V3)>,
}

impl Plan {
    /// Extent of the grid (east-west, north-south), degrees.
    pub fn extent_deg(m: &Mosaic) -> (f64, f64) {
        ((m.cols - 1) as f64 * m.step_x_deg + m.field_w_deg, (m.rows - 1) as f64 * m.step_y_deg + m.field_h_deg)
    }
    /// Side of the square overview that contains every grid cell, degrees.
    pub fn span_deg(m: &Mosaic) -> f64 {
        let (w, h) = Plan::extent_deg(m);
        w.max(h)
    }
}

/// Tangent-plane offset (east, north; degrees) of grid cell (col, row).
pub fn cell_offset(col: u32, row: u32, cols: u32, rows: u32, step_x: f64, step_y: f64) -> (f64, f64) {
    // Column 0 is the east-most (left on a north-up, east-left image); row 0 the north-most.
    let x = ((cols - 1) as f64 / 2.0 - col as f64) * step_x;
    let y = ((rows - 1) as f64 / 2.0 - row as f64) * step_y;
    (x, y)
}

/// Point at tangent-plane offset (xi east, eta north; degrees) from `c`.
pub fn offset_dir(c: V3, xi_deg: f64, eta_deg: f64) -> V3 {
    let (e, n) = crate::obs::tangent_basis(c);
    (c + e * xi_deg.to_radians().tan() + n * eta_deg.to_radians().tan()).unit()
}

/// Plan a mosaic over an ellipse: semi-axes `a_deg` ≥ `b_deg`, position angle
/// `pa_deg` (north through east), with fields `fw` (east-west) × `fh`
/// (north-south) degrees.
pub fn plan(center: V3, a_deg: f64, b_deg: f64, pa_deg: f64, fw: f64, fh: f64) -> Result<Plan, String> {
    let (s, c) = pa_deg.to_radians().sin_cos();
    let hx = ((a_deg * s).powi(2) + (b_deg * c).powi(2)).sqrt();
    let hy = ((a_deg * c).powi(2) + (b_deg * s).powi(2)).sqrt();
    if 2.0 * hx <= fw && 2.0 * hy <= fh {
        return Err(format!(
            "fits in one field ({:.0}′ × {:.0}′ against {:.0}′ × {:.0}′): no mosaic needed",
            2.0 * hx * 60.0,
            2.0 * hy * 60.0,
            fw * 60.0,
            fh * 60.0
        ));
    }
    plan_region(center, a_deg, b_deg, pa_deg, fw, fh, MAX_TILES)
}

/// Tile any ellipse (one field if it fits), up to `max_tiles`.
pub fn plan_region(center: V3, a_deg: f64, b_deg: f64, pa_deg: f64, fw: f64, fh: f64, max_tiles: usize) -> Result<Plan, String> {
    let (s, c) = pa_deg.to_radians().sin_cos();
    // Half-extents of the rotated ellipse along east and north.
    let hx = ((a_deg * s).powi(2) + (b_deg * c).powi(2)).sqrt();
    let hy = ((a_deg * c).powi(2) + (b_deg * s).powi(2)).sqrt();
    let (sx, sy) = (fw * (1.0 - OVERLAP), fh * (1.0 - OVERLAP));
    let count = |w: f64, f: f64, st: f64| if w <= f { 1 } else { ((w - f) / st).ceil() as u32 + 1 };
    let (cols, rows) = (count(2.0 * hx, fw, sx), count(2.0 * hy, fh, sy));
    let inside = |x: f64, y: f64| {
        let u = x * s + y * c;
        let v = -x * c + y * s;
        (u / a_deg).powi(2) + (v / b_deg).powi(2) <= 1.0
    };
    let mut tiles = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let (x0, y0) = cell_offset(col, row, cols, rows, sx, sy);
            let k = 12;
            let hit = (0..=k).any(|i| {
                (0..=k).any(|j| inside(x0 - fw / 2.0 + fw * i as f64 / k as f64, y0 - fh / 2.0 + fh * j as f64 / k as f64))
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
    Ok(Plan { cols, rows, step_x_deg: sx, step_y_deg: sy, tiles })
}

/// Plan for a catalogue object.
pub fn plan_object(d: &crate::catalog::Dso, fw: f64, fh: f64) -> Result<Plan, String> {
    let major = d.major.ok_or("no catalogue size")? as f64 / 60.0;
    let minor = d.minor.map(|m| m as f64 / 60.0).filter(|m| *m > 0.0).unwrap_or(major);
    plan(d.dir, major / 2.0, minor / 2.0, d.pa.unwrap_or(0.0) as f64, fw, fh)
}

/// Grey out the parts of an overview frame (side `w` px, `span_deg` across,
/// centred on the mosaic centre) that no observed tile covers.
pub fn mask_unobserved(px: &mut [u8], w: usize, span_deg: f64, m: &Mosaic, observed: &[(u32, u32)]) {
    let scale = span_deg / w as f64; // deg per pixel
    let (hw, hh) = (m.field_w_deg / 2.0, m.field_h_deg / 2.0);
    let cells: Vec<(f64, f64)> = observed.iter().map(|&(c, r)| cell_offset(c, r, m.cols, m.rows, m.step_x_deg, m.step_y_deg)).collect();
    for y in 0..w {
        let eta = (w as f64 / 2.0 - (y as f64 + 0.5)) * scale;
        for x in 0..w {
            let xi = (w as f64 / 2.0 - (x as f64 + 0.5)) * scale;
            let covered = cells.iter().any(|&(cx, cy)| (xi - cx).abs() <= hw && (eta - cy).abs() <= hh);
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
        let p = plan_object(d, 1.289, 0.860).unwrap();
        // M31: 190′ × 60′ at PA 35°; 1.29° × 0.86° fields.
        for name in ["M31", "M42", "M33", "NGC 7000"] {
            if let Some(o) = crate::catalog::find_dso(name).and_then(|o| plan_object(o, 1.289, 0.860).ok()) {
                eprintln!("{name}: {} tiles on {} x {}", o.tiles.len(), o.cols, o.rows);
            }
        }
        assert!(p.cols >= 2 && p.rows >= 3, "{p:?}");
        assert!(p.tiles.len() >= 5 && p.tiles.len() <= MAX_TILES, "{p:?}");
        // Tiles sit within the object's extent.
        for (_, _, t) in &p.tiles {
            assert!(t.angle(d.dir).to_degrees() < 1.6);
        }
    }

    #[test]
    fn small_objects_need_no_mosaic() {
        let d = crate::catalog::find_dso("M57").unwrap();
        assert!(plan_object(d, 1.289, 0.860).is_err());
    }

    #[test]
    fn neighbouring_tiles_overlap() {
        let d = crate::catalog::find_dso("M31").unwrap();
        let p = plan_object(d, 1.289, 0.860).unwrap();
        // East-west neighbours closer than the field width, north-south
        // neighbours closer than its height.
        let at = |c: u32, r: u32| p.tiles.iter().find(|t| t.0 == c && t.1 == r).map(|t| t.2);
        let (mut ew, mut ns) = (false, false);
        for &(c, r, d) in &p.tiles {
            if let Some(e) = at(c + 1, r) {
                let sep = d.angle(e).to_degrees();
                assert!(sep < 1.289 && sep > 1.0, "east-west {sep}");
                ew = true;
            }
            if let Some(n) = at(c, r + 1) {
                let sep = d.angle(n).to_degrees();
                assert!(sep < 0.860 && sep > 0.7, "north-south {sep}");
                ns = true;
            }
        }
        assert!(ew && ns);
    }
}
