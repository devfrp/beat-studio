// Project model, sequencer, mixer and sound synthesis. Numbers come from MECHANICS.md.

use std::f32::consts::TAU;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::effects::{Effect, FX_SLOTS, PARAMS};
use crate::plugin::{PluginEffect, PluginInstrument, PluginSlot, Transport};

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
/// Voices of the metronome are not tied to a channel.
const METRONOME: usize = usize::MAX;
/// Gate for notes played live: they ring until the key is released.
const HELD_GATE_SECS: f32 = 30.0;
/// Fade at the end of a sample cut short by its note.
const SAMPLE_RELEASE_SECS: f32 = 0.02;
const MAX_VOICES: usize = 96;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Instrument {
    Kick,
    Clap,
    HiHat,
    Snare,
    Synth,
    /// Plays a sound file (`Channel::sample`).
    Sampler,
    /// A CLAP instrument plugin (`Channel::plugin`).
    Plugin,
    /// Metronome click; not offered as a channel.
    Click,
}

impl Instrument {
    /// The built-in sounds offered in the menus.
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
            Instrument::Sampler => "Sampler",
            Instrument::Plugin => "Plugin",
            Instrument::Click => "Metronome",
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
            Instrument::Sampler => "sampler",
            Instrument::Plugin => "plugin",
            Instrument::Click => "click",
        }
    }

    pub fn from_key(s: &str) -> Option<Instrument> {
        Instrument::ALL.into_iter().chain([Instrument::Sampler, Instrument::Plugin]).find(|i| i.key() == s)
    }
}

/// A decoded sound file, kept at its own sample rate.
pub struct SampleData {
    pub rate: f32,
    pub frames: Vec<[f32; 2]>,
}

impl SampleData {
    pub fn seconds(&self) -> f32 {
        self.frames.len() as f32 / self.rate
    }
}

/// The file a sampler channel plays; `data` arrives once the file is decoded.
pub struct SampleRef {
    pub path: PathBuf,
    pub data: Option<Arc<SampleData>>,
}

pub struct Channel {
    pub name: String,
    pub instrument: Instrument,
    pub volume: f32,
    pub pan: f32,
    pub enabled: bool,
    pub mixer: usize,
    pub sample: Option<SampleRef>,
    pub plugin: Option<Box<PluginSlot<PluginInstrument>>>,
}

impl Channel {
    pub fn new(name: String, instrument: Instrument, mixer: usize) -> Channel {
        Channel { name, instrument, volume: CHANNEL_VOLUME, pan: 0.0, enabled: true, mixer, sample: None, plugin: None }
    }
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
        self.notes[ch].iter().any(|n| n.start == t && n.vel > 0)
    }

    pub fn set_step(&mut self, ch: usize, step: usize, on: bool) {
        let t = step as u32 * TICKS_PER_STEP;
        let list = &mut self.notes[ch];
        list.retain(|n| n.start != t || n.vel == 0);
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

pub struct MixerTrack {
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub enabled: bool,
    pub solo: bool,
    /// Where the sound goes, with a level: the master (0) by default.
    pub routes: Vec<(usize, f32)>,
    pub fx: Vec<Option<Effect>>,
    pub peak: [f32; 2],
}

impl MixerTrack {
    fn new(i: usize) -> MixerTrack {
        MixerTrack {
            name: if i == 0 { "Master".into() } else { format!("Insert {i}") },
            volume: FADER_DEFAULT,
            pan: 0.0,
            enabled: true,
            solo: false,
            routes: if i == 0 { Vec::new() } else { vec![(0, 1.0)] },
            fx: (0..FX_SLOTS).map(|_| None).collect(),
            peak: [0.0; 2],
        }
    }

    pub fn default_routes(&self, i: usize) -> bool {
        if i == 0 {
            self.routes.is_empty()
        } else {
            self.routes == [(0, 1.0)]
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Pattern,
    Song,
}

/// What an automation clip moves.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Target {
    Tempo,
    ChannelVolume(usize),
    ChannelPan(usize),
    MixerVolume(usize),
    MixerPan(usize),
    /// Insert, slot, knob.
    Effect(usize, usize, usize),
}

impl Target {
    pub fn range(self) -> (f32, f32) {
        match self {
            Target::Tempo => (60.0, 200.0),
            Target::ChannelVolume(_) => (0.0, VOL_SCALE),
            Target::MixerVolume(_) => (0.0, FADER_MAX),
            Target::ChannelPan(_) | Target::MixerPan(_) => (-PAN_CENTRE, PAN_CENTRE),
            Target::Effect(..) => (0.0, 1.0),
        }
    }

    pub fn key(self) -> String {
        match self {
            Target::Tempo => "tempo".into(),
            Target::ChannelVolume(c) => format!("chvol {c}"),
            Target::ChannelPan(c) => format!("chpan {c}"),
            Target::MixerVolume(i) => format!("mixvol {i}"),
            Target::MixerPan(i) => format!("mixpan {i}"),
            Target::Effect(i, s, k) => format!("fx {i} {s} {k}"),
        }
    }

    pub fn parse(s: &str) -> Option<Target> {
        let mut w = s.split_whitespace();
        let kind = w.next()?;
        let mut n = || w.next().and_then(|v| v.parse::<usize>().ok());
        Some(match kind {
            "tempo" => Target::Tempo,
            "chvol" => Target::ChannelVolume(n()?),
            "chpan" => Target::ChannelPan(n()?),
            "mixvol" => Target::MixerVolume(n()?.min(MIXER_INSERTS)),
            "mixpan" => Target::MixerPan(n()?.min(MIXER_INSERTS)),
            "fx" => Target::Effect(n()?.min(MIXER_INSERTS), n()?.min(FX_SLOTS - 1), n()?.min(PARAMS - 1)),
            _ => return None,
        })
    }

    pub fn label(self, p: &Project) -> String {
        let ch = |c: usize| p.channels.get(c).map_or("?".to_string(), |c| c.name.clone());
        let mx = |i: usize| p.mixer.get(i).map_or("?".to_string(), |t| t.name.clone());
        match self {
            Target::Tempo => "Tempo".into(),
            Target::ChannelVolume(c) => format!("{} volume", ch(c)),
            Target::ChannelPan(c) => format!("{} pan", ch(c)),
            Target::MixerVolume(i) => format!("{} volume", mx(i)),
            Target::MixerPan(i) => format!("{} pan", mx(i)),
            Target::Effect(i, s, k) => match p.mixer.get(i).and_then(|t| t.fx[s].as_ref()) {
                Some(e) => format!("{} {} {}", mx(i), e.name(), e.kind.param_names()[k]),
                None => format!("{} slot {} (empty)", mx(i), s + 1),
            },
        }
    }

    /// The value the target has without automation.
    pub fn base_value(self, p: &Project) -> Option<f32> {
        Some(match self {
            Target::Tempo => p.tempo as f32,
            Target::ChannelVolume(c) => p.channels.get(c)?.volume,
            Target::ChannelPan(c) => p.channels.get(c)?.pan,
            Target::MixerVolume(i) => p.mixer.get(i)?.volume,
            Target::MixerPan(i) => p.mixer.get(i)?.pan,
            Target::Effect(i, s, k) => p.mixer.get(i)?.fx[s].as_ref()?.params[k],
        })
    }
}

/// An automation clip: points (song tick, value) joined by straight lines.
#[derive(Clone, PartialEq, Debug)]
pub struct Automation {
    pub target: Target,
    pub points: Vec<(u32, f32)>,
}

impl Automation {
    pub fn value_at(&self, tick: f64) -> Option<f32> {
        let first = self.points.first()?;
        if tick <= first.0 as f64 {
            return Some(first.1);
        }
        for w in self.points.windows(2) {
            let ((t0, v0), (t1, v1)) = (w[0], w[1]);
            if tick < t1 as f64 {
                let f = ((tick - t0 as f64) / (t1 - t0).max(1) as f64) as f32;
                return Some(v0 + (v1 - v0) * f);
            }
        }
        self.points.last().map(|p| p.1)
    }

    pub fn sort(&mut self) {
        self.points.sort_by_key(|p| p.0);
    }

    /// Free drawing: the mouse moved from (t0, v0) to (t1, v1). Points in
    /// [t0, t1] are replaced by one point per `step`, on the straight line.
    pub fn draw(&mut self, t0: u32, v0: f32, t1: u32, v1: f32, step: u32) {
        let min_t = t0.min(t1);
        let max_t = t0.max(t1);

        // Remove points with tick in [min_t, max_t]
        self.points.retain(|&(t, _)| t < min_t || t > max_t);

        if t0 == t1 {
            // Only add (t1, v1)
            self.points.push((t1, v1));
        } else {
            // Find the first multiple of step >= min_t
            let mut t = if min_t.is_multiple_of(step) {
                min_t
            } else {
                min_t + step - (min_t % step)
            };
            while t <= max_t {
                let fraction = (t as f32 - t0 as f32) / (t1 as f32 - t0 as f32);
                let value = v0 + (v1 - v0) * fraction;
                self.points.push((t, value));
                t += step;
            }
        }

        // Sort by tick
        self.points.sort_by_key(|&(t, _)| t);
    }
}

pub struct Voice {
    channel: usize,
    key: u8,
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
    sample: Option<Arc<SampleData>>,
    pos: f64,
    step: f64,
    /// Sample voices: play to the end instead of stopping with the note.
    one_shot: bool,
    /// Frames to wait before starting, so notes land inside a block.
    delay: u32,
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
    pub automations: Vec<Automation>,
    pub mode: Mode,
    pub playing: bool,
    pub position: f64,
    pub sample_rate: f32,
    pub audio_ok: bool,
    pub previews: Vec<(usize, u8)>,
    /// Notes played live (channel, key, velocity): they sound until listed in `releases`.
    pub held: Vec<(usize, u8, u8)>,
    pub releases: Vec<(usize, u8)>,
    pub metronome: bool,
    voices: Vec<Voice>,
    mix: MixState,
}

/// Buffers the mixer reuses from one block to the next.
#[derive(Default)]
struct MixState {
    buses: Vec<Vec<[f32; 2]>>,
    signal: Vec<bool>,
    click: Vec<[f32; 2]>,
    scratch: Vec<[f32; 2]>,
    order: Vec<usize>,
    indegree: Vec<usize>,
    audible: Vec<bool>,
    live_ch: Vec<(f32, f32)>,
    live_mix: Vec<(f32, f32)>,
    live_fx: Vec<(usize, usize, usize, f32)>,
    live_tempo: f64,
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
        let channels: Vec<Channel> =
            defaults.iter().enumerate().map(|(i, (name, inst))| Channel::new(name.to_string(), *inst, i + 1)).collect();
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
            mixer: (0..=MIXER_INSERTS).map(MixerTrack::new).collect(),
            automations: Vec::new(),
            mode: Mode::Pattern,
            playing: false,
            position: 0.0,
            sample_rate: 48000.0,
            audio_ok: false,
            previews: Vec::new(),
            held: Vec::new(),
            releases: Vec::new(),
            metronome: false,
            voices: Vec::new(),
            mix: MixState { live_tempo: DEFAULT_TEMPO, ..Default::default() },
        }
    }

    /// A free name like "Kick", "Kick #2"...
    pub fn channel_name(&self, base: &str) -> String {
        let count = self.channels.iter().filter(|c| c.name == base || c.name.starts_with(&format!("{base} #"))).count();
        if count == 0 { base.to_string() } else { format!("{base} #{}", count + 1) }
    }

    pub fn add_channel(&mut self, inst: Instrument) {
        let name = self.channel_name(inst.label());
        self.push_channel(Channel::new(name, inst, 0));
    }

    /// Adds a channel on its own mixer track, with an empty note list in every pattern.
    pub fn push_channel(&mut self, mut c: Channel) {
        c.mixer = (self.channels.len() + 1).min(MIXER_INSERTS);
        self.channels.push(c);
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
            if v.channel > ch && v.channel != METRONOME {
                v.channel -= 1;
            }
        }
        self.automations.retain(|a| !matches!(a.target, Target::ChannelVolume(c) | Target::ChannelPan(c) if c == ch));
        for a in &mut self.automations {
            match &mut a.target {
                Target::ChannelVolume(c) | Target::ChannelPan(c) if *c > ch => *c -= 1,
                _ => {}
            }
        }
    }

    /// Removes an effect and the automation clips that moved it.
    pub fn remove_effect(&mut self, insert: usize, slot: usize) {
        self.mixer[insert].fx[slot] = None;
        self.automations.retain(|a| !matches!(a.target, Target::Effect(i, s, _) if i == insert && s == slot));
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
            v.one_shot = false;
        }
        for c in &mut self.channels {
            if let Some(pl) = c.plugin.as_mut().and_then(|s| s.live.as_mut()) {
                pl.all_off();
            }
        }
    }

    /// True when sound sent from `from` ends up in `to` (directly or through other tracks).
    pub fn reaches(&self, from: usize, to: usize) -> bool {
        let mut seen = vec![false; self.mixer.len()];
        let mut stack = vec![from];
        while let Some(i) = stack.pop() {
            if i == to {
                return true;
            }
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            stack.extend(self.mixer[i].routes.iter().map(|r| r.0).filter(|&t| t < self.mixer.len()));
        }
        false
    }

    /// Sending `from` to `to` is allowed when it would not make a loop.
    pub fn can_route(&self, from: usize, to: usize) -> bool {
        from != 0 && from != to && !self.reaches(to, from)
    }

    fn swing_offset(&self, tick: u32) -> f64 {
        if tick % (TICKS_PER_STEP * 2) == TICKS_PER_STEP {
            (self.swing * 0.5 * TICKS_PER_STEP as f32) as f64
        } else {
            0.0
        }
    }

    /// Notes whose (swung) start falls in [from, to) of the loop, with how
    /// many ticks after `from` they start.
    fn notes_in(&self, from: f64, to: f64, out: &mut Vec<(usize, Note, f64)>) {
        let mut push = |pat: &Pattern, base: u32| {
            for (ch, list) in pat.notes.iter().enumerate() {
                for n in list {
                    let at = (base + n.start) as f64 + self.swing_offset(n.start);
                    if at >= from && at < to && n.vel > 0 {
                        out.push((ch, *n, at - from));
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

    fn secs_per_tick(&self) -> f32 {
        15.0 / (self.mix.live_tempo.max(TEMPO_MIN) as f32 * PPQ as f32 / 4.0)
    }

    /// Starts a note. `gate` is how long the key is held (None: until released),
    /// `delay` the frames to wait inside the coming block.
    fn start_note(&mut self, ch: usize, key: u8, vel: u8, gate: Option<f32>, one_shot: bool, delay: u32) {
        let sr = self.sample_rate;
        let Some(c) = self.channels.get_mut(ch) else { return };
        if !c.enabled {
            return;
        }
        let gain = vel as f32 / 128.0;
        match c.instrument {
            Instrument::Plugin => {
                if let Some(pl) = c.plugin.as_mut().and_then(|s| s.live.as_mut()) {
                    pl.note_on(key, vel as f32 / 127.0, delay, gate.map(|g| (g * sr) as i64));
                }
            }
            Instrument::Sampler => {
                let Some(data) = c.sample.as_ref().and_then(|s| s.data.clone()) else { return };
                let step = 2f64.powf((key as f64 - 60.0) / 12.0) * data.rate as f64 / sr as f64;
                let mut v = self.new_voice(ch, Instrument::Sampler, key, 0.0, gain, gate.unwrap_or(HELD_GATE_SECS), delay);
                v.sample = Some(data);
                v.step = step;
                v.one_shot = one_shot && gate.is_some();
                self.push(v);
            }
            inst => {
                let base = match inst {
                    Instrument::Kick => 52.0,
                    Instrument::Snare => 185.0,
                    _ => 261.63,
                };
                let freq = base * 2f32.powf((key as f32 - 60.0) / 12.0);
                let v = self.new_voice(ch, inst, key, freq, gain, gate.unwrap_or(HELD_GATE_SECS), delay);
                self.push(v);
            }
        }
    }

    fn release(&mut self, ch: usize, key: u8) {
        for v in self.voices.iter_mut().filter(|v| v.channel == ch && v.key == key && v.gate > v.t) {
            v.gate = v.t;
            v.one_shot = false;
        }
        if let Some(pl) = self.channels.get_mut(ch).and_then(|c| c.plugin.as_mut()).and_then(|s| s.live.as_mut()) {
            pl.note_off(key);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn new_voice(&self, channel: usize, instrument: Instrument, key: u8, freq: f32, gain: f32, gate: f32, delay: u32) -> Voice {
        let seed = 0x9E37_79B9u32 ^ ((channel as u32).wrapping_add(1).wrapping_mul(7919)) ^ ((key as u32) << 16) ^ self.voices.len() as u32;
        Voice {
            channel,
            key,
            instrument,
            freq,
            gain,
            gate,
            t: 0.0,
            phase: 0.0,
            phase2: 0.0,
            seed: seed | 1,
            lp: 0.0,
            hp_prev: 0.0,
            hp: 0.0,
            sample: None,
            pos: 0.0,
            step: 1.0,
            one_shot: false,
            delay,
        }
    }

    fn push(&mut self, v: Voice) {
        self.voices.push(v);
        if self.voices.len() > MAX_VOICES {
            self.voices.remove(0);
        }
    }

    /// Metronome clicks for the beats in [from, to) (accent, ticks after `from`);
    /// the first beat of a bar is higher.
    fn clicks_in(&self, from: f64, to: f64, out: &mut Vec<(bool, f64)>) {
        let mut beat = (from / PPQ as f64).ceil() as u32;
        while ((beat * PPQ) as f64) < to {
            out.push(((beat * PPQ).is_multiple_of(TICKS_PER_BAR), (beat * PPQ) as f64 - from));
            beat += 1;
        }
    }

    /// Adds a live-played note to the current pattern at the nearest step.
    /// Returns true when that step is still ahead, so the sequencer will play it.
    pub fn record_note(&mut self, ch: usize, key: u8, vel: u8) -> bool {
        if !self.playing || self.mode != Mode::Pattern || ch >= self.channels.len() {
            return false;
        }
        let pat = &mut self.patterns[self.current_pattern];
        let len = pat.length_ticks();
        let steps = len / TICKS_PER_STEP;
        let step = (self.position / TICKS_PER_STEP as f64).round() as u32 % steps;
        let start = step * TICKS_PER_STEP;
        let list = &mut pat.notes[ch];
        if !list.iter().any(|n| n.start == start && n.key == key) {
            list.push(Note { key, start, len: TICKS_PER_STEP, vel });
        }
        (start as f64) > self.position || (step == 0 && self.position > (len - TICKS_PER_STEP) as f64)
    }

    /// Where automation is read: the song position, or in pattern mode the
    /// place of the pattern's first clip in the playlist plus the position
    /// inside the pattern (None when the pattern is not in the playlist).
    pub fn automation_tick(&self) -> Option<f64> {
        match self.mode {
            Mode::Song => Some(self.position),
            Mode::Pattern => {
                let start = self.clips.iter().filter(|c| c.pattern == self.current_pattern).map(|c| c.start_bar).min()?;
                Some((start * TICKS_PER_BAR) as f64 + self.position)
            }
        }
    }

    /// Values for this block: the saved ones, replaced by automation.
    fn update_live(&mut self) {
        let tick = self.automation_tick();
        let m = &mut self.mix;
        m.live_ch.clear();
        m.live_ch.extend(self.channels.iter().map(|c| (c.volume, c.pan)));
        m.live_mix.clear();
        m.live_mix.extend(self.mixer.iter().map(|t| (t.volume, t.pan)));
        m.live_fx.clear();
        m.live_tempo = self.tempo;
        let Some(tick) = tick else { return };
        for a in &self.automations {
            let Some(v) = a.value_at(tick) else { continue };
            match a.target {
                Target::Tempo => m.live_tempo = (v as f64).clamp(TEMPO_MIN, TEMPO_MAX),
                Target::ChannelVolume(c) => {
                    if let Some(x) = m.live_ch.get_mut(c) {
                        x.0 = v;
                    }
                }
                Target::ChannelPan(c) => {
                    if let Some(x) = m.live_ch.get_mut(c) {
                        x.1 = v;
                    }
                }
                Target::MixerVolume(i) => m.live_mix[i].0 = v,
                Target::MixerPan(i) => m.live_mix[i].1 = v,
                Target::Effect(i, s, k) => m.live_fx.push((i, s, k, v)),
            }
        }
    }

    /// The value automation gives a target right now, for drawing knobs.
    pub fn live_tempo(&self) -> f64 {
        self.mix.live_tempo
    }

    fn transport(&self) -> Transport {
        let beats = self.position / PPQ as f64;
        Transport { tempo: self.mix.live_tempo, playing: self.playing, beats }
    }

    /// Fill an interleaved buffer (stereo in the first two channels).
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        let frames = out.len() / channels;
        let sr = self.sample_rate;
        let dt = 1.0 / sr;
        let tracks = self.mixer.len();
        {
            let m = &mut self.mix;
            m.buses.resize_with(tracks, Vec::new);
            for b in &mut m.buses {
                b.clear();
                b.resize(frames, [0.0; 2]);
            }
            m.signal.clear();
            m.signal.resize(tracks, false);
            m.click.clear();
            m.click.resize(frames, [0.0; 2]);
        }
        for tr in &mut self.mixer {
            tr.peak[0] *= 0.6;
            tr.peak[1] *= 0.6;
        }
        self.update_live();
        let transport = self.transport();

        let step_gate = TICKS_PER_STEP as f32 * 2.0 * self.secs_per_tick();
        for (ch, key) in std::mem::take(&mut self.previews) {
            self.start_note(ch, key, NOTE_VELOCITY, Some(step_gate), true, 0);
        }
        for (ch, key, vel) in std::mem::take(&mut self.held) {
            self.start_note(ch, key, vel, None, false, 0);
        }
        for (ch, key) in std::mem::take(&mut self.releases) {
            self.release(ch, key);
        }

        const CHUNK: usize = 32;
        let mut events = Vec::new();
        let mut clicks = Vec::new();
        let mut frame = 0;
        while frame < frames {
            let n = CHUNK.min(frames - frame);
            if self.playing {
                // sub_1311510: samples per tick = rate x 15 / (BPM x PPQ / 4).
                let ticks_per_sample = (self.mix.live_tempo * PPQ as f64 / 4.0) / (sr as f64 * 15.0);
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
                    let before = events.len();
                    self.notes_in(0.0, to - len, &mut events);
                    for e in &mut events[before..] {
                        e.2 += len - from;
                    }
                }
                let spt = self.secs_per_tick();
                let last = frames as u32 - 1;
                for &(ch, note, ahead) in &events {
                    let delay = (frame as u32 + (ahead / ticks_per_sample) as u32).min(last);
                    let one_shot = note.len <= TICKS_PER_STEP;
                    self.start_note(ch, note.key, note.vel, Some(note.len as f32 * spt), one_shot, delay);
                }
                if self.metronome {
                    clicks.clear();
                    if to <= len {
                        self.clicks_in(from, to, &mut clicks);
                    } else {
                        self.clicks_in(from, len, &mut clicks);
                        let before = clicks.len();
                        self.clicks_in(0.0, to - len, &mut clicks);
                        for c in &mut clicks[before..] {
                            c.1 += len - from;
                        }
                    }
                    for &(accent, ahead) in &clicks {
                        let freq = if accent { 1760.0 } else { 1320.0 };
                        let delay = (frame as u32 + (ahead.max(0.0) / ticks_per_sample) as u32).min(last);
                        let v = self.new_voice(METRONOME, Instrument::Click, 0, freq, if accent { 0.5 } else { 0.35 }, 0.03, delay);
                        self.push(v);
                    }
                }
                self.position = if to >= len { to - len } else { to };
            }

            let Project { voices, channels: chs, mix, .. } = self;
            for v in voices.iter_mut() {
                if v.channel == METRONOME {
                    for f in mix.click[frame..frame + n].iter_mut() {
                        if v.delay > 0 {
                            v.delay -= 1;
                            continue;
                        }
                        let s = voice_sample(v, dt);
                        f[0] += s[0] * v.gain;
                        f[1] += s[1] * v.gain;
                    }
                    continue;
                }
                let Some(ch) = chs.get(v.channel) else { continue };
                let (vol, pan) = mix.live_ch.get(v.channel).copied().unwrap_or((ch.volume, ch.pan));
                let pan = (pan / PAN_CENTRE).clamp(-1.0, 1.0);
                let g = v.gain * vol / VOL_SCALE * if ch.enabled { 1.0 } else { 0.0 };
                let (gl, gr) = (g * (1.0 - pan).min(1.0), g * (1.0 + pan).min(1.0));
                let idx = ch.mixer.min(MIXER_INSERTS);
                mix.signal[idx] = true;
                for f in mix.buses[idx][frame..frame + n].iter_mut() {
                    if v.delay > 0 {
                        v.delay -= 1;
                        continue;
                    }
                    let s = voice_sample(v, dt);
                    f[0] += s[0] * gl;
                    f[1] += s[1] * gr;
                }
            }
            voices.retain(|v| !voice_done(v));
            frame += n;
        }

        // Instrument plugins render the whole block at once.
        let Project { channels: chs, mix, .. } = self;
        for (ci, c) in chs.iter_mut().enumerate() {
            let Some(pl) = c.plugin.as_mut().and_then(|s| s.live.as_mut()) else { continue };
            mix.scratch.clear();
            mix.scratch.resize(frames, [0.0; 2]);
            pl.render(&mut mix.scratch, &transport);
            let (vol, pan) = mix.live_ch.get(ci).copied().unwrap_or((c.volume, c.pan));
            let pan = (pan / PAN_CENTRE).clamp(-1.0, 1.0);
            let g = vol / VOL_SCALE * if c.enabled { 1.0 } else { 0.0 };
            let (gl, gr) = (g * (1.0 - pan).min(1.0), g * (1.0 + pan).min(1.0));
            let idx = c.mixer.min(MIXER_INSERTS);
            mix.signal[idx] = true;
            for (b, s) in mix.buses[idx].iter_mut().zip(&mix.scratch) {
                b[0] += s[0] * gl;
                b[1] += s[1] * gr;
            }
        }

        self.mix_down(&transport);
        for (i, f) in self.mix.buses[0].iter().enumerate() {
            let base = i * channels;
            out[base] = f[0];
            if channels > 1 {
                out[base + 1] = f[1];
            }
            for c in 2..channels {
                out[base + c] = 0.0;
            }
        }
    }

    /// Order in which inserts are processed: each after those that send to it.
    /// The master is processed last, apart.
    fn mix_order(&mut self) {
        let n = self.mixer.len();
        let m = &mut self.mix;
        m.indegree.clear();
        m.indegree.resize(n, 0);
        for t in &self.mixer[1..] {
            for &(to, _) in &t.routes {
                if to < n {
                    m.indegree[to] += 1;
                }
            }
        }
        m.order.clear();
        m.order.extend((1..n).filter(|&i| m.indegree[i] == 0));
        let mut at = 0;
        while at < m.order.len() {
            let i = m.order[at];
            at += 1;
            for &(to, _) in &self.mixer[i].routes {
                if to > 0 && to < n {
                    m.indegree[to] -= 1;
                    if m.indegree[to] == 0 {
                        m.order.push(to);
                    }
                }
            }
        }
        // A loop from a hand-edited file: process the rest by number.
        if m.order.len() < n - 1 {
            for i in 1..n {
                if !m.order.contains(&i) {
                    m.order.push(i);
                }
            }
        }
    }

    /// Solo: soloed inserts stay audible, with what feeds them and where they go.
    fn update_audible(&mut self) {
        let n = self.mixer.len();
        let soloed: Vec<usize> = (1..n).filter(|&i| self.mixer[i].solo).collect();
        self.mix.audible.clear();
        if soloed.is_empty() {
            self.mix.audible.resize(n, true);
            return;
        }
        let audible: Vec<bool> =
            (0..n).map(|i| i == 0 || soloed.iter().any(|&s| s == i || self.reaches(i, s) || self.reaches(s, i))).collect();
        self.mix.audible = audible;
    }

    fn mix_down(&mut self, transport: &Transport) {
        self.mix_order();
        self.update_audible();
        let sr = self.sample_rate;
        let order = std::mem::take(&mut self.mix.order);
        for &i in order.iter().chain(std::iter::once(&0)) {
            let has_fx = self.mixer[i].fx.iter().flatten().any(|e| e.enabled);
            if !self.mix.signal[i] && !has_fx && i != 0 {
                continue;
            }
            let mut buf = std::mem::take(&mut self.mix.buses[i]);
            for (s, slot) in self.mixer[i].fx.iter_mut().enumerate() {
                let Some(e) = slot else { continue };
                let mut params = e.params;
                for &(fi, fs, k, v) in &self.mix.live_fx {
                    if fi == i && fs == s {
                        params[k] = v.clamp(0.0, 1.0);
                    }
                }
                e.process(&mut buf, &params, sr, transport);
            }
            if i == 0 {
                // The metronome skips the master effects but follows the master fader.
                for (b, c) in buf.iter_mut().zip(&self.mix.click) {
                    b[0] += c[0];
                    b[1] += c[1];
                }
            }
            let tr = &mut self.mixer[i];
            let (vol, pan) = self.mix.live_mix[i];
            let on = tr.enabled && self.mix.audible[i];
            let g = if on { vol / VOL_SCALE } else { 0.0 };
            let p = (pan / PAN_CENTRE).clamp(-1.0, 1.0);
            let (gl, gr) = (g * (1.0 - p).min(1.0), g * (1.0 + p).min(1.0));
            let mut peak = [0.0f32; 2];
            for f in buf.iter_mut() {
                f[0] *= gl;
                f[1] *= gr;
                if i == 0 {
                    f[0] = soft_clip(f[0]);
                    f[1] = soft_clip(f[1]);
                }
                peak[0] = peak[0].max(f[0].abs());
                peak[1] = peak[1].max(f[1].abs());
            }
            tr.peak[0] = tr.peak[0].max(peak[0]);
            tr.peak[1] = tr.peak[1].max(peak[1]);
            if on && i != 0 {
                for &(to, level) in &tr.routes {
                    if to >= self.mix.buses.len() || to == i {
                        continue;
                    }
                    self.mix.signal[to] = true;
                    for (d, s) in self.mix.buses[to].iter_mut().zip(&buf) {
                        d[0] += s[0] * level;
                        d[1] += s[1] * level;
                    }
                }
            }
            self.mix.buses[i] = buf;
        }
        self.mix.order = order;
    }

    /// After undo: keeps what is not saved in the project text (the sound
    /// device state, decoded samples, running plugins, effect tails).
    pub fn keep_runtime(&mut self, old: &mut Project) {
        self.sample_rate = old.sample_rate;
        self.audio_ok = old.audio_ok;
        self.metronome = old.metronome;
        self.playing = old.playing;
        self.position = old.position;
        self.voices = std::mem::take(&mut old.voices);
        let n = self.channels.len();
        self.voices.retain(|v| v.channel == METRONOME || v.channel < n);
        self.mix = std::mem::take(&mut old.mix);
        for c in &mut self.channels {
            if let Some(s) = &mut c.sample {
                s.data = old
                    .channels
                    .iter()
                    .filter_map(|o| o.sample.as_ref())
                    .find(|o| o.path == s.path && o.data.is_some())
                    .and_then(|o| o.data.clone());
            }
            if let Some(slot) = &mut c.plugin {
                let found = old.channels.iter_mut().filter_map(|o| o.plugin.as_mut()).find(|o| o.info.id == slot.info.id && o.live.is_some());
                if let Some(o) = found {
                    slot.live = o.live.take();
                    slot.state = None;
                    slot.restored = true;
                }
            }
        }
        for (i, t) in self.mixer.iter_mut().enumerate() {
            for (s, e) in t.fx.iter_mut().enumerate() {
                let Some(e) = e else { continue };
                if let Some(slot) = &mut e.plugin {
                    let id = slot.info.id.clone();
                    let usable = |o: &&mut Box<PluginSlot<PluginEffect>>| o.info.id == id && o.live.is_some();
                    let here = old.mixer[i].fx[s].as_mut().and_then(|o| o.plugin.as_mut()).filter(|o| usable(o)).is_some();
                    let found = if here {
                        old.mixer[i].fx[s].as_mut().and_then(|o| o.plugin.as_mut())
                    } else {
                        old.mixer.iter_mut().flat_map(|t| t.fx.iter_mut().flatten()).filter_map(|o| o.plugin.as_mut()).find(|o| usable(o))
                    };
                    if let Some(o) = found {
                        slot.live = o.live.take();
                        slot.state = None;
                        slot.restored = true;
                    }
                } else if let Some(o) = old.mixer[i].fx[s].as_mut() {
                    e.keep_state(o);
                }
            }
        }
    }

    /// Every plugin slot of the project, for loading and housekeeping.
    #[allow(clippy::type_complexity)]
    pub fn plugin_slots(&mut self) -> (Vec<&mut PluginSlot<PluginInstrument>>, Vec<&mut PluginSlot<PluginEffect>>) {
        let inst = self.channels.iter_mut().filter_map(|c| c.plugin.as_deref_mut()).collect();
        let fx = self.mixer.iter_mut().flat_map(|t| t.fx.iter_mut().flatten()).filter_map(|e| e.plugin.as_deref_mut()).collect();
        (inst, fx)
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
    if v.delay > 0 {
        return false;
    }
    let tail = match v.instrument {
        Instrument::Kick => 0.7,
        Instrument::Clap => 0.45,
        Instrument::HiHat => 0.25,
        Instrument::Snare => 0.45,
        Instrument::Synth => return v.t > v.gate + 0.35,
        Instrument::Sampler => {
            let len = v.sample.as_ref().map_or(0, |s| s.frames.len());
            return v.pos >= len as f64 || (!v.one_shot && v.t > v.gate + SAMPLE_RELEASE_SECS);
        }
        Instrument::Plugin => return true,
        Instrument::Click => 0.05,
    };
    v.t > tail
}

fn voice_sample(v: &mut Voice, dt: f32) -> [f32; 2] {
    let t = v.t;
    v.t += dt;
    let mono = match v.instrument {
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
        Instrument::Sampler => {
            let Some(s) = &v.sample else { return [0.0; 2] };
            let i = v.pos as usize;
            if i + 1 >= s.frames.len() {
                v.pos = s.frames.len() as f64;
                return [0.0; 2];
            }
            let f = (v.pos - i as f64) as f32;
            let (a, b) = (s.frames[i], s.frames[i + 1]);
            v.pos += v.step;
            let env = if !v.one_shot && t > v.gate { (1.0 - (t - v.gate) / SAMPLE_RELEASE_SECS).max(0.0) } else { 1.0 };
            // The voice gain (velocity / 128) would leave samples quieter than recorded.
            let g = env * 1.25;
            return [(a[0] + (b[0] - a[0]) * f) * g, (a[1] + (b[1] - a[1]) * f) * g];
        }
        Instrument::Plugin => 0.0,
        Instrument::Click => {
            v.phase = (v.phase + v.freq * dt).fract();
            (v.phase * TAU).sin() * (-t * 90.0).exp()
        }
    };
    [mono, mono]
}

/// Rounds to the engine's BPM x 1000 steps inside 10000..=522000.
pub fn quantize_tempo(bpm: f64) -> f64 {
    (bpm * TEMPO_SCALE).round().clamp(TEMPO_RAW_MIN, TEMPO_RAW_MAX) / TEMPO_SCALE
}

/// Names of the sound outputs the system offers.
pub fn output_devices() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let Ok(devices) = host.output_devices() else { return Vec::new() };
    devices.filter_map(|d| d.name().ok()).collect()
}

/// Opens a sound output (the named one, else the default) with the given buffer
/// size if the device accepts it. Returns the stream so it stays alive.
pub fn start_audio(shared: Shared, device: Option<&str>, buffer: Option<u32>) -> Option<cpal::Stream> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    if let Ok(mut p) = shared.lock() {
        p.audio_ok = false;
    }
    let host = cpal::default_host();
    let named = device.and_then(|name| host.output_devices().ok()?.find(|d| d.name().ok().as_deref() == Some(name)));
    let device = named.or_else(|| host.default_output_device())?;
    let supported = device.default_output_config().ok()?;
    let format = supported.sample_format();
    let base: cpal::StreamConfig = supported.into();
    let chans = base.channels as usize;
    if let Ok(mut p) = shared.lock() {
        p.sample_rate = base.sample_rate.0 as f32;
    }
    let build = |config: &cpal::StreamConfig| {
        let err = |e| eprintln!("audio error: {e}");
        match format {
            cpal::SampleFormat::F32 => {
                let s = shared.clone();
                device.build_output_stream(
                    config,
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
                let mut scratch: Vec<f32> = Vec::new();
                device.build_output_stream(
                    config,
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
            _ => Err(cpal::BuildStreamError::StreamConfigNotSupported),
        }
    };
    let fixed = buffer.map(|b| cpal::StreamConfig { buffer_size: cpal::BufferSize::Fixed(b), ..base.clone() });
    // A device may refuse the buffer size; fall back to its own choice.
    let stream = fixed.and_then(|c| build(&c).ok()).or_else(|| build(&base).ok())?;
    stream.play().ok()?;
    if let Ok(mut p) = shared.lock() {
        p.audio_ok = true;
    }
    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::EffectKind;

    fn loudest(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |m, x| m.max(x.abs()))
    }

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

    #[test]
    fn metronome_clicks_on_beats() {
        let mut p = Project::new();
        p.metronome = true;
        p.playing = true;
        let mut buf = vec![0.0; 48000 * 2];
        p.render(&mut buf, 2);
        assert!(buf.iter().any(|x| x.abs() > 0.05));
        let mut clicks = Vec::new();
        p.clicks_in(0.0, TICKS_PER_BAR as f64, &mut clicks);
        let accents: Vec<bool> = clicks.iter().map(|c| c.0).collect();
        assert_eq!(accents, vec![true, false, false, false]);
    }

    #[test]
    fn recording_snaps_to_steps() {
        let mut p = Project::new();
        p.playing = true;
        p.position = 25.0;
        assert!(!p.record_note(0, 60, NOTE_VELOCITY)); // step 2 starts at 24, already passed
        p.position = 40.0;
        assert!(p.record_note(0, 60, NOTE_VELOCITY)); // step 3 at 48 is still ahead
        assert!(p.patterns[0].has_step(0, 1));
        assert!(p.patterns[0].has_step(0, 2));
        p.position = (TICKS_PER_BAR - 5) as f64;
        assert!(p.record_note(1, 60, NOTE_VELOCITY)); // wraps to the first step
        assert!(p.patterns[0].has_step(1, 0));
    }

    #[test]
    fn mute_solo_and_routing() {
        let mut p = Project::new();
        p.patterns[0].set_step(0, 0, true);
        p.playing = true;
        let mut buf = vec![0.0; 9600 * 2];
        p.render(&mut buf, 2);
        assert!(loudest(&buf) > 0.05);

        // Soloing another insert silences the kick on insert 1.
        let mut p2 = Project::new();
        p2.patterns[0].set_step(0, 0, true);
        p2.playing = true;
        p2.mixer[3].solo = true;
        p2.render(&mut buf, 2);
        assert!(loudest(&buf) < 1e-4);

        // Insert 1 sent through insert 10 (soloed) still plays.
        p2.position = 0.0;
        p2.mixer[3].solo = false;
        p2.mixer[10].solo = true;
        p2.mixer[1].routes = vec![(10, 1.0)];
        p2.render(&mut buf, 2);
        assert!(loudest(&buf) > 0.05);
        assert!(!p2.can_route(10, 1));
        assert!(p2.can_route(1, 11));
    }

    #[test]
    fn effects_on_inserts_change_the_sound() {
        let render = |reverb: bool| {
            let mut p = Project::new();
            p.patterns[0].set_step(0, 0, true);
            p.playing = true;
            if reverb {
                p.mixer[1].fx[0] = Some(Effect::new(EffectKind::Reverb));
            }
            let mut buf = vec![0.0; 36000 * 2];
            p.render(&mut buf, 2);
            buf
        };
        // The kick has died away after 0.7 s; the reverb is still ringing.
        let tail = |b: &[f32]| loudest(&b[34000 * 2..]);
        assert!(tail(&render(true)) > tail(&render(false)) * 10.0 + 1e-5);
    }

    #[test]
    fn sampler_plays_the_sound_file() {
        let mut p = Project::new();
        let mut c = Channel::new("Tone".into(), Instrument::Sampler, 0);
        let frames = (0..24000).map(|i| [(i as f32 * 0.05).sin() * 0.5; 2]).collect();
        c.sample = Some(SampleRef { path: "tone.wav".into(), data: Some(Arc::new(SampleData { rate: 48000.0, frames })) });
        p.push_channel(c);
        let ch = p.channels.len() - 1;
        p.patterns[0].set_step(ch, 0, true);
        p.playing = true;
        let mut buf = vec![0.0; 40000 * 2];
        p.render(&mut buf, 2);
        // A one-step note plays the whole half-second file.
        assert!(loudest(&buf[20000 * 2..23000 * 2]) > 0.05);
        assert!(loudest(&buf[30000 * 2..]) < 1e-3);
    }

    #[test]
    fn automation_follows_its_points() {
        let a = Automation { target: Target::Tempo, points: vec![(0, 100.0), (384, 200.0)] };
        assert_eq!(a.value_at(-5.0), Some(100.0));
        assert_eq!(a.value_at(192.0), Some(150.0));
        assert_eq!(a.value_at(9999.0), Some(200.0));
        let mut p = Project::new();
        p.mode = Mode::Song;
        p.automations.push(Automation { target: Target::MixerVolume(1), points: vec![(0, 0.0)] });
        p.patterns[0].set_step(0, 0, true);
        p.clips.push(Clip { track: 0, start_bar: 0, pattern: 0 });
        p.playing = true;
        let mut buf = vec![0.0; 9600 * 2];
        p.render(&mut buf, 2);
        assert!(loudest(&buf) < 1e-4, "insert 1 is automated to silence");
        assert_eq!(Target::parse(&Target::Effect(3, 2, 1).key()), Some(Target::Effect(3, 2, 1)));
    }

    #[test]
    fn removing_a_channel_moves_its_automation() {
        let mut p = Project::new();
        p.automations.push(Automation { target: Target::ChannelVolume(1), points: vec![] });
        p.automations.push(Automation { target: Target::ChannelPan(3), points: vec![] });
        p.remove_channel(1);
        assert_eq!(p.automations.len(), 1);
        assert_eq!(p.automations[0].target, Target::ChannelPan(2));
    }
}

#[cfg(test)]
mod draw_tests {
    use super::*;

    #[test]
    fn test_right_draw() {
        let mut auto = Automation {
            target: Target::Tempo,
            points: vec![(0, 5.0), (48, 5.0), (200, 5.0)],
        };
        auto.draw(0, 0.0, 96, 1.0, 24);
        assert_eq!(
            auto.points,
            vec![(0, 0.0), (24, 0.25), (48, 0.5), (72, 0.75), (96, 1.0), (200, 5.0)]
        );
    }

    #[test]
    fn test_left_draw() {
        let mut auto = Automation {
            target: Target::Tempo,
            points: vec![(0, 3.0)],
        };
        auto.draw(96, 1.0, 48, 0.0, 24);
        assert_eq!(
            auto.points,
            vec![(0, 3.0), (48, 0.0), (72, 0.5), (96, 1.0)]
        );
    }

    #[test]
    fn test_same_t() {
        let mut auto = Automation {
            target: Target::Tempo,
            points: vec![(30, 9.0)],
        };
        auto.draw(30, 9.0, 30, 2.0, 24);
        assert_eq!(
            auto.points,
            vec![(30, 2.0)]
        );
    }
}
