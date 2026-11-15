//! Minimal 3-vector maths (f64). Kept dependency-free on purpose.

use std::ops::{Add, AddAssign, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl V3 {
    pub const ZERO: V3 = V3 { x: 0.0, y: 0.0, z: 0.0 };
    pub const Z: V3 = V3 { x: 0.0, y: 0.0, z: 1.0 };

    pub const fn new(x: f64, y: f64, z: f64) -> V3 {
        V3 { x, y, z }
    }
    pub fn dot(self, o: V3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: V3) -> V3 {
        V3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn norm(self) -> f64 {
        self.dot(self).sqrt()
    }
    pub fn unit(self) -> V3 {
        let n = self.norm();
        if n > 0.0 {
            self * (1.0 / n)
        } else {
            self
        }
    }
    /// Unit vector from right ascension / declination (radians), equatorial frame.
    pub fn from_radec(ra: f64, dec: f64) -> V3 {
        V3::new(dec.cos() * ra.cos(), dec.cos() * ra.sin(), dec.sin())
    }
    /// (ra, dec) in radians, ra in [0, 2π).
    pub fn to_radec(self) -> (f64, f64) {
        let u = self.unit();
        let mut ra = u.y.atan2(u.x);
        if ra < 0.0 {
            ra += std::f64::consts::TAU;
        }
        (ra, u.z.clamp(-1.0, 1.0).asin())
    }
    /// Angle between two vectors, radians.
    pub fn angle(self, o: V3) -> f64 {
        // atan2 form is well-conditioned for small angles (near-Sun work).
        self.cross(o).norm().atan2(self.dot(o))
    }
    pub fn rot_x(self, a: f64) -> V3 {
        let (s, c) = a.sin_cos();
        V3::new(self.x, c * self.y - s * self.z, s * self.y + c * self.z)
    }
    pub fn rot_z(self, a: f64) -> V3 {
        let (s, c) = a.sin_cos();
        V3::new(c * self.x - s * self.y, s * self.x + c * self.y, self.z)
    }
    /// Rodrigues rotation of self about unit axis k by angle a.
    pub fn rot_axis(self, k: V3, a: f64) -> V3 {
        let (s, c) = a.sin_cos();
        self * c + k.cross(self) * s + k * (k.dot(self) * (1.0 - c))
    }
    /// Any unit vector perpendicular to self.
    pub fn any_perp(self) -> V3 {
        let u = self.unit();
        let t = if u.z.abs() < 0.9 { V3::Z } else { V3::new(1.0, 0.0, 0.0) };
        u.cross(t).unit()
    }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl AddAssign for V3 {
    fn add_assign(&mut self, o: V3) {
        *self = *self + o;
    }
}
impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Mul<f64> for V3 {
    type Output = V3;
    fn mul(self, s: f64) -> V3 {
        V3::new(self.x * s, self.y * s, self.z * s)
    }
}
impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        V3::new(-self.x, -self.y, -self.z)
    }
}

pub fn deg(r: f64) -> f64 {
    r.to_degrees()
}
pub fn rad(d: f64) -> f64 {
    d.to_radians()
}
/// Wrap degrees into [0, 360).
pub fn wrap360(d: f64) -> f64 {
    d.rem_euclid(360.0)
}
