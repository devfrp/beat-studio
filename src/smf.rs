// Standard MIDI files: export of the song or the current pattern, and import
// into a new pattern. Drum channels use the General MIDI drum notes on MIDI
// channel 10.

use crate::engine::{Channel, Instrument, Mode, Note, Project, PPQ, STEP_KEY, TICKS_PER_BAR};

/// General MIDI drum note for each built-in drum (at the step key, C5).
fn gm_drum(inst: Instrument) -> Option<u8> {
    match inst {
        Instrument::Kick => Some(36),
        Instrument::Snare => Some(38),
        Instrument::Clap => Some(39),
        Instrument::HiHat => Some(42),
        _ => None,
    }
}

/// Which built-in drum plays a General MIDI drum note.
fn drum_for(note: u8) -> Instrument {
    match note {
        35 | 36 => Instrument::Kick,
        37 | 38 | 40 => Instrument::Snare,
        39 => Instrument::Clap,
        _ => Instrument::HiHat,
    }
}

fn vlq(mut v: u32, out: &mut Vec<u8>) {
    let mut bytes = vec![(v & 0x7F) as u8];
    v >>= 7;
    while v > 0 {
        bytes.push((v & 0x7F) as u8 | 0x80);
        v >>= 7;
    }
    out.extend(bytes.iter().rev());
}

fn track_chunk(events: &mut [(u32, Vec<u8>)]) -> Vec<u8> {
    // Note-offs before note-ons at the same time.
    events.sort_by_key(|(t, e)| (*t, e[0] & 0xF0 != 0x80));
    let mut data = Vec::new();
    let mut now = 0;
    for (t, e) in events.iter() {
        vlq(t - now, &mut data);
        data.extend_from_slice(e);
        now = *t;
    }
    data.extend_from_slice(&[0, 0xFF, 0x2F, 0]);
    let mut chunk = b"MTrk".to_vec();
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend(data);
    chunk
}

/// Every note of a channel as played: the song in song mode, else the pattern.
fn played_notes(p: &Project, ch: usize) -> Vec<Note> {
    let mut out = Vec::new();
    let mut add = |pat: usize, base: u32| {
        for n in p.patterns[pat].notes[ch].iter().filter(|n| n.vel > 0) {
            out.push(Note { start: n.start + base, ..*n });
        }
    };
    if p.mode == Mode::Song && !p.clips.is_empty() {
        for c in &p.clips {
            add(c.pattern, c.start_bar * TICKS_PER_BAR);
        }
    } else {
        add(p.current_pattern, 0);
    }
    out
}

/// A type 1 MIDI file: a tempo track, then one track per channel with notes.
pub fn export(p: &Project) -> Vec<u8> {
    let mut tracks = Vec::new();
    let tempo = (60_000_000.0 / p.tempo).round() as u32;
    let mut meta = vec![(0, vec![0xFF, 0x51, 3, (tempo >> 16) as u8, (tempo >> 8) as u8, tempo as u8]), (0, vec![0xFF, 0x58, 4, 4, 2, 24, 8])];
    tracks.push(track_chunk(&mut meta));
    let mut melodic = 0u8;
    for (ch, c) in p.channels.iter().enumerate() {
        let notes = played_notes(p, ch);
        if notes.is_empty() {
            continue;
        }
        let drum = gm_drum(c.instrument);
        let midi_ch = match drum {
            Some(_) => 9,
            None => {
                let m = if melodic >= 9 { melodic + 1 } else { melodic };
                melodic = (melodic + 1) % 15;
                m
            }
        };
        let mut events = Vec::new();
        let mut name = vec![0xFF, 0x03];
        let bytes = c.name.as_bytes();
        name.push(bytes.len().min(127) as u8);
        name.extend_from_slice(&bytes[..bytes.len().min(127)]);
        events.push((0, name));
        for n in notes {
            let key = match drum {
                Some(d) => (d as i32 + n.key as i32 - STEP_KEY as i32).clamp(0, 127) as u8,
                None => n.key.min(127),
            };
            events.push((n.start, vec![0x90 | midi_ch, key, n.vel.clamp(1, 127)]));
            events.push((n.start + n.len.max(1), vec![0x80 | midi_ch, key, 64]));
        }
        tracks.push(track_chunk(&mut events));
    }
    let mut out = b"MThd".to_vec();
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
    out.extend_from_slice(&(PPQ as u16).to_be_bytes());
    for t in tracks {
        out.extend(t);
    }
    out
}

/// What a MIDI file holds, in Beat Studio ticks.
#[derive(Debug, Default)]
pub struct Imported {
    pub tempo: Option<f64>,
    /// (name, drums, notes); drum notes keep their General MIDI number in `key`.
    pub parts: Vec<(String, bool, Vec<Note>)>,
}

fn read_vlq(d: &[u8], at: &mut usize) -> Option<u32> {
    let mut v = 0u32;
    for _ in 0..4 {
        let b = *d.get(*at)?;
        *at += 1;
        v = (v << 7) | (b & 0x7F) as u32;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

pub fn parse(data: &[u8]) -> Result<Imported, String> {
    let bad = || "not a MIDI file".to_string();
    if data.len() < 14 || &data[0..4] != b"MThd" {
        return Err(bad());
    }
    let division = u16::from_be_bytes([data[12], data[13]]);
    if division & 0x8000 != 0 || division == 0 {
        return Err("MIDI files timed in SMPTE frames are not supported".into());
    }
    let scale = PPQ as f64 / division as f64;
    let mut at = 8 + u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let mut out = Imported::default();
    while at + 8 <= data.len() {
        let len = u32::from_be_bytes([data[at + 4], data[at + 5], data[at + 6], data[at + 7]]) as usize;
        let body = data.get(at + 8..at + 8 + len).ok_or_else(bad)?;
        let is_track = &data[at..at + 4] == b"MTrk";
        at += 8 + len;
        if !is_track {
            continue;
        }
        let mut name = String::new();
        // One part per MIDI channel used in the track.
        let mut parts: Vec<(u8, Vec<Note>)> = Vec::new();
        let mut open: Vec<(u8, u8, u32, u8)> = Vec::new();
        let mut i = 0;
        let mut tick = 0u32;
        let mut status = 0u8;
        while i < body.len() {
            tick += read_vlq(body, &mut i).ok_or_else(bad)?;
            let mut b = *body.get(i).ok_or_else(bad)?;
            if b & 0x80 != 0 {
                status = b;
                i += 1;
            } else {
                b = status;
            }
            match b {
                0xFF => {
                    let kind = *body.get(i).ok_or_else(bad)?;
                    i += 1;
                    let l = read_vlq(body, &mut i).ok_or_else(bad)? as usize;
                    let m = body.get(i..i + l).ok_or_else(bad)?;
                    i += l;
                    match kind {
                        0x03 => name = String::from_utf8_lossy(m).into_owned(),
                        0x51 if l == 3 && out.tempo.is_none() => {
                            let us = ((m[0] as u32) << 16) | ((m[1] as u32) << 8) | m[2] as u32;
                            out.tempo = Some(60_000_000.0 / us.max(1) as f64);
                        }
                        0x2F => break,
                        _ => {}
                    }
                }
                0xF0 | 0xF7 => {
                    let l = read_vlq(body, &mut i).ok_or_else(bad)? as usize;
                    i += l;
                }
                _ => {
                    let kind = b & 0xF0;
                    let ch = b & 0x0F;
                    let size = if kind == 0xC0 || kind == 0xD0 { 1 } else { 2 };
                    let d = body.get(i..i + size).ok_or_else(bad)?;
                    i += size;
                    let t = (tick as f64 * scale).round() as u32;
                    let off = kind == 0x80 || (kind == 0x90 && d[1] == 0);
                    if kind == 0x90 && !off {
                        open.push((ch, d[0], t, d[1]));
                    } else if off {
                        if let Some(k) = open.iter().position(|o| o.0 == ch && o.1 == d[0]) {
                            let (ch, key, start, vel) = open.remove(k);
                            let note = Note { key, start, len: t.saturating_sub(start).max(1), vel };
                            match parts.iter_mut().find(|p| p.0 == ch) {
                                Some(p) => p.1.push(note),
                                None => parts.push((ch, vec![note])),
                            }
                        }
                    }
                }
            }
        }
        for (ch, notes) in parts {
            let label = if name.trim().is_empty() { format!("MIDI {}", ch + 1) } else { name.trim().to_string() };
            out.parts.push((label, ch == 9, notes));
        }
    }
    if out.parts.is_empty() {
        return Err("the MIDI file has no notes".into());
    }
    Ok(out)
}

/// Puts an imported file into a new pattern: one synth channel per melodic
/// part, one built-in drum channel per drum sound. Returns the pattern index.
pub fn import_into(p: &mut Project, imp: &Imported, name: &str) -> usize {
    if let Some(t) = imp.tempo {
        p.tempo = crate::engine::quantize_tempo(t);
    }
    let pat = p.patterns.len();
    p.ensure_pattern(pat);
    p.patterns[pat].name = name.to_string();
    for (label, drums, notes) in &imp.parts {
        if *drums {
            for n in notes {
                let inst = drum_for(n.key);
                let ch = match p.channels.iter().position(|c| c.instrument == inst && c.name == format!("{label} {}", inst.label())) {
                    Some(c) => c,
                    None => {
                        p.push_channel(Channel::new(format!("{label} {}", inst.label()), inst, 0));
                        p.channels.len() - 1
                    }
                };
                p.patterns[pat].notes[ch].push(Note { key: STEP_KEY, ..*n });
            }
        } else {
            let cname = p.channel_name(label);
            p.push_channel(Channel::new(cname, Instrument::Synth, 0));
            let ch = p.channels.len() - 1;
            p.patterns[pat].notes[ch].extend(notes.iter().copied());
        }
    }
    p.current_pattern = pat;
    pat
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_then_import_keeps_the_notes() {
        let mut p = Project::new();
        p.tempo = 100.0;
        p.patterns[0].set_step(0, 0, true);
        p.patterns[0].set_step(3, 4, true);
        p.patterns[0].notes[4].push(Note { key: 64, start: 96, len: 48, vel: 90 });
        let bytes = export(&p);
        assert_eq!(&bytes[0..4], b"MThd");
        let imp = parse(&bytes).unwrap();
        assert!((imp.tempo.unwrap() - 100.0).abs() < 0.01);
        let drums: Vec<u8> = imp.parts.iter().filter(|x| x.1).flat_map(|x| x.2.iter().map(|n| n.key)).collect();
        assert!(drums.contains(&36) && drums.contains(&38));
        let synth = imp.parts.iter().find(|x| !x.1).unwrap();
        assert_eq!(synth.2, vec![Note { key: 64, start: 96, len: 48, vel: 90 }]);

        let mut q = Project::new();
        let before = q.channels.len();
        let pat = import_into(&mut q, &imp, "Imported");
        assert_eq!(q.current_pattern, pat);
        assert_eq!(q.channels.len(), before + 3);
        let kick = q.channels.iter().position(|c| c.instrument == Instrument::Kick && c.name.ends_with("Kick") && c.name != "808 Kick").unwrap();
        assert!(q.patterns[pat].has_step(kick, 0));
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse(b"hello world, not midi").is_err());
    }
}
