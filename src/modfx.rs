// Modulation effects for the mixer: chorus and phaser. Four knobs 0..1, like
// the other built-in effects. The phaser was first written by the local model
// (Cline) and fixed; the chorus was rewritten.

use std::f32::consts::PI;

/// A short delay line whose length moves with a slow sine; the two sides are
/// a quarter turn apart for width.
pub struct Chorus {
    buf: Vec<[f32; 2]>,
    at: usize,
    phase: f32,
}

impl Chorus {
    /// 60 ms of delay line.
    pub fn new(rate: f32) -> Chorus {
        let len = ((rate * 0.06) as usize).max(4);
        Chorus { buf: vec![[0.0; 2]; len], at: 0, phase: 0.0 }
    }

    /// Knobs: rate (0.1 to 5 Hz), depth (0 to 8 ms), base delay (5 to 25 ms), mix.
    pub fn process(&mut self, frames: &mut [[f32; 2]], p: &[f32; 4], rate: f32) {
        let len = self.buf.len();
        let freq = 0.1 * 50f32.powf(p[0]);
        let depth = p[1] * 0.008;
        let base = 0.005 + p[2] * 0.020;
        let mix = p[3];
        for f in frames.iter_mut() {
            self.buf[self.at] = *f;
            let mut wet = [0.0; 2];
            for (s, w) in wet.iter_mut().enumerate() {
                let lfo = 0.5 * (1.0 + (2.0 * PI * (self.phase + 0.25 * s as f32)).sin());
                let d = ((base + depth * lfo) * rate).clamp(1.0, (len - 2) as f32);
                let pos = (self.at as f32 - d).rem_euclid(len as f32);
                let i0 = (pos as usize).min(len - 1);
                let i1 = (i0 + 1) % len;
                let frac = pos - i0 as f32;
                *w = self.buf[i0][s] + (self.buf[i1][s] - self.buf[i0][s]) * frac;
            }
            f[0] = f[0] * (1.0 - mix) + wet[0] * mix;
            f[1] = f[1] * (1.0 - mix) + wet[1] * mix;
            self.at = (self.at + 1) % len;
            self.phase += freq / rate;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}

/// Four first-order all-pass filters swept by a slow sine, with feedback.
pub struct Phaser {
    z: [[f32; 4]; 2],
    fb: [f32; 2],
    phase: f32,
}

impl Phaser {
    pub fn new() -> Phaser {
        Phaser { z: [[0.0; 4]; 2], fb: [0.0; 2], phase: 0.0 }
    }

    /// Knobs: rate (0.05 to 2 Hz), depth (sweep from 200 Hz up to 2 kHz), feedback, mix.
    pub fn process(&mut self, frames: &mut [[f32; 2]], p: &[f32; 4], rate: f32) {
        let freq = 0.05 * 40f32.powf(p[0]);
        let feedback = p[2] * 0.9;
        let mix = p[3];
        for f in frames.iter_mut() {
            let lfo = 0.5 * (1.0 + (2.0 * PI * self.phase).sin());
            let sweep = 200.0 * 10f32.powf(p[1] * lfo);
            let t = (PI * sweep / rate).tan();
            let a = (t - 1.0) / (t + 1.0);
            for (s, x_out) in f.iter_mut().enumerate() {
                let input = *x_out;
                let mut x = input + self.fb[s] * feedback;
                for z in &mut self.z[s] {
                    let y = a * x + *z;
                    *z = x - a * y;
                    x = y;
                }
                self.fb[s] = x;
                *x_out = input * (1.0 - mix) + x * mix;
            }
            self.phase += freq / rate;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48000.0;

    fn sine(frames: usize) -> Vec<[f32; 2]> {
        (0..frames).map(|i| [(i as f32 / RATE * 2.0 * PI * 440.0).sin(); 2]).collect()
    }

    fn both(buf: &mut [[f32; 2]], p: &[f32; 4]) {
        Chorus::new(RATE).process(buf, p, RATE);
        Phaser::new().process(buf, p, RATE);
    }

    #[test]
    fn silence_stays_silent() {
        let mut buf = vec![[0.0f32; 2]; 4800];
        both(&mut buf, &[0.7, 0.7, 0.7, 1.0]);
        assert!(buf.iter().all(|f| f[0] == 0.0 && f[1] == 0.0));
    }

    #[test]
    fn mix_zero_leaves_the_sound() {
        let orig = sine(4800);
        let mut buf = orig.clone();
        both(&mut buf, &[0.5, 0.5, 0.5, 0.0]);
        assert!(buf.iter().zip(&orig).all(|(a, b)| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6));
    }

    #[test]
    fn stay_finite_at_full_settings() {
        let mut buf = sine(48000);
        both(&mut buf, &[1.0; 4]);
        assert!(buf.iter().all(|f| f[0].is_finite() && f[1].is_finite()));
    }

    #[test]
    fn chorus_delays_by_its_base_time() {
        // 5 ms at 48 kHz = 240 frames, no depth.
        let mut c = Chorus::new(RATE);
        let mut buf = vec![[0.0f32; 2]; 4800];
        buf[0] = [1.0, 1.0];
        c.process(&mut buf, &[0.0, 0.0, 0.0, 1.0], RATE);
        let peak = (0..buf.len()).max_by(|&a, &b| buf[a][0].abs().total_cmp(&buf[b][0].abs())).unwrap();
        assert!((238..=242).contains(&peak), "peak at {peak}");
    }

    #[test]
    fn chorus_keeps_its_place_across_blocks() {
        // The echo of an impulse in the first block comes out in the second.
        let mut c = Chorus::new(RATE);
        let p = [0.0, 0.0, 0.0, 1.0];
        let mut a = vec![[0.0f32; 2]; 128];
        a[0] = [1.0, 1.0];
        c.process(&mut a, &p, RATE);
        let mut b = vec![[0.0f32; 2]; 256];
        c.process(&mut b, &p, RATE);
        assert!(b[100..120].iter().any(|f| f[0] > 0.5));
    }
}
