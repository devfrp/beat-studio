// Piano roll tools: quantize, legato, chop and reverse a list of notes.
// First written by the local model (Cline), reviewed and fixed.

use crate::engine::Note;

/// Starts and lengths to the nearest multiple of `grid` (halfway rounds up), lengths at least `grid`.
pub fn quantize(notes: &mut [Note], grid: u32) {
    if grid == 0 {
        return;
    }
    let half = grid / 2;
    for note in notes.iter_mut() {
        let r = note.start % grid;
        let new_start = if r < half {
            (note.start / grid) * grid
        } else {
            ((note.start / grid) + 1) * grid
        };
        let new_len = if note.len < grid {
            grid
        } else {
            let rlen = note.len % grid;
            if rlen < half {
                note.len - rlen
            } else {
                note.len + (grid - rlen)
            }
        };
        note.start = new_start;
        note.len = new_len;
    }
}

/// Each note lasts until the next later start (of any key); the last ones keep their length.
pub fn legato(notes: &mut [Note]) {
    let starts: Vec<u32> = notes.iter().map(|n| n.start).collect();
    for note in notes.iter_mut() {
        if let Some(next) = starts.iter().copied().filter(|&s| s > note.start).min() {
            note.len = next - note.start;
        }
    }
}

/// Pieces of `piece` ticks (the last one takes what is left), note after note.
pub fn chop(notes: &[Note], piece: u32) -> Vec<Note> {
    let mut result = Vec::new();
    for note in notes {
        let mut remaining = note.len;
        let mut cur_start = note.start;
        while remaining > 0 {
            let len_chunk = remaining.min(piece);
            result.push(Note {
                key: note.key,
                start: cur_start,
                len: len_chunk,
                vel: note.vel,
            });
            cur_start += len_chunk;
            remaining -= len_chunk;
        }
    }
    result
}

/// Mirrors the notes in time inside the span they cover.
pub fn reverse(notes: &mut [Note]) {
    if notes.is_empty() {
        return;
    }
    let first = notes.iter().map(|n| n.start).min().unwrap();
    let last = notes.iter().map(|n| n.start + n.len).max().unwrap();
    for note in notes.iter_mut() {
        note.start = first + (last - (note.start + note.len));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(key: u8, start: u32, len: u32) -> Note {
        Note { key, start, len, vel: 100 }
    }

    #[test]
    fn test_quantize() {
        let mut notes = [n(60, 10, 30), n(60, 12, 40), n(60, 5, 3)];
        quantize(&mut notes, 24);
        assert_eq!(notes[0], n(60, 0, 24));
        assert_eq!(notes[1], n(60, 24, 48));
        assert_eq!(notes[2], n(60, 0, 24));
    }

    #[test]
    fn test_legato() {
        let mut notes = [n(60, 0, 10), n(64, 0, 10), n(67, 48, 10), n(60, 96, 30)];
        legato(&mut notes);
        assert_eq!(notes[0], n(60, 0, 48));
        assert_eq!(notes[1], n(64, 0, 48));
        assert_eq!(notes[2], n(67, 48, 48));
        assert_eq!(notes[3], n(60, 96, 30));
    }

    #[test]
    fn test_chop() {
        let notes = [n(60, 0, 60)];
        let result = chop(&notes, 24);
        assert_eq!(result, vec![n(60, 0, 24), n(60, 24, 24), n(60, 48, 12)]);
    }

    #[test]
    fn test_reverse() {
        let mut notes = [n(60, 0, 24), n(62, 48, 48)];
        reverse(&mut notes);
        assert_eq!(notes[0], n(60, 72, 24));
        assert_eq!(notes[1], n(62, 0, 48));
    }

    #[test]
    fn test_empty() {
        let mut empty: [Note; 0] = [];
        quantize(&mut empty, 24);
        legato(&mut empty);
        reverse(&mut empty);
        let chop_result = chop(&empty, 24);
        assert!(chop_result.is_empty());
    }
}

/// Moves every key by `semitones`; changes nothing and returns false if a note would leave 0..=127.
pub fn transpose(notes: &mut [Note], semitones: i32) -> bool {
    if notes.is_empty() {
        return true;
    }
    for note in notes.iter() {
        let new_key = (note.key as i32) + semitones;
        if !(0..=127).contains(&new_key) {
            return false;
        }
    }
    for note in notes.iter_mut() {
        note.key = ((note.key as i32) + semitones) as u8;
    }
    true
}

/// Mirrors the keys between the lowest and the highest.
pub fn flip_keys(notes: &mut [Note]) {
    if notes.is_empty() {
        return;
    }
    let lo = notes.iter().map(|n| n.key).min().unwrap();
    let hi = notes.iter().map(|n| n.key).max().unwrap();
    for note in notes.iter_mut() {
        note.key = lo + hi - note.key;
    }
}

/// Spreads each chord (notes with the same start), lowest key first, `step` ticks apart;
/// lengths shrink by the same amount but not below `step`.
pub fn arpeggiate(notes: &mut [Note], step: u32) {
    if notes.is_empty() {
        return;
    }
    let mut groups: Vec<(u32, Vec<usize>)> = Vec::new();
    for (i, note) in notes.iter().enumerate() {
        let start_val = note.start;
        if let Some(g) = groups.iter_mut().find(|(s, _)| *s == start_val) {
            g.1.push(i);
        } else {
            groups.push((start_val, vec![i]));
        }
    }
    for (start_val, indices) in groups {
        if indices.len() <= 1 {
            continue;
        }
        let mut key_idx: Vec<(u8, usize)> = indices.iter()
            .map(|&i| (notes[i].key, i))
            .collect();
        key_idx.sort_by_key(|&(k, _)| k);
        for (rank, (_, idx)) in key_idx.into_iter().enumerate() {
            let orig_len = notes[idx].len;
            let new_start = start_val + step * rank as u32;
            let new_len = if orig_len >= step {
                orig_len.saturating_sub(step * rank as u32).max(step)
            } else {
                orig_len
            };
            notes[idx].start = new_start;
            notes[idx].len = new_len;
        }
    }
}

/// Moves each velocity by a pseudo-random amount in [-amount, amount] (xorshift32), kept in 1..=127.
pub fn humanize(notes: &mut [Note], amount: u8, seed: u32) {
    if amount == 0 || notes.is_empty() {
        return;
    }
    let mut state = seed | 1;
    for note in notes.iter_mut() {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let mod_val = 2 * amount as u32 + 1;
        let r = state % mod_val;
        let offset = (r as i32) - amount as i32;
        let new_vel = note.vel as i32 + offset;
        note.vel = new_vel.clamp(1, 127) as u8;
    }
}

#[cfg(test)]
mod tests2 {
    use super::*;

    fn n(key: u8, start: u32, len: u32) -> Note {
        Note { key, start, len, vel: 100 }
    }

    #[test]
    fn test_transpose() {
        let mut notes = vec![n(60, 0, 24), n(64, 0, 24)];
        assert!(transpose(&mut notes, 12));
        assert_eq!(notes[0].key, 72);
        assert_eq!(notes[1].key, 76);
        let mut notes2 = vec![n(60, 0, 24)];
        assert!(!transpose(&mut notes2, 70));
        assert_eq!(notes2[0].key, 60);
    }

    #[test]
    fn test_flip_keys() {
        let mut notes = vec![n(60, 0, 24), n(64, 24, 24), n(67, 48, 24)];
        flip_keys(&mut notes);
        assert_eq!(notes[0].key, 67);
        assert_eq!(notes[1].key, 63);
        assert_eq!(notes[2].key, 60);
    }

    #[test]
    fn test_arpeggiate1() {
        let mut notes = vec![n(67, 0, 96), n(60, 0, 96), n(64, 0, 96), n(72, 96, 48)];
        arpeggiate(&mut notes, 24);
        assert_eq!(notes[0].key, 67);
        assert_eq!(notes[0].start, 48);
        assert_eq!(notes[0].len, 48);
        assert_eq!(notes[1].key, 60);
        assert_eq!(notes[1].start, 0);
        assert_eq!(notes[1].len, 96);
        assert_eq!(notes[2].key, 64);
        assert_eq!(notes[2].start, 24);
        assert_eq!(notes[2].len, 72);
        assert_eq!(notes[3].key, 72);
        assert_eq!(notes[3].start, 96);
        assert_eq!(notes[3].len, 48);
    }

    #[test]
    fn test_arpeggiate2() {
        let mut notes = vec![n(60, 0, 10), n(64, 0, 10)];
        arpeggiate(&mut notes, 24);
        assert_eq!(notes[0].key, 60);
        assert_eq!(notes[0].start, 0);
        assert_eq!(notes[0].len, 10);
        assert_eq!(notes[1].key, 64);
        assert_eq!(notes[1].start, 24);
        assert_eq!(notes[1].len, 10);
    }

    #[test]
    fn test_humanize() {
        let mut notes = Vec::new();
        for _ in 0..50 {
            notes.push(n(60, 0, 24));
        }
        humanize(&mut notes, 10, 7);
        let vels: Vec<u8> = notes.iter().map(|n| n.vel).collect();
        for &v in &vels {
            assert!((90..=110).contains(&v));
        }
        let first = vels[0];
        let all_equal = vels.iter().all(|&x| x == first);
        assert!(!all_equal);
        let mut notes2 = vec![n(60, 0, 24)];
        humanize(&mut notes2, 0, 7);
        assert_eq!(notes2[0].vel, 100);
        let mut notes3 = vec![n(60, 0, 24)];
        humanize(&mut notes3, 127, 7);
        assert!((1..=127).contains(&notes3[0].vel));
    }

    #[test]
    fn arpeggiate_big_chord_of_short_notes() {
        let mut notes = vec![n(60, 0, 48), n(62, 0, 48), n(64, 0, 48), n(65, 0, 48)];
        arpeggiate(&mut notes, 24);
        assert_eq!(notes.iter().map(|n| (n.start, n.len)).collect::<Vec<_>>(), vec![(0, 48), (24, 24), (48, 24), (72, 24)]);
    }

    #[test]
    fn test_empty() {
        let mut notes: Vec<Note> = vec![];
        transpose(&mut notes, 5);
        flip_keys(&mut notes);
        arpeggiate(&mut notes, 10);
        humanize(&mut notes, 10, 7);
    }
}
