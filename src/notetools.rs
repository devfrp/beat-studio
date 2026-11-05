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

// ---------------------------------------------------------------- channel rack steps

/// Replaces the step notes with one every `every` steps (other notes and markers stay).
pub fn fill_every(notes: &mut Vec<Note>, key: u8, step_len: u32, every: u32, vel: u8, total_ticks: u32) {
    let step_size = every * step_len;
    let mut filtered: Vec<Note> = notes.iter()
        .filter(|note| !(note.key == key && note.len == step_len && note.vel > 0))
        .copied()
        .collect();
    let mut i: u32 = 0;
    while i * step_size < total_ticks {
        filtered.push(Note {
            key,
            start: i * step_size,
            len: step_len,
            vel,
        });
        i += 1;
    }
    *notes = filtered;
}

/// Moves the notes by `by` ticks, wrapping around the pattern; markers stay.
pub fn rotate(notes: &mut [Note], by: i64, total_ticks: u32) {
    let total = total_ticks as i64;
    for note in notes.iter_mut() {
        if note.vel > 0 {
            let new_start = ((note.start as i64 + by).rem_euclid(total)) as u32;
            note.start = new_start;
        }
    }
}

/// Replaces the step notes with random ones, about `percent` in 100 (xorshift32).
pub fn random_steps(notes: &mut Vec<Note>, key: u8, step_len: u32, vel: u8, total_ticks: u32, percent: u32, seed: u32) {
    let step_size = step_len;
    let mut filtered: Vec<Note> = notes.iter()
        .filter(|note| !(note.key == key && note.len == step_len && note.vel > 0))
        .copied()
        .collect();
    let mut state: u32 = seed | 1;
    let mut i: u32 = 0;
    while i * step_size < total_ticks {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        if state % 100 < percent {
            filtered.push(Note {
                key,
                start: i * step_size,
                len: step_len,
                vel,
            });
        }
        i += 1;
    }
    *notes = filtered;
}

#[cfg(test)]
mod tests3 {
    use super::*;

    fn s(start: u32) -> Note {
        Note { key: 60, start, len: 24, vel: 100 }
    }

    #[test]
    fn test_fill_every() {
        let mut notes = vec![
            s(24),
            Note { key: 60, start: 0, len: 384, vel: 0 },
            Note { key: 64, start: 0, len: 48, vel: 90 },
        ];
        fill_every(&mut notes, 60, 24, 4, 100, 384);
        let expected = vec![
            Note { key: 60, start: 0, len: 384, vel: 0 },
            Note { key: 64, start: 0, len: 48, vel: 90 },
            s(0),
            s(96),
            s(192),
            s(288),
        ];
        assert_eq!(notes, expected);
    }

    #[test]
    fn test_rotate() {
        let mut notes = vec![s(0), s(360)];
        rotate(&mut notes, 24, 384);
        assert_eq!(notes, vec![s(24), s(0)]);
        let marker = Note { key: 60, start: 0, len: 384, vel: 0 };
        let mut notes3 = vec![marker];
        rotate(&mut notes3, 24, 384);
        assert_eq!(notes3, vec![marker]);

        let mut notes2 = vec![s(0)];
        rotate(&mut notes2, -24, 384);
        assert_eq!(notes2, vec![s(360)]);
    }

    #[test]
    fn test_random_steps() {
        let mut notes = vec![s(0)];
        random_steps(&mut notes, 60, 24, 100, 384, 0, 3);
        assert!(notes.is_empty());

        let mut notes2 = vec![s(0)];
        random_steps(&mut notes2, 60, 24, 100, 384, 100, 3);
        for note in &notes2 {
            assert_eq!(note.len, 24);
            assert!(note.start.is_multiple_of(24));
        }
        assert_eq!(notes2.len(), 16);

        let mut notes3 = vec![s(0)];
        random_steps(&mut notes3, 60, 24, 100, 384, 50, 3);
        assert!((1..=15).contains(&notes3.len()));
        for note in &notes3 {
            assert_eq!(note.len, 24);
            assert!(note.start.is_multiple_of(24));
        }
    }
}

// ---------------------------------------------------------------- scales, chords, ramps

/// Each key to the nearest key of the scale (`root` 0 = C, offsets 0..=11); a tie goes down.
pub fn snap_to_scale(notes: &mut [Note], root: u8, scale: &[u8]) {
    if scale.is_empty() {
        return;
    }
    for note in notes {
        let k = note.key;
        let mut best_t = 0u8;
        let mut best_dist = i32::MAX;
        for t in 0..=127u8 {
            let pitch_class = (t as i32 - root as i32).rem_euclid(12) as u8;
            if scale.contains(&pitch_class) {
                let dist = (k as i32 - t as i32).abs();
                if dist < best_dist || (dist == best_dist && t < best_t) {
                    best_dist = dist;
                    best_t = t;
                }
            }
        }
        note.key = best_t;
    }
}

/// Each note followed by a copy per interval above it (copies above key 127 are skipped).
pub fn make_chords(notes: &[Note], intervals: &[u8]) -> Vec<Note> {
    let mut result = Vec::new();
    for note in notes {
        result.push(*note);
        for &interval in intervals {
            let new_key = note.key as u16 + interval as u16;
            if new_key <= 127 {
                result.push(Note { key: new_key as u8, ..*note });
            }
        }
    }
    result
}

/// Velocities from `from` (earliest note) to `to` (latest), in a straight line.
pub fn velocity_ramp(notes: &mut [Note], from: u8, to: u8) {
    let n = notes.len();
    if n == 0 {
        return;
    }
    let mut order: Vec<usize> = (0..n).collect();
    // Stable sort: ties keep their slice order.
    order.sort_by_key(|&i| notes[i].start);
    for (pos, i) in order.into_iter().enumerate() {
        let v = if n == 1 {
            from as f64
        } else {
            from as f64 + (to as f64 - from as f64) * (pos as f64 / (n - 1) as f64)
        };
        notes[i].vel = (v.round() as i32).clamp(1, 127) as u8;
    }
}

#[cfg(test)]
mod tests4 {
    use super::*;

    fn n(key: u8, start: u32) -> Note {
        Note { key, start, len: 24, vel: 100 }
    }

    #[test]
    fn test_snap_to_scale() {
        let mut notes = vec![
            n(61, 0),
            n(63, 0),
            n(66, 0),
            n(60, 0),
            n(127, 0),
        ];
        let major = [0, 2, 4, 5, 7, 9, 11];
        snap_to_scale(&mut notes, 0, &major);
        assert_eq!(notes[0].key, 60);
        assert_eq!(notes[1].key, 62);
        assert_eq!(notes[2].key, 65);
        assert_eq!(notes[3].key, 60);
        assert_eq!(notes[4].key, 127);

        let mut notes2 = vec![
            n(61, 0),
            n(63, 0),
            n(66, 0),
            n(60, 0),
            n(127, 0),
        ];
        snap_to_scale(&mut notes2, 2, &major);
        assert_eq!(notes2[3].key, 59);

        let mut notes3 = vec![n(60, 0)];
        snap_to_scale(&mut notes3, 0, &[]);
        assert_eq!(notes3[0].key, 60);
    }

    #[test]
    fn test_make_chords() {
        let notes = vec![
            n(60, 0),
            n(125, 24),
        ];
        let chords = make_chords(&notes, &[4, 7]);
        assert_eq!(chords.len(), 4);
        assert_eq!(chords[0].key, 60);
        assert_eq!(chords[1].key, 64);
        assert_eq!(chords[2].key, 67);
        assert_eq!(chords[3].key, 125);
    }

    #[test]
    fn test_velocity_ramp() {
        let mut notes = vec![
            n(60, 48),
            n(60, 0),
            n(60, 24),
            n(60, 72),
            n(60, 96),
        ];
        velocity_ramp(&mut notes, 20, 100);
        assert_eq!(notes[0].vel, 60);
        assert_eq!(notes[1].vel, 20);
        assert_eq!(notes[2].vel, 40);
        assert_eq!(notes[3].vel, 80);
        assert_eq!(notes[4].vel, 100);
        let mut one = vec![n(60, 0)];
        velocity_ramp(&mut one, 20, 100);
        assert_eq!(one[0].vel, 20);
    }

    #[test]
    fn chords_never_overflow() {
        assert_eq!(make_chords(&[n(127, 0)], &[200]), vec![n(127, 0)]);
    }
}

// ---------------------------------------------------------------- time and clean-up

/// Stretches the notes in time by num / den around the earliest start.
pub fn stretch(notes: &mut [Note], num: u32, den: u32) {
    if notes.is_empty() {
        return;
    }
    let first = notes.iter().map(|n| n.start).min().unwrap();
    for note in notes.iter_mut() {
        let new_start = first + ((note.start as u64 - first as u64) * num as u64 / den as u64) as u32;
        let new_len = (note.len as u64 * num as u64 / den as u64).max(1) as u32;
        note.start = new_start;
        note.len = new_len;
    }
}

/// Keeps the first note of each (key, start).
pub fn remove_duplicates(notes: &mut Vec<Note>) {
    let mut seen = std::collections::HashSet::new();
    let mut kept = Vec::new();
    for note in notes.iter() {
        let key_start = (note.key, note.start);
        if seen.insert(key_start) {
            kept.push(*note);
        }
    }
    *notes = kept;
}

/// Shortens notes that run into the next note of the same key (ties: slice order).
pub fn trim_overlaps(notes: &mut [Note]) {
    let mut order: Vec<usize> = (0..notes.len()).collect();
    // Stable: same key and start keep their slice order.
    order.sort_by_key(|&i| (notes[i].key, notes[i].start));
    for w in order.windows(2) {
        let (cur, next) = (w[0], w[1]);
        if notes[cur].key == notes[next].key && notes[cur].start + notes[cur].len > notes[next].start {
            notes[cur].len = (notes[next].start - notes[cur].start).max(1);
        }
    }
}

#[cfg(test)]
mod tests5 {
    use super::*;

    fn n(key: u8, start: u32, len: u32) -> Note {
        Note { key, start, len, vel: 100 }
    }

    #[test]
    fn test_stretch() {
        let mut notes = vec![
            n(60, 96, 24),
            n(62, 144, 48)
        ];
        stretch(&mut notes, 2, 1);
        assert_eq!(notes, vec![
            n(60, 96, 48),
            n(62, 192, 96)
        ]);

        let mut notes2 = vec![
            n(60, 0, 48),
            n(60, 48, 1)
        ];
        stretch(&mut notes2, 1, 2);
        assert_eq!(notes2, vec![
            n(60, 0, 24),
            n(60, 24, 1)
        ]);
    }

    #[test]
    fn test_remove_duplicates() {
        let mut notes = vec![
            n(60, 0, 24),
            n(62, 0, 24),
            n(60, 0, 48),
            n(60, 24, 24)
        ];
        remove_duplicates(&mut notes);
        assert_eq!(notes, vec![
            n(60, 0, 24),
            n(62, 0, 24),
            n(60, 24, 24)
        ]);
    }

    #[test]
    fn test_trim_overlaps() {
        let mut notes = vec![
            n(60, 24, 48),
            n(60, 0, 48),
            n(62, 0, 96),
            n(60, 24, 10)
        ];
        trim_overlaps(&mut notes);
        assert_eq!(notes, vec![
            n(60, 24, 1),
            n(60, 0, 24),
            n(62, 0, 96),
            n(60, 24, 10)
        ]);
    }

    #[test]
    fn test_empty() {
        let mut empty: Vec<Note> = vec![];
        stretch(&mut empty, 2, 1);
        remove_duplicates(&mut empty);
        trim_overlaps(&mut empty);
    }
}

// ---------------------------------------------------------------- riff generator

fn next_xorshift(state: &mut u32) -> u32 {
    let mut s = *state;
    s ^= s << 13;
    s ^= s >> 17;
    s ^= s << 5;
    *state = s;
    s
}

/// A random melody in a scale (xorshift32 from `seed`): about `density` steps in 100 start a
/// note, moving by small jumps along the scale, one or two steps long, never overlapping.
#[allow(clippy::too_many_arguments)]
pub fn riff(root: u8, scale: &[u8], low: u8, high: u8, steps: u32, step_len: u32, density: u32, seed: u32) -> Vec<Note> {
    let allowed: Vec<u8> = (low..=high.min(127)).filter(|&k| scale.contains(&((k as i32 - root as i32).rem_euclid(12) as u8))).collect();

    if allowed.is_empty() {
        return Vec::new();
    }

    let len = allowed.len();
    let mid = len / 2;

    let mut state: u32 = seed | 1;
    let mut notes: Vec<Note> = Vec::new();
    let mut i: u32 = 0;
    while i < steps {
        let r = next_xorshift(&mut state) % 100;
        if r >= density {
            i += 1;
            continue;
        }

        let start = i * step_len;

        let j = (next_xorshift(&mut state) % 5) as i32 - 2;
        let key_index = match notes.last() {
            None => mid,
            Some(prev) => {
                let prev_idx = allowed.iter().position(|&k| k == prev.key).unwrap_or(mid);
                (prev_idx as i32 + j).clamp(0, len as i32 - 1) as usize
            }
        };
        let key = allowed[key_index];

        let len_factor = next_xorshift(&mut state) % 2;
        let mut note_len = step_len * (1 + len_factor);
        let max_start = steps * step_len;
        if start + note_len > max_start {
            note_len = max_start - start;
        }

        let vel = 80 + (next_xorshift(&mut state) % 41) as u8;

        notes.push(Note { key, start, len: note_len, vel });

        i += note_len / step_len;
    }

    notes
}

#[cfg(test)]
mod tests_riff {
    use super::*;

    #[test]
    fn test_density() {
        let scale = [0u8, 2, 4, 5, 7, 9, 11];
        let notes = riff(0, &scale, 48, 72, 16, 24, 0, 1);
        assert!(notes.is_empty());

        let notes = riff(0, &scale, 48, 72, 16, 24, 100, 1);
        assert!(!notes.is_empty());
        assert!(notes.len() >= 8);
    }

    #[test]
    fn test_c_major() {
        let scale = [0u8, 2, 4, 5, 7, 9, 11];
        for seed in 1..=20 {
            let notes = riff(0, &scale, 48, 72, 16, 24, 60, seed);
            for note in &notes {
                assert!(matches!(note.key % 12, 0 | 2 | 4 | 5 | 7 | 9 | 11));
                assert!((48..=72).contains(&note.key));
            }
            for note in &notes {
                assert_eq!(note.start % 24, 0);
            }
            for w in notes.windows(2) {
                assert!(w[1].start >= w[0].start + w[0].len, "overlap");
            }
            let max_time = 16 * 24;
            for note in &notes {
                assert!(note.start + note.len <= max_time);
            }
            for note in &notes {
                assert!((80..=120).contains(&note.vel));
            }
        }
    }

    #[test]
    fn test_determinism() {
        let scale = [0u8, 2, 4, 5, 7, 9, 11];
        let notes1: Vec<Note> = riff(0, &scale, 48, 72, 16, 24, 60, 1);
        let notes2: Vec<Note> = riff(0, &scale, 48, 72, 16, 24, 60, 1);
        assert_eq!(notes1, notes2);

        let notes1: Vec<Note> = riff(0, &scale, 48, 72, 16, 24, 60, 1);
        let notes3: Vec<Note> = riff(0, &scale, 48, 72, 16, 24, 60, 2);
        assert_ne!(notes1, notes3);
    }

    #[test]
    fn test_empty_scale() {
        let scale: [u8; 0] = [];
        let notes = riff(0, &scale, 48, 72, 16, 24, 60, 1);
        assert!(notes.is_empty());
    }
}
