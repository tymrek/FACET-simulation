//! Time, Earth orientation, and low-precision Sun / Moon / planet positions.
//!
//! Algorithms after Paul Schlyter, "How to compute planetary positions"
//! (accuracy ~1-2 arcmin for Sun/planets, a few arcmin for the Moon), with the
//! equinox-of-date results precessed back to J2000 so they share a frame with
//! the star catalogue. That's plenty for a planning planetarium; it is NOT an
//! ephemeris for real pointing.
//!
//! Frame convention throughout the app ("ECI"): J2000 equatorial, x toward the
//! vernal equinox, z toward the north celestial pole. Distances in km.

use crate::math::{rad, wrap360, V3};

pub const AU_KM: f64 = 149_597_870.7;
pub const EARTH_RADIUS_KM: f64 = 6378.137;
pub const MU_EARTH: f64 = 398_600.4418; // km^3/s^2
pub const J2: f64 = 1.082_626_68e-3;
pub const OBLIQUITY_J2000_DEG: f64 = 23.4393;
pub const JD_J2000: f64 = 2_451_545.0;

/// Julian date (UTC treated as TT; the ~69 s difference is irrelevant here).
pub fn jd_from_unix(secs: f64) -> f64 {
    secs / 86_400.0 + 2_440_587.5
}

pub fn jd_now() -> f64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    jd_from_unix(secs)
}

/// Greenwich mean sidereal time, radians.
pub fn gmst(jd: f64) -> f64 {
    rad(wrap360(280.460_618_37 + 360.985_647_366_29 * (jd - JD_J2000)))
}

/// Calendar date/time (UTC) from a Julian date. Meeus, Astronomical Algorithms ch. 7.
pub fn calendar(jd: f64) -> (i64, u32, u32, u32, u32, u32) {
    let jd5 = jd + 0.5;
    let z = jd5.floor();
    let f = jd5 - z;
    let a = if z < 2_299_161.0 {
        z
    } else {
        let alpha = ((z - 1_867_216.25) / 36_524.25).floor();
        z + 1.0 + alpha - (alpha / 4.0).floor()
    };
    let b = a + 1524.0;
    let c = ((b - 122.1) / 365.25).floor();
    let d = (365.25 * c).floor();
    let e = ((b - d) / 30.6001).floor();
    let day_f = b - d - (30.6001 * e).floor() + f;
    let month = if e < 14.0 { e - 1.0 } else { e - 13.0 };
    let year = if month > 2.0 { c - 4716.0 } else { c - 4715.0 };
    let day = day_f.floor();
    let mut secs = ((day_f - day) * 86_400.0).round() as u32;
    if secs >= 86_400 {
        secs = 86_399;
    }
    (
        year as i64,
        month as u32,
        day as u32,
        secs / 3600,
        (secs / 60) % 60,
        secs % 60,
    )
}

pub fn format_utc(jd: f64) -> String {
    let (y, mo, d, h, mi, s) = calendar(jd);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC")
}

/// Schlyter's day number (0.0 = 1999 Dec 31, 0h).
fn day_number(jd: f64) -> f64 {
    jd - 2_451_543.5
}

fn sind(d: f64) -> f64 {
    rad(d).sin()
}
fn cosd(d: f64) -> f64 {
    rad(d).cos()
}

/// Solve Kepler's equation; angles in degrees.
fn kepler_e(m_deg: f64, e: f64) -> f64 {
    let m = rad(wrap360(m_deg));
    let mut ea = m + e * m.sin() * (1.0 + e * m.cos());
    for _ in 0..8 {
        let d = (ea - e * ea.sin() - m) / (1.0 - e * ea.cos());
        ea -= d;
        if d.abs() < 1e-12 {
            break;
        }
    }
    ea
}

struct Elements {
    n: f64,
    i: f64,
    w: f64,
    a: f64,
    e: f64,
    m: f64,
}

/// Heliocentric (or geocentric for the Moon) ecliptic position from elements.
fn position_from_elements(el: &Elements) -> V3 {
    let ea = kepler_e(el.m, el.e);
    let xv = el.a * (ea.cos() - el.e);
    let yv = el.a * (1.0 - el.e * el.e).sqrt() * ea.sin();
    let v = yv.atan2(xv);
    let r = (xv * xv + yv * yv).sqrt();
    let (n, i) = (rad(el.n), rad(el.i));
    let vw = v + rad(el.w);
    V3::new(
        r * (n.cos() * vw.cos() - n.sin() * vw.sin() * i.cos()),
        r * (n.sin() * vw.cos() + n.cos() * vw.sin() * i.cos()),
        r * (vw.sin() * i.sin()),
    )
}

fn sun_elements(d: f64) -> (f64, f64, f64) {
    // (w, e, M)
    (
        282.9404 + 4.70935e-5 * d,
        0.016709 - 1.151e-9 * d,
        356.0470 + 0.985_600_258_5 * d,
    )
}

/// Geocentric ecliptic (equinox of date) Sun vector, AU.
fn sun_ecliptic(d: f64) -> V3 {
    let (w, e, m) = sun_elements(d);
    let ea = kepler_e(m, e);
    let xv = ea.cos() - e;
    let yv = (1.0 - e * e).sqrt() * ea.sin();
    let v = yv.atan2(xv);
    let r = (xv * xv + yv * yv).sqrt();
    let lon = v + rad(w);
    V3::new(r * lon.cos(), r * lon.sin(), 0.0)
}

/// Geocentric ecliptic (equinox of date) Moon vector, Earth radii, with the
/// main perturbation terms.
fn moon_ecliptic(d: f64) -> V3 {
    let el = Elements {
        n: 125.1228 - 0.052_953_808_3 * d,
        i: 5.1454,
        w: 318.0634 + 0.164_357_322_3 * d,
        a: 60.2666,
        e: 0.054900,
        m: 115.3654 + 13.064_992_950_9 * d,
    };
    let p = position_from_elements(&el);
    let mut lon = p.y.atan2(p.x).to_degrees();
    let mut lat = p.z.atan2((p.x * p.x + p.y * p.y).sqrt()).to_degrees();
    let mut r = p.norm();

    let (ws, _, ms) = sun_elements(d);
    let mm = el.m;
    let ls = ms + ws;
    let lm = mm + el.w + el.n;
    let dd = lm - ls;
    let f = lm - el.n;
    lon += -1.274 * sind(mm - 2.0 * dd) + 0.658 * sind(2.0 * dd) - 0.186 * sind(ms)
        - 0.059 * sind(2.0 * mm - 2.0 * dd)
        - 0.057 * sind(mm - 2.0 * dd + ms)
        + 0.053 * sind(mm + 2.0 * dd)
        + 0.046 * sind(2.0 * dd - ms)
        + 0.041 * sind(mm - ms)
        - 0.035 * sind(dd)
        - 0.031 * sind(mm + ms)
        - 0.015 * sind(2.0 * f - 2.0 * dd)
        + 0.011 * sind(mm - 4.0 * dd);
    lat += -0.173 * sind(f - 2.0 * dd) - 0.055 * sind(mm - f - 2.0 * dd)
        - 0.046 * sind(mm + f - 2.0 * dd)
        + 0.033 * sind(f + 2.0 * dd)
        + 0.017 * sind(2.0 * mm + f);
    r += -0.58 * cosd(mm - 2.0 * dd) - 0.46 * cosd(2.0 * dd);
    V3::new(r * cosd(lat) * cosd(lon), r * cosd(lat) * sind(lon), r * sind(lat))
}

fn planet_elements(idx: usize, d: f64) -> Elements {
    match idx {
        0 => Elements {
            n: 48.3313 + 3.24587e-5 * d,
            i: 7.0047 + 5.00e-8 * d,
            w: 29.1241 + 1.01444e-5 * d,
            a: 0.387098,
            e: 0.205635 + 5.59e-10 * d,
            m: 168.6562 + 4.092_334_436_8 * d,
        },
        1 => Elements {
            n: 76.6799 + 2.46590e-5 * d,
            i: 3.3946 + 2.75e-8 * d,
            w: 54.8910 + 1.38374e-5 * d,
            a: 0.723330,
            e: 0.006773 - 1.302e-9 * d,
            m: 48.0052 + 1.602_130_224_4 * d,
        },
        2 => Elements {
            n: 49.5574 + 2.11081e-5 * d,
            i: 1.8497 - 1.78e-8 * d,
            w: 286.5016 + 2.92961e-5 * d,
            a: 1.523688,
            e: 0.093405 + 2.516e-9 * d,
            m: 18.6021 + 0.524_020_776_6 * d,
        },
        3 => Elements {
            n: 100.4542 + 2.76854e-5 * d,
            i: 1.3030 - 1.557e-7 * d,
            w: 273.8777 + 1.64505e-5 * d,
            a: 5.20256,
            e: 0.048498 + 4.469e-9 * d,
            m: 19.8950 + 0.083_085_300_1 * d,
        },
        4 => Elements {
            n: 113.6634 + 2.38980e-5 * d,
            i: 2.4886 - 1.081e-7 * d,
            w: 339.3939 + 2.97661e-5 * d,
            a: 9.55475,
            e: 0.055546 - 9.499e-9 * d,
            m: 316.9670 + 0.033_444_228_2 * d,
        },
        5 => Elements {
            n: 74.0005 + 1.3978e-5 * d,
            i: 0.7733 + 1.9e-8 * d,
            w: 96.6612 + 3.0565e-5 * d,
            a: 19.18171 - 1.55e-8 * d,
            e: 0.047318 + 7.45e-9 * d,
            m: 142.5905 + 0.011_725_806 * d,
        },
        _ => Elements {
            n: 131.7806 + 3.0173e-5 * d,
            i: 1.7700 - 2.55e-7 * d,
            w: 272.8461 - 6.027e-6 * d,
            a: 30.05826 + 3.313e-8 * d,
            e: 0.008606 + 2.15e-9 * d,
            m: 260.2471 + 0.005_995_147 * d,
        },
    }
}

/// Heliocentric ecliptic planet vector (AU), with Jupiter/Saturn great-inequality terms.
fn planet_helio(idx: usize, d: f64) -> V3 {
    let el = planet_elements(idx, d);
    let p = position_from_elements(&el);
    if idx != 3 && idx != 4 {
        return p;
    }
    let mj = planet_elements(3, d).m;
    let ms = planet_elements(4, d).m;
    let mut lon = p.y.atan2(p.x).to_degrees();
    let mut lat = p.z.atan2((p.x * p.x + p.y * p.y).sqrt()).to_degrees();
    let r = p.norm();
    if idx == 3 {
        lon += -0.332 * sind(2.0 * mj - 5.0 * ms - 67.6) - 0.056 * sind(2.0 * mj - 2.0 * ms + 21.0)
            + 0.042 * sind(3.0 * mj - 5.0 * ms + 21.0)
            - 0.036 * sind(mj - 2.0 * ms)
            + 0.022 * cosd(mj - ms)
            + 0.023 * sind(2.0 * mj - 3.0 * ms + 52.0)
            - 0.016 * sind(mj - 5.0 * ms - 69.0);
    } else {
        lon += 0.812 * sind(2.0 * mj - 5.0 * ms - 67.6) - 0.229 * cosd(2.0 * mj - 4.0 * ms - 2.0)
            + 0.119 * sind(mj - 2.0 * ms - 3.0)
            + 0.046 * sind(2.0 * mj - 6.0 * ms - 69.0)
            + 0.014 * sind(mj - 3.0 * ms + 32.0);
        lat += -0.020 * cosd(2.0 * mj - 4.0 * ms - 2.0) + 0.018 * sind(2.0 * mj - 6.0 * ms - 49.0);
    }
    V3::new(r * cosd(lat) * cosd(lon), r * cosd(lat) * sind(lon), r * sind(lat))
}

/// Equinox-of-date ecliptic → J2000 equatorial.
fn ecl_date_to_eq2000(v: V3, d: f64) -> V3 {
    // Precess ecliptic longitude back to J2000 (Schlyter's approximation),
    // then rotate by the J2000 obliquity.
    let v = v.rot_z(-rad(3.823_94e-5 * d));
    v.rot_x(rad(OBLIQUITY_J2000_DEG))
}

pub const BODY_NAMES: [&str; 9] = [
    "Sun", "Moon", "Mercury", "Venus", "Mars", "Jupiter", "Saturn", "Uranus", "Neptune",
];

#[derive(Clone, Copy, Debug)]
pub struct Body {
    /// Unit vector, geocentric J2000 equatorial.
    pub dir: V3,
    /// Geocentric distance, km.
    pub dist_km: f64,
    /// Physical radius, km (for apparent size).
    pub radius_km: f64,
}

const BODY_RADII_KM: [f64; 9] = [
    695_700.0, 1737.4, 2439.7, 6051.8, 3389.5, 69_911.0, 58_232.0, 25_362.0, 24_622.0,
];

/// Geocentric positions of the Sun, Moon and planets.
pub fn bodies(jd: f64) -> [Body; 9] {
    let d = day_number(jd);
    let sun_ecl = sun_ecliptic(d); // AU, geocentric
    let mut out = [Body {
        dir: V3::Z,
        dist_km: 1.0,
        radius_km: 1.0,
    }; 9];
    let sun = ecl_date_to_eq2000(sun_ecl, d);
    out[0] = Body {
        dir: sun.unit(),
        dist_km: sun.norm() * AU_KM,
        radius_km: BODY_RADII_KM[0],
    };
    let moon = ecl_date_to_eq2000(moon_ecliptic(d), d);
    out[1] = Body {
        dir: moon.unit(),
        dist_km: moon.norm() * EARTH_RADIUS_KM,
        radius_km: BODY_RADII_KM[1],
    };
    for p in 0..7 {
        let geo = planet_helio(p, d) + sun_ecl;
        let eq = ecl_date_to_eq2000(geo, d);
        out[p + 2] = Body {
            dir: eq.unit(),
            dist_km: eq.norm() * AU_KM,
            radius_km: BODY_RADII_KM[p + 2],
        };
    }
    out
}

/// Unit vector of the ecliptic at ecliptic longitude `lon` (J2000).
pub fn ecliptic_point(lon: f64) -> V3 {
    V3::new(lon.cos(), lon.sin(), 0.0).rot_x(rad(OBLIQUITY_J2000_DEG))
}

/// Earth-fixed (lon, lat in degrees, on the surface) → ECI unit-sphere point scaled by `r`.
pub fn ecef_to_eci(lon_deg: f64, lat_deg: f64, r: f64, gmst: f64) -> V3 {
    let p = V3::from_radec(rad(lon_deg), rad(lat_deg)) * r;
    p.rot_z(gmst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::deg;

    #[test]
    fn gmst_at_j2000() {
        assert!((deg(gmst(JD_J2000)) - 280.46061837).abs() < 1e-6);
    }

    #[test]
    fn calendar_roundtrip() {
        // 2026-11-14 00:00 UTC = unix 1794614400
        let jd = jd_from_unix(1_794_614_400.0);
        assert_eq!(calendar(jd), (2026, 11, 14, 0, 0, 0));
    }

    #[test]
    fn sun_near_equinox() {
        // March equinox 2026: 2026-03-20 14:46 UTC → Sun RA ≈ 0, Dec ≈ 0.
        let jd = jd_from_unix(1_774_017_960.0);
        // At the equinox the Sun sits on the equinox *of date*; in the J2000
        // frame that is ~26 yr × 50.3″/yr ≈ 0.36° of ecliptic longitude
        // behind, i.e. RA(J2000) ≈ −0.33°.
        let (ra, dec) = bodies(jd)[0].dir.to_radec();
        let ra_j2000 = deg(ra) - 360.0;
        assert!((ra_j2000 + 0.33).abs() < 0.05, "ra {ra_j2000}");
        assert!((deg(dec) + 0.14).abs() < 0.05, "dec {}", deg(dec));
    }

    #[test]
    fn moon_distance_sane() {
        let b = bodies(jd_from_unix(1_794_614_400.0));
        assert!(b[1].dist_km > 356_000.0 && b[1].dist_km < 407_000.0);
    }

    #[test]
    fn planets_near_ecliptic_and_sane_distance() {
        let b = bodies(jd_from_unix(1_794_614_400.0));
        let ecl_pole = V3::Z.rot_x(rad(OBLIQUITY_J2000_DEG));
        for (i, body) in b.iter().enumerate().skip(2) {
            let lat = deg(body.dir.dot(ecl_pole).asin());
            assert!(lat.abs() < 8.0, "{} lat {lat}", BODY_NAMES[i]);
        }
        // Jupiter geocentric distance always 3.9–6.5 AU
        let j = b[5].dist_km / AU_KM;
        assert!(j > 3.9 && j < 6.5);
    }
}

#[cfg(test)]
mod reference {
    use super::*;
    use crate::math::deg;
    /// GCRS RA/Dec from astropy 7 'builtin' (ERFA) ephemeris, 2026-11-14 00:00 UTC.
    const REF: [(f64, f64); 9] = [
        (228.818, -18.069),
        (284.704, -26.110),
        (213.113, -11.099),
        (200.167, -10.292),
        (147.438, 15.135),
        (147.837, 13.775),
        (8.576, 0.815),
        (61.818, 20.759),
        (1.908, -0.708),
    ];
    #[test]
    fn against_erfa() {
        let b = bodies(jd_from_unix(1_794_614_400.0));
        for (i, (ra, dec)) in REF.iter().enumerate() {
            let sep = deg(b[i].dir.angle(V3::from_radec(ra.to_radians(), dec.to_radians())));
            println!("{:8} sep {:.3}°", BODY_NAMES[i], sep);
            // Moon gets a looser bound (truncated series + no topocentric terms).
            let tol = if i == 1 { 0.3 } else { 0.08 };
            assert!(sep < tol, "{} off by {sep}°", BODY_NAMES[i]);
        }
    }
}
