//! The function pointer table ([`Fns`]): one `unsafe extern "C" fn` per FFmpeg function
//! the crate calls, bound with `Library::get` by the loader. Signatures follow the 9.0
//! headers; every function listed exists unchanged in 7.1 and 8.x (`avcodec_close`,
//! removed in 8.0, is deliberately absent: `avcodec_free_context` closes too).
//!
//! The encode / mux / hardware functions are bound now so wave 2 (`transcode`) and wave
//! 4 (`hw`) only use them (review must-fix 10): nothing after V1a edits this table.
//! [`OptionalFns`] holds the few functions a supported release may lack.

use std::ffi::{c_char, c_int, c_uint, c_void};

use libloading::Library;

use super::layout::*;
use super::types::*;

/// Binds `name` from `lib` as a function pointer of type `T` (the one `unsafe` site of
/// symbol binding; every bound function is called elsewhere through its typed pointer).
fn bind<T: Copy>(lib: &Library, name: &'static str) -> Result<T, (&'static str, String)> {
    let mut sym = Vec::with_capacity(name.len() + 1);
    sym.extend_from_slice(name.as_bytes());
    sym.push(0);
    // SAFETY: the signature `T` was transcribed from the FFmpeg headers of the supported
    // majors (symbols.rs doc comment); the library stays loaded for the life of the
    // process (the loader never drops it), so the copied pointer never dangles.
    let s = unsafe { lib.get::<T>(&sym) };
    match s {
        Ok(s) => Ok(*s),
        Err(e) => Err((name, e.to_string())),
    }
}

/// The five libraries, in load order.
pub(crate) struct Libs {
    pub avutil: Library,
    pub swresample: Library,
    pub swscale: Library,
    pub avcodec: Library,
    pub avformat: Library,
}

macro_rules! symbols {
    ($( $lib:ident : { $( fn $name:ident ( $($argn:ident : $arg:ty),* $(,)? ) $(-> $ret:ty)? ; )* } )*) => {
        /// Every required function, bound (the encode / mux / hardware ones wait for
        /// waves 2 and 4).
        #[allow(non_snake_case, dead_code)]
        pub(crate) struct Fns {
            $( $( pub $name: unsafe extern "C" fn($($arg),*) $(-> $ret)?, )* )*
        }

        impl Fns {
            /// Bind every function; `Err((symbol, reason))` names the first one missing.
            pub(crate) fn bind(libs: &Libs) -> Result<Fns, (&'static str, String)> {
                Ok(Fns {
                    $( $( $name: bind(&libs.$lib, stringify!($name))?, )* )*
                })
            }
        }

        /// The names of every required function, by library in load order (the
        /// reviewable list; `Fns::bind` is what the loader uses).
        #[cfg(test)]
        pub(crate) const SYMBOLS: &[&str] = &[ $( $( stringify!($name), )* )* ];
    };
}

symbols! {
    avutil: {
        fn avutil_version() -> c_uint;
        fn avutil_configuration() -> *const c_char;
        fn av_log_set_level(level: c_int);
        fn av_strerror(errnum: c_int, errbuf: *mut c_char, errbuf_size: usize) -> c_int;
        fn av_malloc(size: usize) -> *mut c_void;
        fn av_free(ptr: *mut c_void);
        fn av_freep(ptr: *mut c_void);
        fn av_dict_get(m: *const AVDictionary, key: *const c_char, prev: *const AVDictionaryEntry, flags: c_int) -> *mut AVDictionaryEntry;
        fn av_dict_set(pm: *mut *mut AVDictionary, key: *const c_char, value: *const c_char, flags: c_int) -> c_int;
        fn av_dict_count(m: *const AVDictionary) -> c_int;
        fn av_dict_free(m: *mut *mut AVDictionary);
        fn av_opt_set(obj: *mut c_void, name: *const c_char, val: *const c_char, search_flags: c_int) -> c_int;
        fn av_opt_set_int(obj: *mut c_void, name: *const c_char, val: i64, search_flags: c_int) -> c_int;
        fn av_opt_set_q(obj: *mut c_void, name: *const c_char, val: AVRational, search_flags: c_int) -> c_int;
        fn av_opt_get(obj: *mut c_void, name: *const c_char, search_flags: c_int, out_val: *mut *mut u8) -> c_int;
        fn av_opt_get_int(obj: *mut c_void, name: *const c_char, search_flags: c_int, out_val: *mut i64) -> c_int;
        fn av_opt_set_chlayout(obj: *mut c_void, name: *const c_char, layout: *const AVChannelLayout, search_flags: c_int) -> c_int;
        fn av_opt_get_chlayout(obj: *mut c_void, name: *const c_char, search_flags: c_int, layout: *mut AVChannelLayout) -> c_int;
        fn av_frame_alloc() -> *mut c_void;
        fn av_frame_free(frame: *mut *mut c_void);
        fn av_frame_unref(frame: *mut c_void);
        fn av_frame_get_buffer(frame: *mut c_void, align: c_int) -> c_int;
        fn av_frame_make_writable(frame: *mut c_void) -> c_int;
        fn av_frame_side_data_get_c(sd: *const *const AVFrameSideData, nb_sd: c_int, r#type: c_int) -> *const AVFrameSideData;
        fn av_image_get_buffer_size(pix_fmt: c_int, width: c_int, height: c_int, align: c_int) -> c_int;
        fn av_image_copy_to_buffer(dst: *mut u8, dst_size: c_int, src_data: *const *const u8, src_linesize: *const c_int, pix_fmt: c_int, width: c_int, height: c_int, align: c_int) -> c_int;
        fn av_get_pix_fmt_name(pix_fmt: c_int) -> *const c_char;
        fn av_get_pix_fmt(name: *const c_char) -> c_int;
        fn av_get_sample_fmt_name(sample_fmt: c_int) -> *const c_char;
        fn av_get_sample_fmt(name: *const c_char) -> c_int;
        fn av_get_bytes_per_sample(sample_fmt: c_int) -> c_int;
        fn av_sample_fmt_is_planar(sample_fmt: c_int) -> c_int;
        fn av_samples_get_buffer_size(linesize: *mut c_int, nb_channels: c_int, nb_samples: c_int, sample_fmt: c_int, align: c_int) -> c_int;
        fn av_channel_layout_describe(channel_layout: *const AVChannelLayout, buf: *mut c_char, buf_size: usize) -> c_int;
        fn av_channel_layout_default(ch_layout: *mut AVChannelLayout, nb_channels: c_int);
        fn av_channel_layout_uninit(channel_layout: *mut AVChannelLayout);
        fn av_channel_layout_copy(dst: *mut AVChannelLayout, src: *const AVChannelLayout) -> c_int;
        fn av_rescale_q(a: i64, bq: AVRational, cq: AVRational) -> i64;
        fn av_rescale_q_rnd(a: i64, bq: AVRational, cq: AVRational, rnd: c_int) -> i64;
        fn av_display_rotation_get(matrix: *const i32) -> f64;
        fn av_color_range_name(range: c_int) -> *const c_char;
        fn av_color_primaries_name(primaries: c_int) -> *const c_char;
        fn av_color_transfer_name(transfer: c_int) -> *const c_char;
        fn av_color_space_name(space: c_int) -> *const c_char;
        fn av_hwdevice_ctx_create(device_ctx: *mut *mut AVBufferRef, r#type: c_int, device: *const c_char, opts: *mut AVDictionary, flags: c_int) -> c_int;
        fn av_hwframe_transfer_data(dst: *mut c_void, src: *const c_void, flags: c_int) -> c_int;
        fn av_buffer_ref(buf: *const AVBufferRef) -> *mut AVBufferRef;
        fn av_buffer_unref(buf: *mut *mut AVBufferRef);
    }
    swresample: {
        fn swresample_version() -> c_uint;
        fn swr_alloc() -> *mut SwrContext;
        fn swr_alloc_set_opts2(ps: *mut *mut SwrContext, out_ch_layout: *const AVChannelLayout, out_sample_fmt: c_int, out_sample_rate: c_int, in_ch_layout: *const AVChannelLayout, in_sample_fmt: c_int, in_sample_rate: c_int, log_offset: c_int, log_ctx: *mut c_void) -> c_int;
        fn swr_init(s: *mut SwrContext) -> c_int;
        fn swr_free(s: *mut *mut SwrContext);
        fn swr_convert(s: *mut SwrContext, out: *const *mut u8, out_count: c_int, input: *const *const u8, in_count: c_int) -> c_int;
        fn swr_get_delay(s: *mut SwrContext, base: i64) -> i64;
        fn swr_get_out_samples(s: *mut SwrContext, in_samples: c_int) -> c_int;
    }
    swscale: {
        fn swscale_version() -> c_uint;
        fn sws_getContext(src_w: c_int, src_h: c_int, src_format: c_int, dst_w: c_int, dst_h: c_int, dst_format: c_int, flags: c_int, src_filter: *mut SwsFilter, dst_filter: *mut SwsFilter, param: *const f64) -> *mut SwsContext;
        fn sws_freeContext(ctx: *mut SwsContext);
        fn sws_scale(c: *mut SwsContext, src_slice: *const *const u8, src_stride: *const c_int, src_slice_y: c_int, src_slice_h: c_int, dst: *const *mut u8, dst_stride: *const c_int) -> c_int;
    }
    avcodec: {
        fn avcodec_version() -> c_uint;
        fn avcodec_configuration() -> *const c_char;
        fn avcodec_find_decoder(id: c_int) -> *const AVCodec;
        fn avcodec_find_decoder_by_name(name: *const c_char) -> *const AVCodec;
        fn avcodec_find_encoder(id: c_int) -> *const AVCodec;
        fn avcodec_find_encoder_by_name(name: *const c_char) -> *const AVCodec;
        fn avcodec_alloc_context3(codec: *const AVCodec) -> *mut AVCodecContext;
        fn avcodec_free_context(avctx: *mut *mut AVCodecContext);
        fn avcodec_open2(avctx: *mut AVCodecContext, codec: *const AVCodec, options: *mut *mut AVDictionary) -> c_int;
        fn avcodec_parameters_alloc() -> *mut AVCodecParameters;
        fn avcodec_parameters_free(par: *mut *mut AVCodecParameters);
        fn avcodec_parameters_copy(dst: *mut AVCodecParameters, src: *const AVCodecParameters) -> c_int;
        fn avcodec_parameters_to_context(codec: *mut AVCodecContext, par: *const AVCodecParameters) -> c_int;
        fn avcodec_parameters_from_context(par: *mut AVCodecParameters, codec: *const AVCodecContext) -> c_int;
        fn avcodec_send_packet(avctx: *mut AVCodecContext, avpkt: *const AVPacket) -> c_int;
        fn avcodec_receive_frame(avctx: *mut AVCodecContext, frame: *mut c_void) -> c_int;
        fn avcodec_send_frame(avctx: *mut AVCodecContext, frame: *const c_void) -> c_int;
        fn avcodec_receive_packet(avctx: *mut AVCodecContext, avpkt: *mut AVPacket) -> c_int;
        fn avcodec_flush_buffers(avctx: *mut AVCodecContext);
        fn avcodec_get_name(id: c_int) -> *const c_char;
        fn avcodec_profile_name(codec_id: c_int, profile: c_int) -> *const c_char;
        fn avcodec_descriptor_get(id: c_int) -> *const AVCodecDescriptor;
        fn avcodec_descriptor_next(prev: *const AVCodecDescriptor) -> *const AVCodecDescriptor;
        fn avcodec_get_hw_config(codec: *const AVCodec, index: c_int) -> *const AVCodecHWConfig;
        fn av_codec_iterate(opaque: *mut *mut c_void) -> *const AVCodec;
        fn av_codec_is_decoder(codec: *const AVCodec) -> c_int;
        fn av_codec_is_encoder(codec: *const AVCodec) -> c_int;
        fn av_packet_alloc() -> *mut AVPacket;
        fn av_packet_free(pkt: *mut *mut AVPacket);
        fn av_packet_unref(pkt: *mut AVPacket);
        fn av_packet_ref(dst: *mut AVPacket, src: *const AVPacket) -> c_int;
        fn av_packet_rescale_ts(pkt: *mut AVPacket, tb_src: AVRational, tb_dst: AVRational);
        fn av_packet_side_data_get(sd: *const AVPacketSideData, nb_sd: c_int, r#type: c_int) -> *const AVPacketSideData;
    }
    avformat: {
        fn avformat_version() -> c_uint;
        fn avformat_configuration() -> *const c_char;
        fn avformat_alloc_context() -> *mut AVFormatContext;
        fn avformat_free_context(s: *mut AVFormatContext);
        fn avformat_open_input(ps: *mut *mut AVFormatContext, url: *const c_char, fmt: *const AVInputFormat, options: *mut *mut AVDictionary) -> c_int;
        fn avformat_close_input(s: *mut *mut AVFormatContext);
        fn avformat_find_stream_info(ic: *mut AVFormatContext, options: *mut *mut AVDictionary) -> c_int;
        fn av_find_best_stream(ic: *mut AVFormatContext, r#type: c_int, wanted_stream_nb: c_int, related_stream: c_int, decoder_ret: *mut *const AVCodec, flags: c_int) -> c_int;
        fn av_read_frame(s: *mut AVFormatContext, pkt: *mut AVPacket) -> c_int;
        fn av_seek_frame(s: *mut AVFormatContext, stream_index: c_int, timestamp: i64, flags: c_int) -> c_int;
        fn avformat_seek_file(s: *mut AVFormatContext, stream_index: c_int, min_ts: i64, ts: i64, max_ts: i64, flags: c_int) -> c_int;
        fn avformat_flush(s: *mut AVFormatContext) -> c_int;
        fn av_guess_frame_rate(ctx: *mut AVFormatContext, stream: *mut AVStream, frame: *mut c_void) -> AVRational;
        fn av_guess_sample_aspect_ratio(format: *mut AVFormatContext, stream: *mut AVStream, frame: *mut c_void) -> AVRational;
        fn avio_alloc_context(buffer: *mut u8, buffer_size: c_int, write_flag: c_int, opaque: *mut c_void, read_packet: Option<ReadPacketFn>, write_packet: Option<WritePacketFn>, seek: Option<SeekFn>) -> *mut AVIOContext;
        fn avio_context_free(s: *mut *mut AVIOContext);
        fn avio_open2(s: *mut *mut AVIOContext, url: *const c_char, flags: c_int, int_cb: *const AVIOInterruptCB, options: *mut *mut AVDictionary) -> c_int;
        fn avio_closep(s: *mut *mut AVIOContext) -> c_int;
        fn avio_flush(s: *mut AVIOContext);
        fn avformat_alloc_output_context2(ctx: *mut *mut AVFormatContext, oformat: *const AVOutputFormat, format_name: *const c_char, filename: *const c_char) -> c_int;
        fn avformat_new_stream(s: *mut AVFormatContext, c: *const AVCodec) -> *mut AVStream;
        fn av_guess_format(short_name: *const c_char, filename: *const c_char, mime_type: *const c_char) -> *const AVOutputFormat;
        fn avformat_write_header(s: *mut AVFormatContext, options: *mut *mut AVDictionary) -> c_int;
        fn av_interleaved_write_frame(s: *mut AVFormatContext, pkt: *mut AVPacket) -> c_int;
        fn av_write_frame(s: *mut AVFormatContext, pkt: *mut AVPacket) -> c_int;
        fn av_write_trailer(s: *mut AVFormatContext) -> c_int;
        fn av_dump_format(ic: *mut AVFormatContext, index: c_int, url: *const c_char, is_output: c_int);
    }
}

/// Functions a supported release may lack; `None` when the library has no such symbol.
#[allow(non_snake_case)]
pub(crate) struct OptionalFns {
    /// 7.1.0 and later (`avcodec` 61.13+): the supported pixel / sample formats of a
    /// codec; the encoder open-probe falls back to common formats without it.
    pub avcodec_get_supported_config: Option<
        unsafe extern "C" fn(
            avctx: *const AVCodecContext,
            codec: *const AVCodec,
            config: c_int,
            flags: c_uint,
            out_configs: *mut *const c_void,
            out_num_configs: *mut c_int,
        ) -> c_int,
    >,
    /// Every supported release has it; optional so hardware decoding (V4b, the only
    /// user: it cross-checks the `hw_device_ctx` offset) can never break loading.
    pub av_opt_find: Option<
        unsafe extern "C" fn(
            obj: *mut c_void,
            name: *const c_char,
            unit: *const c_char,
            opt_flags: c_int,
            search_flags: c_int,
        ) -> *const AVOptionPrefix,
    >,
}

impl OptionalFns {
    pub(crate) fn bind(libs: &Libs) -> OptionalFns {
        OptionalFns {
            avcodec_get_supported_config: bind(&libs.avcodec, "avcodec_get_supported_config").ok(),
            av_opt_find: bind(&libs.avutil, "av_opt_find").ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SYMBOLS;

    #[test]
    fn symbol_table_is_unique_and_covers_every_library() {
        let mut sorted = SYMBOLS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), SYMBOLS.len(), "duplicate symbol names");
        for prefix in ["avutil_version", "swresample_version", "swscale_version", "avcodec_version", "avformat_version"]
        {
            assert!(SYMBOLS.contains(&prefix), "{prefix} is the version check of its library");
        }
        // The encode / mux / hardware names wave 2 and wave 4 need (review must-fix 10).
        for s in [
            "avcodec_parameters_to_context",
            "avformat_alloc_output_context2",
            "avformat_new_stream",
            "avio_open2",
            "avformat_write_header",
            "av_interleaved_write_frame",
            "av_write_trailer",
            "av_packet_rescale_ts",
            "avcodec_find_encoder_by_name",
            "avcodec_flush_buffers",
            "av_hwdevice_ctx_create",
            "av_hwframe_transfer_data",
        ] {
            assert!(SYMBOLS.contains(&s), "{s}");
        }
        assert!(!SYMBOLS.contains(&"avcodec_close"), "removed in FFmpeg 8.0");
        assert!(SYMBOLS.contains(&"av_frame_side_data_get_c"), "the exported name of av_frame_side_data_get");
        assert!(SYMBOLS.len() > 110);
    }
}
