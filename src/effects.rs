// Mixer effects. Every effect has four knobs stored as 0..1; `describe` turns
// a knob into the value shown to the user. The sound state (filters, delay
// lines) is not saved and starts again when a project is opened.

use std::f32::consts::PI;

use crate::modfx::{Chorus, Phaser};
use crate::plugin::{PluginEffect, PluginSlot, Transport};

pub const FX_SLOTS: usize = 10;
pub const PARAMS: usize = 4;
/// Longest echo the delay keeps, in seconds.
const DELAY_MAX_SECS: f32 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EffectKind {
    Reverb,
    Delay,
    Eq,
    Compressor,
    Filter,
    Distortion,
    Chorus,
    Phaser,
    /// A CLAP plugin; its own parameters live in the plugin.
    Plugin,
}

impl EffectKind {
    pub const BUILT_IN: [EffectKind; 8] = [
        EffectKind::Reverb,
        EffectKind::Delay,
        EffectKind::Eq,
        EffectKind::Compressor,
        EffectKind::Filter,
        EffectKind::Distortion,
        EffectKind::Chorus,
        EffectKind::Phaser,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EffectKind::Reverb => "Reverb",
            EffectKind::Delay => "Delay",
            EffectKind::Eq => "EQ",
            EffectKind::Compressor => "Compressor",
            EffectKind::Filter => "Filter",
            EffectKind::Distortion => "Distortion",
            EffectKind::Chorus => "Chorus",
            EffectKind::Phaser => "Phaser",
            EffectKind::Plugin => "Plugin",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            EffectKind::Reverb => "reverb",
            EffectKind::Delay => "delay",
            EffectKind::Eq => "eq",
            EffectKind::Compressor => "compressor",
            EffectKind::Filter => "filter",
            EffectKind::Distortion => "distortion",
            EffectKind::Chorus => "chorus",
            EffectKind::Phaser => "phaser",
            EffectKind::Plugin => "plugin",
        }
    }

    pub fn from_key(s: &str) -> Option<EffectKind> {
        EffectKind::BUILT_IN.into_iter().find(|k| k.key() == s)
    }

    pub fn param_names(self) -> [&'static str; PARAMS] {
        match self {
            EffectKind::Reverb => ["Size", "Damping", "Width", "Mix"],
            EffectKind::Delay => ["Time", "Feedback", "Tone", "Mix"],
            EffectKind::Eq => ["Low", "Mid", "Mid freq", "High"],
            EffectKind::Compressor => ["Threshold", "Ratio", "Release", "Gain"],
            EffectKind::Filter => ["Cutoff", "Resonance", "Type", "Mix"],
            EffectKind::Distortion => ["Drive", "Tone", "Mix", "Output"],
            EffectKind::Chorus => ["Rate", "Depth", "Delay", "Mix"],
            EffectKind::Phaser => ["Rate", "Depth", "Feedback", "Mix"],
            EffectKind::Plugin => ["", "", "", ""],
        }
    }

    pub fn defaults(self) -> [f32; PARAMS] {
        match self {
            EffectKind::Reverb => [0.6, 0.5, 1.0, 0.3],
            EffectKind::Delay => [3.0 / 15.0, 0.4, 0.6, 0.3],
            EffectKind::Eq => [0.5, 0.5, 0.5, 0.5],
            EffectKind::Compressor => [0.7, 0.25, 0.3, 0.0],
            EffectKind::Filter => [0.6, 0.1, 0.0, 1.0],
            EffectKind::Distortion => [0.4, 0.7, 1.0, 0.6],
            EffectKind::Chorus => [0.3, 0.4, 0.3, 0.5],
            EffectKind::Phaser => [0.3, 0.7, 0.5, 0.5],
            EffectKind::Plugin => [0.0; PARAMS],
        }
    }

    /// What a knob means, for the hint line and the effect panel.
    pub fn describe(self, i: usize, v: f32) -> String {
        let db = |x: f32| format!("{x:+.1} dB");
        match (self, i) {
            (EffectKind::Delay, 0) => format!("{} steps", delay_steps(v)),
            (EffectKind::Eq, 2) => format!("{:.0} Hz", eq_mid_freq(v)),
            (EffectKind::Eq, _) => db(eq_gain(v)),
            (EffectKind::Compressor, 0) => db(comp_threshold(v)),
            (EffectKind::Compressor, 1) => format!("{:.1}:1", comp_ratio(v)),
            (EffectKind::Compressor, 2) => format!("{:.0} ms", comp_release(v) * 1000.0),
            (EffectKind::Compressor, 3) => db(v * 24.0),
            (EffectKind::Filter, 0) => format!("{:.0} Hz", filter_cutoff(v)),
            (EffectKind::Filter, 1) => format!("Q {:.1}", filter_q(v)),
            (EffectKind::Filter, 2) => ["Low pass", "Band pass", "High pass"][filter_type(v)].into(),
            (EffectKind::Distortion, 0) => db(v * 36.0),
            (EffectKind::Distortion, 3) => db((v - 0.6) * 30.0),
            (EffectKind::Chorus, 0) => format!("{:.2} Hz", 0.1 * 50f32.powf(v)),
            (EffectKind::Chorus, 1) => format!("{:.1} ms", v * 8.0),
            (EffectKind::Chorus, 2) => format!("{:.0} ms", 5.0 + v * 20.0),
            (EffectKind::Phaser, 0) => format!("{:.2} Hz", 0.05 * 40f32.powf(v)),
            _ => format!("{:.0}%", v * 100.0),
        }
    }
}

fn delay_steps(v: f32) -> u32 {
    1 + (v.clamp(0.0, 1.0) * 15.0).round() as u32
}
fn eq_gain(v: f32) -> f32 {
    (v - 0.5) * 24.0
}
fn eq_mid_freq(v: f32) -> f32 {
    200.0 * 25f32.powf(v)
}
fn comp_threshold(v: f32) -> f32 {
    (v - 1.0) * 60.0
}
fn comp_ratio(v: f32) -> f32 {
    1.0 + v * 19.0
}
fn comp_release(v: f32) -> f32 {
    0.02 + v * 0.48
}
fn filter_cutoff(v: f32) -> f32 {
    20.0 * 1000f32.powf(v)
}
fn filter_q(v: f32) -> f32 {
    0.5 + v * 9.5
}
fn filter_type(v: f32) -> usize {
    ((v * 3.0) as usize).min(2)
}
fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// RBJ-cookbook biquad, one per stereo side.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [[f32; 2]; 2],
}

#[derive(Clone, Copy)]
enum Shape {
    LowPass,
    BandPass,
    HighPass,
    Peak(f32),
    LowShelf(f32),
    HighShelf(f32),
}

impl Biquad {
    fn set(&mut self, shape: Shape, freq: f32, q: f32, rate: f32) {
        let w = 2.0 * PI * (freq / rate).clamp(1e-4, 0.49);
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match shape {
            Shape::LowPass => ((1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            Shape::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            Shape::HighPass => ((1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            Shape::Peak(db) => {
                let a = 10f32.powf(db / 40.0);
                (1.0 + alpha * a, -2.0 * cs, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cs, 1.0 - alpha / a)
            }
            Shape::LowShelf(db) | Shape::HighShelf(db) => {
                let a = 10f32.powf(db / 40.0);
                let s = 2.0 * a.sqrt() * alpha;
                let sign = if matches!(shape, Shape::LowShelf(_)) { 1.0 } else { -1.0 };
                (
                    a * ((a + 1.0) - sign * (a - 1.0) * cs + s),
                    sign * 2.0 * a * ((a - 1.0) - sign * (a + 1.0) * cs),
                    a * ((a + 1.0) - sign * (a - 1.0) * cs - s),
                    (a + 1.0) + sign * (a - 1.0) * cs + s,
                    -sign * 2.0 * ((a - 1.0) + sign * (a + 1.0) * cs),
                    (a + 1.0) + sign * (a - 1.0) * cs - s,
                )
            }
        };
        self.b = [b0 / a0, b1 / a0, b2 / a0];
        self.a = [a1 / a0, a2 / a0];
    }

    fn run(&mut self, side: usize, x: f32) -> f32 {
        let z = &mut self.z[side];
        let y = self.b[0] * x + z[0];
        z[0] = self.b[1] * x - self.a[0] * y + z[1];
        z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// Freeverb: eight comb filters and four all-pass filters per side.
struct Reverb {
    combs: Vec<(Vec<f32>, usize, f32)>,
    allpasses: Vec<(Vec<f32>, usize)>,
}

const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const SPREAD: usize = 23;

impl Reverb {
    fn new(rate: f32) -> Reverb {
        let scale = rate / 44100.0;
        let len = |n: usize| ((n as f32 * scale) as usize).max(1);
        let mut combs = Vec::new();
        let mut allpasses = Vec::new();
        for side in 0..2 {
            for n in COMBS {
                combs.push((vec![0.0; len(n + side * SPREAD)], 0, 0.0));
            }
            for n in ALLPASSES {
                allpasses.push((vec![0.0; len(n + side * SPREAD)], 0));
            }
        }
        Reverb { combs, allpasses }
    }

    fn run(&mut self, side: usize, x: f32, feedback: f32, damp: f32) -> f32 {
        let mut out = 0.0;
        for (buf, i, store) in &mut self.combs[side * 8..side * 8 + 8] {
            let y = buf[*i];
            *store = y * (1.0 - damp) + *store * damp;
            buf[*i] = x + *store * feedback;
            *i = (*i + 1) % buf.len();
            out += y;
        }
        for (buf, i) in &mut self.allpasses[side * 4..side * 4 + 4] {
            let b = buf[*i];
            buf[*i] = out + b * 0.5;
            *i = (*i + 1) % buf.len();
            out = b - out;
        }
        out
    }
}

enum State {
    Reverb(Box<Reverb>),
    Delay { buf: Vec<[f32; 2]>, at: usize, lp: [f32; 2] },
    Eq([Biquad; 3]),
    Compressor { env: f32 },
    Filter(Biquad),
    Distortion { lp: [f32; 2] },
    Chorus(Box<Chorus>),
    Phaser(Phaser),
}

pub struct Effect {
    pub kind: EffectKind,
    pub params: [f32; PARAMS],
    pub enabled: bool,
    /// Set for `EffectKind::Plugin`.
    pub plugin: Option<Box<PluginSlot<PluginEffect>>>,
    state: Option<State>,
    rate: f32,
}

impl Effect {
    pub fn new(kind: EffectKind) -> Effect {
        Effect { kind, params: kind.defaults(), enabled: true, plugin: None, state: None, rate: 0.0 }
    }

    pub fn plugin(slot: PluginSlot<PluginEffect>) -> Effect {
        Effect { kind: EffectKind::Plugin, params: [0.0; PARAMS], enabled: true, plugin: Some(Box::new(slot)), state: None, rate: 0.0 }
    }

    pub fn name(&self) -> String {
        match &self.plugin {
            Some(p) => p.info.name.clone(),
            None => self.kind.label().to_string(),
        }
    }

    /// Takes over the sound state of an effect of the same kind (after undo),
    /// so reverb and echo tails keep ringing.
    pub fn keep_state(&mut self, old: &mut Effect) {
        if old.kind == self.kind && self.kind != EffectKind::Plugin {
            self.state = old.state.take();
            self.rate = old.rate;
        }
    }

    fn fresh_state(&self, rate: f32) -> State {
        match self.kind {
            EffectKind::Reverb => State::Reverb(Box::new(Reverb::new(rate))),
            EffectKind::Delay => State::Delay { buf: vec![[0.0; 2]; (rate * DELAY_MAX_SECS) as usize], at: 0, lp: [0.0; 2] },
            EffectKind::Eq => State::Eq([Biquad::default(); 3]),
            EffectKind::Compressor => State::Compressor { env: 0.0 },
            EffectKind::Filter => State::Filter(Biquad::default()),
            EffectKind::Distortion => State::Distortion { lp: [0.0; 2] },
            EffectKind::Chorus => State::Chorus(Box::new(Chorus::new(rate))),
            EffectKind::Phaser => State::Phaser(Phaser::new()),
            EffectKind::Plugin => unreachable!("plugins are processed by their slot"),
        }
    }

    /// Processes interleaved stereo frames in place. `p` are the knobs to use
    /// (the saved ones, or automated values).
    pub fn process(&mut self, buf: &mut [[f32; 2]], p: &[f32; PARAMS], rate: f32, tr: &Transport) {
        if !self.enabled {
            return;
        }
        if let Some(slot) = &mut self.plugin {
            if let Some(live) = &mut slot.live {
                live.process(buf, tr);
            }
            return;
        }
        let secs_per_step = (15.0 / tr.tempo) as f32;
        if self.state.is_none() || self.rate != rate {
            // Only when an effect is added or the sound output changes.
            self.state = Some(self.fresh_state(rate));
            self.rate = rate;
        }
        let Some(state) = self.state.as_mut() else { return };
        match state {
            State::Reverb(rv) => {
                let feedback = p[0] * 0.28 + 0.7;
                let damp = p[1] * 0.4;
                let width = p[2];
                let (wet, dry) = (p[3], 1.0 - p[3]);
                let (w1, w2) = (wet * (width / 2.0 + 0.5), wet * ((1.0 - width) / 2.0));
                for f in buf.iter_mut() {
                    let input = (f[0] + f[1]) * 0.015;
                    let l = rv.run(0, input, feedback, damp);
                    let r = rv.run(1, input, feedback, damp);
                    f[0] = f[0] * dry + (l * w1 + r * w2) * 3.0;
                    f[1] = f[1] * dry + (r * w1 + l * w2) * 3.0;
                }
            }
            State::Delay { buf: line, at, lp } => {
                let len = line.len();
                let d = ((delay_steps(p[0]) as f32 * secs_per_step * rate) as usize).clamp(1, len - 1);
                let fb = p[1] * 0.95;
                let tone = 0.05 + p[2] * 0.9;
                for f in buf.iter_mut() {
                    let read = (*at + len - d) % len;
                    let echo = line[read];
                    for s in 0..2 {
                        lp[s] += (echo[s] - lp[s]) * tone;
                    }
                    // Ping-pong: each echo moves to the other side.
                    line[*at] = [f[0] + lp[1] * fb, f[1] + lp[0] * fb];
                    *at = (*at + 1) % len;
                    f[0] += echo[0] * p[3];
                    f[1] += echo[1] * p[3];
                }
            }
            State::Eq(bands) => {
                bands[0].set(Shape::LowShelf(eq_gain(p[0])), 200.0, 0.707, rate);
                bands[1].set(Shape::Peak(eq_gain(p[1])), eq_mid_freq(p[2]), 0.9, rate);
                bands[2].set(Shape::HighShelf(eq_gain(p[3])), 5000.0, 0.707, rate);
                for f in buf.iter_mut() {
                    for (s, x) in f.iter_mut().enumerate() {
                        let low = bands[0].run(s, *x);
                        let mid = bands[1].run(s, low);
                        *x = bands[2].run(s, mid);
                    }
                }
            }
            State::Compressor { env } => {
                let threshold = comp_threshold(p[0]);
                let ratio = comp_ratio(p[1]);
                let attack = (-1.0 / (0.005 * rate)).exp();
                let release = (-1.0 / (comp_release(p[2]) * rate)).exp();
                let makeup = db_to_gain(p[3] * 24.0);
                for f in buf.iter_mut() {
                    let level = f[0].abs().max(f[1].abs());
                    let coef = if level > *env { attack } else { release };
                    *env = level + coef * (*env - level);
                    let over = 20.0 * env.max(1e-6).log10() - threshold;
                    let gain = if over > 0.0 { db_to_gain(-over * (1.0 - 1.0 / ratio)) } else { 1.0 } * makeup;
                    f[0] *= gain;
                    f[1] *= gain;
                }
            }
            State::Filter(bq) => {
                let shape = [Shape::LowPass, Shape::BandPass, Shape::HighPass][filter_type(p[2])];
                bq.set(shape, filter_cutoff(p[0]), filter_q(p[1]), rate);
                for f in buf.iter_mut() {
                    for (s, x) in f.iter_mut().enumerate() {
                        let y = bq.run(s, *x);
                        *x = *x * (1.0 - p[3]) + y * p[3];
                    }
                }
            }
            State::Distortion { lp } => {
                let drive = db_to_gain(p[0] * 36.0);
                let tone = 0.05 + p[1] * 0.95;
                let out = db_to_gain((p[3] - 0.6) * 30.0);
                for f in buf.iter_mut() {
                    for (s, x) in f.iter_mut().enumerate() {
                        let shaped = (*x * drive).tanh();
                        lp[s] += (shaped - lp[s]) * tone;
                        *x = (*x * (1.0 - p[2]) + lp[s] * p[2]) * out;
                    }
                }
            }
            State::Chorus(c) => c.process(buf, p, rate),
            State::Phaser(ph) => ph.process(buf, p, rate),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 150 BPM: one step lasts 0.1 s.
    const TR: Transport = Transport { tempo: 150.0, playing: true, beats: 0.0 };

    fn impulse(kind: EffectKind, frames: usize) -> Vec<[f32; 2]> {
        let mut e = Effect::new(kind);
        let mut buf = vec![[0.0; 2]; frames];
        buf[0] = [1.0, 1.0];
        let p = e.params;
        e.process(&mut buf, &p, 48000.0, &TR);
        buf
    }

    #[test]
    fn reverb_and_delay_ring_after_the_sound() {
        let rv = impulse(EffectKind::Reverb, 48000);
        assert!(rv[24000..].iter().any(|f| f[0].abs() > 1e-4));
        let dl = impulse(EffectKind::Delay, 48000);
        // 4 steps of 0.1 s: the first echo comes back at 0.4 s, on the other side too.
        assert!(dl[19100..19300].iter().any(|f| f[0].abs() > 0.1 && f[1].abs() > 0.1));
    }

    #[test]
    fn effects_stay_finite() {
        for kind in EffectKind::BUILT_IN {
            for f in impulse(kind, 4800) {
                assert!(f[0].is_finite() && f[1].is_finite(), "{kind:?}");
            }
        }
    }

    #[test]
    fn compressor_turns_loud_sounds_down() {
        let mut e = Effect::new(EffectKind::Compressor);
        let mut buf = vec![[0.9, 0.9]; 4800];
        let p = [0.7, 1.0, 0.3, 0.0];
        e.process(&mut buf, &p, 48000.0, &TR);
        assert!(buf[4000][0] < 0.5);
    }
}
