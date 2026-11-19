//! Fourier-optics point-spread function.
//!
//! The PSF is computed from first principles, Fraunhofer diffraction of the
//! actual pupil, instead of a Gaussian stand-in:
//!
//!   * Pupil: circular aperture D, central obstruction ε (secondary mirror),
//!     N spider vanes of width w holding the secondary.
//!   * Wavefront error: a fixed mix of low-order Zernike terms (defocus,
//!     astigmatism, coma, trefoil, spherical) scaled to the specified RMS —
//!     a stand-in for manufacturing/alignment/thermal error of small optics.
//!   * Polychromatic: monochromatic PSFs |FFT(A·e^{i2πW/λ})|² at 7
//!     wavelengths across 400–700 nm, rescaled to a common angular grid and
//!     averaged (flat photon spectrum × flat QE).
//!   * Pointing jitter: convolved with a 2-D Gaussian of the RMS jitter.
//!
//! The result is cached per optical configuration; FWHM, Strehl ratio
//! (Maréchal, band-averaged) and encircled energy are measured from it.

use crate::imaging::Hardware;
use rustfft::{num_complex::Complex, FftPlanner};
use std::collections::HashMap;
use std::f64::consts::PI;
use std::sync::{Arc, Mutex, OnceLock};

const ARCSEC: f64 = 206_264.806;
/// Pupil diameter in grid samples, and FFT size (padding ⇒ sampling).
const NP: usize = 128;
const NFFT: usize = 512;
/// Common output grid.
const M: usize = 384;
const LAMBDAS_NM: [f64; 7] = [420.0, 465.0, 510.0, 555.0, 600.0, 645.0, 690.0];

pub struct Psf {
    /// arcsec per fine-grid pixel
    pub scale: f64,
    pub n: usize,
    /// Normalised to unit sum, centre at (n/2, n/2).
    pub data: Vec<f64>,
    pub strehl: f64,
    pub fwhm: f64,
    pub ee50_r: f64,
    pub ee80_r: f64,
    /// Encircled energy profile: ee[k] = fraction within radius (k+0.5)·scale.
    pub ee: Vec<f64>,
}

impl Psf {
    /// Encircled energy within radius r (arcsec).
    pub fn ee_at(&self, r: f64) -> f64 {
        let k = (r / self.scale - 0.5).max(0.0);
        let i = k.floor() as usize;
        if i + 1 >= self.ee.len() {
            return *self.ee.last().unwrap_or(&1.0);
        }
        let f = k - i as f64;
        self.ee[i] * (1.0 - f) + self.ee[i + 1] * f
    }

    /// Bilinear sample of the (unit-sum) fine PSF at offset (dx, dy) arcsec.
    fn sample(&self, dx: f64, dy: f64) -> f64 {
        let c = self.n as f64 / 2.0;
        let x = dx / self.scale + c;
        let y = dy / self.scale + c;
        if x < 0.0 || y < 0.0 || x >= (self.n - 1) as f64 || y >= (self.n - 1) as f64 {
            return 0.0;
        }
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let g = |i: usize, j: usize| self.data[j * self.n + i];
        g(x0, y0) * (1.0 - fx) * (1.0 - fy)
            + g(x0 + 1, y0) * fx * (1.0 - fy)
            + g(x0, y0 + 1) * (1.0 - fx) * fy
            + g(x0 + 1, y0 + 1) * fx * fy
    }

    /// The PSF integrated over pixels of size `scale` arcsec: an odd-sized,
    /// unit-sum kernel for convolving a scene rendered on that grid.
    pub fn kernel(&self, scale: f64) -> (usize, Vec<f64>) {
        let half_field = self.n as f64 / 2.0 * self.scale;
        let half = ((half_field / scale).floor() as usize).max(1);
        let k = 2 * half + 1;
        let mut ker = vec![0.0; k * k];
        // Integrate each output pixel with an s×s sub-grid of samples.
        let s = ((scale / self.scale).ceil() as usize).clamp(1, 12);
        for j in 0..k {
            for i in 0..k {
                let cx = (i as f64 - half as f64) * scale;
                let cy = (j as f64 - half as f64) * scale;
                let mut acc = 0.0;
                for b in 0..s {
                    for a in 0..s {
                        let dx = cx + ((a as f64 + 0.5) / s as f64 - 0.5) * scale;
                        let dy = cy + ((b as f64 + 0.5) / s as f64 - 0.5) * scale;
                        acc += self.sample(dx, dy);
                    }
                }
                ker[j * k + i] = acc;
            }
        }
        let sum: f64 = ker.iter().sum();
        if sum > 0.0 {
            ker.iter_mut().for_each(|v| *v /= sum);
        }
        (k, ker)
    }

    /// Log-stretched 8-bit image of the fine PSF for reports.
    pub fn png(&self) -> Vec<u8> {
        let peak = self.data.iter().cloned().fold(0.0, f64::max).max(1e-300);
        let px: Vec<u8> = self
            .data
            .iter()
            .map(|&v| {
                let l = (v / peak).max(1e-6).log10(); // 0 … -6
                (((l + 6.0) / 6.0).clamp(0.0, 1.0) * 255.0) as u8
            })
            .collect();
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.n as u32, self.n as u32);
            enc.set_color(png::ColorType::Grayscale);
            enc.set_depth(png::BitDepth::Eight);
            if let Ok(mut w) = enc.write_header() {
                let _ = w.write_image_data(&px);
            }
        }
        out
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
struct Key {
    d_um: u64,
    eps_m: u64,
    spider_um: u64,
    vanes: u32,
    wfe_pm: u64,
    jitter_mas: u64,
}

static CACHE: OnceLock<Mutex<HashMap<Key, Arc<Psf>>>> = OnceLock::new();

/// The effective PSF for this hardware with the given RMS pointing jitter (arcsec).
pub fn psf(hw: &Hardware, jitter_arcsec: f64) -> Arc<Psf> {
    let key = Key {
        d_um: (hw.aperture_cm * 1e4).round() as u64,
        eps_m: (hw.obstruction * 1e3).round() as u64,
        spider_um: (hw.spider_mm * 1e3).round() as u64,
        vanes: hw.spider_vanes,
        wfe_pm: (hw.wfe_nm * 1e3).round() as u64,
        jitter_mas: (jitter_arcsec * 1e3).round() as u64,
    };
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(p) = cache.lock().unwrap().get(&key) {
        return p.clone();
    }
    let p = Arc::new(compute(hw, jitter_arcsec));
    let mut c = cache.lock().unwrap();
    if c.len() > 64 {
        c.clear();
    }
    c.insert(key, p.clone());
    p
}

/// Zernike mix (Noll 4–11) standing in for small-telescope figure/alignment error.
fn wavefront(rho: f64, th: f64) -> f64 {
    let z4 = 3f64.sqrt() * (2.0 * rho * rho - 1.0);
    let z5 = 6f64.sqrt() * rho * rho * (2.0 * th).sin();
    let z6 = 6f64.sqrt() * rho * rho * (2.0 * th).cos();
    let z7 = 8f64.sqrt() * (3.0 * rho.powi(3) - 2.0 * rho) * th.sin();
    let z8 = 8f64.sqrt() * (3.0 * rho.powi(3) - 2.0 * rho) * th.cos();
    let z9 = 8f64.sqrt() * rho.powi(3) * (3.0 * th).sin();
    let z10 = 8f64.sqrt() * rho.powi(3) * (3.0 * th).cos();
    let z11 = 5f64.sqrt() * (6.0 * rho.powi(4) - 6.0 * rho * rho + 1.0);
    0.30 * z4 + 0.45 * z5 + 0.25 * z6 + 0.35 * z7 + 0.20 * z8 + 0.15 * z9 + 0.10 * z10 + 0.30 * z11
}

fn in_spider(x: f64, y: f64, vanes: u32, half_w: f64) -> bool {
    if vanes == 0 {
        return false;
    }
    for k in 0..vanes {
        let a = k as f64 * 2.0 * PI / vanes as f64;
        let (ux, uy) = (a.cos(), a.sin());
        let along = x * ux + y * uy;
        let across = (-x * uy + y * ux).abs();
        if along > 0.0 && across < half_w {
            return true;
        }
    }
    false
}

fn compute(hw: &Hardware, jitter: f64) -> Psf {
    let d_m = hw.aperture_cm / 100.0;
    let eps = hw.obstruction.clamp(0.0, 0.9);
    // Spider half-width in units of the pupil radius
    let half_w = (hw.spider_mm / 1000.0) / d_m;
    // Pupil amplitude and unit-RMS wavefront shape over the clear aperture.
    let mut amp = vec![0.0f64; NP * NP];
    let mut wshape = vec![0.0f64; NP * NP];
    let (mut s1, mut s2, mut cnt) = (0.0, 0.0, 0.0);
    // Anti-aliased pupil: fractional transmission from 4×4 sub-samples per
    // grid cell, so thin vanes and edges diffract correctly.
    const SS: usize = 4;
    for j in 0..NP {
        for i in 0..NP {
            let mut t = 0.0;
            for b in 0..SS {
                for a in 0..SS {
                    let x = (i as f64 + (a as f64 + 0.5) / SS as f64) / NP as f64 * 2.0 - 1.0;
                    let y = (j as f64 + (b as f64 + 0.5) / SS as f64) / NP as f64 * 2.0 - 1.0;
                    let rho = x.hypot(y);
                    if rho <= 1.0 && rho >= eps && !in_spider(x, y, hw.spider_vanes, half_w) {
                        t += 1.0;
                    }
                }
            }
            let t = t / (SS * SS) as f64;
            if t > 0.0 {
                let x = (i as f64 + 0.5) / NP as f64 * 2.0 - 1.0;
                let y = (j as f64 + 0.5) / NP as f64 * 2.0 - 1.0;
                let rho = x.hypot(y).min(1.0);
                amp[j * NP + i] = t;
                let w = wavefront(rho, y.atan2(x));
                wshape[j * NP + i] = w;
                s1 += w * t;
                s2 += w * w * t;
                cnt += t;
            }
        }
    }
    // Remove piston, scale to the requested RMS (nm).
    let mean = s1 / cnt;
    let rms = ((s2 / cnt) - mean * mean).max(1e-12).sqrt();
    let wfe_nm: Vec<f64> = wshape.iter().map(|w| (w - mean) / rms * hw.wfe_nm).collect();

    // Common output grid: finest natural sampling (bluest λ).
    let scale = LAMBDAS_NM[0] * 1e-9 / d_m * (NP as f64 / NFFT as f64) * ARCSEC;
    let mut acc = vec![0.0f64; M * M];
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(NFFT);
    let mut buf = vec![Complex::new(0.0, 0.0); NFFT * NFFT];
    let mut col = vec![Complex::new(0.0, 0.0); NFFT];
    let mut strehl = 0.0;
    for &lam in &LAMBDAS_NM {
        buf.iter_mut().for_each(|c| *c = Complex::new(0.0, 0.0));
        let off = (NFFT - NP) / 2;
        for j in 0..NP {
            for i in 0..NP {
                let a = amp[j * NP + i];
                if a > 0.0 {
                    let ph = 2.0 * PI * wfe_nm[j * NP + i] / lam;
                    buf[(j + off) * NFFT + i + off] = Complex::from_polar(a, ph);
                }
            }
        }
        for row in buf.chunks_mut(NFFT) {
            fft.process(row);
        }
        for i in 0..NFFT {
            for j in 0..NFFT {
                col[j] = buf[j * NFFT + i];
            }
            fft.process(&mut col);
            for j in 0..NFFT {
                buf[j * NFFT + i] = col[j];
            }
        }
        // |E|², fftshift, normalise.
        let mut inten = vec![0.0f64; NFFT * NFFT];
        let h = NFFT / 2;
        for j in 0..NFFT {
            for i in 0..NFFT {
                let v = buf[j * NFFT + i].norm_sqr();
                inten[((j + h) % NFFT) * NFFT + (i + h) % NFFT] = v;
            }
        }
        let tot: f64 = inten.iter().sum();
        inten.iter_mut().for_each(|v| *v /= tot);
        // Resample onto the common grid (this λ's natural pixel is λ/λ₀ × larger).
        let dl = lam * 1e-9 / d_m * (NP as f64 / NFFT as f64) * ARCSEC;
        let mut tmp = vec![0.0f64; M * M];
        let c = NFFT as f64 / 2.0;
        for j in 0..M {
            for i in 0..M {
                let dx = (i as f64 - M as f64 / 2.0) * scale;
                let dy = (j as f64 - M as f64 / 2.0) * scale;
                let x = dx / dl + c;
                let y = dy / dl + c;
                let (x0, y0) = (x.floor() as usize, y.floor() as usize);
                if x0 + 1 >= NFFT || y0 + 1 >= NFFT {
                    continue;
                }
                let (fx, fy) = (x - x0 as f64, y - y0 as f64);
                let g = |a: usize, b: usize| inten[b * NFFT + a];
                tmp[j * M + i] = g(x0, y0) * (1.0 - fx) * (1.0 - fy)
                    + g(x0 + 1, y0) * fx * (1.0 - fy)
                    + g(x0, y0 + 1) * (1.0 - fx) * fy
                    + g(x0 + 1, y0 + 1) * fx * fy;
            }
        }
        let ts: f64 = tmp.iter().sum();
        if ts > 0.0 {
            for (a, t) in acc.iter_mut().zip(&tmp) {
                *a += t / ts / LAMBDAS_NM.len() as f64;
            }
        }
        strehl += (-(2.0 * PI * hw.wfe_nm / lam).powi(2)).exp() / LAMBDAS_NM.len() as f64;
    }

    // Pointing jitter: Gaussian blur (σ per axis = RMS jitter).
    gaussian_blur(&mut acc, M, jitter / scale);

    // Circular apodisation of the finite field so very bright, saturated
    // stars don't show the square edge of the computed PSF.
    let c = M as f64 / 2.0;
    for j in 0..M {
        for i in 0..M {
            let r = (i as f64 - c).hypot(j as f64 - c) / c;
            let w = if r < 0.75 { 1.0 } else if r >= 1.0 { 0.0 } else { (0.5 * (1.0 + (PI * (r - 0.75) / 0.25).cos())).powi(2) };
            acc[j * M + i] *= w;
        }
    }
    let s: f64 = acc.iter().sum();
    acc.iter_mut().for_each(|v| *v /= s);

    let mut p = Psf { scale, n: M, data: acc, strehl, fwhm: 0.0, ee50_r: 0.0, ee80_r: 0.0, ee: Vec::new() };
    measure(&mut p);
    p
}

fn gaussian_blur(img: &mut [f64], n: usize, sigma: f64) {
    if sigma < 0.2 {
        return;
    }
    let r = (4.0 * sigma).ceil() as i64;
    let ker: Vec<f64> = (-r..=r).map(|k| (-(k * k) as f64 / (2.0 * sigma * sigma)).exp()).collect();
    let ks: f64 = ker.iter().sum();
    let mut tmp = vec![0.0; n * n];
    for y in 0..n {
        for x in 0..n {
            let mut a = 0.0;
            for (k, kv) in ker.iter().enumerate() {
                let xx = x as i64 + k as i64 - r;
                if xx >= 0 && (xx as usize) < n {
                    a += img[y * n + xx as usize] * kv;
                }
            }
            tmp[y * n + x] = a / ks;
        }
    }
    for y in 0..n {
        for x in 0..n {
            let mut a = 0.0;
            for (k, kv) in ker.iter().enumerate() {
                let yy = y as i64 + k as i64 - r;
                if yy >= 0 && (yy as usize) < n {
                    a += tmp[yy as usize * n + x] * kv;
                }
            }
            img[y * n + x] = a / ks;
        }
    }
    let s: f64 = img.iter().sum();
    if s > 0.0 {
        img.iter_mut().for_each(|v| *v /= s);
    }
}

fn measure(p: &mut Psf) {
    let n = p.n;
    let c = n as f64 / 2.0;
    let nr = n / 2;
    let mut ring = vec![0.0f64; nr + 1];
    let mut cnt = vec![0.0f64; nr + 1];
    for j in 0..n {
        for i in 0..n {
            let r = ((i as f64 - c).hypot(j as f64 - c)).round() as usize;
            if r <= nr {
                ring[r] += p.data[j * n + i];
                cnt[r] += 1.0;
            }
        }
    }
    // Encircled energy
    let mut ee = Vec::with_capacity(nr + 1);
    let mut cum = 0.0;
    for r in &ring {
        cum += r;
        ee.push(cum);
    }
    // FWHM from the azimuthal mean profile
    let prof: Vec<f64> = ring.iter().zip(&cnt).map(|(s, c)| if *c > 0.0 { s / c } else { 0.0 }).collect();
    let peak = p.data[(n / 2) * n + n / 2].max(prof[0]);
    let mut half_r = nr as f64;
    for k in 1..prof.len() {
        if prof[k] < peak / 2.0 {
            let (a, b) = (prof[k - 1], prof[k]);
            let f = if a > b { (a - peak / 2.0) / (a - b) } else { 0.0 };
            half_r = (k - 1) as f64 + f;
            break;
        }
    }
    p.fwhm = 2.0 * half_r * p.scale;
    let rad = |target: f64| ee.iter().position(|&e| e >= target).unwrap_or(nr) as f64 * p.scale;
    p.ee50_r = rad(0.5);
    p.ee80_r = rad(0.8);
    p.ee = ee;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obs::ObsParams;

    fn hw(wfe: f64, eps: f64, vanes: u32) -> Hardware {
        let mut h = Hardware::from_obs(&ObsParams::default());
        h.wfe_nm = wfe;
        h.obstruction = eps;
        h.spider_vanes = vanes;
        h
    }

    #[test]
    fn perfect_aperture_fwhm_matches_theory() {
        // Unobstructed, aberration-free 20 cm: FWHM ≈ 1.03 λ/D at the band centre (≈0.58″)
        let p = psf(&hw(0.0, 0.0, 0), 0.0);
        assert!((p.fwhm - 0.58).abs() < 0.08, "fwhm {}", p.fwhm);
        assert!((p.strehl - 1.0).abs() < 1e-9);
        // Airy: ~84 % of the light inside the first dark ring (1.22 λ/D ≈ 0.69″ at 555 nm)
        let e = p.ee_at(0.69);
        assert!(e > 0.74 && e < 0.88, "EE(first ring) {e}");
    }

    #[test]
    fn obstruction_and_wfe_degrade_as_expected() {
        let clean = psf(&hw(0.0, 0.0, 0), 0.0);
        let obst = psf(&hw(0.0, 0.33, 0), 0.0);
        // Obstruction moves light from core into the rings
        assert!(obst.ee_at(0.69) < clean.ee_at(0.69) - 0.05);
        let aber = psf(&hw(60.0, 0.33, 4), 0.0);
        // Maréchal Strehl for 60 nm RMS averaged across 420–690 nm ≈ 0.6
        assert!(aber.strehl > 0.5 && aber.strehl < 0.72, "strehl {}", aber.strehl);
        assert!(aber.ee80_r > obst.ee80_r);
    }

    #[test]
    fn spiders_make_spikes() {
        // Compare light along the spike directions with the azimuthal mean at
        // the same radius, well outside the core (Airy halo falls as r⁻³,
        // spikes as r⁻²).
        let p = psf(&hw(0.0, 0.33, 4), 0.0);
        let without = psf(&hw(0.0, 0.33, 0), 0.0);
        let ratio = |p: &Psf, r: f64| {
            let n = p.n as f64;
            let c = n / 2.0;
            let at = |a: f64| {
                let (x, y) = ((c + r * a.cos()) as usize, (c + r * a.sin()) as usize);
                p.data[y * p.n + x]
            };
            let spike: f64 = (0..4).map(|k| at(k as f64 * PI / 2.0)).sum::<f64>() / 4.0;
            let ring: f64 = (0..360).map(|k| at(k as f64 * PI / 180.0)).sum::<f64>() / 360.0;
            spike / ring
        };
        let r = 100.0; // ≈11″
        let with_spider = ratio(&p, r);
        let no_spider = ratio(&without, r);
        println!("spike/ring at 11″: with spiders {with_spider:.1}, without {no_spider:.2}");
        assert!(with_spider > 5.0 && no_spider < 2.0, "{with_spider} {no_spider}");
    }

    #[test]
    fn jitter_broadens() {
        let a = psf(&hw(60.0, 0.33, 4), 0.0);
        let b = psf(&hw(60.0, 0.33, 4), 1.0);
        assert!(b.fwhm > 2.0 && b.fwhm < 3.0, "{}", b.fwhm); // ≈ √(0.7² + 2.35²)
        assert!(b.fwhm > a.fwhm);
    }

    #[test]
    fn kernel_is_normalised() {
        let p = psf(&hw(60.0, 0.33, 4), 1.0);
        for s in [0.05, 0.376, 5.0] {
            let (_, k) = p.kernel(s);
            let sum: f64 = k.iter().sum();
            assert!((sum - 1.0).abs() < 1e-9);
        }
    }
}

/// Linear 2-D convolution of `img` (w×h) with an odd k×k kernel, same-size
/// output, via zero-padded FFTs.
pub fn convolve(img: &[f64], w: usize, h: usize, ker: &[f64], k: usize) -> Vec<f64> {
    if k == 1 {
        return img.iter().map(|v| v * ker[0]).collect();
    }
    let pw = (w + k).next_power_of_two();
    let ph = (h + k).next_power_of_two();
    let mut planner = FftPlanner::<f64>::new();
    let fw = planner.plan_fft_forward(pw);
    let fh = planner.plan_fft_forward(ph);
    let iw = planner.plan_fft_inverse(pw);
    let ih = planner.plan_fft_inverse(ph);
    let fft2 = |buf: &mut Vec<Complex<f64>>, rows: &Arc<dyn rustfft::Fft<f64>>, cols: &Arc<dyn rustfft::Fft<f64>>| {
        for row in buf.chunks_mut(pw) {
            rows.process(row);
        }
        let mut col = vec![Complex::new(0.0, 0.0); ph];
        for i in 0..pw {
            for j in 0..ph {
                col[j] = buf[j * pw + i];
            }
            cols.process(&mut col);
            for j in 0..ph {
                buf[j * pw + i] = col[j];
            }
        }
    };
    let mut a = vec![Complex::new(0.0, 0.0); pw * ph];
    for y in 0..h {
        for x in 0..w {
            a[y * pw + x] = Complex::new(img[y * w + x], 0.0);
        }
    }
    // Kernel centred at the origin with wrap-around.
    let mut b = vec![Complex::new(0.0, 0.0); pw * ph];
    let half = (k / 2) as i64;
    for j in 0..k {
        for i in 0..k {
            let x = (i as i64 - half).rem_euclid(pw as i64) as usize;
            let y = (j as i64 - half).rem_euclid(ph as i64) as usize;
            b[y * pw + x] = Complex::new(ker[j * k + i], 0.0);
        }
    }
    fft2(&mut a, &fw, &fh);
    fft2(&mut b, &fw, &fh);
    for (x, y) in a.iter_mut().zip(&b) {
        *x *= y;
    }
    fft2(&mut a, &iw, &ih);
    let norm = (pw * ph) as f64;
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = (a[y * pw + x].re / norm).max(0.0);
        }
    }
    out
}

#[cfg(test)]
mod conv_tests {
    #[test]
    fn convolution_conserves_flux_and_centres() {
        let (w, h) = (40, 30);
        let mut img = vec![0.0; w * h];
        img[15 * w + 20] = 100.0;
        let k = 5;
        let ker: Vec<f64> = (0..k * k).map(|i| if i == 12 { 0.5 } else { 0.5 / 24.0 }).collect();
        let out = super::convolve(&img, w, h, &ker, k);
        let s: f64 = out.iter().sum();
        assert!((s - 100.0).abs() < 1e-6, "{s}");
        assert!((out[15 * w + 20] - 50.0).abs() < 1e-6);
    }
}

// ------------------------------------------------------------ blur budget

/// Where the delivered image size comes from, term by term (arcsec FWHM).
#[derive(Clone, serde::Serialize)]
pub struct BlurBudget {
    /// Perfect optics: diffraction of the actual pupil, band-averaged.
    pub diffraction: f64,
    /// … plus the as-built wavefront error.
    pub optics: f64,
    pub strehl: f64,
    /// … plus pointing jitter over a sub-frame.
    pub with_jitter: f64,
    pub jitter_rms: f64,
    /// Pixel size, arcsec, and its Gaussian-equivalent FWHM (0.68 × pixel).
    pub pixel: f64,
    pub pixel_term: f64,
    /// On-axis delivered FWHM (optics + jitter ⊕ pixel).
    pub delivered: f64,
    /// Pixels per delivered FWHM.
    pub sampling: f64,
    /// Field diagonal and the design's corrected-field diameter, deg.
    pub field_diag_deg: f64,
    pub corrected_deg: f64,
    /// ASSUMED extra blur at the field corners when the sensor overfills the
    /// corrected field: 1.5″ × ((diagonal / corrected)² − 1).
    pub corner_extra: f64,
    pub corner_delivered: f64,
    /// Typical good ground seeing for comparison (the ground model's value).
    pub ground_seeing: f64,
}

pub fn blur_budget(o: &crate::obs::ObsParams, sub_s: f64) -> BlurBudget {
    let hw = Hardware::from_obs(o);
    let mut perfect = hw.clone();
    perfect.wfe_nm = 0.0;
    let diffraction = psf(&perfect, 0.0).fwhm;
    let p_opt = psf(&hw, 0.0);
    let jit = hw.jitter_for(sub_s);
    let p_all = psf(&hw, jit);
    let pixel = hw.pixel_scale();
    let pixel_term = 0.68 * pixel;
    let delivered = p_all.fwhm.hypot(pixel_term);
    let (_, _, diag) = o.field_deg();
    let corrected = o.corrected_field_deg.max(0.01);
    let corner_extra = if diag > corrected { 1.5 * ((diag / corrected).powi(2) - 1.0) } else { 0.0 };
    BlurBudget {
        diffraction,
        optics: p_opt.fwhm,
        strehl: p_opt.strehl,
        with_jitter: p_all.fwhm,
        jitter_rms: jit,
        pixel,
        pixel_term,
        delivered,
        sampling: delivered / pixel,
        field_diag_deg: diag,
        corrected_deg: corrected,
        corner_extra,
        corner_delivered: delivered.hypot(corner_extra),
        ground_seeing: crate::ground::COND.seeing_arcsec,
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use crate::obs::{ObsParams, DESIGNS, POINTING, SENSORS};

    #[test]
    fn default_budget_beats_ground_seeing() {
        let o = ObsParams::default();
        let b = blur_budget(&o, 60.0);
        eprintln!("default: diff {:.2} opt {:.2} (S {:.2}) jit {:.2} pix {:.2} -> {:.2} ({:.1} px) corner {:.2}", b.diffraction, b.optics, b.strehl, b.with_jitter, b.pixel, b.delivered, b.sampling, b.corner_delivered);
        assert!(b.delivered < b.ground_seeing && b.delivered > b.diffraction);
        assert!(b.corner_extra == 0.0);
    }

    #[test]
    #[ignore]
    fn table() {
        for (pk, p) in POINTING.iter().enumerate() {
            for (dk, d) in DESIGNS.iter().enumerate() {
                for (sk, s) in SENSORS.iter().enumerate() {
                    let mut o = ObsParams::default();
                    o.apply_design(dk);
                    o.apply_sensor(sk);
                    o.apply_pointing(pk);
                    let b = blur_budget(&o, 60.0);
                    let hw = crate::imaging::Hardware::from_obs(&o);
                    let lim = crate::imaging::point_lim_mag(&hw, 60.0, 60.0, 22.0);
                    let (w, h, _) = o.field_deg();
                    eprintln!("{:4} {:5} {:10} f/{:.0} {:.2}\"/px field {:.2}x{:.2} ({:.2}) FWHM {:.2} ({:.1}px) corner {:.2} V60 {:.2}", p.0, d.key, s.key, o.focal_mm / 200.0, o.plate_scale(), w, h, w * h, b.delivered, b.sampling, b.corner_delivered, lim);
                }
            }
        }
    }
}
