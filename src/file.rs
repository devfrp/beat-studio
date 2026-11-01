// Project files (.bsp, plain text, one item per line) and WAV export.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::engine::{
    Channel, Clip, Instrument, Mode, Note, Pattern, Project, FADER_DEFAULT, MIXER_INSERTS, PLAYLIST_TRACKS, PPQ,
};

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

/// Project files in the projects folder, most recently changed first.
pub fn list_projects() -> Vec<PathBuf> {
    let Ok(dir) = fs::read_dir(projects_dir()) else { return Vec::new() };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == EXTENSION))
        .filter_map(|p| Some((fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.into_iter().map(|(_, p)| p).collect()
}

/// Turns a typed name into a path in the projects folder, adding the extension.
pub fn path_for(name: &str, ext: &str) -> PathBuf {
    let clean: String = name.trim().chars().map(|c| if c == '/' || c == '\\' { '_' } else { c }).collect();
    let clean = if clean.is_empty() { "Untitled".to_string() } else { clean };
    let mut path = projects_dir().join(clean);
    if path.extension().is_none_or(|e| e != ext) {
        let mut s = path.into_os_string();
        s.push(".");
        s.push(ext);
        path = PathBuf::from(s);
    }
    path
}

pub fn display_name(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

fn bit(b: bool) -> u8 {
    b as u8
}

/// Writes the musical state (not the playback state) as text.
pub fn to_text(p: &Project) -> String {
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
    }
    for (i, t) in p.mixer.iter().enumerate() {
        if t.volume != FADER_DEFAULT || t.pan != 0.0 || !t.enabled {
            s.push_str(&format!("mixer {i} {} {} {}\n", t.volume, t.pan, bit(t.enabled)));
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
    s
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
    for (no, line) in lines.enumerate() {
        let bad = || format!("line {}: cannot read \"{line}\"", no + 2);
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
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
                p.channels.push(Channel {
                    name,
                    instrument: inst,
                    volume: volume.clamp(0.0, FADER_DEFAULT),
                    pan: pan.clamp(-6400.0, 6400.0),
                    enabled: enabled != 0.0,
                    mixer: (mixer as usize).min(MIXER_INSERTS),
                });
            }
            "mixer" => {
                let i = num(0)? as usize;
                if let Some(t) = p.mixer.get_mut(i) {
                    t.volume = (num(1)? as f32).clamp(0.0, crate::engine::FADER_MAX);
                    t.pan = (num(2)? as f32).clamp(-6400.0, 6400.0);
                    t.enabled = num(3)? != 0.0;
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
    Ok(p)
}

/// Writes through a temporary file so a failed save never destroys the old one.
fn write_safely(path: &Path, bytes: &[u8]) -> io::Result<()> {
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
    write_safely(path, to_text(p).as_bytes())
}

pub fn open(path: &Path) -> Result<Project, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    from_text(&text)
}

/// Renders the song (or the current pattern when the playlist is empty) once,
/// plus a short tail so the last sounds can ring out. Returns the samples
/// (interleaved stereo) at `EXPORT_RATE`.
pub fn render_song(p: &Project) -> Vec<f32> {
    let mut q = from_text(&to_text(p)).unwrap_or_else(|_| Project::new());
    q.mode = if q.clips.is_empty() { Mode::Pattern } else { Mode::Song };
    q.current_pattern = p.current_pattern;
    q.sample_rate = EXPORT_RATE as f32;
    q.playing = true;
    q.position = 0.0;
    let ticks_per_sample = (q.tempo * PPQ as f64 / 4.0) / (EXPORT_RATE as f64 * 15.0);
    // Stop one frame early so the loop never wraps back to the start.
    let frames = ((q.loop_length() as f64 / ticks_per_sample).floor() as usize).saturating_sub(1);
    let tail = (EXPORT_TAIL_SECS * EXPORT_RATE as f32) as usize;
    let mut out = vec![0.0f32; (frames + tail) * 2];
    q.render(&mut out[..frames * 2], 2);
    q.playing = false;
    q.render(&mut out[frames * 2..], 2);
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

        let q = from_text(&to_text(&p)).unwrap();
        assert_eq!(q.title, "My song");
        assert_eq!(q.tempo, 97.5);
        assert_eq!(q.channels.len(), 6);
        assert_eq!(q.channels[5].name, "Lead with spaces");
        assert!(q.patterns[0].has_step(0, 4));
        assert_eq!(q.patterns[1].notes[5], p.patterns[1].notes[5]);
        assert_eq!(q.clips.len(), 1);
        assert_eq!(q.mixer[7].volume, 9000.0);
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
}
