//! Containers: which muxer a file extension means, which codecs a container can hold,
//! and where a copied audio stream goes ([`audio_copy_container`]). Pure tables (no
//! library needed, so [`crate::transcode::plan`] and its unit tests can reason without
//! one); the FFmpeg side (`sys::mux::Muxer`, `sys::encode::Encoder`) is re-exported
//! here for `transcode`.

pub(crate) use crate::sys::encode::{AudioConv, EncSpec, Encoder, Got, OwnedPar, PixConv};
pub(crate) use crate::sys::mux::{
    Muxer, OutChapter, OutStream, input_chapters, output_format, pkt_clear_pos, pkt_rescale_ts, pkt_set_stream_index,
    pkt_set_ts, rescale,
};

/// The muxer for a file extension (without the dot, any case), for the containers the
/// media tools write.
pub fn container_for_ext(ext: &str) -> Option<&'static str> {
    Some(match ext.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => "mp4",
        "m4a" | "m4b" => "ipod",
        "mov" => "mov",
        "mkv" | "mka" | "mks" => "matroska",
        "webm" => "webm",
        "avi" => "avi",
        "ogg" | "oga" | "ogv" | "opus" => "ogg",
        "flac" => "flac",
        "wav" => "wav",
        "mp3" => "mp3",
        "mp2" => "mp2",
        "aac" => "adts",
        "ac3" => "ac3",
        "eac3" => "eac3",
        "dts" => "dts",
        "caf" => "caf",
        "ts" | "m2ts" | "mts" => "mpegts",
        "wma" | "asf" | "wmv" => "asf",
        _ => return None,
    })
}

/// The usual extension of a muxer's files (without the dot).
pub fn ext_for_container(container: &str) -> &str {
    match container {
        "matroska" => "mkv",
        "ipod" => "m4a",
        "adts" => "aac",
        "mpegts" => "ts",
        "asf" => "wma",
        other => other,
    }
}

/// Whether `container` (a muxer name) can hold a stream of `codec` (an FFmpeg codec
/// name). Matroska takes everything; the others follow their specifications and what
/// FFmpeg's muxers accept without `-strict experimental`.
pub fn container_accepts(container: &str, codec: &str) -> bool {
    if codec.is_empty() {
        return false;
    }
    let pcm = codec.starts_with("pcm_");
    let mp4_video = matches!(
        codec,
        "h264"
            | "hevc"
            | "mpeg4"
            | "av1"
            | "vp9"
            | "mjpeg"
            | "h263"
            | "mpeg2video"
            | "mpeg1video"
            | "png"
            | "jpeg2000"
            | "vc1"
            | "vvc"
    );
    let mp4_audio = matches!(codec, "aac" | "alac" | "ac3" | "eac3" | "mp3" | "mp2" | "opus" | "dts" | "mp1");
    match container {
        "matroska" => true,
        "webm" => matches!(codec, "vp8" | "vp9" | "av1" | "opus" | "vorbis" | "webvtt"),
        "mp4" => mp4_video || mp4_audio || codec == "mov_text",
        "ipod" => matches!(codec, "h264" | "hevc" | "mpeg4" | "aac" | "alac" | "ac3" | "mp3" | "mov_text"),
        "mov" => {
            mp4_video
                || mp4_audio
                || pcm
                || matches!(
                    codec,
                    "prores" | "dnxhd" | "rawvideo" | "qtrle" | "cinepak" | "dvvideo" | "flac" | "mov_text" | "eia_608"
                )
        }
        "avi" => {
            matches!(
                codec,
                "mjpeg"
                    | "mpeg4"
                    | "msmpeg4v3"
                    | "msmpeg4v2"
                    | "h264"
                    | "huffyuv"
                    | "ffv1"
                    | "rawvideo"
                    | "dvvideo"
                    | "mpeg2video"
                    | "mpeg1video"
                    | "wmv1"
                    | "wmv2"
                    | "h263"
                    | "mp3"
                    | "mp2"
                    | "ac3"
                    | "aac"
                    | "wmav1"
                    | "wmav2"
                    | "adpcm_ima_wav"
                    | "adpcm_ms"
            ) || matches!(codec, "pcm_s16le" | "pcm_u8" | "pcm_s24le" | "pcm_s32le")
        }
        "ogg" => matches!(codec, "theora" | "vorbis" | "opus" | "flac" | "speex" | "vp8"),
        "flac" => codec == "flac",
        "wav" => pcm || matches!(codec, "adpcm_ima_wav" | "adpcm_ms" | "mp3"),
        "mp3" => codec == "mp3",
        "mp2" => codec == "mp2",
        "adts" => codec == "aac",
        "ac3" => codec == "ac3",
        "eac3" => codec == "eac3",
        "dts" => codec == "dts",
        "caf" => pcm || matches!(codec, "alac" | "aac" | "opus" | "ac3" | "mp3" | "adpcm_ima_qt"),
        "mpegts" => matches!(
            codec,
            "h264" | "hevc" | "mpeg2video" | "mpeg4" | "aac" | "ac3" | "eac3" | "mp2" | "mp3" | "opus" | "dts"
        ),
        "asf" => matches!(codec, "wmv1" | "wmv2" | "wmv3" | "vc1" | "wmav1" | "wmav2" | "wmapro" | "mp3"),
        _ => false,
    }
}

/// Whether `container` writes chapters (MP4 / MOV chapter lists, Matroska / WebM
/// `Chapters`, Ogg / FLAC `CHAPTERxx` comments, MP3 ID3v2 `CHAP` frames). The others
/// (AVI, WAV, MPEG-TS, raw audio, ASF, CAF) have no place for them.
pub fn container_keeps_chapters(container: &str) -> bool {
    matches!(container, "mp4" | "ipod" | "mov" | "matroska" | "webm" | "ogg" | "flac" | "mp3")
}

/// Whether `container` stores a video stream's rotation (the display matrix): the MP4 /
/// MOV track matrix and the Matroska / WebM projection pose. AVI, MPEG-TS, Ogg and the
/// rest have no field for it (the picture is kept as coded, unrotated).
pub fn container_keeps_rotation(container: &str) -> bool {
    matches!(container, "mp4" | "ipod" | "mov" | "matroska" | "webm")
}

/// Whether `container` can carry cover art (an attached picture) coded as `codec`:
/// MP4 / M4A / MOV (`covr`), MP3 (ID3v2 `APIC`), FLAC (`PICTURE` blocks) and Matroska
/// (attachments). Ogg / Opus, WAV, WebM and the rest cannot (FFmpeg's Ogg muxer writes
/// no `METADATA_BLOCK_PICTURE`), so the picture is left out there.
pub fn container_holds_cover(container: &str, codec: &str) -> bool {
    match container {
        "mp4" | "ipod" | "mov" => matches!(codec, "mjpeg" | "png" | "bmp"),
        "mp3" | "flac" | "matroska" => matches!(codec, "mjpeg" | "png" | "bmp" | "gif"),
        _ => false,
    }
}

/// Where an audio stream of `codec` goes when it is copied without re-encoding:
/// `(muxer, extension)`. Matroska (`.mka`) holds whatever has no native container.
pub fn audio_copy_container(codec: &str) -> (&'static str, &'static str) {
    match codec {
        "aac" | "alac" => ("ipod", "m4a"),
        "mp3" => ("mp3", "mp3"),
        "mp2" => ("mp2", "mp2"),
        "opus" => ("ogg", "opus"),
        "vorbis" => ("ogg", "ogg"),
        "flac" => ("flac", "flac"),
        "pcm_s16le" | "pcm_s24le" | "pcm_s32le" | "pcm_u8" | "pcm_f32le" | "pcm_f64le" => ("wav", "wav"),
        "ac3" => ("ac3", "ac3"),
        "eac3" => ("eac3", "eac3"),
        "dts" => ("dts", "dts"),
        "wmav1" | "wmav2" | "wmapro" => ("asf", "wma"),
        _ => ("matroska", "mka"),
    }
}

/// The muxer to write a trimmed copy of an input into: the destination's extension
/// when it names a container, else the input's own format (`"mov,mp4,m4a,3gp,3g2,mj2"`
/// → `mp4`, `"matroska,webm"` → `matroska`).
pub fn container_for_trim(input_format: &str, dest_ext: Option<&str>) -> Option<&'static str> {
    if let Some(c) = dest_ext.and_then(container_for_ext) {
        return Some(c);
    }
    let first = input_format.split(',').next().unwrap_or("");
    Some(match first {
        "mov" => "mp4",
        "matroska" => "matroska",
        "avi" => "avi",
        "webm" => "webm",
        "ogg" => "ogg",
        "flac" => "flac",
        "wav" => "wav",
        "mp3" => "mp3",
        "mpegts" => "mpegts",
        "asf" => "asf",
        "aac" => "adts",
        "ac3" => "ac3",
        "eac3" => "eac3",
        "dts" => "dts",
        "caf" => "caf",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_and_containers_round_trip() {
        for ext in ["mp4", "m4a", "mov", "mkv", "webm", "avi", "ogg", "opus", "flac", "wav", "mp3"] {
            let c = container_for_ext(ext).unwrap_or_else(|| panic!("{ext}"));
            // The usual extension maps back to the same muxer.
            assert_eq!(container_for_ext(ext_for_container(c)), Some(c), "{ext} -> {c}");
        }
        assert_eq!(container_for_ext("MKV"), Some("matroska"));
        assert_eq!(container_for_ext("xyz"), None);
        assert_eq!(ext_for_container("matroska"), "mkv");
        assert_eq!(ext_for_container("ipod"), "m4a");
        assert_eq!(ext_for_container("webm"), "webm");
    }

    #[test]
    fn container_compatibility() {
        assert!(container_accepts("matroska", "h264") && container_accepts("matroska", "pcm_s24be"));
        assert!(!container_accepts("matroska", ""));
        assert!(
            container_accepts("mp4", "h264") && container_accepts("mp4", "aac") && container_accepts("mp4", "mjpeg")
        );
        assert!(!container_accepts("mp4", "prores") && !container_accepts("mp4", "pcm_s16le"));
        assert!(!container_accepts("mp4", "flac"), "FLAC in MP4 needs -strict experimental");
        assert!(container_accepts("mov", "prores") && container_accepts("mov", "pcm_s16le"));
        assert!(container_accepts("webm", "vp9") && container_accepts("webm", "opus"));
        assert!(!container_accepts("webm", "h264") && !container_accepts("webm", "aac"));
        assert!(
            container_accepts("avi", "mjpeg")
                && container_accepts("avi", "pcm_s16le")
                && container_accepts("avi", "mp2")
        );
        assert!(!container_accepts("avi", "hevc"));
        assert!(container_accepts("ogg", "opus") && !container_accepts("ogg", "aac"));
        assert!(container_accepts("wav", "pcm_f32le") && !container_accepts("wav", "aac"));
        assert!(container_accepts("ipod", "alac") && !container_accepts("ipod", "opus"));
        assert!(!container_accepts("no-such-muxer", "aac"));
    }

    #[test]
    fn audio_copy_targets() {
        assert_eq!(audio_copy_container("aac"), ("ipod", "m4a"));
        assert_eq!(audio_copy_container("opus"), ("ogg", "opus"));
        assert_eq!(audio_copy_container("pcm_s16le"), ("wav", "wav"));
        assert_eq!(audio_copy_container("pcm_s16be"), ("matroska", "mka"));
        assert_eq!(audio_copy_container("truehd"), ("matroska", "mka"));
        // Every target accepts the codec it was chosen for.
        for codec in ["aac", "alac", "mp3", "opus", "vorbis", "flac", "pcm_s16le", "ac3", "wmav2", "ape"] {
            let (c, _) = audio_copy_container(codec);
            assert!(container_accepts(c, codec), "{codec} -> {c}");
        }
    }

    #[test]
    fn chapters_rotation_and_cover_slots() {
        for c in ["mp4", "ipod", "mov", "matroska", "webm", "ogg", "flac", "mp3"] {
            assert!(container_keeps_chapters(c), "{c}");
        }
        for c in ["avi", "wav", "mpegts", "adts", "asf", "caf"] {
            assert!(!container_keeps_chapters(c), "{c}");
        }
        assert!(container_keeps_rotation("webm") && container_keeps_rotation("mov"));
        assert!(!container_keeps_rotation("avi") && !container_keeps_rotation("mpegts"));
        assert!(container_holds_cover("ipod", "png") && container_holds_cover("flac", "mjpeg"));
        assert!(container_holds_cover("matroska", "gif") && !container_holds_cover("mp4", "gif"));
        assert!(!container_holds_cover("ogg", "png") && !container_holds_cover("wav", "png"));
    }

    #[test]
    fn trim_container_follows_the_input() {
        assert_eq!(container_for_trim("mov,mp4,m4a,3gp,3g2,mj2", None), Some("mp4"));
        assert_eq!(container_for_trim("matroska,webm", None), Some("matroska"));
        assert_eq!(container_for_trim("matroska,webm", Some("webm")), Some("webm"));
        assert_eq!(
            container_for_trim("avi", Some("part")),
            Some("avi"),
            "an unknown extension keeps the input's format"
        );
        assert_eq!(container_for_trim("rm", None), None);
    }
}
