//! Maps user settings to the recorder configuration. This lives in the tray
//! app so `jondisclip-core` (shared with the UI) never depends on the engine.

use jondisclip_core::settings::{CodecPref, Settings};
use jondisclip_engine::audio::AudioConfig;
use jondisclip_engine::encoder::Codec;
use jondisclip_engine::recorder::RecorderConfig;

pub fn recorder_config(s: &Settings) -> RecorderConfig {
    RecorderConfig {
        fps: s.fps,
        output_height: s.resolution.height(),
        codec: match s.codec {
            CodecPref::Auto | CodecPref::H264 => Codec::H264,
            CodecPref::Hevc => Codec::Hevc,
            CodecPref::Av1 => Codec::Av1,
        },
        quality_scale: s.quality.scale(),
        qp_floor: Some(s.quality.qp_floor()),
        replay: s.replay(),
        audio: AudioConfig {
            system: s.system_audio,
            mic: s.microphone,
            mic_device: (!s.microphone_device.is_empty()).then(|| s.microphone_device.clone()),
            ..AudioConfig::default()
        },
        ..RecorderConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_codec_is_h264() {
        assert_eq!(recorder_config(&Settings::default()).codec, Codec::H264);
        let (s, _) = Settings::from_toml("codec = \"hevc\"\n");
        assert_eq!(recorder_config(&s).codec, Codec::Hevc);
    }

    #[test]
    fn maps_quality_and_audio() {
        let (s, _) = Settings::from_toml("quality = \"high\"\nmicrophone = true\n");
        let c = recorder_config(&s);
        assert!((c.quality_scale - 1.5).abs() < f64::EPSILON);
        assert_eq!(c.qp_floor, Some(16));
        assert!(c.audio.mic && c.audio.system);
    }
}
