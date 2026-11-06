//! Mic check: analyses a short test recording of the user speaking and turns
//! it into gain-staging advice. It runs on demand in the settings window,
//! never in the recorder.
//!
//! Processing matches the recorder exactly (MonoFold → optional low-cut →
//! gain), so "voice level" and "peaks" mean what will land in clips. Levels
//! *before* JondisClip's gain show what the microphone itself delivers. That's
//! where clipping in the mic's own converter shows up, which no software can
//! repair.

use serde::Serialize;

use crate::audio_dsp::{GainRamp, HighPass, LOW_CUT_HZ, MonoFold, db_to_gain};

const WIN: usize = 2400; // 50 ms analysis windows
/// Target active-speech RMS (dBFS) in the recording. Peaks then land around
/// -12 to -6 dBFS, leaving headroom for shouting without hitting the limiter.
pub const TARGET_SPEECH_DB: f32 = -20.0;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MicReport {
    pub seconds: f32,
    /// "mono" (same mic on both channels), "two_track" (mic left, instrument
    /// right: G-Track Pro 2-Track mode) or "stereo".
    pub mode: String,
    /// Peak delivered by the microphone, before JondisClip's gain (dBFS).
    pub raw_peak_db: f32,
    /// Samples at full scale before gain: the mic's converter clipped.
    pub adc_clipped: u32,
    /// Active speech level before JondisClip's gain (dBFS RMS).
    pub raw_speech_db: f32,
    /// As recorded (after fold, low-cut, gain), before the safety limiter.
    pub speech_db: f32,
    pub peak_db: f32,
    pub noise_floor_db: f32,
    /// Milliseconds the safety limiter would have to act (peaks over -1 dBFS).
    pub limited_ms: f32,
    /// Energy below 60 Hz relative to all energy during speech (dB),
    /// measured without the low-cut.
    pub low_freq_db: f32,
    pub speech_detected: bool,
    /// "good", "quiet", "hot", "clipping" or "no_speech".
    pub verdict: String,
    pub tips: Vec<String>,
    /// A tip says to turn the mic's own gain knob up (the window then
    /// doesn't offer JondisClip's gain instead).
    pub knob_up: bool,
    /// Gain (dB) JondisClip would need to reach the target with the current
    /// physical setting.
    pub suggested_gain_db: f32,
}

fn db(v: f64) -> f32 {
    if v > 1e-10 {
        (20.0 * v.log10()) as f32
    } else {
        -200.0
    }
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

fn percentile(mut v: Vec<f32>, p: f32) -> f32 {
    if v.is_empty() {
        return -200.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() - 1) as f32 * p).round() as usize]
}

/// Analyses interleaved stereo `raw` (as captured) with the user's current
/// `gain_db` and `low_cut`. Returns the report and the processed mono audio
/// (what the Microphone track would contain) for listening back.
pub fn analyze(raw: &[f32], gain_db: f32, low_cut: bool) -> (MicReport, Vec<f32>) {
    let frames = raw.len() / 2;
    let seconds = frames as f32 / 48_000.0;

    // --- Mic-side facts (before any processing) ---
    let raw_peak = raw.iter().fold(0f32, |m, v| m.max(v.abs()));
    let adc_clipped = raw.iter().filter(|v| v.abs() >= 0.999).count() as u32;
    let (mut sl, mut sr, mut slr) = (0f64, 0f64, 0f64);
    for f in raw.as_chunks::<2>().0 {
        let (l, r) = (f64::from(f[0]), f64::from(f[1]));
        sl += l * l;
        sr += r * r;
        slr += l * r;
    }
    let corr = slr / (sl * sr).sqrt().max(1e-30);
    let imbalance_db = 10.0 * (sl.max(1e-30) / sr.max(1e-30)).log10();
    let mode = if corr > 0.9 {
        "mono"
    } else if imbalance_db.abs() > 20.0 {
        "two_track"
    } else {
        "stereo"
    };

    // --- The recorder's chain, block by block ---
    let mut fold = MonoFold::default();
    let mut mono = vec![0f32; frames];
    for (inp, out) in raw.chunks(2048).zip(mono.chunks_mut(1024)) {
        let n = inp.len() / 2;
        fold.process(inp, &mut out[..n]);
    }
    let unity = mono.clone(); // folded, no low-cut, no gain
    let mut processed = mono;
    if low_cut {
        HighPass::new(LOW_CUT_HZ).process(&mut processed);
    }
    let mut g = GainRamp::new(db_to_gain(gain_db));
    g.apply(db_to_gain(gain_db), &mut processed);

    // --- Levels over 50 ms windows ---
    let win_raw: Vec<f32> = unity.chunks(WIN).map(|w| db(rms(w))).collect();
    // Digital silence (stream start-up zeros, a muted device) isn't room noise.
    let audible = |v: &Vec<f32>| {
        v.iter()
            .copied()
            .filter(|d| *d > -140.0)
            .collect::<Vec<_>>()
    };
    let noise_raw = percentile(audible(&win_raw), 0.1);
    let speech_thresh = noise_raw + 15.0;
    let speech_idx: Vec<usize> = win_raw
        .iter()
        .enumerate()
        .filter(|(_, d)| **d > speech_thresh && **d > -60.0)
        .map(|(i, _)| i)
        .collect();
    let speech_detected = !win_raw.is_empty() && speech_idx.len() * 10 >= win_raw.len(); // at least 10 % of the time
    let raw_speech = percentile(speech_idx.iter().map(|&i| win_raw[i]).collect(), 0.5);
    let gain_lin_db = gain_db;
    let win_proc: Vec<f32> = processed.chunks(WIN).map(|w| db(rms(w))).collect();
    let speech_db = percentile(speech_idx.iter().map(|&i| win_proc[i]).collect(), 0.5);
    let noise_floor_db = percentile(audible(&win_proc), 0.1);
    let peak = processed.iter().fold(0f32, |m, v| m.max(v.abs()));
    let ceiling = db_to_gain(-1.0);
    let limited_ms = processed.iter().filter(|v| v.abs() > ceiling).count() as f32 / 48.0;

    // Rumble/boom share during speech, without the low-cut: only energy the
    // low-cut would remove (< 60 Hz), via a 4th-order low-pass, so a deep
    // voice's fundamental (85-180 Hz) isn't mistaken for rumble.
    let mut lp = unity.clone();
    let mut f1 = LowPass::new(60.0);
    let mut f2 = LowPass::new(60.0);
    f1.process(&mut lp);
    f2.process(&mut lp);
    let (mut e_lf, mut e_all) = (0f64, 0f64);
    for &i in &speech_idx {
        let r = i * WIN..((i + 1) * WIN).min(frames);
        e_lf += lp[r.clone()]
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>();
        e_all += unity[r].iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
    }
    let low_freq_db = if e_all > 0.0 {
        (10.0 * (e_lf / e_all).log10()) as f32
    } else {
        -200.0
    };

    // --- Verdict and advice ---
    let mut tips: Vec<String> = Vec::new();
    let mut knob_up = false;
    let suggested_gain_db = if speech_detected {
        (TARGET_SPEECH_DB - raw_speech).clamp(-12.0, 24.0).round()
    } else {
        gain_db
    };
    let verdict = if adc_clipped > 0 {
        tips.push(crate::tr!("The microphone itself clipped, before JondisClip. Turn the G-Track's MIC GAIN knob down until this no longer appears. This distortion can't be fixed afterwards."));
        "clipping"
    } else if !speech_detected {
        tips.push(crate::tr!("No speech found. Speak normally for the whole test, about 20-30 cm from the mic, into the front (logo side)."));
        "no_speech"
    } else if speech_db < TARGET_SPEECH_DB - 6.0 {
        "quiet"
    } else if speech_db > TARGET_SPEECH_DB + 6.0 || limited_ms > 50.0 {
        "hot"
    } else {
        "good"
    };
    if speech_detected && adc_clipped == 0 {
        // Gain staging: prefer the physical knob (better signal-to-noise)
        // when the mic has headroom; JondisClip's gain only scales noise and voice together.
        let headroom = -db(f64::from(raw_peak)); // dB to full scale at the mic
        let needed = TARGET_SPEECH_DB - raw_speech; // dB from mic level to target
        if needed > 3.0 && headroom > needed + 3.0 {
            knob_up = true;
            tips.push(crate::tr!(
                "Your voice reaches the computer at {level} dBFS. Turn the G-Track's MIC GAIN knob up (aim for about {more} dB more) and set Microphone gain here back to 0 dB. A louder signal from the mic itself has less hiss than digital boost.",
                level = format!("{raw_speech:.0}"),
                more = format!("{needed:.0}")
            ));
        } else if needed > 3.0 {
            tips.push(crate::tr!(
                "Peaks from the mic are already close to full scale, so raise the level here instead: set Microphone gain to about {gain} dB.",
                gain = format!("{suggested_gain_db:+.0}")
            ));
        } else if needed < -6.0 {
            tips.push(crate::tr!(
                "Your voice is louder than needed. Turn the MIC GAIN knob down a little (about {less} dB), or set Microphone gain to {gain} dB.",
                less = format!("{:.0}", -needed),
                gain = format!("{suggested_gain_db:+.0}")
            ));
        }
        if gain_lin_db > 6.0 && needed < 3.0 {
            tips.push(crate::tr!(
                "Microphone gain here is {gain} dB. With the knob set right, 0 dB is best.",
                gain = format!("{gain_lin_db:+.0}")
            ));
        }
        if limited_ms > 50.0 {
            tips.push(crate::tr!(
                "Peaks hit the safety limiter for {ms} ms. Back off a little from the mic, or lower the gain.",
                ms = format!("{limited_ms:.0}")
            ));
        }
    }
    if noise_floor_db > -50.0 && speech_detected {
        tips.push(crate::tr!(
            "Background noise is fairly high ({level} dBFS). Get closer to the mic and use less gain, and point the back of the mic towards fans or the PC (cardioid pattern rejects sound from behind).",
            level = format!("{noise_floor_db:.0}")
        ));
    }
    if !low_cut && low_freq_db > -20.0 && speech_detected {
        tips.push(crate::tr!(
            "Strong rumble below 60 Hz while you speak ({level} dB): proximity boom, desk bumps or breath pops. Turn on Low-cut filter.",
            level = format!("{low_freq_db:.0}")
        ));
    }
    if mode == "two_track" {
        tips.push(crate::tr!("The G-Track Pro is in 2-Track mode (mic on the left channel only). JondisClip handles it, but MONO mode is simpler and recommended: press the mode button so MONO is lit."));
    }
    if verdict == "good" && tips.is_empty() {
        tips.push(crate::tr!(
            "Your microphone is set up well. No changes needed."
        ));
    }

    let report = MicReport {
        seconds,
        mode: mode.into(),
        raw_peak_db: db(f64::from(raw_peak)),
        adc_clipped,
        raw_speech_db: raw_speech,
        speech_db,
        peak_db: db(f64::from(peak)),
        noise_floor_db,
        limited_ms,
        low_freq_db,
        speech_detected,
        verdict: verdict.into(),
        tips,
        knob_up,
        suggested_gain_db,
    };
    (report, processed)
}

/// 2nd-order Butterworth low-pass (RBJ), used only for analysis.
struct LowPass {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl LowPass {
    fn new(freq: f64) -> Self {
        let w = 2.0 * std::f64::consts::PI * freq / 48_000.0;
        let (cos, alpha) = (w.cos(), w.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2));
        let a0 = 1.0 + alpha;
        Self {
            b: [
                (1.0 - cos) / 2.0 / a0,
                (1.0 - cos) / a0,
                (1.0 - cos) / 2.0 / a0,
            ],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        for v in buf {
            let x0 = f64::from(*v);
            let y0 = self.b[0] * x0 + self.b[1] * self.x[0] + self.b[2] * self.x[1]
                - self.a[0] * self.y[0]
                - self.a[1] * self.y[1];
            self.x = [x0, self.x[0]];
            self.y = [y0, self.y[0]];
            *v = y0 as f32;
        }
    }
}

/// Mono 16-bit PCM WAV of `samples` (48 kHz), for listening back.
pub fn wav_bytes(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&48_000u32.to_le_bytes());
    w.extend_from_slice(&(48_000u32 * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        w.extend_from_slice(&v.to_le_bytes());
    }
    w
}

/// Standard base64 (no dependency for one call site).
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speech-like test signal: 1 s bursts of a voiced tone (fundamental plus
    /// harmonics) separated by pauses, over a noise floor. Interleaved stereo.
    fn speechy(speech_rms_db: f32, noise_db: f32, bass: f32, two_track: bool) -> Vec<f32> {
        let a = db_to_gain(speech_rms_db) * 1.6; // harmonic mix has crest ~1.6
        let nz = db_to_gain(noise_db) * 1.7;
        let mut seed = 12345u32;
        let mut out = Vec::with_capacity(48_000 * 8 * 2);
        for i in 0..48_000 * 8 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = ((seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5) * nz;
            let t = i as f32 / 48_000.0;
            let on = (t % 2.0) < 1.2;
            let w = 2.0 * std::f32::consts::PI * 140.0 * t;
            let voice = if on {
                a * (0.6 * w.sin() + 0.3 * (2.0 * w).sin() + 0.2 * (3.0 * w).sin())
            } else {
                0.0
            };
            // Proximity boom / breath: bass that comes with the voice.
            let rumble = if on {
                bass * (2.0 * std::f32::consts::PI * 45.0 * t).sin()
            } else {
                0.0
            };
            let l = voice + noise + rumble;
            let r = if two_track { noise * 0.05 } else { l };
            out.push(l);
            out.push(r);
        }
        out
    }

    #[test]
    fn good_level_needs_no_changes() {
        let (r, processed) = analyze(&speechy(-20.0, -75.0, 0.0, false), 0.0, false);
        assert!(
            r.low_freq_db < -20.0,
            "a 140 Hz voice is not rumble: {}",
            r.low_freq_db
        );
        assert!(
            !r.tips.iter().any(|t| t.contains("Low-cut")),
            "{:?}",
            r.tips
        );
        assert_eq!(r.mode, "mono");
        assert!(r.speech_detected);
        assert!((r.speech_db - -20.0).abs() < 3.0, "speech {}", r.speech_db);
        assert_eq!(r.verdict, "good", "{r:?}");
        assert_eq!(processed.len(), 48_000 * 8);
    }

    #[test]
    fn quiet_mic_with_headroom_recommends_the_physical_knob() {
        let (r, _) = analyze(&speechy(-34.0, -82.0, 0.0, false), 0.0, false);
        assert_eq!(r.verdict, "quiet");
        assert!(
            r.tips.iter().any(|t| t.contains("MIC GAIN knob up")),
            "{:?}",
            r.tips
        );
        assert!((r.suggested_gain_db - 14.0).abs() <= 3.0);
    }

    #[test]
    fn digital_gain_counts_and_too_much_is_flagged() {
        let s = speechy(-34.0, -82.0, 0.0, false);
        let (r0, _) = analyze(&s, 0.0, false);
        let (r, _) = analyze(&s, 18.0, false);
        assert!(
            (r.speech_db - r0.speech_db - 18.0).abs() < 0.5,
            "gain applied to voice"
        );
        assert!(
            (r.noise_floor_db - r0.noise_floor_db - 18.0).abs() < 0.5,
            "and equally to the noise floor"
        );
        assert!(r.tips.iter().any(|t| t.contains("knob up")), "{:?}", r.tips);
    }

    #[test]
    fn converter_clipping_is_reported_first() {
        let mut s = speechy(-20.0, -75.0, 0.0, false);
        for v in s.iter_mut().step_by(997) {
            *v = 1.0;
        }
        let (r, _) = analyze(&s, 0.0, false);
        assert_eq!(r.verdict, "clipping");
        assert!(r.adc_clipped > 0);
    }

    #[test]
    fn two_track_mode_and_bass_are_detected() {
        let (r, _) = analyze(&speechy(-20.0, -70.0, 0.2, true), 0.0, false);
        assert_eq!(r.mode, "two_track");
        assert!(r.low_freq_db > -10.0, "bass {}", r.low_freq_db);
        assert!(r.tips.iter().any(|t| t.contains("Low-cut")));
        assert!(r.tips.iter().any(|t| t.contains("2-Track")));
        // With the low-cut on, 45 Hz rumble drops by about 10 dB (a gentle 2nd-order
        // slope by design); compare with the same signal, low-cut off.
        let sig = speechy(-80.0, -100.0, 0.2, false);
        let (_, off) = analyze(&sig, 0.0, false);
        let (_, on) = analyze(&sig, 0.0, true);
        let ratio = rms(&on[24_000..]) / rms(&off[24_000..]);
        assert!(
            ratio < 0.4,
            "rumble reduced by at least 8 dB: ratio {ratio:.3}"
        );
    }

    #[test]
    fn startup_zeros_do_not_fake_a_perfect_noise_floor() {
        let mut s = speechy(-20.0, -80.0, 0.0, false);
        for v in &mut s[..48_000] {
            *v = 0.0; // half a second of zeros at stream start
        }
        let (r, _) = analyze(&s, 0.0, false);
        assert!(
            r.noise_floor_db > -100.0 && r.noise_floor_db < -80.0,
            "noise {}",
            r.noise_floor_db
        );
        assert_eq!(r.verdict, "good");
    }

    #[test]
    fn silence_is_no_speech() {
        let (r, _) = analyze(&speechy(-120.0, -80.0, 0.0, false), 0.0, false);
        assert!(!r.speech_detected);
        assert_eq!(r.verdict, "no_speech");
    }

    #[test]
    fn wav_and_base64_are_well_formed() {
        let w = wav_bytes(&[0.0, 0.5, -0.5]);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(w.len(), 44 + 6);
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }
}
