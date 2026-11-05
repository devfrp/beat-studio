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
