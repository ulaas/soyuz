//! Built-in conversion presets (the Convert dialog, `JobKind::Media`). User presets are a
//! later phase; the built-ins are filtered by what the loaded build can encode
//! ([`presets`]): FFmpeg's own encoders, libopus and libvpx, and the hardware H.264 /
//! HEVC encoders of the machine (probed by opening them once, `Runtime::has_encoder`).
//! Software H.264 / HEVC is impossible under the LGPL (x264 / x265 are GPL).

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
    /// HEVC in MP4 through the platform's hardware encoder.
    Mp4HevcHw,
    /// VP9 + Opus in WebM (libvpx, libopus).
    WebmVp9,
    /// VP8 + Opus in WebM.
    WebmVp8,
    /// ProRes (`prores_ks`) + PCM in MOV.
    MovProres,
    /// MPEG-4 part 2 + AAC in MP4 (software, plays everywhere).
    Mp4Mpeg4,
    /// MPEG-4 part 2 + MP2 in AVI.
    AviMpeg4,
    AudioFlac,
    AudioAac,
    AudioOpus,
    AudioWav,
    AudioAlac,
}

impl PresetId {
    /// Serialized name (`"mp4-h264-hw"`), stable once shipped (job history, config).
    pub fn name(self) -> &'static str {
        match self {
            PresetId::MkvCopy => "mkv-copy",
            PresetId::Mp4Copy => "mp4-copy",
            PresetId::Mp4H264Hw => "mp4-h264-hw",
            PresetId::Mp4HevcHw => "mp4-hevc-hw",
            PresetId::WebmVp9 => "webm-vp9",
            PresetId::WebmVp8 => "webm-vp8",
            PresetId::MovProres => "mov-prores",
            PresetId::Mp4Mpeg4 => "mp4-mpeg4",
            PresetId::AviMpeg4 => "avi-mpeg4",
            PresetId::AudioFlac => "audio-flac",
            PresetId::AudioAac => "audio-aac",
            PresetId::AudioOpus => "audio-opus",
            PresetId::AudioWav => "audio-wav",
            PresetId::AudioAlac => "audio-alac",
        }
    }

    pub fn parse(name: &str) -> Option<PresetId> {
        BUILTIN.iter().map(|p| p.id).find(|id| id.name() == name)
    }

    /// The built-in table row.
    pub fn preset(self) -> &'static Preset {
        BUILTIN.iter().find(|p| p.id == self).expect("every id is in BUILTIN")
    }
}

/// A video encoder choice: one or more encoder names (first available wins) and the
/// options set on it (`av_opt_set`, codec-private options included).
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

impl Preset {
    /// Both encoder choices are `"copy"`: a remux, every stream the container accepts
    /// is kept (not only one video and one audio stream).
    pub fn is_copy(&self) -> bool {
        self.video.is_some_and(|v| v.encoders == COPY) && self.audio.is_some_and(|a| a.encoders == COPY)
    }
}

const COPY: &[&str] = &["copy"];
const H264_HW: &[&str] = &["h264_nvenc", "h264_amf", "h264_qsv", "h264_videotoolbox", "h264_vaapi"];
const HEVC_HW: &[&str] = &["hevc_nvenc", "hevc_amf", "hevc_qsv", "hevc_videotoolbox", "hevc_vaapi"];
/// `-q:v 4` for the MPEG-4 part 2 encoder (`global_quality` is in lambda units,
/// `FF_QP2LAMBDA` = 118).
const MPEG4_OPTS: &[(&str, &str)] = &[("flags", "+qscale"), ("global_quality", "472")];

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
        video: Some(VideoEnc { encoders: H264_HW, options: &[] }),
        audio: Some(AudioEnc { encoders: &["aac"], options: &[("b", "192k")] }),
        ext: "mp4",
    },
    Preset {
        id: PresetId::Mp4HevcHw,
        label: "MP4 / HEVC (hardware encoder)",
        container: "mp4",
        video: Some(VideoEnc { encoders: HEVC_HW, options: &[] }),
        audio: Some(AudioEnc { encoders: &["aac"], options: &[("b", "192k")] }),
        ext: "mp4",
    },
    Preset {
        id: PresetId::WebmVp9,
        label: "WebM / VP9 + Opus",
        container: "webm",
        video: Some(VideoEnc {
            encoders: &["libvpx-vp9"],
            options: &[("crf", "32"), ("b", "0"), ("row-mt", "1"), ("cpu-used", "2")],
        }),
        audio: Some(AudioEnc { encoders: &["libopus"], options: &[("b", "128k")] }),
        ext: "webm",
    },
    Preset {
        id: PresetId::WebmVp8,
        label: "WebM / VP8 + Opus",
        container: "webm",
        video: Some(VideoEnc { encoders: &["libvpx"], options: &[("crf", "10"), ("b", "2M"), ("cpu-used", "2")] }),
        audio: Some(AudioEnc { encoders: &["libopus"], options: &[("b", "128k")] }),
        ext: "webm",
    },
    Preset {
        id: PresetId::MovProres,
        label: "MOV / ProRes HQ",
        container: "mov",
        video: Some(VideoEnc { encoders: &["prores_ks"], options: &[("profile", "3")] }),
        audio: Some(AudioEnc { encoders: &["pcm_s16le"], options: &[] }),
        ext: "mov",
    },
    Preset {
        id: PresetId::Mp4Mpeg4,
        label: "MP4 / MPEG-4 + AAC",
        container: "mp4",
        video: Some(VideoEnc { encoders: &["mpeg4"], options: MPEG4_OPTS }),
        audio: Some(AudioEnc { encoders: &["aac"], options: &[("b", "160k")] }),
        ext: "mp4",
    },
    Preset {
        id: PresetId::AviMpeg4,
        label: "AVI / MPEG-4 + MP2",
        container: "avi",
        video: Some(VideoEnc { encoders: &["mpeg4"], options: MPEG4_OPTS }),
        audio: Some(AudioEnc { encoders: &["mp2", "pcm_s16le"], options: &[("b", "160k")] }),
        ext: "avi",
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
    Preset {
        id: PresetId::AudioAlac,
        label: "Apple Lossless (M4A)",
        container: "ipod",
        video: None,
        audio: Some(AudioEnc { encoders: &["alac"], options: &[] }),
        ext: "m4a",
    },
];

/// Quality settings for the hardware encoders (their rate-control options differ per
/// vendor; without them most default to a low constant bitrate). Applied after the
/// preset's own options.
pub fn hw_options(encoder: &str) -> &'static [(&'static str, &'static str)] {
    match encoder {
        "h264_nvenc" | "hevc_nvenc" => &[("rc", "vbr"), ("cq", "23"), ("b", "0"), ("preset", "p5")],
        "h264_amf" | "hevc_amf" => &[("rc", "cqp"), ("qp_i", "23"), ("qp_p", "23"), ("quality", "quality")],
        "h264_qsv" | "hevc_qsv" => &[("global_quality", "23"), ("preset", "medium")],
        "h264_videotoolbox" | "hevc_videotoolbox" => &[("b", "6M")],
        "h264_vaapi" | "hevc_vaapi" => &[("rc_mode", "CQP"), ("qp", "23")],
        _ => &[],
    }
}

/// Whether `has_encoder` satisfies every encoder choice of `p` (`"copy"` always does).
pub fn available(p: &Preset, has_encoder: &dyn Fn(&str) -> bool) -> bool {
    let ok = |encoders: &[&str]| encoders.iter().any(|e| *e == "copy" || has_encoder(e));
    p.video.is_none_or(|v| ok(v.encoders)) && p.audio.is_none_or(|a| ok(a.encoders))
}

/// The first encoder of `encoders` that `has_encoder` accepts (`"copy"` included).
pub fn pick_encoder<'a>(encoders: &[&'a str], has_encoder: &dyn Fn(&str) -> bool) -> Option<&'a str> {
    encoders.iter().copied().find(|e| *e == "copy" || has_encoder(e))
}

/// The built-in presets whose encoders exist (and open) in this build, in menu order.
/// Hardware encoders are probed by opening them once (`Runtime::has_encoder`).
pub fn presets(rt: &Runtime) -> Vec<Preset> {
    BUILTIN.iter().filter(|p| available(p, &|name| rt.has_encoder(name))).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::container_accepts;

    #[test]
    fn names_round_trip_and_are_unique() {
        for p in BUILTIN {
            assert_eq!(PresetId::parse(p.id.name()), Some(p.id));
            assert!(!p.ext.starts_with('.'));
            assert_eq!(p.id.preset().label, p.label);
        }
        let mut names: Vec<&str> = BUILTIN.iter().map(|p| p.id.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), BUILTIN.len());
        assert_eq!(BUILTIN.len(), 14);
        assert_eq!(PresetId::parse("x264"), None);
        for name in [
            "mkv-copy",
            "mp4-copy",
            "mp4-h264-hw",
            "mp4-hevc-hw",
            "webm-vp9",
            "webm-vp8",
            "mov-prores",
            "avi-mpeg4",
            "audio-flac",
            "audio-aac",
            "audio-opus",
            "audio-wav",
            "audio-alac",
        ] {
            assert!(PresetId::parse(name).is_some(), "{name}");
        }
    }

    #[test]
    fn availability_follows_the_encoders() {
        let none = |_: &str| false;
        let copy_only: Vec<PresetId> = BUILTIN.iter().filter(|p| available(p, &none)).map(|p| p.id).collect();
        assert_eq!(copy_only, [PresetId::MkvCopy, PresetId::Mp4Copy]);
        assert!(PresetId::MkvCopy.preset().is_copy() && !PresetId::WebmVp9.preset().is_copy());
        let native = |n: &str| matches!(n, "aac" | "flac" | "alac" | "pcm_s16le" | "mpeg4" | "prores_ks");
        let with_native: Vec<PresetId> = BUILTIN.iter().filter(|p| available(p, &native)).map(|p| p.id).collect();
        for id in [PresetId::Mp4Mpeg4, PresetId::AviMpeg4, PresetId::MovProres, PresetId::AudioWav, PresetId::AudioAlac]
        {
            assert!(with_native.contains(&id), "{id:?}");
        }
        assert!(!with_native.contains(&PresetId::Mp4H264Hw), "needs a hardware encoder");
        assert!(!with_native.contains(&PresetId::WebmVp9), "needs libvpx and libopus");
        let hw = |n: &str| n == "h264_amf" || n == "aac";
        assert!(available(PresetId::Mp4H264Hw.preset(), &hw), "any one hardware encoder is enough");
        assert_eq!(pick_encoder(H264_HW, &hw), Some("h264_amf"));
        assert_eq!(pick_encoder(COPY, &none), Some("copy"));
        assert_eq!(pick_encoder(&["mp2", "pcm_s16le"], &native), Some("pcm_s16le"), "AVI audio falls back to PCM");
        assert!(!hw_options("h264_nvenc").is_empty() && hw_options("mpeg4").is_empty());
    }

    /// Every preset's container can hold what its encoders produce.
    #[test]
    fn preset_codecs_fit_their_containers() {
        let codec_of = |encoder: &'static str| -> &'static str {
            match encoder {
                "h264_nvenc" | "h264_amf" | "h264_qsv" | "h264_videotoolbox" | "h264_vaapi" => "h264",
                "hevc_nvenc" | "hevc_amf" | "hevc_qsv" | "hevc_videotoolbox" | "hevc_vaapi" => "hevc",
                "libvpx-vp9" => "vp9",
                "libvpx" => "vp8",
                "prores_ks" => "prores",
                "libopus" => "opus",
                other => other,
            }
        };
        for p in BUILTIN {
            for enc in p.video.iter().flat_map(|v| v.encoders).chain(p.audio.iter().flat_map(|a| a.encoders)) {
                if *enc == "copy" {
                    continue;
                }
                assert!(container_accepts(p.container, codec_of(enc)), "{}: {enc} in {}", p.id.name(), p.container);
            }
            assert_eq!(crate::mux::container_for_ext(p.ext), Some(p.container), "{}", p.id.name());
        }
    }
}
