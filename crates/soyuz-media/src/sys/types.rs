//! Constants and opaque handle types of the FFmpeg API (values verified against the
//! 7.1 / 8.1 / 9.0 headers; FFmpeg only appends to its enums, so these are stable).

use std::ffi::{c_int, c_void};

/// Declares opaque C types we only ever hold pointers to.
macro_rules! opaque {
    ($($name:ident),* $(,)?) => {
        $(
            #[repr(C)]
            pub(crate) struct $name {
                _private: [u8; 0],
            }
        )*
    };
}

opaque!(
    AVClass,
    AVCodec,
    AVCodecContext,
    AVDictionary,
    AVBuffer,
    AVOutputFormat,
    AVProgram,
    AVStreamGroup,
    AVIOContext
);
opaque!(SwsContext, SwsFilter, SwrContext, AVProfile);

/// `AVMediaType`.
pub(crate) const AVMEDIA_TYPE_VIDEO: c_int = 0;
pub(crate) const AVMEDIA_TYPE_AUDIO: c_int = 1;
pub(crate) const AVMEDIA_TYPE_DATA: c_int = 2;
pub(crate) const AVMEDIA_TYPE_SUBTITLE: c_int = 3;
pub(crate) const AVMEDIA_TYPE_ATTACHMENT: c_int = 4;

/// `AVCodecID` values the self-test expects.
pub(crate) const AV_CODEC_ID_RAWVIDEO: c_int = 13;
pub(crate) const AV_CODEC_ID_PCM_S16LE: c_int = 0x10000;

/// `AVPixelFormat` / `AVSampleFormat`: only the "none" sentinel is hard-coded; real
/// formats are looked up by name (`av_get_pix_fmt("rgba")`) so no table can drift.
pub(crate) const AV_PIX_FMT_NONE: c_int = -1;
pub(crate) const AV_SAMPLE_FMT_NONE: c_int = -1;

/// `AVColorTransferCharacteristic` values that mean HDR.
pub(crate) const AVCOL_TRC_SMPTE2084: c_int = 16;
pub(crate) const AVCOL_TRC_ARIB_STD_B67: c_int = 18;
/// "Unspecified" is 0 for `AVColorRange` and 2 for primaries, transfer and matrix.
pub(crate) const AVCOL_RANGE_UNSPECIFIED: c_int = 0;
pub(crate) const AVCOL_UNSPECIFIED: c_int = 2;

/// `AVPacketSideDataType`.
pub(crate) const AV_PKT_DATA_DISPLAYMATRIX: c_int = 5;
pub(crate) const AV_PKT_DATA_MASTERING_DISPLAY_METADATA: c_int = 20;
pub(crate) const AV_PKT_DATA_CONTENT_LIGHT_LEVEL: c_int = 22;
pub(crate) const AV_PKT_DATA_DOVI_CONF: c_int = 29;
pub(crate) const AV_PKT_DATA_DYNAMIC_HDR10_PLUS: c_int = 31;

/// `AV_DISPOSITION_*`.
pub(crate) const AV_DISPOSITION_DEFAULT: c_int = 1 << 0;
pub(crate) const AV_DISPOSITION_FORCED: c_int = 1 << 6;
pub(crate) const AV_DISPOSITION_ATTACHED_PIC: c_int = 1 << 10;

/// `AV_PKT_FLAG_KEY`.
pub(crate) const AV_PKT_FLAG_KEY: c_int = 1;

/// `AVSEEK_FLAG_*` of `av_seek_frame` / `avformat_seek_file`.
pub(crate) const AVSEEK_FLAG_BACKWARD: c_int = 1;
/// `AVSEEK_SIZE` / `AVSEEK_FORCE`: the `whence` extensions of the AVIO seek callback.
pub(crate) const AVSEEK_SIZE: c_int = 0x10000;
pub(crate) const AVSEEK_FORCE: c_int = 0x20000;
pub(crate) const SEEK_SET: c_int = 0;
pub(crate) const SEEK_CUR: c_int = 1;
pub(crate) const SEEK_END: c_int = 2;

/// `AV_LOG_*`.
pub(crate) const AV_LOG_QUIET: c_int = -8;
pub(crate) const AV_LOG_ERROR: c_int = 16;
pub(crate) const AV_LOG_WARNING: c_int = 24;

/// `AV_DICT_IGNORE_SUFFIX`: iterate every entry with an empty key.
pub(crate) const AV_DICT_IGNORE_SUFFIX: c_int = 2;

/// `AV_NOPTS_VALUE` and `AV_TIME_BASE`.
pub(crate) const AV_NOPTS_VALUE: i64 = i64::MIN;
pub(crate) const AV_TIME_BASE: i64 = 1_000_000;

/// `AVCodecConfig` of `avcodec_get_supported_config`.
pub(crate) const AV_CODEC_CONFIG_PIX_FORMAT: c_int = 0;
pub(crate) const AV_CODEC_CONFIG_SAMPLE_FORMAT: c_int = 3;

/// `AVHWDeviceType` (hardware decoding, V4b).
pub(crate) const AV_HWDEVICE_TYPE_VAAPI: c_int = 3;
pub(crate) const AV_HWDEVICE_TYPE_VIDEOTOOLBOX: c_int = 6;
pub(crate) const AV_HWDEVICE_TYPE_D3D11VA: c_int = 7;
/// `AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX`: the decoder accepts `hw_device_ctx`.
pub(crate) const AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX: c_int = 1;

/// `SWS_*` scaler flags.
pub(crate) const SWS_FAST_BILINEAR: c_int = 1;
pub(crate) const SWS_BILINEAR: c_int = 2;
pub(crate) const SWS_AREA: c_int = 0x20;

/// `FFERRTAG(a, b, c, d)`: `-MKTAG(a, b, c, d)`.
const fn fferrtag(a: u8, b: u8, c: u8, d: u8) -> c_int {
    -((a as c_int) | ((b as c_int) << 8) | ((c as c_int) << 16) | ((d as c_int) << 24))
}

pub(crate) const AVERROR_EOF: c_int = fferrtag(b'E', b'O', b'F', b' ');
pub(crate) const AVERROR_EXIT: c_int = fferrtag(b'E', b'X', b'I', b'T');
pub(crate) const AVERROR_EXTERNAL: c_int = fferrtag(b'E', b'X', b'T', b' ');
pub(crate) const AVERROR_INVALIDDATA: c_int = fferrtag(b'I', b'N', b'D', b'A');
/// `AVERROR(EAGAIN)`: FFmpeg uses the platform's errno (35 on macOS, 11 elsewhere).
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
pub(crate) const AVERROR_EAGAIN: c_int = -35;
#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "freebsd")))]
pub(crate) const AVERROR_EAGAIN: c_int = -11;
pub(crate) const AVERROR_ENOMEM: c_int = -12;
pub(crate) const AVERROR_EINVAL: c_int = -22;

/// The AVIO callback signatures (`avio_alloc_context`).
pub(crate) type ReadPacketFn = unsafe extern "C" fn(opaque: *mut c_void, buf: *mut u8, buf_size: c_int) -> c_int;
pub(crate) type WritePacketFn = unsafe extern "C" fn(opaque: *mut c_void, buf: *const u8, buf_size: c_int) -> c_int;
pub(crate) type SeekFn = unsafe extern "C" fn(opaque: *mut c_void, offset: i64, whence: c_int) -> i64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_tags_match_the_headers() {
        // MKTAG('E','O','F',' ') = 0x20464F45.
        assert_eq!(AVERROR_EOF, -0x2046_4F45);
        assert_eq!(AVERROR_EXIT, -0x5449_5845);
        assert_eq!(AVERROR_EXTERNAL, -0x2054_5845);
        assert_eq!(AVERROR_INVALIDDATA, -0x4144_4E49);
        assert_eq!(fferrtag(0xF8, b'D', b'E', b'C'), -0x4345_44F8, "AVERROR_DECODER_NOT_FOUND");
    }
}
