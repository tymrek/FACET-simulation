//! Export the current shell as Two-Line Element sets, so it can be loaded into
//! Stellarium's Satellites plugin (or any SGP4 tool).
//!
//! Caveats: elements are our J2000-frame circular mean elements written as if
//! they were TEME/Brouwer mean elements (≈0.4° RAAN frame offset, and SGP4's
//! own J2 model will drift slightly differently). Fine for visualisation.

use crate::astro::calendar;
use crate::constellation::Shell;

fn checksum(line: &str) -> u32 {
    line.chars()
        .map(|c| match c {
            '0'..='9' => c as u32 - '0' as u32,
            '-' => 1,
            _ => 0,
        })
        .sum::<u32>()
        % 10
}

/// Julian date of 0h UTC on a Gregorian date (valid 1901–2099).
fn jd_ymd(y: i64, m: i64, d: i64) -> f64 {
    (367 * y - (7 * (y + (m + 9) / 12)) / 4 + (275 * m) / 9 + d) as f64 + 1_721_013.5
}

pub fn shell_to_tle(shell: &Shell, jd: f64) -> String {
    let (inc, mm, els) = shell.elements_at(jd);
    let (y, _, _, _, _, _) = calendar(jd);
    let doy = jd - jd_ymd(y, 1, 1) + 1.0;
    let mut out = String::with_capacity(els.len() * 170);
    for (i, (raan, u)) in els.iter().enumerate() {
        let num = 90_000 + i; // 90000–99999: unassigned-looking analyst range
        let l1 = format!(
            "1 {:05}U {:<8} {:02}{:012.8} {:>10} {:>8} {:>8} 0 {:>4}",
            num, "26999A", y % 100, doy, " .00000000", " 00000-0", " 00000-0", 999
        );
        let l2 = format!(
            "2 {:05} {:8.4} {:8.4} {:07} {:8.4} {:8.4} {:11.8}{:05}",
            num, inc, raan, 0, 0.0, u, mm, 0
        );
        out.push_str(&format!("FACET-{:05}\n", i + 1));
        out.push_str(&format!("{l1}{}\n", checksum(&l1)));
        out.push_str(&format!("{l2}{}\n", checksum(&l2)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constellation::ShellParams;

    #[test]
    fn iss_checksum() {
        assert_eq!(checksum("1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  292"), 7);
        assert_eq!(checksum("2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537"[..68].as_ref()), 7);
    }

    #[test]
    fn line_lengths() {
        let s = Shell::new(ShellParams { n: 10_000, ..Default::default() }, 2_461_358.5);
        let t = shell_to_tle(&s, 2_461_358.75);
        for (k, l) in t.lines().enumerate() {
            if k % 3 != 0 {
                assert_eq!(l.len(), 69, "{l}");
            }
        }
        assert!(t.contains("FACET-10000"));
    }
}

