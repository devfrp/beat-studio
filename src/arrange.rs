// Playlist editing: insert or delete bars, moving clips and automation
// points with them. First written by the local model (Cline), reviewed.

use crate::engine::Clip;

/// Every clip from bar `at` on moves `count` bars later.
pub fn insert_bars(clips: &mut [Clip], at: u32, count: u32) {
    for clip in clips.iter_mut() {
        if clip.start_bar >= at {
            clip.start_bar += count;
        }
    }
}

/// Removes the bars [at, at + count): clips starting inside go, later clips move back.
pub fn delete_bars(clips: &mut Vec<Clip>, at: u32, count: u32) {
    clips.retain(|c| c.start_bar < at || c.start_bar >= at + count);
    for c in clips.iter_mut().filter(|c| c.start_bar >= at + count) {
        c.start_bar -= count;
    }
}

/// Every automation point from `at` on moves `len` ticks later.
pub fn insert_ticks(points: &mut [(u32, f32)], at: u32, len: u32) {
    for p in points.iter_mut().filter(|p| p.0 >= at) {
        p.0 += len;
    }
}

/// Removes the points in [at, at + len); later points move back by `len`.
pub fn delete_ticks(points: &mut Vec<(u32, f32)>, at: u32, len: u32) {
    let range_start = at;
    let range_end = at + len;
    let mut result = Vec::new();
    for p in points.iter() {
        let tick = p.0;
        if tick >= range_start && tick < range_end {
            // removed
            continue;
        } else if tick >= range_end {
            // move earlier
            result.push((tick - len, p.1));
        } else {
            result.push(*p);
        }
    }
    points.clear();
    points.extend(result);
}

#[cfg(test)]
mod tests_bars {
    use super::*;

    fn c(start_bar: u32) -> Clip {
        Clip { track: 0, start_bar, pattern: 0 }
    }

    #[test]
    fn test_insert_bars() {
        let mut clips = vec![c(0), c(2), c(5)];
        insert_bars(&mut clips, 2, 3);
        assert_eq!(clips, vec![c(0), c(5), c(8)]);
    }

    #[test]
    fn test_delete_bars() {
        let mut clips = vec![c(0), c(2), c(3), c(4), c(6)];
        delete_bars(&mut clips, 2, 2);
        assert_eq!(clips, vec![c(0), c(2), c(4)]);
    }

    #[test]
    fn test_insert_ticks() {
        let mut points = vec![(0, 1.0), (384, 2.0), (500, 3.0)];
        insert_ticks(&mut points, 384, 384);
        assert_eq!(points, vec![(0, 1.0), (768, 2.0), (884, 3.0)]);
    }

    #[test]
    fn test_delete_ticks() {
        let mut points = vec![(0, 1.0), (384, 2.0), (700, 3.0), (768, 4.0), (900, 5.0)];
        delete_ticks(&mut points, 384, 384);
        assert_eq!(points, vec![(0, 1.0), (384, 4.0), (516, 5.0)]);
    }

    #[test]
    fn zero_changes_nothing() {
        let mut clips = vec![c(0), c(3)];
        delete_bars(&mut clips, 0, 0);
        insert_bars(&mut clips, 0, 0);
        assert_eq!(clips, vec![c(0), c(3)]);
    }

    #[test]
    fn test_empty() {
        let mut clips: Vec<Clip> = vec![];
        insert_bars(&mut clips, 2, 3);
        assert_eq!(clips, vec![]);

        let mut points: Vec<(u32, f32)> = vec![];
        insert_ticks(&mut points, 384, 384);
        assert_eq!(points, vec![]);

        let mut clips2: Vec<Clip> = vec![];
        delete_bars(&mut clips2, 2, 2);
        assert_eq!(clips2, vec![]);

        let mut points2: Vec<(u32, f32)> = vec![];
        delete_ticks(&mut points2, 384, 384);
        assert_eq!(points2, vec![]);
    }
}
