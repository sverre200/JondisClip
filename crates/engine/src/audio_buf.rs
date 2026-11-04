//! Timeline-addressed sample buffer between an audio capture thread and the
//! mixer.
//!
//! Each source pushes interleaved stereo f32 chunks at an absolute sample
//! index (48 kHz, counted from the recorder's QPC epoch). The mixer reads
//! fixed blocks by index. Gaps read as silence, late data is dropped, and
//! memory is bounded. Pure logic, so it's unit-tested without devices.

use std::collections::VecDeque;

pub const CHANNELS: usize = 2;

#[derive(Debug)]
pub struct SourceBuf {
    /// Sample index of the first frame in `data` (None until the first push).
    start: Option<i64>,
    /// Interleaved stereo samples.
    data: VecDeque<f32>,
    max_frames: usize,
    /// Frames that arrived after the mixer had already passed them.
    pub late_frames: u64,
}

impl SourceBuf {
    pub fn new(max_frames: usize) -> Self {
        Self {
            start: None,
            data: VecDeque::with_capacity(max_frames * CHANNELS),
            max_frames,
            late_frames: 0,
        }
    }

    fn end(&self) -> Option<i64> {
        self.start.map(|s| s + (self.data.len() / CHANNELS) as i64)
    }

    /// Appends `samples` (interleaved stereo) whose first frame is at `at`.
    /// `silent` writes zeros of the same length (WASAPI SILENT flag).
    pub fn push(&mut self, at: i64, samples: &[f32], silent: bool) {
        let frames = (samples.len() / CHANNELS) as i64;
        if frames == 0 {
            return;
        }
        let mut skip = 0i64;
        match self.end() {
            None => self.start = Some(at),
            Some(end) if at > end => {
                let gap = at - end;
                if gap as usize > self.max_frames {
                    // Long dropout (device switch): restart the timeline.
                    self.data.clear();
                    self.start = Some(at);
                } else {
                    self.data
                        .extend(std::iter::repeat_n(0.0, gap as usize * CHANNELS));
                }
            }
            Some(end) if at < end => {
                skip = end - at;
                if skip >= frames {
                    self.late_frames += frames as u64;
                    return;
                }
                self.late_frames += skip as u64;
            }
            Some(_) => {}
        }
        let from = skip as usize * CHANNELS;
        if silent {
            self.data
                .extend(std::iter::repeat_n(0.0, samples.len() - from));
        } else {
            self.data.extend(samples[from..].iter().copied());
        }
        // Bound memory if the mixer stalls: drop the oldest frames.
        let excess = (self.data.len() / CHANNELS).saturating_sub(self.max_frames);
        if excess > 0 {
            self.data.drain(..excess * CHANNELS);
            self.start = self.start.map(|s| s + excess as i64);
        }
    }

    /// Fills `out` (interleaved, `out.len() / 2` frames) with the frames
    /// starting at index `k`. Missing frames read as silence. Everything before
    /// `k + frames` is consumed, and later pushes for that range count as late.
    pub fn read(&mut self, k: i64, out: &mut [f32]) {
        out.fill(0.0);
        let n = (out.len() / CHANNELS) as i64;
        let Some(start) = self.start else {
            // Nothing has arrived yet. Anchor here so late data is rejected.
            self.start = Some(k + n);
            return;
        };
        let avail = (self.data.len() / CHANNELS) as i64;
        let first = (k - start).max(0); // first buffered frame we can use
        let last = (k + n - start).min(avail); // one past the last
        if last > first {
            let dst_off = (start + first - k) as usize * CHANNELS;
            for (i, v) in self
                .data
                .range(first as usize * CHANNELS..last as usize * CHANNELS)
                .enumerate()
            {
                out[dst_off + i] = *v;
            }
        }
        let consume = (k + n - start).clamp(0, avail);
        self.data.drain(..consume as usize * CHANNELS);
        self.start = Some(start.max(k + n).max(start + consume));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(vals: &[f32]) -> Vec<f32> {
        vals.iter().flat_map(|&v| [v, v]).collect()
    }

    fn read(b: &mut SourceBuf, k: i64, n: usize) -> Vec<f32> {
        let mut out = vec![9.0; n * CHANNELS];
        b.read(k, &mut out);
        out.chunks(2).map(|c| c[0]).collect()
    }

    #[test]
    fn reads_what_was_pushed() {
        let mut b = SourceBuf::new(100);
        b.push(10, &st(&[1.0, 2.0, 3.0]), false);
        assert_eq!(read(&mut b, 10, 3), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn missing_ranges_read_as_silence() {
        let mut b = SourceBuf::new(100);
        b.push(12, &st(&[1.0, 2.0]), false);
        assert_eq!(read(&mut b, 10, 5), vec![0.0, 0.0, 1.0, 2.0, 0.0]);
    }

    #[test]
    fn empty_source_is_silence_and_rejects_older_data() {
        let mut b = SourceBuf::new(100);
        assert_eq!(read(&mut b, 0, 4), vec![0.0; 4]);
        b.push(2, &st(&[5.0, 6.0, 7.0]), false); // frames 2,3 already played
        assert_eq!(b.late_frames, 2);
        assert_eq!(read(&mut b, 4, 2), vec![7.0, 0.0]);
    }

    #[test]
    fn gap_between_pushes_is_filled_with_silence() {
        let mut b = SourceBuf::new(100);
        b.push(0, &st(&[1.0]), false);
        b.push(3, &st(&[4.0]), false);
        assert_eq!(read(&mut b, 0, 4), vec![1.0, 0.0, 0.0, 4.0]);
    }

    #[test]
    fn overlapping_push_is_trimmed() {
        let mut b = SourceBuf::new(100);
        b.push(0, &st(&[1.0, 2.0, 3.0]), false);
        b.push(2, &st(&[9.0, 4.0]), false); // frame 2 already present
        assert_eq!(read(&mut b, 0, 4), vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(b.late_frames, 1);
    }

    #[test]
    fn consecutive_reads_continue_without_loss() {
        let mut b = SourceBuf::new(100);
        b.push(0, &st(&[1.0, 2.0, 3.0, 4.0, 5.0]), false);
        assert_eq!(read(&mut b, 0, 2), vec![1.0, 2.0]);
        assert_eq!(read(&mut b, 2, 2), vec![3.0, 4.0]);
        assert_eq!(read(&mut b, 4, 2), vec![5.0, 0.0]);
    }

    #[test]
    fn silent_flag_writes_zeros() {
        let mut b = SourceBuf::new(100);
        b.push(0, &st(&[1.0, 1.0]), true);
        assert_eq!(read(&mut b, 0, 2), vec![0.0, 0.0]);
    }

    #[test]
    fn memory_is_bounded_when_not_read() {
        let mut b = SourceBuf::new(4);
        b.push(0, &st(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), false);
        assert_eq!(read(&mut b, 2, 4), vec![3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn long_dropout_restarts_timeline() {
        let mut b = SourceBuf::new(4);
        b.push(0, &st(&[1.0]), false);
        b.push(1000, &st(&[2.0]), false);
        assert_eq!(read(&mut b, 1000, 1), vec![2.0]);
    }
}
