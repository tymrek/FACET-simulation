//! Batched drawing helpers: thousands of points/lines as one egui mesh.

use egui::{epaint::Mesh, Color32, Pos2, Shape, Vec2};

#[derive(Default)]
pub struct Batch {
    pub mesh: Mesh,
}

impl Batch {
    pub fn point(&mut self, p: Pos2, r: f32, c: Color32) {
        let m = &mut self.mesh;
        let i = m.vertices.len() as u32;
        if r <= 1.8 {
            m.colored_vertex(p + Vec2::new(-r, -r), c);
            m.colored_vertex(p + Vec2::new(r, -r), c);
            m.colored_vertex(p + Vec2::new(r, r), c);
            m.colored_vertex(p + Vec2::new(-r, r), c);
            m.add_triangle(i, i + 1, i + 2);
            m.add_triangle(i, i + 2, i + 3);
        } else {
            let k = ((r / 1.5) as usize).clamp(8, 160);
            m.colored_vertex(p, c);
            for s in 0..k {
                let a = s as f32 / k as f32 * std::f32::consts::TAU;
                m.colored_vertex(p + Vec2::new(a.cos(), a.sin()) * r, c);
            }
            for s in 0..k as u32 {
                m.add_triangle(i, i + 1 + s, i + 1 + (s + 1) % k as u32);
            }
        }
    }

    /// Soft glow: bright core plus a transparent halo.
    pub fn glow_point(&mut self, p: Pos2, r: f32, c: Color32) {
        if r > 2.2 {
            self.point(p, r * 2.2, c.gamma_multiply(0.12));
        }
        self.point(p, r, c);
    }

    pub fn line(&mut self, a: Pos2, b: Pos2, w: f32, c: Color32) {
        let d = b - a;
        let len = d.length();
        if len < 1e-3 {
            return;
        }
        let n = Vec2::new(-d.y, d.x) / len * (w * 0.5);
        let m = &mut self.mesh;
        let i = m.vertices.len() as u32;
        m.colored_vertex(a + n, c);
        m.colored_vertex(b + n, c);
        m.colored_vertex(b - n, c);
        m.colored_vertex(a - n, c);
        m.add_triangle(i, i + 1, i + 2);
        m.add_triangle(i, i + 2, i + 3);
    }

    pub fn polyline(&mut self, pts: &[Pos2], w: f32, c: Color32, closed: bool) {
        for s in pts.windows(2) {
            self.line(s[0], s[1], w, c);
        }
        if closed && pts.len() > 2 {
            self.line(pts[pts.len() - 1], pts[0], w, c);
        }
    }

    /// Convex polygon fill (fan).
    pub fn convex_fill(&mut self, pts: &[Pos2], c: Color32) {
        if pts.len() < 3 {
            return;
        }
        let m = &mut self.mesh;
        let i = m.vertices.len() as u32;
        for &p in pts {
            m.colored_vertex(p, c);
        }
        for k in 1..pts.len() as u32 - 1 {
            m.add_triangle(i, i + k, i + k + 1);
        }
    }

    pub fn shape(self) -> Shape {
        Shape::mesh(self.mesh)
    }
}

pub fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, (a.clamp(0.0, 1.0) * 255.0) as u8)
}
