//! ReplayGain measurement: BS.1770-style K-weighted loudness with
//! absolute + relative gating over 400 ms blocks, plus the sample
//! peak. The reference level is −18 LUFS (ReplayGain 2.0), so
//! `gain_db = −18 − loudness`.

/// One channel's K-weighting filter chain (shelf, then high-pass).
struct KFilter {
    shelf_b: [f64; 3],
    shelf_a: [f64; 2],
    hp_b: [f64; 3],
    hp_a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
    x2: [f64; 2],
    y2: [f64; 2],
}

impl KFilter {
    fn new(sample_rate: u32) -> Self {
        let fs = sample_rate as f64;
        // Pre-filter (shelf), BS.1770 coefficients by formula.
        let (b, a) = {
            let f0 = 1681.974450955533;
            let g = 3.999843853973347;
            let q = 0.7071752369554196;
            let k = (std::f64::consts::PI * f0 / fs).tan();
            let vh = 10f64.powf(g / 20.0);
            let vb = vh.powf(0.4996667741545416);
            let a0 = 1.0 + k / q + k * k;
            (
                [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
                [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            )
        };
        // RLB high-pass.
        let (b2, a2) = {
            let f0 = 38.13547087602444;
            let q = 0.5003270373238773;
            let k = (std::f64::consts::PI * f0 / fs).tan();
            let a0 = 1.0 + k / q + k * k;
            ([1.0 / a0, -2.0 / a0, 1.0 / a0], [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0])
        };
        KFilter { shelf_b: b, shelf_a: a, hp_b: b2, hp_a: a2, x: [0.0; 2], y: [0.0; 2], x2: [0.0; 2], y2: [0.0; 2] }
    }

    fn step(&mut self, input: f64) -> f64 {
        let o = self.shelf_b[0] * input + self.shelf_b[1] * self.x[0] + self.shelf_b[2] * self.x[1]
            - self.shelf_a[0] * self.y[0]
            - self.shelf_a[1] * self.y[1];
        self.x[1] = self.x[0];
        self.x[0] = input;
        self.y[1] = self.y[0];
        self.y[0] = o;
        let o2 = self.hp_b[0] * o + self.hp_b[1] * self.x2[0] + self.hp_b[2] * self.x2[1]
            - self.hp_a[0] * self.y2[0]
            - self.hp_a[1] * self.y2[1];
        self.x2[1] = self.x2[0];
        self.x2[0] = o;
        self.y2[1] = self.y2[0];
        self.y2[0] = o2;
        o2
    }
}

/// The result of measuring one track.
#[derive(Debug, Clone, Copy)]
pub struct Measurement {
    /// Integrated loudness in LUFS.
    pub loudness: f32,
    /// ReplayGain track gain in dB (target −18 LUFS).
    pub gain_db: f32,
    /// Sample peak, linear (1.0 = full scale).
    pub peak: f32,
}

/// Measures interleaved `samples`. Silence (or near-silence) yields
/// a zero gain — there is nothing meaningful to normalize.
pub fn measure(samples: &[f32], channels: usize, sample_rate: u32) -> Measurement {
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    let mut peak = 0.0f32;
    for &s in samples {
        peak = peak.max(s.abs());
    }
    if frames == 0 || peak == 0.0 {
        return Measurement { loudness: -70.0, gain_db: 0.0, peak };
    }

    // Filter every channel and keep the filtered stream for blocks.
    let mut filtered = vec![0.0f64; frames * channels];
    for ch in 0..channels {
        let mut f = KFilter::new(sample_rate);
        for i in 0..frames {
            filtered[i * channels + ch] = f.step(samples[i * channels + ch] as f64);
        }
    }

    // 400 ms blocks, 100 ms hop. Per block: sum over channels of the
    // block's mean square.
    let block = ((sample_rate as usize * 400) / 1000).max(1);
    let hop = ((sample_rate as usize * 100) / 1000).max(1);
    let mut block_power: Vec<f64> = Vec::new();
    let mut start = 0;
    while start + block <= frames {
        let mut sum = 0.0;
        for ch in 0..channels {
            let mut acc = 0.0;
            for i in start..start + block {
                let v = filtered[i * channels + ch];
                acc += v * v;
            }
            sum += acc / block as f64;
        }
        block_power.push(sum);
        start += hop;
    }
    if block_power.is_empty() {
        // Shorter than one block: measure the whole thing as one block.
        let mut sum = 0.0;
        for ch in 0..channels {
            let mut acc = 0.0;
            for i in 0..frames {
                let v = filtered[i * channels + ch];
                acc += v * v;
            }
            sum += acc / frames as f64;
        }
        block_power.push(sum);
    }

    let loudness_of = |z: f64| -> f64 { -0.691 + 10.0 * z.log10() };
    // Absolute gate at −70 LUFS.
    let abs: Vec<f64> = block_power.iter().copied().filter(|&z| z > 0.0 && loudness_of(z) > -70.0).collect();
    if abs.is_empty() {
        return Measurement { loudness: -70.0, gain_db: 0.0, peak };
    }
    // Relative gate: −10 LU under the absolute-gated average.
    let avg = abs.iter().sum::<f64>() / abs.len() as f64;
    let rel_threshold = loudness_of(avg) - 10.0;
    let kept: Vec<f64> = abs.iter().copied().filter(|&z| loudness_of(z) > rel_threshold).collect();
    let z = if kept.is_empty() { avg } else { kept.iter().sum::<f64>() / kept.len() as f64 };
    let loudness = loudness_of(z) as f32;
    let gain_db = (-18.0 - loudness).clamp(-30.0, 30.0);
    Measurement { loudness, gain_db, peak }
}

/// Converts a dB gain to a linear multiplier that never pushes the
/// peak past full scale (cuts always apply in full).
pub fn gain_linear_peak_safe(gain_db: f32, peak: f32) -> f32 {
    let linear = 10f32.powf(gain_db / 20.0);
    if peak > 0.0 && peak * linear > 1.0 {
        1.0 / peak
    } else {
        linear
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, seconds: f32, rate: u32, amp: f32, channels: usize) -> Vec<f32> {
        let frames = (seconds * rate as f32) as usize;
        let mut v = Vec::with_capacity(frames * channels);
        for i in 0..frames {
            let s = amp * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin();
            for _ in 0..channels {
                v.push(s);
            }
        }
        v
    }

    #[test]
    fn full_scale_sine_measures_near_reference() {
        // A 0 dBFS 1 kHz stereo sine: the two channels' mean squares
        // (0.5 each) sum to 1.0, the −0.691 offset and the shelf's
        // small lift at 1 kHz put it just under 0 LUFS, so the
        // ReplayGain adjustment is ≈ −18 dB. The same sine in mono
        // measures ≈ 3 dB lower.
        let s = sine(1000.0, 2.0, 48_000, 1.0, 2);
        let m = measure(&s, 2, 48_000);
        assert!(m.peak > 0.99, "peak {}", m.peak);
        assert!(m.loudness > -1.2 && m.loudness < 0.3, "loudness {}", m.loudness);
        assert!(m.gain_db > -19.2 && m.gain_db < -17.7, "gain {}", m.gain_db);
        let mono = sine(1000.0, 2.0, 48_000, 1.0, 1);
        let mm = measure(&mono, 1, 48_000);
        assert!(mm.loudness > -4.2 && mm.loudness < -2.7, "mono loudness {}", mm.loudness);
    }

    #[test]
    fn silence_gets_no_gain() {
        let s = vec![0.0f32; 48_000];
        let m = measure(&s, 1, 48_000);
        assert_eq!(m.gain_db, 0.0);
        assert_eq!(m.peak, 0.0);
    }

    #[test]
    fn quiet_signal_gets_positive_gain() {
        let s = sine(1000.0, 2.0, 44_100, 0.05, 2);
        let m = measure(&s, 2, 44_100);
        assert!(m.gain_db > 5.0, "gain {}", m.gain_db);
    }

    #[test]
    fn peak_safe_caps_boost() {
        // +20 dB on a 0.9-peak signal must cap at 1/0.9.
        let l = gain_linear_peak_safe(20.0, 0.9);
        assert!((l - 1.0 / 0.9).abs() < 1e-5);
        // A cut applies in full.
        let cut = gain_linear_peak_safe(-6.0, 0.5);
        assert!((cut - 10f32.powf(-6.0 / 20.0)).abs() < 1e-5);
    }
}
