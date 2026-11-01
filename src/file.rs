// Project files (.bsp, plain text, one item per line), WAV / MP3 export,
// templates and the demo projects.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::effects::{Effect, EffectKind, FX_SLOTS, PARAMS};
use crate::engine::{
    Automation, Channel, Clip, Instrument, Mode, Note, Pattern, Project, SampleData, SampleRef, Target, FADER_DEFAULT, MIXER_INSERTS,
    PLAYLIST_TRACKS, PPQ, TICKS_PER_BAR, TICKS_PER_STEP,
};
use crate::plugin::{self, HasCore, PluginCore, PluginEffect, PluginInfo, PluginInstrument, PluginSlot};

pub const EXTENSION: &str = "bsp";
pub const AUTOSAVE_NAME: &str = "Autosave";
const MAGIC: &str = "BEATSTUDIO 1";
pub const EXPORT_RATE: u32 = 44100;
const EXPORT_TAIL_SECS: f32 = 2.0;

/// Where projects and exports go: ~/Documents/Beat Studio (or ~/Beat Studio).
pub fn projects_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let docs = home.join("Documents");
    if docs.is_dir() { docs.join("Beat Studio") } else { home.join("Beat Studio") }
}

/// Sub-folders of the projects folder, as shown in the Browser.
pub fn folder(name: &str) -> PathBuf {
    projects_dir().join(name)
}

pub const TEMPLATES: &str = "Templates";
pub const RECORDED: &str = "Recorded";
pub const RENDERED: &str = "Rendered";
pub const SAMPLES: &str = "Samples";
pub const SCORES: &str = "Scores";

/// Files in a folder with one of the endings, most recently changed first.
pub fn list_files(dir: &Path, exts: &[&str]) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| exts.contains(&e.to_lowercase().as_str())))
        .filter_map(|p| Some((fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.into_iter().map(|(_, p)| p).collect()
}

/// Project files in the projects folder, most recently changed first.
pub fn list_projects() -> Vec<PathBuf> {
    list_files(&projects_dir(), &[EXTENSION])
}

/// Turns a typed name into a path in `dir`, adding the extension.
pub fn path_in(dir: &Path, name: &str, ext: &str) -> PathBuf {
    let clean: String = name.trim().chars().map(|c| if c == '/' || c == '\\' { '_' } else { c }).collect();
    let clean = if clean.is_empty() { "Untitled".to_string() } else { clean };
    let mut path = dir.join(clean);
    if path.extension().is_none_or(|e| e != ext) {
        let mut s = path.into_os_string();
        s.push(".");
        s.push(ext);
        path = PathBuf::from(s);
    }
    path
}

/// Turns a typed name into a path in the projects folder, adding the extension.
pub fn path_for(name: &str, ext: &str) -> PathBuf {
    path_in(&projects_dir(), name, ext)
}

pub fn display_name(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r', '\t'], " ")
}

fn bit(b: bool) -> u8 {
    b as u8
}

fn plugin_line(info: &PluginInfo) -> String {
    format!("{}\t{}\t{}\t{}", info.path.display(), info.id, one_line(&info.name), one_line(&info.vendor))
}

/// Parameter values and (when saving a file) the plugin's own saved state.
fn plugin_details<T: HasCore>(s: &mut String, slot: &PluginSlot<T>, with_state: bool) {
    match &slot.live {
        Some(live) => {
            let core = live.core();
            // Only changed values: some plugins have thousands of parameters.
            for p in core.params() {
                if let Some(v) = core.value(p.id).filter(|&v| v != p.default) {
                    s.push_str(&format!("pvalue {} {v}\n", p.id));
                }
            }
            if with_state {
                if let Some(st) = core.save_state() {
                    s.push_str(&format!("pstate {}\n", plugin::hex(&st)));
                }
            }
        }
        None => {
            for (id, v) in &slot.values {
                s.push_str(&format!("pvalue {id} {v}\n"));
            }
            if let (true, Some(st)) = (with_state, &slot.state) {
                s.push_str(&format!("pstate {}\n", plugin::hex(st)));
            }
        }
    }
}

/// Writes the musical state (not the playback state) as text. Plugin states
/// are only written to files (`with_state`): reading them from every plugin
/// is too slow for the change tracking that compares texts.
fn write_text(p: &Project, with_state: bool) -> String {
    let mut s = String::new();
    s.push_str(MAGIC);
    s.push('\n');
    s.push_str(&format!("title {}\n", one_line(&p.title)));
    s.push_str(&format!("author {}\n", one_line(&p.author)));
    s.push_str(&format!("tempo {:.3}\n", p.tempo));
    s.push_str(&format!("swing {}\n", p.swing));
    s.push_str(&format!("mode {}\n", if p.mode == Mode::Song { "song" } else { "pattern" }));
    s.push_str(&format!("current {}\n", p.current_pattern));
    for c in &p.channels {
        s.push_str(&format!(
            "channel {} {} {} {} {} {}\n",
            c.instrument.key(),
            c.volume,
            c.pan,
            bit(c.enabled),
            c.mixer,
            one_line(&c.name)
        ));
        if let Some(sm) = &c.sample {
            s.push_str(&format!("sample {}\n", sm.path.display()));
        }
        if let Some(slot) = &c.plugin {
            s.push_str(&format!("chplugin\t{}\n", plugin_line(&slot.info)));
            plugin_details(&mut s, slot, with_state);
        }
    }
    for (i, t) in p.mixer.iter().enumerate() {
        if t.volume != FADER_DEFAULT || t.pan != 0.0 || !t.enabled {
            s.push_str(&format!("mixer {i} {} {} {}\n", t.volume, t.pan, bit(t.enabled)));
        }
        if t.solo {
            s.push_str(&format!("solo {i}\n"));
        }
        if !t.default_routes(i) {
            let list: Vec<String> = t.routes.iter().map(|(to, l)| format!("{to}:{l}")).collect();
            s.push_str(&format!("routes {i} {}\n", list.join(" ")));
        }
        for (slot, e) in t.fx.iter().enumerate() {
            let Some(e) = e else { continue };
            match &e.plugin {
                Some(pl) => {
                    s.push_str(&format!("fxplugin {i} {slot} {}\t{}\n", bit(e.enabled), plugin_line(&pl.info)));
                    plugin_details(&mut s, pl, with_state);
                }
                None => {
                    let ps: Vec<String> = e.params.iter().map(|v| v.to_string()).collect();
                    s.push_str(&format!("fx {i} {slot} {} {} {}\n", e.kind.key(), bit(e.enabled), ps.join(" ")));
                }
            }
        }
    }
    for pat in &p.patterns {
        s.push_str(&format!("pattern {}\n", one_line(&pat.name)));
        for (ch, list) in pat.notes.iter().enumerate() {
            for n in list {
                s.push_str(&format!("note {ch} {} {} {} {}\n", n.key, n.start, n.len, n.vel));
            }
        }
    }
    for c in &p.clips {
        s.push_str(&format!("clip {} {} {}\n", c.track, c.start_bar, c.pattern));
    }
    for a in &p.automations {
        s.push_str(&format!("auto {}\n", a.target.key()));
        for (t, v) in &a.points {
            s.push_str(&format!("point {t} {v}\n"));
        }
    }
    s
}

/// The project as text without plugin states, for change tracking and undo.
pub fn to_text(p: &Project) -> String {
    write_text(p, false)
}

fn parse_plugin(fields: &str, instrument: bool) -> Option<PluginInfo> {
    let mut f = fields.split('\t');
    let path = PathBuf::from(f.next()?);
    let id = f.next()?.to_string();
    let name = f.next().unwrap_or(&id).to_string();
    let vendor = f.next().unwrap_or("").to_string();
    Some(PluginInfo { path, id, name, vendor, instrument })
}

/// The plugin the next `pvalue` / `pstate` lines belong to.
enum LastPlugin {
    None,
    Channel(usize),
    Effect(usize, usize),
}

/// Reads a project written by `to_text`. Out-of-range items are dropped.
pub fn from_text(text: &str) -> Result<Project, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(MAGIC) {
        return Err("not a Beat Studio project".into());
    }
    let mut p = Project::new();
    p.channels.clear();
    p.patterns.clear();
    let mut notes: Vec<(usize, usize, Note)> = Vec::new();
    let mut clips: Vec<Clip> = Vec::new();
    let mut last = LastPlugin::None;
    for (no, line) in lines.enumerate() {
        let bad = || format!("line {}: cannot read \"{line}\"", no + 2);
        let (word, rest) = line.split_once([' ', '\t']).unwrap_or((line, ""));
        let nums: Vec<&str> = rest.split_whitespace().collect();
        let num = |i: usize| nums.get(i).and_then(|v| v.parse::<f64>().ok()).ok_or_else(bad);
        match word {
            "title" => p.title = rest.to_string(),
            "author" => p.author = rest.to_string(),
            "tempo" => p.tempo = crate::engine::quantize_tempo(num(0)?),
            "swing" => p.swing = (num(0)? as f32).clamp(0.0, 1.0),
            "mode" => p.mode = if rest.trim() == "song" { Mode::Song } else { Mode::Pattern },
            "current" => p.current_pattern = num(0)? as usize,
            "channel" => {
                let mut parts = rest.splitn(6, ' ');
                let inst = parts.next().and_then(Instrument::from_key).ok_or_else(bad)?;
                let mut field = || parts.next().and_then(|v| v.parse::<f32>().ok()).ok_or_else(bad);
                let (volume, pan, enabled, mixer) = (field()?, field()?, field()?, field()?);
                let name = parts.next().unwrap_or(inst.label()).to_string();
                let mut c = Channel::new(name, inst, (mixer as usize).min(MIXER_INSERTS));
                c.volume = volume.clamp(0.0, FADER_DEFAULT);
                c.pan = pan.clamp(-6400.0, 6400.0);
                c.enabled = enabled != 0.0;
                p.channels.push(c);
                last = LastPlugin::None;
            }
            "sample" => {
                let c = p.channels.last_mut().ok_or_else(bad)?;
                c.sample = Some(SampleRef { path: PathBuf::from(rest), data: None });
            }
            "chplugin" => {
                let info = parse_plugin(rest, true).ok_or_else(bad)?;
                let c = p.channels.last_mut().ok_or_else(bad)?;
                c.plugin = Some(Box::new(PluginSlot::new(info)));
                last = LastPlugin::Channel(p.channels.len() - 1);
            }
            "mixer" => {
                let i = num(0)? as usize;
                if let Some(t) = p.mixer.get_mut(i) {
                    t.volume = (num(1)? as f32).clamp(0.0, crate::engine::FADER_MAX);
                    t.pan = (num(2)? as f32).clamp(-6400.0, 6400.0);
                    t.enabled = num(3)? != 0.0;
                }
            }
            "solo" => {
                if let Some(t) = p.mixer.get_mut(num(0)? as usize) {
                    t.solo = true;
                }
            }
            "routes" => {
                let i = num(0)? as usize;
                let routes: Vec<(usize, f32)> = nums[1..]
                    .iter()
                    .filter_map(|r| {
                        let (to, l) = r.split_once(':')?;
                        Some((to.parse::<usize>().ok()?.min(MIXER_INSERTS), l.parse::<f32>().ok()?.clamp(0.0, 2.0)))
                    })
                    .filter(|r| r.0 != i)
                    .collect();
                if let (true, Some(t)) = (i > 0, p.mixer.get_mut(i)) {
                    t.routes = routes;
                }
            }
            "fx" => {
                let (i, slot) = (num(0)? as usize, num(1)? as usize);
                let kind = nums.get(2).and_then(|k| EffectKind::from_key(k)).ok_or_else(bad)?;
                if i <= MIXER_INSERTS && slot < FX_SLOTS {
                    let mut e = Effect::new(kind);
                    e.enabled = num(3)? != 0.0;
                    for k in 0..PARAMS {
                        e.params[k] = (num(4 + k)? as f32).clamp(0.0, 1.0);
                    }
                    p.mixer[i].fx[slot] = Some(e);
                }
            }
            "fxplugin" => {
                let (head, fields) = rest.split_once('\t').ok_or_else(bad)?;
                let h: Vec<usize> = head.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                let (i, slot, enabled) = (*h.first().ok_or_else(bad)?, *h.get(1).ok_or_else(bad)?, *h.get(2).ok_or_else(bad)?);
                let info = parse_plugin(fields, false).ok_or_else(bad)?;
                if i <= MIXER_INSERTS && slot < FX_SLOTS {
                    let mut e = Effect::plugin(PluginSlot::new(info));
                    e.enabled = enabled != 0;
                    p.mixer[i].fx[slot] = Some(e);
                    last = LastPlugin::Effect(i, slot);
                }
            }
            "pvalue" | "pstate" => {
                let slot_values = match last {
                    LastPlugin::Channel(c) => p.channels[c].plugin.as_mut().map(|s| (&mut s.values, &mut s.state)),
                    LastPlugin::Effect(i, s) => p.mixer[i].fx[s].as_mut().and_then(|e| e.plugin.as_mut()).map(|s| (&mut s.values, &mut s.state)),
                    LastPlugin::None => None,
                };
                if let Some((values, state)) = slot_values {
                    if word == "pvalue" {
                        values.push((num(0)? as u32, num(1)?));
                    } else {
                        *state = plugin::unhex(rest);
                    }
                }
            }
            "pattern" => {
                let mut pat = Pattern::new(p.patterns.len(), 0);
                if !rest.is_empty() {
                    pat.name = rest.to_string();
                }
                p.patterns.push(pat);
            }
            "note" => {
                if p.patterns.is_empty() {
                    return Err(bad());
                }
                let n = Note {
                    key: num(1)?.clamp(0.0, 127.0) as u8,
                    start: num(2)? as u32,
                    len: (num(3)? as u32).max(1),
                    vel: num(4)?.clamp(0.0, 128.0) as u8,
                };
                notes.push((p.patterns.len() - 1, num(0)? as usize, n));
            }
            "clip" => clips.push(Clip {
                track: (num(0)? as usize).min(PLAYLIST_TRACKS - 1),
                start_bar: num(1)? as u32,
                pattern: num(2)? as usize,
            }),
            "auto" => {
                let target = Target::parse(rest).ok_or_else(bad)?;
                p.automations.push(Automation { target, points: Vec::new() });
            }
            "point" => {
                let a = p.automations.last_mut().ok_or_else(bad)?;
                let (lo, hi) = a.target.range();
                a.points.push((num(0)? as u32, (num(1)? as f32).clamp(lo, hi)));
            }
            "" => {}
            _ => {} // unknown lines from newer versions are skipped
        }
    }
    if p.channels.is_empty() {
        return Err("the project has no instruments".into());
    }
    if p.patterns.is_empty() {
        p.patterns.push(Pattern::new(0, 0));
    }
    let n = p.channels.len();
    for pat in &mut p.patterns {
        pat.notes = vec![Vec::new(); n];
    }
    for (pi, ch, note) in notes {
        if ch < n {
            p.patterns[pi].notes[ch].push(note);
        }
    }
    let pats = p.patterns.len();
    p.clips = clips.into_iter().filter(|c| c.pattern < pats).collect();
    p.current_pattern = p.current_pattern.min(pats - 1);
    p.automations.retain(|a| match a.target {
        Target::ChannelVolume(c) | Target::ChannelPan(c) => c < n,
        _ => true,
    });
    for a in &mut p.automations {
        a.sort();
    }
    Ok(p)
}

/// Starts plugins that are waiting to load (after opening a project or undo)
/// and applies parameter values restored by undo. Returns error messages.
pub fn start_plugins(p: &mut Project) -> Vec<String> {
    fn start<T>(slot: &mut PluginSlot<T>, rate: f32, wrap: fn(PluginCore) -> T, core_of: fn(&mut T) -> &mut PluginCore) -> Option<String> {
        if let Some(live) = &mut slot.live {
            let core = core_of(live);
            let values = std::mem::take(&mut slot.values);
            if std::mem::take(&mut slot.restored) {
                let targets: Vec<(u32, f64)> =
                    core.params().iter().map(|p| (p.id, values.iter().find(|v| v.0 == p.id).map_or(p.default, |v| v.1))).collect();
                for (id, v) in targets {
                    if core.value(id) != Some(v) {
                        core.set(id, v);
                    }
                }
            } else {
                for (id, v) in values {
                    core.set(id, v);
                }
            }
            return None;
        }
        if slot.error.is_some() {
            return None;
        }
        match PluginCore::load(&slot.info, rate, slot.state.as_deref()) {
            Ok(mut core) => {
                for (id, v) in std::mem::take(&mut slot.values) {
                    core.set(id, v);
                }
                slot.state = None;
                slot.live = Some(wrap(core));
                None
            }
            Err(e) => {
                let msg = format!("{}: {e}", slot.info.name);
                slot.error = Some(e);
                Some(msg)
            }
        }
    }
    let rate = p.sample_rate;
    let (inst, fx) = p.plugin_slots();
    let mut errors = Vec::new();
    for s in inst {
        errors.extend(start(s, rate, PluginInstrument::new, |l| &mut l.core));
    }
    for s in fx {
        errors.extend(start(s, rate, PluginEffect::new, |l| &mut l.core));
    }
    errors
}

/// Writes through a temporary file so a failed save never destroys the old one.
pub fn write_safely(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })();
    match result {
        Ok(()) => fs::rename(&tmp, path),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

pub fn save(p: &Project, path: &Path) -> io::Result<()> {
    write_safely(path, write_text(p, true).as_bytes())
}

pub fn open(path: &Path) -> Result<Project, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    from_text(&text)
}

/// A full copy of the project (decoded samples shared, plugins started
/// fresh from their saved state) at another sample rate.
fn copy_for_render(p: &Project, rate: f32) -> Project {
    let mut q = from_text(&write_text(p, true)).unwrap_or_else(|_| Project::new());
    for (c, orig) in q.channels.iter_mut().zip(&p.channels) {
        if let (Some(s), Some(o)) = (&mut c.sample, &orig.sample) {
            s.data = o.data.clone();
        }
    }
    q.sample_rate = rate;
    start_plugins(&mut q);
    q
}

/// Renders the song (or the current pattern when the playlist is empty) once,
/// plus a short tail so the last sounds can ring out. Returns the samples
/// (interleaved stereo) at `EXPORT_RATE`.
pub fn render_song(p: &Project) -> Vec<f32> {
    let mut q = copy_for_render(p, EXPORT_RATE as f32);
    q.mode = if q.clips.is_empty() { Mode::Pattern } else { Mode::Song };
    q.current_pattern = p.current_pattern;
    q.playing = true;
    q.position = 0.0;
    let ticks_per_sample = (q.tempo * PPQ as f64 / 4.0) / (EXPORT_RATE as f64 * 15.0);
    // Stop one frame early so the loop never wraps back to the start.
    let frames = ((q.loop_length() as f64 / ticks_per_sample).floor() as usize).saturating_sub(1);
    let tail = (EXPORT_TAIL_SECS * EXPORT_RATE as f32) as usize;
    let mut out = vec![0.0f32; (frames + tail) * 2];
    // Blocks like a sound card's, so automation and plugins behave the same.
    let block = 1024 * 2;
    let mut at = 0;
    while at < frames * 2 {
        let end = (at + block).min(frames * 2);
        q.render(&mut out[at..end], 2);
        at = end;
    }
    q.playing = false;
    while at < out.len() {
        let end = (at + block).min(out.len());
        q.render(&mut out[at..end], 2);
        at = end;
    }
    out
}

/// 16-bit stereo PCM WAV.
pub fn wav_bytes(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&2u16.to_le_bytes()); // stereo
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for x in samples {
        b.extend_from_slice(&((x.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    b
}

/// Renders and writes a WAV file. Returns its length in seconds.
pub fn export_wav(p: &Project, path: &Path) -> io::Result<f32> {
    let samples = render_song(p);
    write_safely(path, &wav_bytes(&samples, EXPORT_RATE))?;
    Ok(samples.len() as f32 / 2.0 / EXPORT_RATE as f32)
}

/// Renders and writes an MP3 file. Returns its length in seconds.
pub fn export_mp3(p: &Project, path: &Path) -> Result<f32, String> {
    let samples = render_song(p);
    let bytes = crate::audiofile::mp3_bytes(&samples, EXPORT_RATE)?;
    write_safely(path, &bytes).map_err(|e| e.to_string())?;
    Ok(samples.len() as f32 / 2.0 / EXPORT_RATE as f32)
}

/// Writes stereo frames as a WAV file (recordings).
pub fn write_frames(path: &Path, frames: &[[f32; 2]], rate: u32) -> io::Result<()> {
    let flat: Vec<f32> = frames.iter().flatten().copied().collect();
    write_safely(path, &wav_bytes(&flat, rate))
}

/// A sampler channel playing `path`, named after the file.
pub fn sampler_channel(p: &Project, path: &Path, data: Option<Arc<SampleData>>) -> Channel {
    let mut c = Channel::new(p.channel_name(&display_name(path)), Instrument::Sampler, 0);
    c.sample = Some(SampleRef { path: path.to_path_buf(), data });
    c
}

// ---------------------------------------------------------------- demos

pub const DEMOS: [&str; 2] = ["House groove", "Lo-fi swing"];

/// Small projects that show the studio off, built in code.
pub fn demo(name: &str) -> Project {
    let mut p = Project::new();
    p.title = name.to_string();
    let step = |p: &mut Project, ch: usize, steps: &[usize]| {
        for &s in steps {
            p.patterns[0].set_step(ch, s, true);
        }
    };
    match name {
        "Lo-fi swing" => {
            p.tempo = 86.0;
            p.swing = 0.6;
            step(&mut p, 0, &[0, 7, 10]);
            step(&mut p, 3, &[4, 12]);
            step(&mut p, 2, &[0, 2, 4, 6, 8, 10, 12, 14]);
            for (i, k) in [57u8, 60, 64, 62].into_iter().enumerate() {
                p.patterns[0].notes[4].push(Note { key: k, start: i as u32 * PPQ, len: PPQ, vel: 80 });
            }
            let mut fx = Effect::new(EffectKind::Filter);
            fx.params = [0.55, 0.2, 0.0, 1.0];
            p.mixer[5].fx[0] = Some(fx);
            p.mixer[5].fx[1] = Some(Effect::new(EffectKind::Reverb));
            p.mixer[0].fx[0] = Some(Effect::new(EffectKind::Compressor));
        }
        _ => {
            p.tempo = 124.0;
            step(&mut p, 0, &[0, 4, 8, 12]);
            step(&mut p, 1, &[4, 12]);
            step(&mut p, 2, &[2, 6, 10, 14]);
            for (s, k) in [(0u32, 45u8), (3, 45), (6, 57), (8, 45), (11, 48), (14, 50)] {
                p.patterns[0].notes[4].push(Note { key: k, start: s * TICKS_PER_STEP, len: TICKS_PER_STEP * 2, vel: 100 });
            }
            p.mixer[2].fx[0] = Some(Effect::new(EffectKind::Reverb));
            let mut delay = Effect::new(EffectKind::Delay);
            delay.params[3] = 0.25;
            p.mixer[5].fx[0] = Some(delay);
            p.ensure_pattern(1);
            for s in [0, 4, 8, 12] {
                p.patterns[1].set_step(0, s, true);
            }
            for bar in 0..4 {
                p.clips.push(Clip { track: 0, start_bar: bar, pattern: 0 });
            }
            p.clips.push(Clip { track: 1, start_bar: 4, pattern: 1 });
            // The synth opens up over the first four bars.
            let mut f = Effect::new(EffectKind::Filter);
            f.params = [0.35, 0.3, 0.0, 1.0];
            p.mixer[5].fx[1] = Some(f);
            p.automations.push(Automation { target: Target::Effect(5, 1, 0), points: vec![(0, 0.35), (4 * TICKS_PER_BAR, 0.85)] });
            p.mode = Mode::Song;
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Instrument;

    #[test]
    fn project_round_trips() {
        let mut p = Project::new();
        p.title = "My song".into();
        p.tempo = 97.5;
        p.add_channel(Instrument::Synth);
        p.channels[5].name = "Lead with spaces".into();
        p.patterns[0].set_step(0, 4, true);
        p.ensure_pattern(1);
        p.patterns[1].notes[5].push(Note { key: 67, start: 48, len: 96, vel: 90 });
        p.clips.push(Clip { track: 2, start_bar: 3, pattern: 1 });
        p.mixer[7].volume = 9000.0;
        p.current_pattern = 1;
        let c = sampler_channel(&p, Path::new("/music/my kick.wav"), None);
        p.push_channel(c);
        p.mixer[3].solo = true;
        p.mixer[3].routes = vec![(9, 0.5), (0, 1.0)];
        p.mixer[3].fx[2] = Some(Effect::new(EffectKind::Delay));
        let mut fx = Effect::plugin(PluginSlot::new(PluginInfo {
            path: "/x/My Plug.clap".into(),
            id: "com.x.plug".into(),
            name: "My Plug".into(),
            vendor: "X".into(),
            instrument: false,
        }));
        fx.plugin.as_mut().unwrap().values = vec![(3, 0.25)];
        p.mixer[4].fx[0] = Some(fx);
        p.automations.push(Automation { target: Target::Effect(3, 2, 1), points: vec![(0, 0.2), (384, 0.9)] });

        let q = from_text(&to_text(&p)).unwrap();
        assert_eq!(q.title, "My song");
        assert_eq!(q.tempo, 97.5);
        assert_eq!(q.channels.len(), 7);
        assert_eq!(q.channels[5].name, "Lead with spaces");
        assert_eq!(q.channels[6].sample.as_ref().unwrap().path, Path::new("/music/my kick.wav"));
        assert!(q.patterns[0].has_step(0, 4));
        assert_eq!(q.patterns[1].notes[5], p.patterns[1].notes[5]);
        assert_eq!(q.clips.len(), 1);
        assert_eq!(q.mixer[7].volume, 9000.0);
        assert!(q.mixer[3].solo);
        assert_eq!(q.mixer[3].routes, vec![(9, 0.5), (0, 1.0)]);
        assert_eq!(q.mixer[3].fx[2].as_ref().unwrap().kind, EffectKind::Delay);
        let pl = q.mixer[4].fx[0].as_ref().unwrap().plugin.as_ref().unwrap();
        assert_eq!(pl.info.path, Path::new("/x/My Plug.clap"));
        assert_eq!(pl.values, vec![(3, 0.25)]);
        assert_eq!(q.automations, p.automations);
        assert_eq!(q.current_pattern, 1);
        assert_eq!(to_text(&q), to_text(&p));
    }

    #[test]
    fn rejects_other_files() {
        assert!(from_text("hello").is_err());
    }

    #[test]
    fn export_has_sound_and_header() {
        let mut p = Project::new();
        p.patterns[0].set_step(0, 0, true);
        let samples = render_song(&p);
        assert!(samples.iter().any(|x| x.abs() > 0.05));
        let wav = wav_bytes(&samples, EXPORT_RATE);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(wav.len(), 44 + samples.len() * 2);
    }

    #[test]
    fn demos_play() {
        for name in DEMOS {
            let p = demo(name);
            assert!(render_song(&p).iter().any(|x| x.abs() > 0.05), "{name}");
            assert!(from_text(&to_text(&p)).is_ok());
        }
    }
}

/// A real plugin through a whole project: load, play, save with its state,
/// open again. Needs installed plugins: `cargo test real_plugin_project -- --ignored`.
#[cfg(test)]
mod real {
    use super::*;

    #[test]
    #[ignore]
    fn real_plugin_project() {
        let home = std::env::var("HOME").unwrap();
        let fx = PluginInfo {
            path: PathBuf::from(format!("{home}/.clap/Lemon Reverb.clap")),
            id: "org.openstudio.lemon-reverb".into(),
            name: "Lemon Reverb".into(),
            vendor: "Lemon Studio".into(),
            instrument: false,
        };
        let synth = PluginInfo {
            path: PathBuf::from(format!("{home}/.clap/Citrus.clap")),
            id: "org.openstudio.citrus".into(),
            name: "Citrus".into(),
            vendor: "Lemon Studio".into(),
            instrument: true,
        };
        let mut p = Project::new();
        p.mixer[1].fx[0] = Some(Effect::plugin(PluginSlot::new(fx)));
        let mut c = Channel::new("Citrus".into(), Instrument::Plugin, 0);
        c.plugin = Some(Box::new(PluginSlot::new(synth)));
        p.push_channel(c);
        let ch = p.channels.len() - 1;
        p.patterns[0].notes[ch].push(Note { key: 60, start: 0, len: 96, vel: 100 });
        p.patterns[0].set_step(0, 0, true);
        assert!(start_plugins(&mut p).is_empty());

        // Change a parameter, then check it survives save and open.
        let (id, v) = {
            let core = &mut p.mixer[1].fx[0].as_mut().unwrap().plugin.as_mut().unwrap().live.as_mut().unwrap().core;
            let q = core.params()[0].clone();
            // Stepped parameters round to whole numbers.
            let v = ((q.min + q.max) / 2.0).round();
            core.set(q.id, v);
            (q.id, v)
        };
        p.playing = true;
        let mut buf = vec![0.0f32; 48000 * 2];
        for chunk in buf.chunks_mut(1024) {
            p.render(chunk, 2);
        }
        assert!(buf.iter().any(|x| x.abs() > 0.05), "plugin synth and kick are heard");

        let dir = std::env::temp_dir().join(format!("beatstudio-real-{}", std::process::id()));
        let path = dir.join("p.bsp");
        save(&p, &path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("pstate ") && text.contains("fxplugin 1 0 1\t"));
        let mut q = open(&path).unwrap();
        assert!(start_plugins(&mut q).is_empty());
        let core = &q.mixer[1].fx[0].as_ref().unwrap().plugin.as_ref().unwrap().live.as_ref().unwrap().core;
        assert!((core.value(id).unwrap() - v).abs() < 1e-3, "parameter restored");
        let samples = render_song(&q);
        assert!(samples.iter().any(|x| x.abs() > 0.05), "export with plugins has sound");
        let _ = fs::remove_dir_all(&dir);
    }
}
