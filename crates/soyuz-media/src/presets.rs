//! Built-in conversion presets (the Convert dialog, `JobKind::Media`). User presets are a
//! later phase; the built-ins are filtered by what the loaded build can encode.

use crate::Runtime;

/// The built-in presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PresetId {
    /// Remux every stream into Matroska.
    MkvCopy,
    /// Remux into MP4 (streams the container accepts).
    Mp4Copy,
    /// H.264 in MP4 through the platform's hardware encoder (nvenc / AMF / QSV /
    /// VideoToolbox / VAAPI); no software H.264 under the LGPL.
    Mp4H264Hw,
    /// VP9 + Opus in WebM (libvpx, libopus).
    WebmVp9,
    /// ProRes (`prores_ks`) in MOV.
    Prores,
    /// MPEG-4 part 2 + AAC in MP4 (small, plays everywhere).
    Mpeg4,
    AudioFlac,
    AudioAac,
    AudioOpus,
    AudioWav,
}

impl PresetId {
    /// Serialized name (`"mp4-h264-hw"`), stable once shipped (job history, config).
    pub fn name(self) -> &'static str {
        match self {
            PresetId::MkvCopy => "mkv-copy",
            PresetId::Mp4Copy => "mp4-copy",
            PresetId::Mp4H264Hw => "mp4-h264-hw",
            PresetId::WebmVp9 => "webm-vp9",
            PresetId::Prores => "prores",
            PresetId::Mpeg4 => "mpeg4",
            PresetId::AudioFlac => "audio-flac",
            PresetId::AudioAac => "audio-aac",
            PresetId::AudioOpus => "audio-opus",
            PresetId::AudioWav => "audio-wav",
        }
    }

    pub fn parse(name: &str) -> Option<PresetId> {
        BUILTIN.iter().map(|p| p.id).find(|id| id.name() == name)
    }
}

/// A video encoder choice: one or more encoder names (first available wins) and the
/// options set on it (`av_opt_set`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoEnc {
    /// Encoder names in preference order; `"copy"` = stream copy.
    pub encoders: &'static [&'static str],
    pub options: &'static [(&'static str, &'static str)],
}

/// An audio encoder choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioEnc {
    pub encoders: &'static [&'static str],
    pub options: &'static [(&'static str, &'static str)],
}

/// One preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    pub id: PresetId,
    /// English label; the frontends translate by `id`.
    pub label: &'static str,
    /// Muxer name (`"matroska"`, `"mp4"`).
    pub container: &'static str,
    /// `None` = drop video (audio-only presets).
    pub video: Option<VideoEnc>,
    /// `None` = drop audio.
    pub audio: Option<AudioEnc>,
    /// Output extension without the dot.
    pub ext: &'static str,
}

const COPY: &[&str] = &["copy"];

/// The built-in table, in menu order.
pub const BUILTIN: &[Preset] = &[
    Preset {
        id: PresetId::MkvCopy,
        label: "Matroska (copy streams)",
        container: "matroska",
        video: Some(VideoEnc { encoders: COPY, options: &[] }),
        audio: Some(AudioEnc { encoders: COPY, options: &[] }),
        ext: "mkv",
    },
    Preset {
        id: PresetId::Mp4Copy,
        label: "MP4 (copy streams)",
        container: "mp4",
        video: Some(VideoEnc { encoders: COPY, options: &[] }),
        audio: Some(AudioEnc { encoders: COPY, options: &[] }),
        ext: "mp4",
    },
    Preset {
        id: PresetId::Mp4H264Hw,
        label: "MP4 / H.264 (hardware encoder)",
        container: "mp4",
        video: Some(VideoEnc {
            encoders: &["h264_nvenc", "h264_amf", "h264_qsv", "h264_videotoolbox", "h264_vaapi"],
            options: &[],
        }),
        audio: Some(AudioEnc { encoders: &["aac"], options: &[("b", "192k")] }),
        ext: "mp4",
    },
    Preset {
        id: PresetId::WebmVp9,
        label: "WebM / VP9 + Opus",
        container: "webm",
        video: Some(VideoEnc { encoders: &["libvpx-vp9"], options: &[("crf", "32"), ("b", "0"), ("row-mt", "1")] }),
        audio: Some(AudioEnc { encoders: &["libopus"], options: &[("b", "128k")] }),
        ext: "webm",
    },
    Preset {
        id: PresetId::Prores,
        label: "MOV / ProRes",
        container: "mov",
        video: Some(VideoEnc { encoders: &["prores_ks"], options: &[("profile", "3")] }),
        audio: Some(AudioEnc { encoders: &["pcm_s16le"], options: &[] }),
        ext: "mov",
    },
    Preset {
        id: PresetId::Mpeg4,
        label: "MP4 / MPEG-4 + AAC",
        container: "mp4",
        video: Some(VideoEnc { encoders: &["mpeg4"], options: &[("q", "5")] }),
        audio: Some(AudioEnc { encoders: &["aac"], options: &[("b", "160k")] }),
        ext: "mp4",
    },
    Preset {
        id: PresetId::AudioFlac,
        label: "FLAC",
        container: "flac",
        video: None,
        audio: Some(AudioEnc { encoders: &["flac"], options: &[] }),
        ext: "flac",
    },
    Preset {
        id: PresetId::AudioAac,
        label: "AAC (M4A)",
        container: "ipod",
        video: None,
        audio: Some(AudioEnc { encoders: &["aac"], options: &[("b", "192k")] }),
        ext: "m4a",
    },
    Preset {
        id: PresetId::AudioOpus,
        label: "Opus",
        container: "ogg",
        video: None,
        audio: Some(AudioEnc { encoders: &["libopus"], options: &[("b", "128k")] }),
        ext: "opus",
    },
    Preset {
        id: PresetId::AudioWav,
        label: "WAV (16-bit PCM)",
        container: "wav",
        video: None,
        audio: Some(AudioEnc { encoders: &["pcm_s16le"], options: &[] }),
        ext: "wav",
    },
];

/// Whether `has_encoder` satisfies every encoder choice of `p` (`"copy"` always does).
pub fn available(p: &Preset, has_encoder: &dyn Fn(&str) -> bool) -> bool {
    let ok = |encoders: &[&str]| encoders.iter().any(|e| *e == "copy" || has_encoder(e));
    p.video.is_none_or(|v| ok(v.encoders)) && p.audio.is_none_or(|a| ok(a.encoders))
}

/// The built-in presets whose encoders exist (and open) in this build, in menu order.
/// Hardware encoders are probed by opening them once (`Runtime::has_encoder`).
pub fn presets(rt: &Runtime) -> Vec<Preset> {
    BUILTIN.iter().filter(|p| available(p, &|name| rt.has_encoder(name))).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_are_unique() {
        for p in BUILTIN {
            assert_eq!(PresetId::parse(p.id.name()), Some(p.id));
            assert!(!p.ext.starts_with('.'));
        }
        let mut names: Vec<&str> = BUILTIN.iter().map(|p| p.id.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), BUILTIN.len());
        assert_eq!(PresetId::parse("x264"), None);
    }

    #[test]
    fn availability_follows_the_encoders() {
        let none = |_: &str| false;
        let copy_only: Vec<PresetId> = BUILTIN.iter().filter(|p| available(p, &none)).map(|p| p.id).collect();
        assert_eq!(copy_only, [PresetId::MkvCopy, PresetId::Mp4Copy]);
        let native = |n: &str| matches!(n, "aac" | "flac" | "pcm_s16le" | "mpeg4" | "prores_ks");
        let with_native: Vec<PresetId> = BUILTIN.iter().filter(|p| available(p, &native)).map(|p| p.id).collect();
        assert!(with_native.contains(&PresetId::Mpeg4) && with_native.contains(&PresetId::AudioWav));
        assert!(!with_native.contains(&PresetId::Mp4H264Hw), "needs a hardware encoder");
        assert!(!with_native.contains(&PresetId::WebmVp9), "needs libvpx and libopus");
        let hw = |n: &str| n == "h264_amf" || n == "aac";
        assert!(available(&BUILTIN[2], &hw), "any one hardware encoder is enough");
    }
}
