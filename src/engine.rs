// Project model, sequencer and sound synthesis. Numbers come from MECHANICS.md.

use std::f32::consts::TAU;
use std::sync::{Arc, Mutex};

pub const PPQ: u32 = 96;
pub const TICKS_PER_STEP: u32 = 24;
pub const STEPS_PER_BAR: usize = 16;
pub const BEATS_PER_BAR: u32 = 4;
pub const TICKS_PER_BAR: u32 = PPQ * BEATS_PER_BAR;
pub const DEFAULT_TEMPO: f64 = 130.0;
/// Tempo is kept as BPM x 1000 (sub_d62a00, sub_13114f0) and clamped (sub_1311510).
pub const TEMPO_SCALE: f64 = 1000.0;
pub const TEMPO_RAW_MIN: f64 = 10000.0;
pub const TEMPO_RAW_MAX: f64 = 522000.0;
pub const TEMPO_MIN: f64 = 10.0;
pub const TEMPO_MAX: f64 = 522.0;
pub const MAIN_SWING: f32 = 0.0;
/// Volumes and pans use the engine's 12800-step scale (gain = value / 12800).
pub const VOL_SCALE: f32 = 12800.0;
pub const PAN_CENTRE: f32 = 6400.0;
pub const CHANNEL_VOLUME: f32 = 10000.0;
pub const MIXER_INSERTS: usize = 125;
pub const FADER_DEFAULT: f32 = 12800.0;
pub const FADER_MAX: f32 = 16000.0;
pub const NOTE_VELOCITY: u8 = 100;
pub const PLAYLIST_TRACKS: usize = 24;
pub const STEP_KEY: u8 = 60;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Instrument {
    Kick,
    Clap,
    HiHat,
    Snare,
    Synth,
}

impl Instrument {
    pub const ALL: [Instrument; 5] = [
        Instrument::Kick,
        Instrument::Clap,
        Instrument::HiHat,
        Instrument::Snare,
        Instrument::Synth,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Instrument::Kick => "Kick",
            Instrument::Clap => "Clap",
            Instrument::HiHat => "Hi-hat",
            Instrument::Snare => "Snare",
            Instrument::Synth => "Synth",
        }
    }

    /// Stable name used in project files.
    pub fn key(self) -> &'static str {
        match self {
            Instrument::Kick => "kick",
            Instrument::Clap => "clap",
            Instrument::HiHat => "hihat",
            Instrument::Snare => "snare",
            Instrument::Synth => "synth",
        }
    }

    pub fn from_key(s: &str) -> Option<Instrument> {
        Instrument::ALL.into_iter().find(|i| i.key() == s)
    }
}

#[derive(Clone)]
pub struct Channel {
    pub name: String,
    pub instrument: Instrument,
    pub volume: f32,
    pub pan: f32,
    pub enabled: bool,
    pub mixer: usize,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Note {
    pub key: u8,
    pub start: u32,
    pub len: u32,
    pub vel: u8,
}

#[derive(Clone)]
pub struct Pattern {
    pub name: String,
    /// One list of notes per channel.
    pub notes: Vec<Vec<Note>>,
}

impl Pattern {
    pub fn new(index: usize, channels: usize) -> Self {
        Pattern { name: format!("Pattern {}", index + 1), notes: vec![Vec::new(); channels] }
    }

    /// At least one bar, grown by whole bars to fit the last note.
    pub fn length_ticks(&self) -> u32 {
        let end = self.notes.iter().flatten().map(|n| n.start + n.len.max(1)).max().unwrap_or(0);
        end.div_ceil(TICKS_PER_BAR).max(1) * TICKS_PER_BAR
    }

    pub fn has_step(&self, ch: usize, step: usize) -> bool {
        let t = step as u32 * TICKS_PER_STEP;
        self.notes[ch].iter().any(|n| n.start == t)
    }

    pub fn set_step(&mut self, ch: usize, step: usize, on: bool) {
        let t = step as u32 * TICKS_PER_STEP;
        let list = &mut self.notes[ch];
        list.retain(|n| n.start != t);
        if on {
            list.push(Note { key: STEP_KEY, start: t, len: TICKS_PER_STEP, vel: NOTE_VELOCITY });
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct Clip {
    pub track: usize,
    pub start_bar: u32,
    pub pattern: usize,
}

#[derive(Clone)]
pub struct MixerTrack {
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub enabled: bool,
    pub peak: [f32; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Pattern,
    Song,
}

pub struct Voice {
    channel: usize,
    instrument: Instrument,
    freq: f32,
    gain: f32,
    gate: f32,
    t: f32,
    phase: f32,
    phase2: f32,
    seed: u32,
    lp: f32,
    hp_prev: f32,
    hp: f32,
}

pub struct Project {
    pub title: String,
    pub author: String,
    pub tempo: f64,
    pub swing: f32,
    pub channels: Vec<Channel>,
    pub patterns: Vec<Pattern>,
    pub current_pattern: usize,
    pub clips: Vec<Clip>,
    pub mixer: Vec<MixerTrack>,
    pub mode: Mode,
    pub playing: bool,
    pub position: f64,
    pub sample_rate: f32,
    pub audio_ok: bool,
    pub previews: Vec<(usize, u8)>,
    voices: Vec<Voice>,
}

pub type Shared = Arc<Mutex<Project>>;

impl Project {
    pub fn new() -> Self {
        let defaults = [
            ("808 Kick", Instrument::Kick),
            ("808 Clap", Instrument::Clap),
            ("808 HiHat", Instrument::HiHat),
            ("808 Snare", Instrument::Snare),
            ("Synth pad", Instrument::Synth),
        ];
        let channels: Vec<Channel> = defaults
            .iter()
            .enumerate()
            .map(|(i, (name, inst))| Channel {
                name: name.to_string(),
                instrument: *inst,
                volume: CHANNEL_VOLUME,
                pan: 0.0,
                enabled: true,
                mixer: i + 1,
            })
            .collect();
        let mut mixer = vec![MixerTrack {
            name: "Master".into(),
            volume: FADER_DEFAULT,
            pan: 0.0,
            enabled: true,
            peak: [0.0; 2],
        }];
        for i in 1..=MIXER_INSERTS {
            mixer.push(MixerTrack {
                name: format!("Insert {i}"),
                volume: FADER_DEFAULT,
                pan: 0.0,
                enabled: true,
                peak: [0.0; 2],
            });
        }
        let n = channels.len();
        Project {
            title: String::new(),
            author: String::new(),
            tempo: DEFAULT_TEMPO,
            swing: MAIN_SWING,
            channels,
            patterns: vec![Pattern::new(0, n)],
            current_pattern: 0,
            clips: Vec::new(),
            mixer,
            mode: Mode::Pattern,
            playing: false,
            position: 0.0,
            sample_rate: 48000.0,
            audio_ok: false,
            previews: Vec::new(),
            voices: Vec::new(),
        }
    }

    pub fn add_channel(&mut self, inst: Instrument) {
        let count = self.channels.iter().filter(|c| c.instrument == inst).count();
        let name = if count == 0 { inst.label().to_string() } else { format!("{} #{}", inst.label(), count + 1) };
        let mixer = (self.channels.len() + 1).min(MIXER_INSERTS);
        self.channels.push(Channel { name, instrument: inst, volume: CHANNEL_VOLUME, pan: 0.0, enabled: true, mixer });
        for p in &mut self.patterns {
            p.notes.push(Vec::new());
        }
    }

    pub fn remove_channel(&mut self, ch: usize) {
        if self.channels.len() <= 1 {
            return;
        }
        self.channels.remove(ch);
        for p in &mut self.patterns {
            p.notes.remove(ch);
        }
        self.voices.retain(|v| v.channel != ch);
        for v in &mut self.voices {
            if v.channel > ch {
                v.channel -= 1;
            }
        }
    }

    pub fn ensure_pattern(&mut self, index: usize) {
        while self.patterns.len() <= index {
            let i = self.patterns.len();
            self.patterns.push(Pattern::new(i, self.channels.len()));
        }
    }

    pub fn song_length_ticks(&self) -> u32 {
        let end = self
            .clips
            .iter()
            .map(|c| c.start_bar * TICKS_PER_BAR + self.patterns[c.pattern].length_ticks())
            .max()
            .unwrap_or(0);
        end.max(TICKS_PER_BAR)
    }

    pub fn loop_length(&self) -> u32 {
        match self.mode {
            Mode::Pattern => self.patterns[self.current_pattern].length_ticks(),
            Mode::Song => self.song_length_ticks(),
        }
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.position = 0.0;
        for v in &mut self.voices {
            v.gate = v.gate.min(v.t);
        }
    }

    fn swing_offset(&self, tick: u32) -> f64 {
        if tick % (TICKS_PER_STEP * 2) == TICKS_PER_STEP {
            (self.swing * 0.5 * TICKS_PER_STEP as f32) as f64
        } else {
            0.0
        }
    }

    /// Notes whose (swung) start falls in [from, to) of the loop, as absolute song ticks.
    fn notes_in(&self, from: f64, to: f64, out: &mut Vec<(usize, Note)>) {
        let mut push = |pat: &Pattern, base: u32| {
            for (ch, list) in pat.notes.iter().enumerate() {
                for n in list {
                    let at = (base + n.start) as f64 + self.swing_offset(n.start);
                    if at >= from && at < to {
                        out.push((ch, *n));
                    }
                }
            }
        };
        match self.mode {
            Mode::Pattern => push(&self.patterns[self.current_pattern], 0),
            Mode::Song => {
                for c in &self.clips {
                    push(&self.patterns[c.pattern], c.start_bar * TICKS_PER_BAR);
                }
            }
        }
    }

    fn start_voice(&mut self, ch: usize, key: u8, vel: u8, len_ticks: u32) {
        let Some(c) = self.channels.get(ch) else { return };
        let secs_per_tick = 15.0 / (self.tempo as f32 * PPQ as f32 / 4.0);
        let base = match c.instrument {
            Instrument::Kick => 52.0,
            Instrument::Snare => 185.0,
            _ => 261.63,
        };
        let freq = base * 2f32.powf((key as f32 - 60.0) / 12.0);
        let seed = 0x9E37_79B9u32 ^ ((ch as u32 + 1) * 7919) ^ ((key as u32) << 16) ^ self.voices.len() as u32;
        self.voices.push(Voice {
            channel: ch,
            instrument: c.instrument,
            freq,
            gain: vel as f32 / 128.0,
            gate: len_ticks as f32 * secs_per_tick,
            t: 0.0,
            phase: 0.0,
            phase2: 0.0,
            seed: seed | 1,
            lp: 0.0,
            hp_prev: 0.0,
            hp: 0.0,
        });
        if self.voices.len() > 64 {
            self.voices.remove(0);
        }
    }

    /// Fill an interleaved stereo buffer.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        let frames = out.len() / channels;
        let sr = self.sample_rate;
        let dt = 1.0 / sr;
        for tr in &mut self.mixer {
            tr.peak[0] *= 0.6;
            tr.peak[1] *= 0.6;
        }
        let previews = std::mem::take(&mut self.previews);
        for (ch, key) in previews {
            self.start_voice(ch, key, NOTE_VELOCITY, TICKS_PER_STEP * 2);
        }

        const CHUNK: usize = 32;
        let mut events = Vec::new();
        let mut frame = 0;
        while frame < frames {
            let n = CHUNK.min(frames - frame);
            if self.playing {
                // sub_1311510: samples per tick = rate x 15 / (BPM x PPQ / 4).
                let ticks_per_sample = (self.tempo * PPQ as f64 / 4.0) / (sr as f64 * 15.0);
                let len = self.loop_length() as f64;
                if self.position >= len {
                    self.position %= len;
                }
                let from = self.position;
                let to = from + ticks_per_sample * n as f64;
                events.clear();
                if to <= len {
                    self.notes_in(from, to, &mut events);
                } else {
                    self.notes_in(from, len, &mut events);
                    self.notes_in(0.0, to - len, &mut events);
                }
                for &(ch, note) in &events {
                    self.start_voice(ch, note.key, note.vel, note.len);
                }
                self.position = if to >= len { to - len } else { to };
            }

            for i in 0..n {
                let mut l = 0.0f32;
                let mut r = 0.0f32;
                for v in &mut self.voices {
                    let s = voice_sample(v, dt);
                    let ch = &self.channels[v.channel.min(self.channels.len() - 1)];
                    if !ch.enabled {
                        continue;
                    }
                    let tr = &self.mixer[ch.mixer.min(MIXER_INSERTS)];
                    if !tr.enabled {
                        continue;
                    }
                    let pan = ((ch.pan + tr.pan) / PAN_CENTRE).clamp(-1.0, 1.0);
                    let g = s * v.gain * (ch.volume / VOL_SCALE) * (tr.volume / VOL_SCALE);
                    let gl = g * (1.0 - pan).min(1.0);
                    let gr = g * (1.0 + pan).min(1.0);
                    let idx = ch.mixer.min(MIXER_INSERTS);
                    let pk = &mut self.mixer[idx].peak;
                    pk[0] = pk[0].max(gl.abs());
                    pk[1] = pk[1].max(gr.abs());
                    l += gl;
                    r += gr;
                }
                let master = &mut self.mixer[0];
                let (ml, mr) = if master.enabled {
                    let mv = master.volume / VOL_SCALE;
                    let p = master.pan / PAN_CENTRE;
                    (soft_clip(l * mv * (1.0 - p).min(1.0)), soft_clip(r * mv * (1.0 + p).min(1.0)))
                } else {
                    (0.0, 0.0)
                };
                master.peak[0] = master.peak[0].max(ml.abs());
                master.peak[1] = master.peak[1].max(mr.abs());
                let base = (frame + i) * channels;
                out[base] = ml;
                if channels > 1 {
                    out[base + 1] = mr;
                }
                for c in 2..channels {
                    out[base + c] = 0.0;
                }
            }
            self.voices.retain(|v| !voice_done(v));
            frame += n;
        }
    }
}

fn soft_clip(x: f32) -> f32 {
    x.clamp(-1.5, 1.5) * (1.0 - x.clamp(-1.5, 1.5).powi(2) / 6.75)
}

fn noise(seed: &mut u32) -> f32 {
    let mut x = *seed;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *seed = x;
    (x as f32 / u32::MAX as f32) * 2.0 - 1.0
}

fn voice_done(v: &Voice) -> bool {
    let tail = match v.instrument {
        Instrument::Kick => 0.7,
        Instrument::Clap => 0.45,
        Instrument::HiHat => 0.25,
        Instrument::Snare => 0.45,
        Instrument::Synth => return v.t > v.gate + 0.35,
    };
    v.t > tail
}

fn voice_sample(v: &mut Voice, dt: f32) -> f32 {
    let t = v.t;
    v.t += dt;
    match v.instrument {
        Instrument::Kick => {
            let f = v.freq * (1.0 + 2.2 * (-t * 28.0).exp());
            v.phase = (v.phase + f * dt).fract();
            let click = noise(&mut v.seed) * (-t * 400.0).exp() * 0.25;
            ((v.phase * TAU).sin() * (-t * 6.5).exp() + click) * 1.1
        }
        Instrument::Clap => {
            let n = noise(&mut v.seed);
            v.lp += (n - v.lp) * 0.35;
            let band = v.lp - v.hp;
            v.hp += (v.lp - v.hp) * 0.08;
            let burst = |o: f32| if t >= o { (-(t - o) * 220.0).exp() } else { 0.0 };
            let env = burst(0.0) + burst(0.011) + burst(0.022) + if t > 0.03 { (-(t - 0.03) * 16.0).exp() * 0.8 } else { 0.0 };
            band * env * 1.6
        }
        Instrument::HiHat => {
            let n = noise(&mut v.seed);
            let h = n - v.hp_prev;
            v.hp_prev = n;
            let h2 = h - v.hp;
            v.hp = h;
            h2 * (-t * 48.0).exp() * 0.35
        }
        Instrument::Snare => {
            v.phase = (v.phase + v.freq * (1.0 + 0.5 * (-t * 40.0).exp()) * dt).fract();
            let tone = (v.phase * TAU).sin() * (-t * 22.0).exp() * 0.55;
            let n = noise(&mut v.seed);
            v.lp += (n - v.lp) * 0.55;
            let body = (n - v.lp * 0.5) * (-t * 13.0).exp() * 0.5;
            tone + body
        }
        Instrument::Synth => {
            v.phase = (v.phase + v.freq * dt).fract();
            v.phase2 = (v.phase2 + v.freq * 1.006 * dt).fract();
            let saw = (v.phase * 2.0 - 1.0) + (v.phase2 * 2.0 - 1.0);
            let env = if t < 0.012 {
                t / 0.012
            } else if t < v.gate {
                0.6 + 0.4 * (-(t - 0.012) * 4.0).exp()
            } else {
                let held = if v.gate < 0.012 { v.gate / 0.012 } else { 0.6 + 0.4 * (-(v.gate - 0.012) * 4.0).exp() };
                held * (-(t - v.gate) * 14.0).exp()
            };
            let cutoff = 0.05 + 0.25 * (-t * 5.0).exp();
            v.lp += (saw - v.lp) * cutoff;
            v.lp * env * 0.32
        }
    }
}

/// Rounds to the engine's BPM x 1000 steps inside 10000..=522000.
pub fn quantize_tempo(bpm: f64) -> f64 {
    (bpm * TEMPO_SCALE).round().clamp(TEMPO_RAW_MIN, TEMPO_RAW_MAX) / TEMPO_SCALE
}

/// Opens the default sound output. Returns the stream so it stays alive.
pub fn start_audio(shared: Shared) -> Option<cpal::Stream> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let supported = device.default_output_config().ok()?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let chans = config.channels as usize;
    if let Ok(mut p) = shared.lock() {
        p.sample_rate = config.sample_rate.0 as f32;
    }
    let err = |e| eprintln!("audio error: {e}");
    let mut scratch: Vec<f32> = Vec::new();
    let stream = match format {
        cpal::SampleFormat::F32 => {
            let s = shared.clone();
            device.build_output_stream(
                &config,
                move |data: &mut [f32], _| match s.lock() {
                    Ok(mut p) => p.render(data, chans),
                    Err(_) => data.fill(0.0),
                },
                err,
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let s = shared.clone();
            device.build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    scratch.resize(data.len(), 0.0);
                    if let Ok(mut p) = s.lock() {
                        p.render(&mut scratch, chans);
                    }
                    for (d, x) in data.iter_mut().zip(&scratch) {
                        *d = (x.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    }
                },
                err,
                None,
            )
        }
        _ => return None,
    }
    .ok()?;
    stream.play().ok()?;
    if let Ok(mut p) = shared.lock() {
        p.audio_ok = true;
    }
    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_plays_steps() {
        let mut p = Project::new();
        p.patterns[0].set_step(0, 0, true);
        p.patterns[0].set_step(2, 4, true);
        assert_eq!(p.patterns[0].length_ticks(), TICKS_PER_BAR);
        p.playing = true;
        let mut buf = vec![0.0; 48000 * 2];
        p.render(&mut buf, 2);
        assert!(buf.iter().any(|x| x.abs() > 0.05));
    }
}
