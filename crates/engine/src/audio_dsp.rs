//! Small, allocation-free audio DSP for the capture and mix path
//! (docs/perf/2026-11-04-mic-audio.md, ADR-0009). Every stage is chosen to be
//! *transparent* in normal use: it fixes a defect and otherwise leaves the
//! signal alone.
//!
//! - [`Asrc`]: absorbs the clock difference between an audio device and QPC by
//!   resampling a few ppm (windowed-sinc, 16 taps), so the timeline stays
//!   aligned without sample slips or inserted zeros (which are clicks).
//! - [`Limiter`]: lookahead peak limiter at -1 dBFS. It's inactive unless a
//!   peak would clip (no compression, no pumping in normal levels).
//! - [`MonoFold`]: folds a (nearly always mono) mic to one channel. It
//!   detects mics delivered on one side only (common with USB interfaces).
//! - [`db_to_gain`] and [`GainRamp`]: zipper-free gain changes.

use std::f64::consts::PI;
use std::sync::OnceLock;

pub const SR: f64 = 48_000.0;

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

// ---------------------------------------------------------------- ASRC ----

const TAPS: usize = 16;
const HALF: usize = TAPS / 2;
const PHASES: usize = 1024;
/// Input history kept for the filter (power of two ring).
const RING: usize = 64;
/// Max ratio deviation: covers clocks up to 1000 ppm apart.
const MAX_ADJ: f64 = 1000e-6;
/// A stamp error beyond this restarts at once (a real gap, e.g. loopback
/// resuming after silence). Between `resync_frames` and this, 3 packets in a
/// row are required, so a single jitter spike never causes a jump.
const JUMP_FRAMES: f64 = 4800.0; // 100 ms
/// Correction time constant (s) of the drift controller.
const TAU_S: f64 = 2.0;
/// Timestamp error smoothing per packet (about 0.5 s at 10 ms packets).
const ERR_ALPHA: f64 = 0.02;

/// Windowed-sinc fractional-delay table: `PHASES` rows of `TAPS` weights.
fn table() -> &'static [[f32; TAPS]] {
    static T: OnceLock<Vec<[f32; TAPS]>> = OnceLock::new();
    T.get_or_init(|| {
        // PHASES + 1 rows: row PHASES (t = 1.0) lets the interpolation use pi+1.
        (0..=PHASES)
            .map(|p| {
                let t = p as f64 / PHASES as f64;
                let mut row = [0f32; TAPS];
                let mut sum = 0.0;
                for (k, w) in row.iter_mut().enumerate() {
                    // Tap k multiplies x[i + k - (HALF-1)]. Kernel centred on t.
                    let x = k as f64 - (HALF as f64 - 1.0) - t;
                    let sinc = if x.abs() < 1e-9 {
                        1.0
                    } else {
                        (PI * x).sin() / (PI * x)
                    };
                    // Blackman-Harris window over the 16-tap span.
                    let u = (x + HALF as f64) / TAPS as f64;
                    let win = if (0.0..=1.0).contains(&u) {
                        0.35875 - 0.48829 * (2.0 * PI * u).cos() + 0.14128 * (4.0 * PI * u).cos()
                            - 0.01168 * (6.0 * PI * u).cos()
                    } else {
                        0.0
                    };
                    let v = sinc * win;
                    *w = v as f32;
                    sum += v;
                }
                for w in &mut row {
                    *w = (f64::from(*w) / sum) as f32; // unity DC gain
                }
                row
            })
            .collect()
    })
}

/// Per-source asynchronous sample-rate converter and timeline placer.
///
/// Input: device packets (interleaved stereo) with noisy QPC positions.
/// Output: a continuous stream placed at `out_pos`, consuming input at
/// `1 + adj` frames per output frame, where `adj` (|adj| <= 500 ppm) is steered
/// so each input frame lands where its QPC stamp says.
pub struct Asrc {
    ring: [[f32; 2]; RING],
    /// Total input frames written to the ring.
    written: u64,
    /// Fractional read position in input frames (absolute).
    read: f64,
    step: f64,
    /// Timeline position of the next output frame (None = not started).
    out_pos: Option<i64>,
    err_ema: f64,
    big_err_streak: u32,
}

impl Default for Asrc {
    fn default() -> Self {
        // Build the filter table now, not on the first packet (under a lock).
        let _ = table();
        Self {
            ring: [[0.0; 2]; RING],
            written: 0,
            read: 0.0,
            step: 1.0,
            out_pos: None,
            err_ema: 0.0,
            big_err_streak: 0,
        }
    }
}

/// What the caller must do with the produced frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    /// Timeline position of the first produced frame.
    pub at: i64,
    /// The stream restarted here (dropout or resync): fade around it.
    pub restarted: bool,
}

impl Asrc {
    pub fn is_started(&self) -> bool {
        self.out_pos.is_some()
    }

    /// Feeds one packet and appends the resampled frames (interleaved stereo)
    /// to `out`, which must have capacity for about `packet + 2` frames. Returns
    /// where they go, or None if nothing could be placed yet.
    pub fn process(
        &mut self,
        stamped: Option<i64>,
        input: &[f32],
        discontinuity: bool,
        resync_frames: i64,
        out: &mut Vec<f32>,
    ) -> Option<Placed> {
        let frames = input.len() / 2;
        let mut restarted = false;
        // Error between where this packet's first frame *should* land
        // (its stamp) and where the current stream will put it.
        let n0 = self.written as f64;
        match (self.out_pos, stamped) {
            (None, None) => return None,
            (None, Some(s)) => {
                self.start(s);
                restarted = true;
            }
            (Some(pos), Some(s)) => {
                // The filter only delays *production*; content stays aligned with `read`.
                let lands = pos as f64 + (n0 - self.read) / self.step;
                let err = s as f64 - lands;
                if err.abs() > resync_frames as f64 {
                    self.big_err_streak += 1;
                } else {
                    self.big_err_streak = 0;
                }
                if discontinuity || err.abs() > JUMP_FRAMES || self.big_err_streak >= 3 {
                    self.start(s);
                    restarted = true;
                } else if err.abs() <= resync_frames as f64 {
                    self.err_ema += (err - self.err_ema) * ERR_ALPHA;
                    let k = 1.0 / (SR * TAU_S);
                    self.step = 1.0 - (self.err_ema * k).clamp(-MAX_ADJ, MAX_ADJ);
                }
            }
            (Some(_), None) => {} // bad timestamp: keep the current rate
        }
        let at = self.out_pos?;
        let start_len = out.len();
        let tab = table();
        for (i, f) in input.as_chunks::<2>().0.iter().enumerate() {
            let _ = i;
            self.ring[(self.written as usize) & (RING - 1)] = [f[0], f[1]];
            self.written += 1;
            // Produce every output frame whose filter window is now complete.
            while self.read + HALF as f64 <= self.written as f64 - 1.0 {
                let base = self.read.floor();
                let frac = self.read - base;
                // Linear interpolation between adjacent phase rows keeps the
                // fractional-delay error far below audibility.
                let p = frac * PHASES as f64;
                let pi = (p as usize).min(PHASES - 1);
                let pf = (p - pi as f64) as f32;
                let (row0, row1) = (&tab[pi], &tab[(pi + 1).min(PHASES)]);
                let first = base as i64 - (HALF as i64 - 1);
                let (mut l, mut r) = (0f32, 0f32);
                for (k, (w0, w1)) in row0.iter().zip(row1.iter()).enumerate() {
                    let w = w0 + (w1 - w0) * pf;
                    let idx = first + k as i64;
                    // Before the stream started, history is silence.
                    let s = if idx < 0 || (idx as u64) + RING as u64 <= self.written {
                        [0.0, 0.0]
                    } else {
                        self.ring[(idx as usize) & (RING - 1)]
                    };
                    l += w * s[0];
                    r += w * s[1];
                }
                out.push(l);
                out.push(r);
                self.read += self.step;
            }
        }
        let produced = ((out.len() - start_len) / 2) as i64;
        let _ = frames;
        self.out_pos = Some(at + produced);
        Some(Placed { at, restarted })
    }

    /// Timeline position after the last produced frame.
    pub fn end(&self) -> Option<i64> {
        self.out_pos
    }

    fn start(&mut self, stamped: i64) {
        // The next output frame is the next input frame, at its stamp.
        self.read = self.written as f64;
        self.out_pos = Some(stamped);
        self.step = 1.0;
        self.err_ema = 0.0;
        self.big_err_streak = 0;
    }
}

// ------------------------------------------------------------- Limiter ----

/// Lookahead window (frames). Delay is `LIMIT_DELAY`, compensated in pts.
const LOOK: usize = 64;
pub const LIMIT_DELAY: i64 = LOOK as i64 - 1;

/// Stereo-linked (or mono) lookahead peak limiter. Below the ceiling the
/// output equals the input delayed by `LIMIT_DELAY` frames, bit for bit.
pub struct Limiter<const C: usize> {
    ceiling: f32,
    delay: [[f32; C]; LOOK],
    /// Required gain history (sliding min via monotonic deque of indices).
    req: [f32; LOOK],
    dq: [u64; LOOK],
    dq_head: usize,
    dq_len: usize,
    env: f32,
    release: f32,
    boxbuf: [f32; LOOK],
    box_sum: f64,
    n: u64,
}

impl<const C: usize> Limiter<C> {
    pub fn new(ceiling_db: f32, release_ms: f32) -> Self {
        Self::new_with(
            db_to_gain(ceiling_db),
            1.0 - (-1.0 / (release_ms / 1000.0 * SR as f32)).exp(),
        )
    }

    /// Forgets all state (after the mixer skips ahead in time).
    pub fn reset(&mut self) {
        *self = Self::new_with(self.ceiling, self.release);
    }

    fn new_with(ceiling: f32, release: f32) -> Self {
        Self {
            ceiling,
            delay: [[0.0; C]; LOOK],
            req: [1.0; LOOK],
            dq: [0; LOOK],
            dq_head: 0,
            dq_len: 0,
            env: 1.0,
            release,
            boxbuf: [1.0; LOOK],
            box_sum: LOOK as f64,
            n: 0,
        }
    }

    #[cfg(test)]
    fn deque_len(&self) -> usize {
        self.dq_len
    }

    /// Processes interleaved frames in place.
    pub fn process(&mut self, buf: &mut [f32]) {
        for frame in buf.as_chunks_mut::<C>().0.iter_mut() {
            let i = (self.n as usize) % LOOK;
            let peak = frame.iter().fold(0f32, |m, v| m.max(v.abs()));
            let r = if peak > self.ceiling {
                self.ceiling / peak
            } else {
                1.0
            };
            self.req[i] = r;
            // Sliding minimum of the required gain over the last LOOK frames.
            // Expire first, so the deque never holds more than LOOK entries.
            while self.dq_len > 0 && self.dq[self.dq_head] + LOOK as u64 <= self.n {
                self.dq_head = (self.dq_head + 1) % LOOK;
                self.dq_len -= 1;
            }
            while self.dq_len > 0 {
                let back = self.dq[(self.dq_head + self.dq_len - 1) % LOOK];
                if self.req[(back as usize) % LOOK] >= r {
                    self.dq_len -= 1;
                } else {
                    break;
                }
            }
            debug_assert!(self.dq_len < LOOK);
            self.dq[(self.dq_head + self.dq_len) % LOOK] = self.n;
            self.dq_len += 1;
            let m = self.req[(self.dq[self.dq_head] as usize) % LOOK];
            // Instant attack to the windowed minimum, smooth release.
            self.env = if m < self.env {
                m
            } else {
                self.env + (m - self.env) * self.release
            };
            // Box smoothing over LOOK frames reaches the minimum exactly when
            // the peak leaves the delay line, so there's no overshoot.
            self.box_sum += f64::from(self.env) - f64::from(self.boxbuf[i]);
            self.boxbuf[i] = self.env;
            let mut g = (self.box_sum / LOOK as f64) as f32;
            if g > 0.999_999 {
                g = 1.0;
            }
            // Slot i+1 holds the frame from LOOK-1 frames ago.
            let delayed = self.delay[(i + 1) % LOOK];
            self.delay[i].copy_from_slice(frame);
            for (o, d) in frame.iter_mut().zip(delayed) {
                // Final guard against float rounding at the ceiling.
                *o = (d * g).clamp(-self.ceiling, self.ceiling);
            }
            self.n += 1;
        }
    }
}

// ------------------------------------------------------------ MonoFold ----

/// Folds a stereo mic stream to mono. Normally (L+R)/2, which is exact for the
/// usual duplicated mono. If one side stays silent while the other carries
/// signal for about a second (mic on input 1 of a USB interface), it switches
/// to that side alone, crossfaded over one block.
pub struct MonoFold {
    el: f32,
    er: f32,
    mode: (f32, f32),
    pending: (f32, f32),
    pending_blocks: u32,
}

impl Default for MonoFold {
    fn default() -> Self {
        Self {
            el: 0.0,
            er: 0.0,
            mode: (0.5, 0.5),
            pending: (0.5, 0.5),
            pending_blocks: 0,
        }
    }
}

impl MonoFold {
    pub fn weights(&self) -> (f32, f32) {
        self.mode
    }

    /// `stereo` interleaved, `mono` receives len/2 samples.
    pub fn process(&mut self, stereo: &[f32], mono: &mut [f32]) {
        let n = mono.len().max(1) as f32;
        let (mut sl, mut sr) = (0f32, 0f32);
        for f in stereo.as_chunks::<2>().0.iter() {
            sl += f[0] * f[0];
            sr += f[1] * f[1];
        }
        // ~0.5 s energy averages (blocks of 1024 at 48 kHz).
        self.el += (sl / n - self.el) * 0.1;
        self.er += (sr / n - self.er) * 0.1;
        if !self.el.is_finite() || !self.er.is_finite() {
            // A corrupt sample must not freeze the detector forever.
            self.el = 0.0;
            self.er = 0.0;
        }
        let floor = 1e-7; // -70 dBFS: below this, keep the current mode
        // Hysteresis: go one-sided at 30 dB difference, back to both below 20 dB.
        let one_sided = self.mode != (0.5, 0.5);
        let back = if one_sided { 1e-2 } else { 1e-3 };
        let want = if self.el > floor
            && self.er < self.el * back
            && (one_sided || self.er < self.el * 1e-3)
        {
            (1.0, 0.0)
        } else if self.er > floor
            && self.el < self.er * back
            && (one_sided || self.el < self.er * 1e-3)
        {
            (0.0, 1.0)
        } else if self.el > floor && self.er > floor {
            (0.5, 0.5)
        } else {
            self.mode
        };
        if want == self.pending {
            self.pending_blocks += 1;
        } else {
            self.pending = want;
            self.pending_blocks = 0;
        }
        let from = self.mode;
        let to = if self.pending != self.mode && self.pending_blocks >= 47 {
            self.pending
        } else {
            self.mode
        };
        let len = mono.len();
        for (i, (m, f)) in mono
            .iter_mut()
            .zip(stereo.as_chunks::<2>().0.iter())
            .enumerate()
        {
            let t = i as f32 / len as f32;
            let wl = from.0 + (to.0 - from.0) * t;
            let wr = from.1 + (to.1 - from.1) * t;
            *m = f[0] * wl + f[1] * wr;
        }
        self.mode = to;
    }
}

/// Gain that moves to a new target linearly across one block (no zipper noise).
pub struct GainRamp {
    current: f32,
}

impl GainRamp {
    pub fn new(gain: f32) -> Self {
        Self { current: gain }
    }

    pub fn apply(&mut self, target: f32, buf: &mut [f32]) {
        let len = buf.len().max(1) as f32;
        let from = self.current;
        for (i, v) in buf.iter_mut().enumerate() {
            *v *= from + (target - from) * (i as f32 / len);
        }
        self.current = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, freq: f64, amp: f32, ch: usize) -> Vec<f32> {
        let mut v = Vec::with_capacity(n * ch);
        for i in 0..n {
            let s = amp * (2.0 * PI * freq * i as f64 / SR).sin() as f32;
            for _ in 0..ch {
                v.push(s);
            }
        }
        v
    }

    #[test]
    fn asrc_table_has_unity_gain_and_is_exact_at_zero_phase() {
        for row in table().iter().step_by(37) {
            let sum: f32 = row.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5);
        }
        // Phase 0 is a pure delay: one tap is 1, all others 0.
        let r0 = table()[0];
        assert!((r0[HALF - 1] - 1.0).abs() < 1e-6);
        assert!(
            r0.iter()
                .enumerate()
                .all(|(k, w)| k == HALF - 1 || w.abs() < 1e-6)
        );
    }

    #[test]
    fn asrc_at_unity_rate_passes_signal_through_exactly() {
        let mut a = Asrc::default();
        let input = tone(4800, 997.0, 0.5, 2);
        let mut out = Vec::with_capacity(10_000);
        let mut at = None;
        for (p, chunk) in input.chunks(960).enumerate() {
            let placed = a
                .process(Some(1000 + p as i64 * 480), chunk, false, 960, &mut out)
                .unwrap();
            at.get_or_insert(placed.at);
        }
        // At unity rate the output is the input, placed exactly at its stamp.
        assert_eq!(at, Some(1000));
        let produced = out.len() / 2;
        assert!(produced > 4700);
        for i in 0..produced {
            assert!((out[i * 2] - input[i * 2]).abs() < 1e-5, "sample {i}");
        }
    }

    #[test]
    fn asrc_follows_a_drifting_clock_without_resyncing() {
        // Device runs 100 ppm fast relative to QPC: stamps advance 1.0001x.
        let mut a = Asrc::default();
        let input = tone(48_000 * 30, 440.0, 0.5, 2);
        let mut out = Vec::with_capacity(input.len() + 4096);
        let mut restarts = 0;
        for (p, chunk) in input.chunks(960).enumerate() {
            let stamp = (p as f64 * 480.0 * 1.0001).round() as i64 + 48_000;
            if a.process(Some(stamp), chunk, false, 960, &mut out)
                .unwrap()
                .restarted
            {
                restarts += 1;
            }
        }
        assert_eq!(restarts, 1, "only the initial start");
        // Timeline end tracks the stamped clock within 1 ms after converging.
        let expected_end = (input.len() / 2) as f64 * 1.0001 + 48_000.0;
        let end = a.end().unwrap() as f64;
        assert!(
            (end - expected_end).abs() < 48.0 + HALF as f64,
            "end {end} vs {expected_end}"
        );
    }

    #[test]
    fn limiter_is_bit_transparent_below_ceiling() {
        let mut l = Limiter::<2>::new(-1.0, 150.0);
        let input = tone(4096, 1000.0, 0.7, 2);
        let mut buf = input.clone();
        l.process(&mut buf);
        for i in LIMIT_DELAY as usize..4096 {
            assert_eq!(buf[i * 2], input[(i - LIMIT_DELAY as usize) * 2]);
        }
    }

    #[test]
    fn limiter_never_exceeds_ceiling_and_recovers() {
        let mut l = Limiter::<2>::new(-1.0, 150.0);
        let ceiling = db_to_gain(-1.0);
        // 2x over full scale for 0.2 s, then quiet.
        let mut loud = tone(9600, 300.0, 2.0, 2);
        loud.extend(tone(48_000, 300.0, 0.3, 2));
        l.process(&mut loud);
        assert!(loud.iter().all(|v| v.abs() <= ceiling + 1e-6));
        // After about 1 s of quiet input the gain is back to (nearly) unity.
        let tail_peak = loud[loud.len() - 4000..]
            .iter()
            .fold(0f32, |m, v| m.max(v.abs()));
        assert!(tail_peak > 0.29, "released: {tail_peak}");
    }

    #[test]
    fn limiter_matches_brute_force_on_loud_bass() {
        // Loud low-frequency content makes the required gain rise
        // monotonically for more than LOOK frames: the case that overflowed
        // the deque before the review fix.
        for (freq, amp) in [(30.0, 3.0f32), (20.0, 4.0), (50.0, 2.0)] {
            let mut l = Limiter::<1>::new(-1.0, 150.0);
            let input = tone(48_000, freq, amp, 1);
            let mut out = input.clone();
            for chunk in out.chunks_mut(1024) {
                l.process(chunk);
                assert!(l.deque_len() <= LOOK);
            }
            // Brute-force reference of the same algorithm.
            let ceil = db_to_gain(-1.0);
            let rel = 1.0 - (-1.0f32 / (0.150 * SR as f32)).exp();
            let req: Vec<f32> = input
                .iter()
                .map(|v| if v.abs() > ceil { ceil / v.abs() } else { 1.0 })
                .collect();
            let (mut env, mut envs) = (1.0f32, Vec::with_capacity(req.len()));
            for n in 0..req.len() {
                let m = req[n.saturating_sub(LOOK - 1)..=n]
                    .iter()
                    .cloned()
                    .fold(1.0, f32::min);
                env = if m < env { m } else { env + (m - env) * rel };
                envs.push(env);
            }
            for n in LOOK..input.len() {
                let g = (envs[n + 1 - LOOK..=n]
                    .iter()
                    .map(|v| f64::from(*v))
                    .sum::<f64>()
                    / LOOK as f64) as f32;
                let want = (input[n - (LOOK - 1)] * g).clamp(-ceil, ceil);
                assert!(
                    (out[n] - want).abs() < 2e-4,
                    "{freq} Hz at {n}: {} vs {want}",
                    out[n]
                );
            }
        }
    }

    #[test]
    fn asrc_restarts_immediately_on_a_big_forward_jump() {
        let mut a = Asrc::default();
        let input = tone(480, 440.0, 0.5, 2);
        let mut out = Vec::with_capacity(4096);
        a.process(Some(10_000), &input, false, 960, &mut out);
        a.process(Some(10_480), &input, false, 960, &mut out);
        // Loopback resumes after 3 s of silence: stamp jumps far ahead.
        let p = a
            .process(Some(10_960 + 144_000), &input, false, 960, &mut out)
            .unwrap();
        assert!(p.restarted, "no 3-packet wait for a real gap");
        assert_eq!(p.at, 10_960 + 144_000);
        // A single 2 ms jitter spike still doesn't restart.
        let p = a
            .process(Some(10_960 + 144_480 + 96), &input, false, 960, &mut out)
            .unwrap();
        assert!(!p.restarted);
    }

    #[test]
    fn asrc_phase_interpolation_keeps_error_tiny_off_unity_rate() {
        // 300 ppm off: every fractional phase is visited. Compare to the ideal
        // band-limited tone at the resampled positions.
        let mut a = Asrc::default();
        let input = tone(48_000, 1000.0, 0.5, 2);
        let mut out = Vec::with_capacity(input.len() + 4096);
        for (p, chunk) in input.chunks(960).enumerate() {
            let stamp = (p as f64 * 480.0 * 1.0003).round() as i64;
            a.process(Some(stamp), chunk, false, 960, &mut out);
        }
        // Measure local deviation from a smooth sinusoid (second difference).
        let w2 = (2.0 * PI * 1000.0 / SR).powi(2);
        let mut worst = 0f64;
        for n in 4800..out.len() / 2 - 100 {
            let y = |i: usize| f64::from(out[i * 2]);
            let d2 = y(n + 1) - 2.0 * y(n) + y(n - 1);
            worst = worst.max((d2 + w2 * y(n)).abs() / 0.5);
        }
        assert!(worst < 2e-4, "worst {worst}");
    }

    #[test]
    fn mono_fold_has_hysteresis_and_survives_nan() {
        let mut f = MonoFold::default();
        let mut mono = vec![0f32; 1024];
        // Right side ~25 dB down: averaged mode stays (only 30 dB switches).
        let mut v = tone(1024, 500.0, 0.4, 2);
        for fr in v.as_chunks_mut::<2>().0.iter_mut() {
            fr[1] *= 0.056;
        }
        for _ in 0..80 {
            f.process(&v, &mut mono);
        }
        assert_eq!(f.weights(), (0.5, 0.5));
        let mut bad = tone(1024, 500.0, 0.4, 2);
        bad[10] = f32::NAN;
        f.process(&bad, &mut mono);
        let good = tone(1024, 500.0, 0.4, 2);
        for _ in 0..5 {
            f.process(&good, &mut mono);
        }
        assert!(
            mono.iter().all(|x| x.is_finite()),
            "recovers after a NaN block"
        );
    }

    #[test]
    fn mono_fold_averages_duplicated_mono_and_rescues_one_sided_mics() {
        let mut f = MonoFold::default();
        let mut mono = vec![0f32; 1024];
        let dup = tone(1024, 500.0, 0.4, 2);
        f.process(&dup, &mut mono);
        assert!(
            (mono[100] - dup[200]).abs() < 1e-6,
            "duplicated mono keeps its level"
        );
        // Left-only interface input: after about a second it uses L at full level.
        let mut left_only = tone(1024, 500.0, 0.4, 2);
        for fr in left_only.as_chunks_mut::<2>().0.iter_mut() {
            fr[1] = 0.0;
        }
        let mut g = MonoFold::default();
        for _ in 0..60 {
            g.process(&left_only, &mut mono);
        }
        assert_eq!(g.weights(), (1.0, 0.0));
        assert!((mono[500] - left_only[1000]).abs() < 1e-6);
    }

    #[test]
    fn gain_ramp_has_no_steps() {
        let mut g = GainRamp::new(1.0);
        let mut buf = vec![1.0f32; 1024];
        g.apply(2.0, &mut buf);
        assert!((buf[0] - 1.0).abs() < 1e-6 && buf[1023] > 1.99);
        assert!(buf.windows(2).all(|w| (w[1] - w[0]).abs() < 0.01));
    }
}
