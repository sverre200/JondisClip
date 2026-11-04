//! Segment index for the disk-backed replay buffer (ADR-0002).
//!
//! The encoder/muxer appends one [`Segment`] every ~2 s. The ring keeps just
//! enough segments to cover the retention window, and reports which segment
//! files may be deleted. A clip save *pins* a range so retention never deletes
//! files that a background finalise job is still reading.
//!
//! This module is pure logic (no IO, no Windows APIs) and is called once per
//! segment, not per frame, so small allocations here are acceptable.

use std::collections::{HashMap, VecDeque};

/// One finished segment file in the buffer directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// Monotonic sequence number; also determines the file name.
    pub seq: u64,
    /// Capture run. A new run starts after a display/device change; segments
    /// from different runs cannot be concatenated with stream copy.
    pub run: u32,
    /// Duration in microseconds.
    pub duration_us: i64,
    /// File size in bytes.
    pub bytes: u64,
}

/// Handle for a pinned range returned by [`Ring::pin_last`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PinId(u64);

/// A pinned range of segments to finalise into a clip, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub id: PinId,
    pub segments: Vec<Segment>,
}

impl Pin {
    pub fn duration_us(&self) -> i64 {
        self.segments.iter().map(|s| s.duration_us).sum()
    }

    /// Splits the range into consecutive same-run groups; each group becomes
    /// one output file.
    pub fn runs(&self) -> Vec<&[Segment]> {
        self.segments.chunk_by(|a, b| a.run == b.run).collect()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RingError {
    #[error("retention must be positive, got {0} µs")]
    InvalidRetention(i64),
    #[error("segment duration must be positive, got {0} µs")]
    InvalidSegmentDuration(i64),
    #[error("segment sequence {got} is not after {last}")]
    OutOfOrder { last: u64, got: u64 },
    #[error("requested clip length must be positive, got {0} µs")]
    InvalidClipLength(i64),
    #[error("the buffer is empty")]
    Empty,
    #[error("unknown pin")]
    UnknownPin,
}

#[derive(Debug)]
struct Entry {
    seg: Segment,
    pins: u32,
}

#[derive(Debug)]
pub struct Ring {
    retain_us: i64,
    /// Segments inside the retention window, oldest first.
    live: VecDeque<Entry>,
    /// Segments evicted from the window but still pinned (kept on disk).
    evicted_pinned: HashMap<u64, Entry>,
    pins: HashMap<PinId, Vec<u64>>,
    next_pin: u64,
    live_duration_us: i64,
}

impl Ring {
    pub fn new(retain_us: i64) -> Result<Self, RingError> {
        if retain_us <= 0 {
            return Err(RingError::InvalidRetention(retain_us));
        }
        Ok(Self {
            retain_us,
            live: VecDeque::new(),
            evicted_pinned: HashMap::new(),
            pins: HashMap::new(),
            next_pin: 0,
            live_duration_us: 0,
        })
    }

    /// Changes the retention window (user changed replay duration). Returns
    /// segments that may now be deleted.
    pub fn set_retention(&mut self, retain_us: i64) -> Result<Vec<Segment>, RingError> {
        if retain_us <= 0 {
            return Err(RingError::InvalidRetention(retain_us));
        }
        self.retain_us = retain_us;
        Ok(self.evict())
    }

    /// Appends a finished segment. Returns segments whose files may be deleted.
    pub fn push(&mut self, seg: Segment) -> Result<Vec<Segment>, RingError> {
        if seg.duration_us <= 0 {
            return Err(RingError::InvalidSegmentDuration(seg.duration_us));
        }
        if let Some(last) = self.last_seq()
            && seg.seq <= last
        {
            return Err(RingError::OutOfOrder { last, got: seg.seq });
        }
        self.live_duration_us += seg.duration_us;
        self.live.push_back(Entry { seg, pins: 0 });
        Ok(self.evict())
    }

    /// Pins the newest segments covering at least `len_us` (or everything
    /// available if the buffer is shorter). Never blocks recording.
    pub fn pin_last(&mut self, len_us: i64) -> Result<Pin, RingError> {
        if len_us <= 0 {
            return Err(RingError::InvalidClipLength(len_us));
        }
        if self.live.is_empty() {
            return Err(RingError::Empty);
        }
        let mut covered = 0;
        let mut first = self.live.len();
        while first > 0 && covered < len_us {
            first -= 1;
            covered += self.live[first].seg.duration_us;
        }
        let mut segments = Vec::with_capacity(self.live.len() - first);
        for e in self.live.range_mut(first..) {
            e.pins += 1;
            segments.push(e.seg);
        }
        let id = PinId(self.next_pin);
        self.next_pin += 1;
        self.pins
            .insert(id, segments.iter().map(|s| s.seq).collect());
        Ok(Pin { id, segments })
    }

    /// Releases a pin (finalise job finished or failed). Returns segments that
    /// were evicted while pinned and may now be deleted.
    pub fn unpin(&mut self, id: PinId) -> Result<Vec<Segment>, RingError> {
        let seqs = self.pins.remove(&id).ok_or(RingError::UnknownPin)?;
        let mut deletable = Vec::new();
        for seq in seqs {
            if let Some(e) = self.live.iter_mut().find(|e| e.seg.seq == seq) {
                e.pins -= 1;
            } else if let Some(e) = self.evicted_pinned.get_mut(&seq) {
                e.pins -= 1;
                if e.pins == 0
                    && let Some(e) = self.evicted_pinned.remove(&seq)
                {
                    deletable.push(e.seg);
                }
            }
        }
        deletable.sort_by_key(|s| s.seq);
        Ok(deletable)
    }

    /// Ends the buffer (recorder stopped). Every segment leaves the window:
    /// unpinned ones are returned for deletion now, and pinned ones are returned
    /// by `unpin` when their save finishes.
    pub fn close(&mut self) -> Vec<Segment> {
        let mut deletable = Vec::new();
        while let Some(e) = self.live.pop_front() {
            if e.pins == 0 {
                deletable.push(e.seg);
            } else {
                self.evicted_pinned.insert(e.seg.seq, e);
            }
        }
        self.live_duration_us = 0;
        deletable
    }

    /// True when nothing is on disk any more (every save finished after `close`).
    pub fn is_drained(&self) -> bool {
        self.live.is_empty() && self.evicted_pinned.is_empty()
    }

    /// Total duration currently available for a clip.
    pub fn available_us(&self) -> i64 {
        self.live_duration_us
    }

    /// Bytes on disk, including evicted-but-pinned segments.
    pub fn disk_bytes(&self) -> u64 {
        self.live
            .iter()
            .chain(self.evicted_pinned.values())
            .map(|e| e.seg.bytes)
            .sum()
    }

    fn last_seq(&self) -> Option<u64> {
        self.live.back().map(|e| e.seg.seq)
    }

    /// Drops the oldest segments while the remaining ones still cover the
    /// retention window.
    fn evict(&mut self) -> Vec<Segment> {
        let mut deletable = Vec::new();
        while let Some(front) = self.live.front() {
            if self.live_duration_us - front.seg.duration_us < self.retain_us {
                break;
            }
            let Some(e) = self.live.pop_front() else {
                break;
            };
            self.live_duration_us -= e.seg.duration_us;
            if e.pins == 0 {
                deletable.push(e.seg);
            } else {
                self.evicted_pinned.insert(e.seg.seq, e);
            }
        }
        deletable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: i64 = 1_000_000;

    fn seg(seq: u64) -> Segment {
        Segment {
            seq,
            run: 0,
            duration_us: 2 * S,
            bytes: 10,
        }
    }

    fn filled(retain_s: i64, n: u64) -> Ring {
        let mut r = Ring::new(retain_s * S).unwrap();
        for i in 0..n {
            r.push(seg(i)).unwrap();
        }
        r
    }

    fn seqs(v: &[Segment]) -> Vec<u64> {
        v.iter().map(|s| s.seq).collect()
    }

    #[test]
    fn rejects_invalid_input() {
        assert_eq!(Ring::new(0).unwrap_err(), RingError::InvalidRetention(0));
        let mut r = filled(10, 1);
        assert_eq!(
            r.push(seg(0)).unwrap_err(),
            RingError::OutOfOrder { last: 0, got: 0 }
        );
        let bad = Segment {
            duration_us: 0,
            ..seg(5)
        };
        assert_eq!(
            r.push(bad).unwrap_err(),
            RingError::InvalidSegmentDuration(0)
        );
        assert_eq!(r.pin_last(0).unwrap_err(), RingError::InvalidClipLength(0));
        assert_eq!(
            Ring::new(S).unwrap().pin_last(S).unwrap_err(),
            RingError::Empty
        );
    }

    #[test]
    fn keeps_exactly_enough_to_cover_retention() {
        let mut r = filled(10, 5); // 10 s
        assert_eq!(r.available_us(), 10 * S);
        assert_eq!(seqs(&r.push(seg(5)).unwrap()), vec![0]);
        assert_eq!(r.available_us(), 10 * S);
    }

    #[test]
    fn never_drops_below_retention_with_uneven_segments() {
        let mut r = Ring::new(5 * S).unwrap();
        r.push(Segment {
            duration_us: 3 * S,
            ..seg(0)
        })
        .unwrap();
        r.push(Segment {
            duration_us: 3 * S,
            ..seg(1)
        })
        .unwrap();
        // Dropping seg 0 would leave 3 s < 5 s, so it must stay.
        assert!(
            r.push(Segment {
                duration_us: S,
                ..seg(2)
            })
            .unwrap()
            .is_empty()
        );
        assert_eq!(r.available_us(), 7 * S);
        // 3 + 1 + 1 = 5 s covers retention without seg 0.
        assert_eq!(
            seqs(
                &r.push(Segment {
                    duration_us: S,
                    ..seg(3)
                })
                .unwrap()
            ),
            vec![0]
        );
    }

    #[test]
    fn pin_last_covers_requested_length() {
        let mut r = filled(60, 30); // 60 s
        let p = r.pin_last(5 * S).unwrap();
        assert_eq!(seqs(&p.segments), vec![27, 28, 29]); // 6 s >= 5 s
        assert_eq!(p.duration_us(), 6 * S);
    }

    #[test]
    fn pin_longer_than_buffer_returns_everything() {
        let mut r = filled(60, 3);
        let p = r.pin_last(300 * S).unwrap();
        assert_eq!(seqs(&p.segments), vec![0, 1, 2]);
    }

    #[test]
    fn pinned_segments_survive_eviction_until_unpinned() {
        let mut r = filled(4, 2); // segs 0,1
        let p = r.pin_last(4 * S).unwrap();
        // Recording continues; 0 and 1 leave the window but are pinned.
        assert!(r.push(seg(2)).unwrap().is_empty());
        assert!(r.push(seg(3)).unwrap().is_empty());
        assert_eq!(r.available_us(), 4 * S);
        assert_eq!(r.disk_bytes(), 40);
        assert_eq!(seqs(&r.unpin(p.id).unwrap()), vec![0, 1]);
        assert_eq!(r.disk_bytes(), 20);
    }

    #[test]
    fn overlapping_pins_release_only_when_both_done() {
        let mut r = filled(4, 2);
        let a = r.pin_last(4 * S).unwrap();
        let b = r.pin_last(2 * S).unwrap(); // seg 1 only
        r.push(seg(2)).unwrap();
        r.push(seg(3)).unwrap();
        assert_eq!(seqs(&r.unpin(a.id).unwrap()), vec![0]);
        assert_eq!(seqs(&r.unpin(b.id).unwrap()), vec![1]);
        assert_eq!(r.unpin(a.id).unwrap_err(), RingError::UnknownPin);
    }

    #[test]
    fn unpin_inside_window_deletes_nothing() {
        let mut r = filled(60, 5);
        let p = r.pin_last(4 * S).unwrap();
        assert!(r.unpin(p.id).unwrap().is_empty());
        // And those segments are evicted normally later.
        let mut r2 = filled(2, 1);
        let p2 = r2.pin_last(S).unwrap();
        r2.unpin(p2.id).unwrap();
        assert_eq!(seqs(&r2.push(seg(1)).unwrap()), vec![0]);
    }

    #[test]
    fn shrinking_retention_evicts_immediately() {
        let mut r = filled(20, 10);
        assert_eq!(
            seqs(&r.set_retention(4 * S).unwrap()),
            (0..8).collect::<Vec<_>>()
        );
        assert_eq!(r.available_us(), 4 * S);
    }

    #[test]
    fn close_keeps_pinned_until_unpinned() {
        let mut r = filled(60, 4);
        let p = r.pin_last(2 * S).unwrap(); // seg 3
        assert_eq!(seqs(&r.close()), vec![0, 1, 2]);
        assert!(!r.is_drained());
        assert_eq!(r.available_us(), 0);
        assert_eq!(seqs(&r.unpin(p.id).unwrap()), vec![3]);
        assert!(r.is_drained());
    }

    #[test]
    fn pin_splits_by_capture_run() {
        let mut r = Ring::new(60 * S).unwrap();
        for (seq, run) in [(0, 0), (1, 0), (2, 1), (3, 1), (4, 1)] {
            r.push(Segment { run, ..seg(seq) }).unwrap();
        }
        let p = r.pin_last(60 * S).unwrap();
        let runs: Vec<Vec<u64>> = p.runs().iter().map(|g| seqs(g)).collect();
        assert_eq!(runs, vec![vec![0, 1], vec![2, 3, 4]]);
    }
}
