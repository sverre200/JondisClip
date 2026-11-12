//! Share to Discord (ADR-0012): one H.264 file with the Mix audio track that
//! fits the user's upload limit. Stream copy when that already fits;
//! otherwise scaled/decimated per `jondisclip_core::discord::plan`, encoded
//! CBR, measured, and re-encoded smaller once if needed. Never hands over a
//! file above the limit.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ffmpeg::software::scaling;
use ffmpeg::{
    ChannelLayout, Dictionary, Packet, Rational, codec, decoder, encoder, format, frame, media,
    picture,
};
use ffmpeg_next as ffmpeg;
use jondisclip_core::discord::{self, Plan, PlanError};
use jondisclip_engine::encoder::Vendor;
use serde::Serialize;

use super::{ExportError, Mux, Pacer, ff, finalize, is_again, secs, to_ts};

#[derive(Debug, Clone)]
pub struct Share {
    pub input: PathBuf,
    pub output: PathBuf,
    pub start: f64,
    pub end: f64,
    pub limit_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShareOutcome {
    pub path: PathBuf,
    pub size: u64,
    pub limit: u64,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Stream copy (no quality loss) instead of a re-encode.
    pub copied: bool,
    /// "copy" or the encoder used.
    pub video: String,
    /// Encodes it took to fit (1 = first try).
    pub attempts: u32,
}

/// What we need to know about the source before deciding.
struct Source {
    vidx: usize,
    mix: Option<usize>,
    width: u32,
    height: u32,
    fps: f64,
    is_h264: bool,
    /// B-frames rule out a lossless copy cut (ADR-0011).
    has_bframes: bool,
    /// Average bytes per second of video + Mix (what a copy would cost).
    copy_rate: f64,
    /// Video bits per second of the source, if known.
    video_bps: Option<f64>,
}

fn inspect(path: &Path) -> Result<Source, ExportError> {
    let ictx = format::input(path).map_err(ff("open the clip"))?;
    let v = ictx
        .streams()
        .best(media::Type::Video)
        .ok_or(ExportError::NoVideo)?;
    let vp = v.parameters();
    let (width, height, vbit) = unsafe {
        let p = vp.as_ptr();
        ((*p).width as u32, (*p).height as u32, (*p).bit_rate)
    };
    let r = v.avg_frame_rate();
    let fps = if r.numerator() > 0 && r.denominator() > 0 {
        f64::from(r)
    } else {
        60.0
    };
    // The Mix is the default audio track (the recorder marks it), else the first.
    let audio: Vec<_> = ictx
        .streams()
        .filter(|s| s.parameters().medium() == media::Type::Audio)
        .collect();
    let mix = audio
        .iter()
        .find(|s| unsafe { (*s.as_ptr()).disposition } & ffmpeg::ffi::AV_DISPOSITION_DEFAULT != 0)
        .or(audio.first())
        .map(|s| s.index());
    let mix_bit = mix
        .and_then(|i| ictx.stream(i))
        .map(|s| unsafe { (*s.parameters().as_ptr()).bit_rate })
        .unwrap_or(0);
    let duration = ictx.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE);
    let file = std::fs::metadata(path)?.len() as f64;
    let copy_rate = if vbit > 0 {
        (vbit + mix_bit.max(0)) as f64 / 8.0
    } else if duration > 0.0 {
        file / duration
    } else {
        f64::MAX
    };
    Ok(Source {
        vidx: v.index(),
        mix,
        width,
        height,
        fps,
        is_h264: vp.id() == codec::Id::H264,
        copy_rate,
        has_bframes: unsafe { (*vp.as_ptr()).video_delay } > 0,
        video_bps: (vbit > 0).then_some(vbit as f64),
    })
}

pub fn share(job: &Share, mut progress: impl FnMut(f64)) -> Result<ShareOutcome, ExportError> {
    let valid =
        job.start.is_finite() && job.end.is_finite() && job.start >= 0.0 && job.end > job.start;
    if !valid || job.limit_bytes == 0 {
        return Err(ExportError::EmptyRange);
    }
    if job.output.exists() {
        return Err(ExportError::Exists(job.output.clone()));
    }
    ffmpeg::init().map_err(ff("FFmpeg init"))?;
    unsafe { ffmpeg::ffi::av_log_set_level(ffmpeg::ffi::AV_LOG_ERROR) };
    let src = inspect(&job.input)?;
    let seconds = job.end - job.start;
    let tmp = job.output.with_extension("mp4.partial");

    // 1. Already fits as a lossless copy? (H.264, start on a keyframe.)
    let on_keyframe = jondisclip_core::library::keyframes_file(&job.input)
        .map(|k| k.iter().any(|t| (t - job.start).abs() <= 1.0 / 120.0))
        .unwrap_or(false);
    // Progress: the copy try is the first 10 %, encodes share the rest.
    let mut copy_progress = |p: f64| progress(p * 0.1);
    if src.is_h264
        && !src.has_bframes
        && on_keyframe
        && discord::copy_fits((src.copy_rate * seconds) as u64, job.limit_bytes)
    {
        let trim = super::Trim {
            input: job.input.clone(),
            output: tmp.clone(),
            start: job.start,
            end: job.end,
            mode: super::Mode::Copy,
        };
        let _ = std::fs::remove_file(&tmp);
        // A copy that fails for any reason other than IO (odd timestamps…)
        // just means "re-encode instead".
        let copied = match super::run_tracks(&trim, &tmp, &mut copy_progress, src.mix) {
            Ok(out) => Some(out),
            Err(ExportError::Io(e)) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(ExportError::Io(e));
            }
            Err(_) => None,
        };
        let size = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(u64::MAX);
        if let Some(out) = copied
            && size <= job.limit_bytes
        {
            finalize(&tmp, &job.output)?;
            progress(1.0);
            return Ok(ShareOutcome {
                path: job.output.clone(),
                size,
                limit: job.limit_bytes,
                duration: out.duration,
                width: src.width,
                height: src.height,
                fps: src.fps.round() as u32,
                copied: true,
                video: "copy".into(),
                attempts: 0,
            });
        }
        let _ = std::fs::remove_file(&tmp);
    }

    // 2. Re-encode to the plan, measure, shrink once if needed.
    let plan = discord::plan(
        seconds,
        src.width,
        src.height,
        src.fps,
        src.video_bps,
        job.limit_bytes,
    )
    .map_err(|e| match e {
        PlanError::TooLong { max_seconds } => ExportError::TooLongForLimit {
            max_seconds,
            limit: job.limit_bytes,
        },
        PlanError::Invalid => ExportError::EmptyRange,
    })?;
    let mut video_bps = plan.video_bps as f64;
    let mut last_size = 0;
    for attempt in 1..=3u32 {
        let _ = std::fs::remove_file(&tmp);
        // Attempt 1 fills 10–80 %, a retry 80–95 %, a last one 95–99 %.
        let (from, span) = match attempt {
            1 => (0.1, 0.7),
            2 => (0.8, 0.15),
            _ => (0.95, 0.04),
        };
        let mut step = |p: f64| progress(from + span * p);
        let result = encode_once(job, &src, &plan, video_bps as u64, &tmp, &mut step);
        let (duration, name) = match result {
            Ok(r) => r,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        };
        let size = std::fs::metadata(&tmp)?.len();
        last_size = size;
        if size <= job.limit_bytes {
            finalize(&tmp, &job.output)?;
            progress(1.0);
            return Ok(ShareOutcome {
                path: job.output.clone(),
                size,
                limit: job.limit_bytes,
                duration,
                width: plan.width,
                height: plan.height,
                fps: plan.fps,
                copied: false,
                video: name.into(),
                attempts: attempt,
            });
        }
        // Over: scale the video budget by how far off we were, with margin.
        video_bps *= job.limit_bytes as f64 / size as f64 * 0.95;
    }
    let _ = std::fs::remove_file(&tmp);
    Err(ExportError::OverLimit {
        size: last_size,
        limit: job.limit_bytes,
    })
}

struct Video {
    dec: decoder::Video,
    enc: encoder::Video,
    name: &'static str,
    in_tb: Rational,
    frame: frame::Video,
    scaled: frame::Video,
    scaler: Option<scaling::Context>,
    width: u32,
    height: u32,
    frame_ticks: i64,
    pacer: Pacer,
}

impl Video {
    fn encode(&mut self, pts: i64, mux: &mut Mux, o: usize) -> Result<(), ExportError> {
        self.pacer.wait(secs(pts, self.in_tb));
        if self.scaler.is_none() {
            // AREA is cheap and sharp for downscaling; BICUBIC otherwise.
            let down = self.width < self.frame.width();
            self.scaler = Some(
                scaling::Context::get(
                    self.frame.format(),
                    self.frame.width(),
                    self.frame.height(),
                    format::Pixel::NV12,
                    self.width,
                    self.height,
                    if down {
                        scaling::Flags::AREA
                    } else {
                        scaling::Flags::BICUBIC
                    },
                )
                .map_err(ff("scale"))?,
            );
        }
        if let Some(s) = self.scaler.as_mut() {
            // An encoder may still hold a reference to the last frame's
            // buffer: never draw into it (no-op when it doesn't).
            unsafe {
                if !self.scaled.is_empty() {
                    ffmpeg::ffi::av_frame_make_writable(self.scaled.as_mut_ptr());
                }
            }
            s.run(&self.frame, &mut self.scaled).map_err(ff("scale"))?;
        }
        self.scaled.set_pts(Some(pts));
        self.scaled.set_kind(picture::Type::None);
        for attempt in 0..2000 {
            match self.enc.send_frame(&self.scaled) {
                Ok(()) => return self.drain(mux, o).map(drop),
                Err(e) if is_again(&e) => {
                    if self.drain(mux, o)? == 0 {
                        std::thread::sleep(Duration::from_millis(if attempt < 10 { 1 } else { 5 }));
                    }
                }
                Err(e) => {
                    return Err(ExportError::Ffmpeg {
                        what: "encode",
                        source: e,
                    });
                }
            }
        }
        Err(ExportError::EncoderBusy)
    }

    fn drain(&mut self, mux: &mut Mux, o: usize) -> Result<usize, ExportError> {
        let mut pkt = Packet::empty();
        let mut n = 0;
        loop {
            match self.enc.receive_packet(&mut pkt) {
                Ok(()) => {
                    if pkt.duration() <= 0 {
                        pkt.set_duration(self.frame_ticks);
                    }
                    mux.write(
                        std::mem::replace(&mut pkt, Packet::empty()),
                        o,
                        self.in_tb,
                        0,
                    )?;
                    n += 1;
                }
                Err(e) if is_again(&e) || matches!(e, ffmpeg::Error::Eof) => return Ok(n),
                Err(e) => {
                    return Err(ExportError::Ffmpeg {
                        what: "encode",
                        source: e,
                    });
                }
            }
        }
    }

    fn next_frame(&mut self) -> Result<bool, ExportError> {
        match self.dec.receive_frame(&mut self.frame) {
            Ok(()) => Ok(true),
            Err(e) if is_again(&e) || matches!(e, ffmpeg::Error::Eof) => Ok(false),
            Err(e) => Err(ExportError::Ffmpeg {
                what: "decode",
                source: e,
            }),
        }
    }
}

/// Mix audio: copied as is, or transcoded to AAC at a lower bitrate.
struct Audio {
    in_tb: Rational,
    transcode: Option<(decoder::Audio, encoder::Audio, frame::Audio)>,
}

impl Audio {
    fn push(&mut self, pkt: Packet, base: f64, mux: &mut Mux, o: usize) -> Result<(), ExportError> {
        let shift = to_ts(base, self.in_tb);
        let Some((dec, enc, frame)) = self.transcode.as_mut() else {
            return mux.write(pkt, o, self.in_tb, shift);
        };
        match dec.send_packet(&pkt) {
            Ok(()) | Err(ffmpeg::Error::InvalidData) => {}
            Err(e) => {
                return Err(ExportError::Ffmpeg {
                    what: "decode audio",
                    source: e,
                });
            }
        }
        loop {
            match dec.receive_frame(frame) {
                Ok(()) => {}
                Err(e) if is_again(&e) || matches!(e, ffmpeg::Error::Eof) => break,
                Err(e) => {
                    return Err(ExportError::Ffmpeg {
                        what: "decode audio",
                        source: e,
                    });
                }
            }
            let pts = frame.pts().map(|p| p - shift);
            frame.set_pts(pts);
            enc.send_frame(frame).map_err(ff("encode audio"))?;
            drain_audio(enc, mux, o)?;
        }
        Ok(())
    }

    fn finish(&mut self, mux: &mut Mux, o: usize) -> Result<(), ExportError> {
        if let Some((_, enc, _)) = self.transcode.as_mut() {
            enc.send_eof().map_err(ff("finish audio"))?;
            drain_audio(enc, mux, o)?;
        }
        Ok(())
    }
}

fn drain_audio(enc: &mut encoder::Audio, mux: &mut Mux, o: usize) -> Result<(), ExportError> {
    let tb = enc.time_base();
    let mut pkt = Packet::empty();
    loop {
        match enc.receive_packet(&mut pkt) {
            Ok(()) => mux.write(std::mem::replace(&mut pkt, Packet::empty()), o, tb, 0)?,
            Err(e) if is_again(&e) || matches!(e, ffmpeg::Error::Eof) => return Ok(()),
            Err(e) => {
                return Err(ExportError::Ffmpeg {
                    what: "encode audio",
                    source: e,
                });
            }
        }
    }
}

fn encode_once(
    job: &Share,
    src: &Source,
    plan: &Plan,
    video_bps: u64,
    tmp: &Path,
    progress: &mut impl FnMut(f64),
) -> Result<(f64, &'static str), ExportError> {
    let mut ictx = format::input(&job.input).map_err(ff("open the clip"))?;
    let mut octx = format::output_as(tmp, "mp4").map_err(ff("create the output"))?;
    let global_header = octx.format().flags().contains(format::Flags::GLOBAL_HEADER);

    // Video: decoder + CBR encoder at the planned size and rate.
    let vst = ictx.stream(src.vidx).ok_or(ExportError::NoVideo)?;
    let in_tb = vst.time_base();
    let mut dctx =
        codec::context::Context::from_parameters(vst.parameters()).map_err(ff("decoder"))?;
    dctx.set_threading(codec::threading::Config {
        kind: codec::threading::Type::Frame,
        count: 4,
    });
    let dec = dctx.decoder().video().map_err(ff("open decoder"))?;
    let (color_space, color_range) = (dec.color_space(), dec.color_range());
    let mut errors = Vec::new();
    let mut opened = None;
    for (id, software) in crate::export_candidates() {
        let Some(codec) = encoder::find_by_name(id.ffmpeg_name()) else {
            continue;
        };
        let attempt = (|| {
            let mut enc = codec::context::Context::new_with_codec(codec)
                .encoder()
                .video()?;
            enc.set_width(plan.width);
            enc.set_height(plan.height);
            // NV12: what AMF/NVENC/QSV/MF all take natively (QSV only takes NV12).
            enc.set_format(format::Pixel::NV12);
            enc.set_time_base(in_tb);
            enc.set_frame_rate(Some(Rational::new(plan.fps as i32, 1)));
            enc.set_bit_rate(video_bps as usize);
            enc.set_max_bit_rate(video_bps as usize);
            enc.set_gop(plan.fps * 2);
            enc.set_max_b_frames(0);
            enc.set_colorspace(color_space);
            enc.set_color_range(color_range);
            if global_header {
                enc.set_flags(codec::Flags::GLOBAL_HEADER);
            }
            unsafe {
                // 1 s of buffer: CBR that respects the size closely.
                (*enc.as_mut_ptr()).rc_buffer_size = video_bps.min(i32::MAX as u64) as i32;
            }
            let mut opts = Dictionary::new();
            match id.vendor {
                Vendor::Amd => {
                    opts.set("usage", "transcoding");
                    opts.set("quality", "balanced");
                    opts.set("rc", "cbr");
                    opts.set("filler_data", "0");
                }
                Vendor::Nvidia => {
                    opts.set("preset", "p5");
                    opts.set("tune", "hq");
                    opts.set("rc", "cbr");
                }
                Vendor::Intel => opts.set("preset", "medium"),
                Vendor::MediaFoundation => {
                    opts.set("hw_encoding", if software { "0" } else { "1" });
                    opts.set("rate_control", "cbr");
                }
            }
            enc.open_as_with(codec, opts)
        })();
        match attempt {
            Ok(enc) => {
                opened = Some((enc, id.ffmpeg_name()));
                break;
            }
            Err(e) => errors.push(format!("{}: {e}", id.ffmpeg_name())),
        }
    }
    let (enc, name) = opened.ok_or_else(|| ExportError::NoEncoder(errors.join("; ")))?;
    let mut ost = octx
        .add_stream(encoder::find(codec::Id::None))
        .map_err(ff("add stream"))?;
    ost.set_parameters(codec::Parameters::from(&*enc));
    ost.set_time_base(in_tb);
    unsafe { (*ost.as_mut_ptr()).disposition = ffmpeg::ffi::AV_DISPOSITION_DEFAULT };
    let mut video = Video {
        dec,
        enc,
        name,
        in_tb,
        frame: frame::Video::empty(),
        scaled: frame::Video::empty(),
        scaler: None,
        width: plan.width,
        height: plan.height,
        frame_ticks: (1.0 / f64::from(plan.fps) / f64::from(in_tb)).round() as i64,
        pacer: Pacer::default(),
    };

    // Audio: the Mix only.
    let mut audio = None;
    if let Some(mi) = src.mix
        && let Some(ast) = ictx.stream(mi)
    {
        let a_tb = ast.time_base();
        let mut aost = octx
            .add_stream(encoder::find(codec::Id::None))
            .map_err(ff("add stream"))?;
        let transcode = if plan.audio_bps == 0 {
            aost.set_parameters(ast.parameters());
            unsafe { (*aost.parameters().as_mut_ptr()).codec_tag = 0 };
            None
        } else {
            let adec = codec::context::Context::from_parameters(ast.parameters())
                .map_err(ff("audio decoder"))?
                .decoder()
                .audio()
                .map_err(ff("open audio decoder"))?;
            let codec =
                encoder::find(codec::Id::AAC).ok_or(ExportError::NoEncoder("aac".into()))?;
            let mut aenc = codec::context::Context::new_with_codec(codec)
                .encoder()
                .audio()
                .map_err(ff("audio encoder"))?;
            aenc.set_rate(adec.rate() as i32);
            // A plain native layout for the channel count (never a copy of the
            // decoder's, which may be custom-ordered).
            aenc.set_channel_layout(ChannelLayout::default(i32::from(adec.channels().max(1))));
            aenc.set_format(adec.format());
            aenc.set_bit_rate(plan.audio_bps as usize);
            aenc.set_time_base(a_tb);
            if global_header {
                aenc.set_flags(codec::Flags::GLOBAL_HEADER);
            }
            let aenc = aenc.open_as(codec).map_err(ff("open audio encoder"))?;
            aost.set_parameters(codec::Parameters::from(&*aenc));
            Some((adec, aenc, frame::Audio::empty()))
        };
        aost.set_time_base(a_tb);
        let mut md = Dictionary::new();
        md.set("handler_name", "Mix");
        aost.set_metadata(md);
        unsafe { (*aost.as_mut_ptr()).disposition = ffmpeg::ffi::AV_DISPOSITION_DEFAULT };
        audio = Some((
            mi,
            Audio {
                in_tb: a_tb,
                transcode,
            },
        ));
    }

    let mut opts = Dictionary::new();
    opts.set("movflags", "+faststart");
    octx.write_header_with(opts)
        .map_err(ff("write the header"))?;
    let out_tbs: Vec<Rational> = octx.streams().map(|s| s.time_base()).collect();
    let mut mux = Mux {
        octx,
        last_dts: vec![None; out_tbs.len()],
        out_tbs,
        written: 0,
        video: 0,
    };

    let start_us = (job.start * 1e6) as i64;
    ictx.seek(start_us, ..start_us).map_err(ff("seek"))?;

    let src_frame = 1.0 / src.fps.max(1.0);
    let step = 1.0 / f64::from(plan.fps);
    let mut next_keep = job.start;
    let mut base: Option<f64> = None;
    let mut video_done = false;
    let mut audio_done = audio.is_none();
    let mut pending: Vec<Packet> = Vec::new();
    let mut video_end = 0.0f64;
    let span = (job.end - job.start).max(1e-6);

    // Decides and encodes one decoded frame.
    let mut take = |video: &mut Video,
                    mux: &mut Mux,
                    base: &mut Option<f64>,
                    video_end: &mut f64|
     -> Result<bool, ExportError> {
        let Some(fp) = video.frame.pts().or(video.frame.timestamp()) else {
            return Ok(false);
        };
        let ft = secs(fp, in_tb);
        if ft >= job.end - 1e-6 {
            return Ok(true); // done
        }
        if ft + src_frame * 0.5 < job.start || ft + src_frame * 0.5 < next_keep {
            return Ok(false); // before the selection, or dropped to reach the target fps
        }
        next_keep = if ft - next_keep > step {
            ft + step
        } else {
            next_keep + step
        };
        let b = *base.get_or_insert(ft);
        video.encode(fp - to_ts(b, in_tb), mux, 0)?;
        *video_end = ft - b + step;
        progress(((ft - job.start) / span).clamp(0.0, 0.99));
        Ok(false)
    };

    for (stream, pkt) in ictx.packets() {
        let i = stream.index();
        let Some(pts) = pkt.pts() else { continue };
        if i == src.vidx && !video_done {
            match video.dec.send_packet(&pkt) {
                Ok(()) => {}
                Err(ffmpeg::Error::InvalidData) => continue,
                Err(e) => {
                    return Err(ExportError::Ffmpeg {
                        what: "decode",
                        source: e,
                    });
                }
            }
            while video.next_frame()? {
                if take(&mut video, &mut mux, &mut base, &mut video_end)? {
                    video_done = true;
                    break;
                }
            }
            if let (Some(b), Some((_, a))) = (base, audio.as_mut()) {
                for p in pending.drain(..) {
                    let t = secs(p.pts().unwrap_or(i64::MIN), a.in_tb);
                    if t >= b - 1e-6 && t < job.end {
                        a.push(p, b, &mut mux, 1)?;
                    }
                }
            }
        } else if let Some((mi, a)) = audio.as_mut()
            && i == *mi
            && !audio_done
        {
            let t = secs(pts, a.in_tb);
            if t >= job.end {
                audio_done = true;
            } else {
                match base {
                    Some(b) if t >= b - 1e-6 => a.push(pkt, b, &mut mux, 1)?,
                    Some(_) => {}
                    None => pending.push(pkt),
                }
            }
        }
        if video_done && audio_done {
            break;
        }
    }
    if !video_done {
        let _ = video.dec.send_eof();
        while video.next_frame()? {
            if take(&mut video, &mut mux, &mut base, &mut video_end)? {
                break;
            }
        }
    }
    video.enc.send_eof().map_err(ff("finish encoding"))?;
    video.drain(&mut mux, 0)?;
    if let Some((_, a)) = audio.as_mut() {
        if let Some(b) = base {
            for p in pending.drain(..) {
                let t = secs(p.pts().unwrap_or(i64::MIN), a.in_tb);
                if t >= b - 1e-6 && t < job.end {
                    a.push(p, b, &mut mux, 1)?;
                }
            }
        }
        a.finish(&mut mux, 1)?;
    }
    if mux.written == 0 || base.is_none() {
        return Err(ExportError::NothingWritten);
    }
    mux.octx.write_trailer().map_err(ff("finish the file"))?;
    Ok((video_end, video.name))
}
