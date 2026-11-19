//! The two main views: an external 3D orbit view and the geocentric sky
//! (planetarium) view.

use crate::app::{App, Target};
use crate::astro::{self, BODY_NAMES, EARTH_RADIUS_KM};
use crate::catalog::bv_to_rgb;
use crate::draw::{rgba, Batch};
use crate::math::{deg, rad, V3};
use crate::attitude::Phase;
use crate::obs::{self, tangent_basis, Mode};
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};

const C_ACTIVE: (u8, u8, u8) = (90, 255, 150);
const C_LIT: (u8, u8, u8) = (255, 236, 205);
const C_ECLIPSED: (u8, u8, u8) = (95, 115, 180);
const C_SLEWING: (u8, u8, u8) = (255, 150, 40);
const C_SETTLING: (u8, u8, u8) = (255, 225, 80);
const C_SAA: (u8, u8, u8) = (255, 90, 200);
const BODY_COLORS: [(u8, u8, u8); 9] = [
    (255, 225, 120),
    (215, 215, 205),
    (190, 180, 170),
    (255, 245, 215),
    (255, 130, 80),
    (235, 205, 160),
    (240, 215, 130),
    (150, 225, 235),
    (110, 150, 255),
];

pub fn fmt_ra(ra: f64) -> String {
    let h = deg(ra).rem_euclid(360.0) / 15.0;
    let hh = h.floor();
    let mm = ((h - hh) * 60.0).floor();
    let ss = ((h - hh) * 60.0 - mm) * 60.0;
    format!("{:02}h{:02}m{:02.0}s", hh as i32, mm as i32, ss.floor())
}
pub fn fmt_dec(dec: f64) -> String {
    let d = deg(dec);
    let s = if d < 0.0 { '-' } else { '+' };
    let a = d.abs();
    let dd = a.floor();
    let mm = ((a - dd) * 60.0).floor();
    format!("{s}{:02}°{:02}'", dd as i32, mm as i32)
}

fn text(p: &egui::Painter, pos: Pos2, align: Align2, s: impl ToString, size: f32, c: Color32) {
    p.text(pos, align, s.to_string(), FontId::proportional(size), c);
}

fn c3(c: (u8, u8, u8), a: f32) -> Color32 {
    rgba(c.0, c.1, c.2, a)
}

impl App {
    // ------------------------------------------------------------------ orbit
    /// External 3D view. With `unified`, the full planetarium sky is drawn as
    /// the celestial sphere behind Earth and the shell, and clicks pick targets.
    pub fn orbit_view(&mut self, ui: &mut egui::Ui, rect: Rect, unified: bool) {
        let resp = ui.interact(rect, ui.id().with(("orbit_view", unified)), Sense::click_and_drag());
        let cam_s = &mut self.orbit_cam;
        if resp.dragged() {
            let d = resp.drag_delta();
            cam_s.yaw -= d.x as f64 * 0.006;
            cam_s.pitch = (cam_s.pitch + d.y as f64 * 0.006).clamp(-1.55, 1.55);
        }
        if resp.hovered() {
            let (sd, shift) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers.shift));
            // Shift turns vertical wheel motion into x on some platforms.
            let s = (sd.y + sd.x) as f64;
            if shift {
                cam_s.fov_deg = (cam_s.fov_deg * (-s * 0.0015).exp()).clamp(5.0, 120.0);
            } else {
                cam_s.dist_km = (cam_s.dist_km * (-s * 0.002).exp()).clamp(7_400.0, 1_200_000.0);
            }
        }
        let painter = ui.painter_at(rect);
        let gm = astro::gmst(self.jd);
        let yaw = cam_s.yaw + if cam_s.earth_fixed { gm } else { 0.0 };
        let cam = V3::new(
            cam_s.dist_km * cam_s.pitch.cos() * yaw.cos(),
            cam_s.dist_km * cam_s.pitch.cos() * yaw.sin(),
            cam_s.dist_km * cam_s.pitch.sin(),
        );
        let f = (-cam).unit();
        let r = f.cross(V3::Z).unit();
        let u = r.cross(f);
        let c = rect.center();
        let focal = rect.height() as f64 * 0.5 / rad(cam_s.fov_deg / 2.0).tan();
        let cam_fov = cam_s.fov_deg;
        let to_px = |x: f64, y: f64| Pos2::new(c.x + x as f32, c.y - y as f32);
        let proj = |p: V3| -> Option<Pos2> {
            let v = p - cam;
            let z = v.dot(f);
            if z < 1.0 {
                return None;
            }
            Some(to_px(v.dot(r) / z * focal, v.dot(u) / z * focal))
        };
        let proj_dir = |d: V3| -> Option<Pos2> {
            let z = d.dot(f);
            if z < 0.05 {
                return None;
            }
            Some(to_px(d.dot(r) / z * focal, d.dot(u) / z * focal))
        };
        let re = EARTH_RADIUS_KM;
        let occluded = |p: V3| -> bool {
            let d = p - cam;
            let t = -cam.dot(d) / d.dot(d);
            t > 0.0 && t < 1.0 && (cam + d * t).norm() < re
        };
        let surf_visible = |p: V3| p.dot(cam) > re * re;
        let sun = self.bodies[0].dir;

        // The celestial sphere: full planetarium layers in unified mode.
        if unified {
            let px_per_rad = |d: V3| focal / d.dot(f).max(0.05);
            let diag_fov = deg(2.0 * ((rect.width().hypot(rect.height()) as f64 * 0.5) / focal).atan());
            self.sky_layers(&painter, rect, &proj_dir, &px_per_rad, f, diag_fov);
        }

        let mut b = Batch::default();

        // Background stars
        if self.show.orbit_stars && !unified {
            for s in self.stars.iter().take_while(|s| s.mag < 5.5) {
                if let Some(p) = proj_dir(s.dir) {
                    if rect.contains(p) {
                        let [cr, cg, cb] = bv_to_rgb(s.ci);
                        let a = ((6.0 - s.mag) / 4.0).clamp(0.25, 1.0);
                        b.point(p, ((5.8 - s.mag) * 0.3).max(0.6), rgba(cr, cg, cb, a));
                    }
                }
            }
        }
        // Sun disk & Moon at true position (the sky layers draw them in unified mode)
        if !unified {
            if let Some(p) = proj_dir(sun) {
                b.glow_point(p, 7.0, c3(BODY_COLORS[0], 1.0));
            }
            let moon_pos = self.bodies[1].dir * self.bodies[1].dist_km;
            if let Some(p) = proj(moon_pos) {
                if !occluded(moon_pos) {
                    b.point(p, 3.0, c3(BODY_COLORS[1], 1.0));
                }
            }
        }

        // Earth: base disk, then a lit sphere mesh over the visible hemisphere.
        let ec = proj(V3::ZERO).unwrap_or(c);
        let ang = (re / cam.norm()).asin();
        let erad = (focal * ang.tan()) as f32;
        b.point(ec, erad * 1.035, rgba(70, 130, 255, 0.10)); // atmosphere glow
        b.point(ec, erad, Color32::from_rgb(5, 9, 20));
        {
            let lat_n = 24;
            let lon_n = 48;
            let vtx = |i: usize, j: usize| {
                let lat = -90.0 + 180.0 * i as f64 / lat_n as f64;
                let lon = 360.0 * j as f64 / lon_n as f64;
                astro::ecef_to_eci(lon, lat, re, gm)
            };
            let shade = |p: V3| {
                let s = p.unit().dot(sun);
                let t = ((s + 0.08) / 0.25).clamp(0.0, 1.0) as f32;
                let mix = |a: f32, bb: f32| (a + (bb - a) * t) as u8;
                Color32::from_rgb(mix(5.0, 28.0), mix(9.0, 68.0), mix(20.0, 140.0))
            };
            for i in 0..lat_n {
                for j in 0..lon_n {
                    let q = [vtx(i, j), vtx(i + 1, j), vtx(i + 1, j + 1), vtx(i, j + 1)];
                    let cen = (q[0] + q[1] + q[2] + q[3]) * 0.25;
                    if cen.unit().dot(cam) <= re * 1.02 {
                        continue;
                    }
                    let pp: Option<Vec<Pos2>> = q.iter().map(|&v| proj(v)).collect();
                    if let Some(pp) = pp {
                        let m = &mut b.mesh;
                        let k = m.vertices.len() as u32;
                        for (n, &v) in q.iter().enumerate() {
                            m.colored_vertex(pp[n], shade(v));
                        }
                        m.add_triangle(k, k + 1, k + 2);
                        m.add_triangle(k, k + 2, k + 3);
                    }
                }
            }
        }
        // Graticule + coastlines
        let seg = |b: &mut Batch, a: V3, bb: V3, day: Color32, night: Color32, w: f32| {
            if surf_visible(a) && surf_visible(bb) {
                if let (Some(pa), Some(pb)) = (proj(a), proj(bb)) {
                    let lit = (a + bb).unit().dot(sun) > 0.0;
                    b.line(pa, pb, w, if lit { day } else { night });
                }
            }
        };
        for lat in (-60..=60).step_by(30) {
            for k in 0..72 {
                let a = astro::ecef_to_eci(k as f64 * 5.0, lat as f64, re, gm);
                let bb = astro::ecef_to_eci((k + 1) as f64 * 5.0, lat as f64, re, gm);
                seg(&mut b, a, bb, rgba(120, 160, 220, 0.18), rgba(80, 100, 160, 0.12), 1.0);
            }
        }
        for lon in (0..360).step_by(30) {
            for k in 0..36 {
                let a = astro::ecef_to_eci(lon as f64, -90.0 + k as f64 * 5.0, re, gm);
                let bb = astro::ecef_to_eci(lon as f64, -90.0 + (k + 1) as f64 * 5.0, re, gm);
                seg(&mut b, a, bb, rgba(120, 160, 220, 0.18), rgba(80, 100, 160, 0.12), 1.0);
            }
        }
        if self.show.coast {
            for line in &self.coast {
                for w in line.windows(2) {
                    let a = astro::ecef_to_eci(w[0].0 as f64, w[0].1 as f64, re, gm);
                    let bb = astro::ecef_to_eci(w[1].0 as f64, w[1].1 as f64, re, gm);
                    seg(&mut b, a, bb, rgba(170, 215, 170, 0.85), rgba(90, 110, 140, 0.6), 1.2);
                }
            }
        }

        // South Atlantic Anomaly: the safe-mode contour at shell altitude,
        // with a fainter footprint on the ground beneath it.
        if self.show.saa && self.env.contour.len() > 2 {
            let rs = re + self.shell.params.alt_km;
            let col = rgba(255, 90, 200, 0.75);
            for w in self.env.contour.windows(2) {
                let a = astro::ecef_to_eci(w[0].1, w[0].0, rs, gm);
                let bb = astro::ecef_to_eci(w[1].1, w[1].0, rs, gm);
                if !occluded(a) && !occluded(bb) {
                    if let (Some(pa), Some(pb)) = (proj(a), proj(bb)) {
                        b.line(pa, pb, 1.6, col);
                    }
                }
                let a = astro::ecef_to_eci(w[0].1, w[0].0, re, gm);
                let bb = astro::ecef_to_eci(w[1].1, w[1].0, re, gm);
                seg(&mut b, a, bb, rgba(255, 90, 200, 0.35), rgba(255, 90, 200, 0.25), 1.0);
            }
        }

        // Sun / target direction guides from Earth
        let guide = |b: &mut Batch, d: V3, col: Color32| -> Option<Pos2> {
            let a = d * (re * 1.03);
            let e = d * (re * 2.6);
            if occluded(e) {
                return None;
            }
            let pa = if occluded(a) { None } else { proj(a) };
            let pe = proj(e)?;
            if let Some(pa) = pa {
                b.line(pa, pe, 1.5, col);
            }
            Some(pe)
        };
        let sun_tip = guide(&mut b, sun, rgba(255, 210, 90, 0.8));
        let tgt = self.target_dir();
        let tgt_tip = tgt.and_then(|t| guide(&mut b, t, rgba(90, 255, 150, 0.8)));
        let pd_dir = self.defence_marker();
        let pd_tip = pd_dir.as_ref().and_then(|(d, _)| guide(&mut b, *d, rgba(255, 80, 80, 0.9)));

        // Satellites, links, rays
        let n = self.shell.len();
        let pos = &self.shell.pos;
        let vis: Vec<Option<Pos2>> = pos
            .iter()
            .map(|&p| if occluded(p) { None } else { proj(p) })
            .collect();
        if self.show.links {
            let a_ok = if n < 300 { 0.55 } else if n < 2000 { 0.28 } else { 0.14 };
            for (k, &(i, j)) in self.shell.links.iter().enumerate() {
                if let (Some(pa), Some(pb)) = (vis[i as usize], vis[j as usize]) {
                    let col = if self.obs.link_blocked.get(k).copied().unwrap_or(false) {
                        rgba(255, 70, 60, 0.6)
                    } else {
                        rgba(80, 200, 255, a_ok)
                    };
                    b.line(pa, pb, 1.0, col);
                }
            }
        }
        if self.show.rays {
            // Thin out rays for big shells so the Earth stays readable.
            let len = cam.norm() * 0.06;
            let stride = (self.obs.n_active / 700).max(1);
            for i in (0..n).step_by(stride) {
                let Some(a) = self.obs.att.get(i) else { continue };
                if a.job == 0 {
                    continue;
                }
                let col = match a.phase() {
                    Phase::Slewing => C_SLEWING,
                    Phase::Settling | Phase::Acquiring => C_SETTLING,
                    _ => C_ACTIVE,
                };
                if let Some(pa) = vis[i] {
                    if let Some(pb) = proj(pos[i] + a.dir * len) {
                        b.line(pa, pb, 1.0, c3(col, 0.4));
                    }
                }
            }
        }
        if self.traffic.cfg.show {
            use crate::traffic::Pop;
            for (o, &p) in self.traffic.objs.iter().zip(&self.traffic.pos) {
                if occluded(p) {
                    continue;
                }
                if let Some(q) = proj(p) {
                    let (r, col) = match o.pop {
                        Pop::Starlink => (0.9, rgba(150, 200, 255, 0.28)),
                        Pop::OneWeb => (1.3, rgba(200, 140, 255, 0.6)),
                        _ => (1.0, rgba(170, 170, 170, 0.45)),
                    };
                    b.point(q, r, col);
                }
            }
        }
        let sr = (3.4 - 0.5 * (n as f32).log10()).max(1.1);
        for i in 0..n {
            if let Some(p) = vis[i] {
                let phase = self.obs.att.get(i).map(|a| a.phase()).unwrap_or(Phase::Idle);
                let saa = self.show.saa
                    && self.env.cfg.saa_policy != crate::environment::SaaPolicy::Off
                    && self.env.in_saa.get(i).copied().unwrap_or(false);
                let col = match phase {
                    _ if saa => C_SAA,
                    Phase::Locked => C_ACTIVE,
                    Phase::Slewing => C_SLEWING,
                    Phase::Settling | Phase::Acquiring => C_SETTLING,
                    Phase::Idle if self.obs.lit[i] => C_LIT,
                    Phase::Idle => C_ECLIPSED,
                };
                b.point(p, sr, c3(col, 1.0));
            }
        }
        painter.add(b.shape());

        if let Some(p) = sun_tip {
            text(&painter, p, Align2::LEFT_BOTTOM, "  Sun", 12.0, rgba(255, 210, 90, 1.0));
        }
        if let Some(p) = tgt_tip {
            text(&painter, p, Align2::LEFT_BOTTOM, format!("  {}", self.target_name()), 12.0, c3(C_ACTIVE, 1.0));
        }
        if let (Some(p), Some((_, label))) = (pd_tip, &pd_dir) {
            text(&painter, p, Align2::LEFT_BOTTOM, format!("  {label}"), 13.0, Color32::from_rgb(255, 110, 110));
        }
        if unified && self.click_target && resp.clicked() {
            if let Some(mp) = resp.interact_pointer_pos() {
                let unproj = |p: Pos2| -> V3 {
                    let x = (p.x - c.x) as f64 / focal;
                    let y = -(p.y - c.y) as f64 / focal;
                    (f + r * x + u * y).unit()
                };
                self.target = Some(self.pick(mp, &proj_dir, &unproj));
            }
        }
        if self.presenting {
            return;
        }
        let sp = &self.shell_params;
        text(
            &painter,
            rect.left_top() + egui::vec2(10.0, 8.0),
            Align2::LEFT_TOP,
            format!(
                "{}  ·  {} satellites, {} planes  ·  {:.0} km  ·  {:.1}°  ·  camera FOV {:.0}°",
                if unified { "UNIFIED" } else { "ORBIT" },
                n,
                sp.effective_planes(),
                sp.alt_km,
                sp.inc_deg,
                cam_fov
            ),
            13.0,
            Color32::from_gray(200),
        );
        legend(&painter, rect);
        text(
            &painter,
            rect.left_bottom() + egui::vec2(10.0, -8.0),
            Align2::LEFT_BOTTOM,
            if unified {
                "drag: rotate · scroll: zoom · shift+scroll: field of view · click sky: set target"
            } else {
                "drag: rotate · scroll: zoom · shift+scroll: field of view"
            },
            11.0,
            Color32::from_gray(110),
        );
    }

    // -------------------------------------------------------------------- sky
    pub fn sky_view(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let resp = ui.interact(rect, ui.id().with("sky_view"), Sense::click_and_drag());
        let c = rect.center();
        let k_px = |fov: f64| rect.width() as f64 * 0.5 / (2.0 * (rad(fov) / 4.0).tan());
        if resp.dragged() {
            let d = resp.drag_delta();
            let k = k_px(self.sky_cam.fov_deg);
            let cd = self.sky_cam.dec.cos().max(0.05);
            self.sky_cam.ra += d.x as f64 / k / cd;
            self.sky_cam.dec = (self.sky_cam.dec + d.y as f64 / k).clamp(rad(-89.9), rad(89.9));
        }
        if resp.hovered() {
            let s = ui.input(|i| i.smooth_scroll_delta.y) as f64;
            self.sky_cam.fov_deg = (self.sky_cam.fov_deg * (-s * 0.0015).exp()).clamp(0.5, 230.0);
        }
        let (ra0, dec0) = (self.sky_cam.ra, self.sky_cam.dec);
        let kp = k_px(self.sky_cam.fov_deg);
        let f = V3::from_radec(ra0, dec0);
        let west = V3::new(ra0.sin(), -ra0.cos(), 0.0);
        let north = V3::new(-dec0.sin() * ra0.cos(), -dec0.sin() * ra0.sin(), dec0.cos());
        let big = rect.expand(rect.width().max(rect.height()));
        let proj = |d: V3| -> Option<Pos2> {
            let z = d.dot(f);
            if z < -0.7 {
                return None;
            }
            let s = 2.0 / (1.0 + z);
            let p = Pos2::new(
                c.x + (d.dot(west) * s * kp) as f32,
                c.y - (d.dot(north) * s * kp) as f32,
            );
            if big.contains(p) {
                Some(p)
            } else {
                None
            }
        };
        let local_scale = |d: V3| 2.0 / (1.0 + d.dot(f));
        let unproj = |p: Pos2| -> V3 {
            let x = (p.x - c.x) as f64 / kp;
            let y = -(p.y - c.y) as f64 / kp;
            let rho2 = x * x + y * y;
            let z = (4.0 - rho2) / (4.0 + rho2);
            let s = (1.0 + z) / 2.0;
            (f * z + west * (x * s) + north * (y * s)).unit()
        };
        let painter = ui.painter_at(rect);
        let px_per_rad = |d: V3| kp * local_scale(d);
        self.sky_layers(&painter, rect, &proj, &px_per_rad, f, self.sky_cam.fov_deg);

        // Picking
        if self.click_target && resp.clicked() {
            if let Some(mp) = resp.interact_pointer_pos() {
                self.target = Some(self.pick(mp, &proj, &unproj));
            }
        }

        // HUD
        if self.presenting {
            return;
        }
        text(
            &painter,
            rect.left_top() + egui::vec2(10.0, 8.0),
            Align2::LEFT_TOP,
            format!(
                "SKY  ·  geocentric J2000  ·  centre {} {}  ·  FOV {:.1}°  ·  {}",
                fmt_ra(ra0),
                fmt_dec(dec0),
                self.sky_cam.fov_deg,
                self.mode.label()
            ),
            13.0,
            Color32::from_gray(200),
        );
        let hint = if let Some(hp) = resp.hover_pos() {
            let (ra, dec) = unproj(hp).to_radec();
            format!("{} {}   ·   drag: pan · scroll: zoom{}", fmt_ra(ra), fmt_dec(dec), if self.click_target { " · click: set target" } else { "" })
        } else {
            format!("drag: pan · scroll: zoom{}", if self.click_target { " · click: set target" } else { "" })
        };
        text(&painter, rect.left_bottom() + egui::vec2(10.0, -8.0), Align2::LEFT_BOTTOM, hint, 11.0, Color32::from_gray(120));
    }

    /// Draw the celestial sphere (everything the planetarium shows) through an
    /// arbitrary direction → screen projection. Shared by the sky view and the
    /// unified view.
    #[allow(clippy::too_many_arguments)]
    pub fn sky_layers(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        proj: &dyn Fn(V3) -> Option<Pos2>,
        px_per_rad: &dyn Fn(V3) -> f64,
        f: V3,
        fov_deg: f64,
    ) {
        let maxjump = rect.width() * 0.6;
        let curve = |b: &mut Batch, pts: &mut dyn Iterator<Item = V3>, w: f32, col: Color32| {
            let mut prev: Option<Pos2> = None;
            for d in pts {
                let p = proj(d);
                if let (Some(a), Some(bb)) = (prev, p) {
                    if (bb - a).length() < maxjump {
                        b.line(a, bb, w, col);
                    }
                }
                prev = p;
            }
        };
        let sun = self.bodies[0].dir;
        let moon = self.bodies[1].dir;
        let mut b = Batch::default();

        // Coverage heat map: RA/Dec quads with per-vertex colour.
        if self.show.coverage && !self.cov.is_empty() {
            let g = &self.grid;
            let maxc = self.cov.iter().copied().max().unwrap_or(1).max(1) as f64;
            let colour = |k: usize| -> Color32 {
                let d = g.pts[k];
                if crate::obs::sky_blocked(&self.obs_params, d, sun, moon).is_some() {
                    return Color32::TRANSPARENT;
                }
                let cnt = self.cov[k];
                if cnt == 0 {
                    rgba(210, 40, 40, 0.40)
                } else {
                    let t = ((cnt as f64 + 1.0).log10() / (maxc + 1.0).log10()) as f32;
                    let mix = |a: f32, bb: f32| (a + (bb - a) * t) as u8;
                    rgba(mix(40.0, 60.0), mix(60.0, 220.0), mix(170.0, 120.0), 0.28)
                }
            };
            let projected: Vec<Option<Pos2>> = g.pts.iter().map(|&d| proj(d)).collect();
            let cols: Vec<Color32> = (0..g.pts.len()).map(colour).collect();
            for i in 0..g.rows - 1 {
                for j in 0..g.cols - 1 {
                    let q = [g.idx(i, j), g.idx(i, j + 1), g.idx(i + 1, j + 1), g.idx(i + 1, j)];
                    let pp: Option<Vec<Pos2>> = q.iter().map(|&k| projected[k]).collect();
                    let Some(pp) = pp else { continue };
                    if (pp[0] - pp[2]).length() > maxjump || (pp[1] - pp[3]).length() > maxjump {
                        continue;
                    }
                    let m = &mut b.mesh;
                    let k0 = m.vertices.len() as u32;
                    for (n, &k) in q.iter().enumerate() {
                        m.colored_vertex(pp[n], cols[k]);
                    }
                    m.add_triangle(k0, k0 + 1, k0 + 2);
                    m.add_triangle(k0, k0 + 2, k0 + 3);
                }
            }
        }

        // Grid, equator, ecliptic
        if self.show.grid {
            let gcol = rgba(90, 120, 170, 0.22);
            for h in 0..12 {
                let ra = rad(h as f64 * 30.0);
                curve(&mut b, &mut (-88..=88).step_by(2).map(|d| V3::from_radec(ra, rad(d as f64))), 1.0, gcol);
            }
            for dd in (-75..=75).step_by(15) {
                let col = if dd == 0 { rgba(110, 160, 230, 0.5) } else { gcol };
                curve(&mut b, &mut (0..=180).map(|k| V3::from_radec(rad(k as f64 * 2.0), rad(dd as f64))), 1.0, col);
            }
            curve(
                &mut b,
                &mut (0..=180).map(|k| astro::ecliptic_point(rad(k as f64 * 2.0))),
                1.2,
                rgba(220, 180, 70, 0.45),
            );
        }

        // Keep-out circles
        let ring = |center: V3, radius_deg: f64| -> Vec<V3> {
            let perp = center.any_perp();
            let edge = center.rot_axis(perp, rad(radius_deg));
            (0..96)
                .map(|k| edge.rot_axis(center, k as f64 / 96.0 * std::f64::consts::TAU))
                .collect()
        };
        let keepout = |b: &mut Batch, center: V3, r_deg: f64, col: (u8, u8, u8), dashed: bool| {
            if r_deg <= 0.0 {
                return;
            }
            let pts = ring(center, r_deg);
            let pp: Vec<Option<Pos2>> = pts.iter().map(|&d| proj(d)).collect();
            if !dashed && pp.iter().all(|p| p.is_some()) && center.dot(f) > -0.2 && pp.iter().all(|p| p.map_or(false, |p| p.x.abs() < 1e6 && p.y.abs() < 1e6)) {
                let v: Vec<Pos2> = pp.iter().map(|p| p.unwrap()).collect();
                b.convex_fill(&v, c3(col, 0.08));
            }
            for k in 0..pp.len() {
                if dashed && k % 2 == 1 {
                    continue;
                }
                if let (Some(a), Some(bb)) = (pp[k], pp[(k + 1) % pp.len()]) {
                    if (bb - a).length() < maxjump {
                        b.line(a, bb, 1.4, c3(col, 0.7));
                    }
                }
            }
        };
        // All-sky survey coverage: tiles done this pass, and tiles being worked.
        if self.survey.show && self.survey_active() && self.survey.tiles() > 0 {
            let s = &self.survey;
            let done_c = Color32::from_rgba_unmultiplied(90, 200, 255, 46);
            let work_c = Color32::from_rgba_unmultiplied(160, 230, 255, 150);
            let half = (0.5 * rad(self.obs_params.fov_deg)) as f64;
            for (k, d) in s.tiling.dirs.iter().enumerate() {
                let working = s.claimed_by[k] != u32::MAX;
                if !s.done[k] && !working {
                    continue;
                }
                if let Some(p) = proj(*d) {
                    let h = ((half * px_per_rad(*d)) as f32).max(0.8);
                    b.square(p, h, if working { work_c } else { done_c });
                }
            }
        }
        keepout(&mut b, sun, self.obs_params.sun_excl_deg, (255, 170, 40), false);
        keepout(&mut b, moon, self.obs_params.moon_excl_deg, (180, 180, 200), false);
        if self.mode == Mode::NeoSweep {
            let e = self.obs_params.neo_elong_deg.max(self.obs_params.sun_excl_deg);
            keepout(&mut b, sun, e, (255, 120, 60), true);
        }

        // Stars
        let zf = (90.0 / fov_deg).powf(0.3).clamp(0.75, 2.2) as f32;
        // Fainter stars as the view zooms in: 5.8 all-sky, 6.5 at 90°,
        // ~8 at 20°, 10 (the catalogue limit) below ~5°.
        let mag_lim = if fov_deg > 150.0 { 5.8 } else { (6.5 + 2.3 * (90.0 / fov_deg).log10()).clamp(6.5, 10.0) as f32 };
        for s in self.stars.iter().take_while(|s| s.mag <= mag_lim) {
            if let Some(p) = proj(s.dir) {
                if rect.contains(p) {
                    let [cr, cg, cb] = bv_to_rgb(s.ci);
                    let a = ((6.9 - s.mag) / 3.0).clamp(0.35, 1.0);
                    let rr = ((6.9 - s.mag) * 0.42).max(0.55) * zf;
                    if s.mag < 1.5 {
                        b.glow_point(p, rr, rgba(cr, cg, cb, a));
                    } else {
                        b.point(p, rr, rgba(cr, cg, cb, a));
                    }
                }
            }
        }

        // Bodies
        let mut labels: Vec<(Pos2, String, Color32)> = Vec::new();
        for (i, body) in self.bodies.iter().enumerate() {
            if let Some(p) = proj(body.dir) {
                let ang = (body.radius_km / body.dist_km).asin();
                let true_r = (ang * px_per_rad(body.dir)) as f32;
                let min_r = match i {
                    0 => 7.0,
                    1 => 5.0,
                    3 | 5 => 3.6,
                    _ => 2.8,
                };
                let col = c3(BODY_COLORS[i], 1.0);
                b.glow_point(p, true_r.max(min_r), col);
                if self.show.names {
                    labels.push((p + egui::vec2(8.0, -6.0), BODY_NAMES[i].into(), col));
                }
            }
        }

        // Telescope footprints on the sky
        let mut stack_count = 0usize;
        if self.show.footprints {
            let h = rad(self.obs_params.fov_deg / 2.0).tan();
            let mut drawn = 0usize;
            let stack = self.mode == Mode::Stack;
            for (i, a) in self.obs.att.iter().enumerate() {
                if a.job == 0 {
                    continue;
                }
                let d = a.dir;
                let phase = a.phase();
                if phase != Phase::Locked {
                    // Moving boresight: marker plus a tick pointing along the
                    // remaining great-circle path toward the commanded target.
                    if let Some(p) = proj(d) {
                        let c = if phase == Phase::Slewing { C_SLEWING } else { C_SETTLING };
                        b.point(p, 2.2, c3(c, 0.95));
                        if phase == Phase::Slewing && d.angle(a.target) > 1e-4 {
                            let ahead = crate::attitude::slerp(d, a.target, (2.0f64.to_radians() / d.angle(a.target)).min(1.0));
                            if let Some(q) = proj(ahead) {
                                let v = q - p;
                                if v.length() > 0.5 {
                                    b.line(p, p + v.normalized() * 9.0, 1.4, c3(c, 0.8));
                                }
                            }
                        }
                    }
                    continue;
                }
                stack_count += 1;
                if (stack && drawn > 0) || drawn > 6000 {
                    continue;
                }
                let (e, n) = tangent_basis(d);
                let corners = [
                    (d + e * h + n * h).unit(),
                    (d - e * h + n * h).unit(),
                    (d - e * h - n * h).unit(),
                    (d + e * h - n * h).unit(),
                ];
                let pp: Option<Vec<Pos2>> = corners.iter().map(|&q| proj(q)).collect();
                if let Some(pp) = pp {
                    let lit = self.obs.lit.get(i).copied().unwrap_or(true);
                    let col = if lit { c3(C_ACTIVE, 0.7) } else { c3((120, 200, 255), 0.7) };
                    if (pp[0] - pp[2]).length() < 2.5 {
                        b.point(pp[0], 1.0, col);
                    } else {
                        b.convex_fill(&pp, col.gamma_multiply(0.15));
                        b.polyline(&pp, 1.0, col, true);
                    }
                    drawn += 1;
                }
            }
        }

        // Deep-sky objects: everything brighter than a zoom-dependent limit
        // (Messier objects always), drawn at true size once they are big
        // enough to see: ellipse for galaxies, circle for clusters, square
        // for nebulae.
        let mut dso_labels: Vec<(Pos2, String)> = Vec::new();
        if self.show.dso {
            let dlim = if fov_deg > 100.0 { 0.0 } else { (8.0 + 3.0 * (90.0 / fov_deg).log10()).clamp(8.0, 14.0) as f32 };
            for o in crate::catalog::dso() {
                let mes = o.messier.is_some();
                if !mes && o.mag.map_or(true, |m| m > dlim) {
                    continue;
                }
                let Some(p) = proj(o.dir) else { continue };
                if !rect.contains(p) {
                    continue;
                }
                let r = ((o.major.unwrap_or(2.0) as f64 / 120.0).to_radians() * px_per_rad(o.dir)) as f32;
                let col = if o.is_galaxy() {
                    rgba(255, 170, 120, 0.75)
                } else if o.is_cluster() {
                    rgba(255, 230, 140, 0.75)
                } else {
                    rgba(130, 220, 190, 0.75)
                };
                let r = r.max(2.5);
                if o.is_galaxy() {
                    let q = o.minor.zip(o.major).map(|(a, b)| (a / b).clamp(0.15, 1.0)).unwrap_or(0.6);
                    let pa = (o.pa.unwrap_or(0.0) as f32).to_radians();
                    let pts: Vec<Pos2> = (0..24)
                        .map(|k| {
                            let t = k as f32 / 24.0 * std::f32::consts::TAU;
                            let (x, y) = (r * q * t.cos(), r * t.sin());
                            // PA measured from north through east (east = left in this view).
                            p + egui::vec2(-(x * pa.cos() + y * pa.sin()), -(-x * pa.sin() + y * pa.cos()))
                        })
                        .collect();
                    b.polyline(&pts, 1.0, col, true);
                } else if o.is_cluster() {
                    let pts: Vec<Pos2> = (0..24).map(|k| { let t = k as f32 / 24.0 * std::f32::consts::TAU; p + egui::vec2(r * t.cos(), r * t.sin()) }).collect();
                    b.polyline(&pts, 1.0, col, true);
                } else {
                    let pts = [p + egui::vec2(-r, -r), p + egui::vec2(r, -r), p + egui::vec2(r, r), p + egui::vec2(-r, r)];
                    b.polyline(&pts, 1.0, col, true);
                }
                if self.show.names && (mes || fov_deg < 25.0) {
                    dso_labels.push((p + egui::vec2(r + 3.0, -2.0), o.messier.clone().or(o.common.clone()).unwrap_or_else(|| o.id.clone())));
                }
            }
        }

        // Known near-Earth asteroids (MPC): brighter than a zoom-dependent
        // limit; potentially hazardous ones in red.
        let mut neo_labels: Vec<(Pos2, String, bool)> = Vec::new();
        if self.neo.show && !self.neo.dir.is_empty() {
            let vlim = (16.0 + 3.0 * (90.0 / fov_deg).log10()).clamp(15.0, 21.0) as f32;
            let all = crate::neo::all();
            for k in 0..self.neo.dir.len() {
                let v = self.neo.vmag[k];
                if v > vlim {
                    continue;
                }
                let Some(p) = proj(self.neo.dir[k]) else { continue };
                if !rect.contains(p) {
                    continue;
                }
                let pha = all[k].pha();
                let col = if pha { rgba(255, 90, 80, 0.9) } else { rgba(200, 160, 255, 0.85) };
                let r = ((vlim + 1.0 - v) * 0.45).clamp(1.2, 3.5);
                b.point(p, r, col);
                if self.show.names && (fov_deg < 20.0 || v < 13.5) {
                    neo_labels.push((p + egui::vec2(4.0, 3.0), all[k].name.to_string(), pha));
                }
            }
        }

        painter.add(b.shape());
        for (p, s, pha) in neo_labels.into_iter().take(200) {
            text(&painter, p, Align2::LEFT_TOP, s, 10.0, if pha { Color32::from_rgb(255, 140, 120) } else { Color32::from_rgb(190, 170, 240) });
        }
        for (p, s) in dso_labels.into_iter().take(400) {
            text(&painter, p, Align2::LEFT_BOTTOM, s, 10.5, Color32::from_rgb(200, 180, 150));
        }

        // Star names
        if self.show.names {
            let lim = if fov_deg > 120.0 { 1.6 } else { 2.6 };
            for s in self.names.iter().filter(|s| s.mag <= lim) {
                if let Some(p) = proj(s.dir) {
                    if rect.contains(p) {
                        text(&painter, p + egui::vec2(6.0, 4.0), Align2::LEFT_TOP, &s.name, 11.0, Color32::from_gray(140));
                    }
                }
            }
        }
        for (p, s, col) in labels {
            if rect.contains(p) {
                text(&painter, p, Align2::LEFT_BOTTOM, s, 12.0, col);
            }
        }

        // Planetary-defence object (once discovered)
        if let Some((d, label)) = self.defence_marker() {
            if let Some(p) = proj(d) {
                let col = Color32::from_rgb(255, 80, 80);
                let r = 11.0;
                painter.add(egui::Shape::closed_line(
                    vec![p + egui::vec2(0.0, -r), p + egui::vec2(r, r * 0.8), p + egui::vec2(-r, r * 0.8)],
                    Stroke::new(2.0_f32, col),
                ));
                text(&painter, p + egui::vec2(14.0, -12.0), Align2::LEFT_BOTTOM, label, 13.0, col);
            }
        }

        // Target marker
        if let Some(t) = self.target_dir() {
            if let Some(p) = proj(t) {
                let col = if self.obs.target_blocked_by.is_some() {
                    Color32::from_rgb(255, 110, 90)
                } else {
                    c3(C_ACTIVE, 1.0)
                };
                painter.circle_stroke(p, 12.0, Stroke::new(1.5_f32, col));
                for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                    let v = egui::vec2(dx, dy);
                    painter.line_segment([p + v * 15.0, p + v * 24.0], Stroke::new(1.5_f32, col));
                }
                let mut s = self.target_name();
                if matches!(self.mode, Mode::Stack | Mode::Mosaic) {
                    if let Some(why) = self.obs.target_blocked_by {
                        s += &format!("  ·  {why}");
                    } else {
                        let incoming: Vec<f64> = self
                            .obs
                            .att
                            .iter()
                            .zip(&self.obs.job)
                            .filter(|(a, &j)| obs::job_tag(j) == obs::TAG_STACK && a.phase() != Phase::Locked && a.target.angle(t) < 1e-6)
                            .map(|(a, _)| a.time_to_lock())
                            .collect();
                        s += &format!("  ·  {} locked / {} can see it", self.obs.n_locked_target, self.obs.n_target_visible);
                        if !incoming.is_empty() {
                            let max = incoming.iter().cloned().fold(0.0, f64::max);
                            s += &format!("  ·  {} slewing in, all locked in ~{:.0} s", incoming.len(), max);
                        }
                    }
                }
                text(&painter, p + egui::vec2(18.0, 14.0), Align2::LEFT_TOP, s, 13.0, col);
                if self.mode == Mode::Stack && stack_count > 0 {
                    text(
                        &painter,
                        p + egui::vec2(18.0, 30.0),
                        Align2::LEFT_TOP,
                        format!("{}° FOV, stacking ×{} locked", self.obs_params.fov_deg, stack_count),
                        11.0,
                        Color32::from_gray(160),
                    );
                }
            }
        }

    }

    fn pick(&self, mp: Pos2, proj: &dyn Fn(V3) -> Option<Pos2>, unproj: &dyn Fn(Pos2) -> V3) -> Target {
        let mut cands: Vec<(Option<Pos2>, Target)> = Vec::new();
        for i in 0..self.bodies.len() {
            cands.push((proj(self.bodies[i].dir), Target::Body(i)));
        }
        for s in &self.names {
            cands.push((proj(s.dir), Target::Fixed { name: s.name.clone(), dir: s.dir }));
        }
        for o in crate::catalog::dso().iter().filter(|o| o.messier.is_some() || o.mag.map_or(false, |m| m <= 10.0)) {
            cands.push((proj(o.dir), Target::Fixed { name: o.name(), dir: o.dir }));
        }
        let mut best = nearest(mp, cands);
        if best.is_none() {
            let faint = self.stars.iter().take_while(|s| s.mag < 5.0).map(|s| {
                let (ra, dec) = s.dir.to_radec();
                (
                    proj(s.dir),
                    Target::Fixed {
                        name: format!("V={:.1} star {} {}", s.mag, fmt_ra(ra), fmt_dec(dec)),
                        dir: s.dir,
                    },
                )
            });
            best = nearest(mp, faint.collect());
        }
        best.unwrap_or_else(|| {
            let d = unproj(mp);
            let (ra, dec) = d.to_radec();
            Target::Fixed {
                name: format!("{} {}", fmt_ra(ra), fmt_dec(dec)),
                dir: d,
            }
        })
    }
}

fn legend(p: &egui::Painter, rect: Rect) {
    let items = [
        (C_ACTIVE, "locked, observing"),
        (C_SLEWING, "slewing"),
        (C_SETTLING, "settling / acquiring guide star"),
        (C_LIT, "idle, sunlit"),
        (C_ECLIPSED, "idle, eclipsed"),
        ((80, 200, 255), "laser link"),
        ((255, 70, 60), "link through atmosphere"),
        (C_SAA, "in the South Atlantic Anomaly"),
    ];
    let mut y = rect.top() + 30.0;
    for (col, s) in items {
        let at = Pos2::new(rect.left() + 16.0, y + 6.0);
        p.circle_filled(at, 3.5, c3(col, 1.0));
        text(p, at + egui::vec2(10.0, 0.0), Align2::LEFT_CENTER, s, 11.0, Color32::from_gray(150));
        y += 16.0;
    }
}

fn nearest(mp: Pos2, cands: Vec<(Option<Pos2>, Target)>) -> Option<Target> {
    cands
        .into_iter()
        .filter_map(|(p, t)| p.map(|p| ((p - mp).length(), t)))
        .filter(|(d, _)| *d < 14.0)
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, t)| t)
}

impl App {
    /// Direction and label of the planetary-defence object, once operators know of it.
    pub fn defence_marker(&self) -> Option<(V3, String)> {
        let d = self.defence.as_ref()?;
        if !d.discovered() || d.impacted {
            return None;
        }
        let dir = V3::from_radec(d.now.ra_deg.to_radians(), d.now.dec_deg.to_radians());
        let ip = d.systems()[0].ip;
        Some((
            dir,
            format!(
                "{} · IP {}",
                d.designation.split(" (").next().unwrap_or(""),
                ip.map(|p| if p > 0.9999 { "certain".to_string() } else { format!("{:.2} %", 100.0 * p) }).unwrap_or("?".into())
            ),
        ))
    }
}
