//! The FFmpeg FFI: the only `unsafe` code of the crate (allow-listed in
//! `scripts/unsafe-allowlist.txt`; CI compares the list with the tree).
//!
//! Layout (V1a fills these in; V2c and V4b extend the symbol table only through
//! [`SYMBOLS`], never by editing another wave's module):
//!
//! - `loader.rs`: finds and loads the five libraries in dependency order (`avutil`,
//!   `swresample`, `swscale`, `avcodec`, `avformat`), each by absolute path. Windows uses
//!   `libloading::os::windows::Library::load_with_flags` with
//!   `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS` explicitly
//!   (`Library::new` passes no flags); our Linux build links with `-Wl,-rpath,'$ORIGIN'`
//!   and the macOS dylibs carry `@rpath` install names plus `@loader_path`. Checks the
//!   version functions against [`crate::Majors::SUPPORTED`] before any struct is read.
//! - `symbols.rs`: one `extern "C"` function pointer per entry of [`SYMBOLS`], bound
//!   with `Library::get`. A missing symbol is [`crate::LoadError::Broken`].
//! - `layout.rs`: `#[repr(C)]` *prefix* structs per major (`AVFrame7`, `AVFrame9`, …) and
//!   `core::mem::offset_of!` tables; checked against `third_party/ffmpeg/layout_probe.c`
//!   in CI. `AVFrame` is never read past `decode_error_flags` (7.1 has `pkt_size` after
//!   it); `AVStream` only up to `attached_pic` (7.1 has deprecated side-data fields
//!   after it); `AVCodecContext` is never read as a struct (AVOptions, plus the one
//!   `hw_device_ctx` offset in V4b).
//! - `types.rs`: constants (`AVMEDIA_TYPE_*`, `AV_PIX_FMT_*`, `AVERROR_*`, flags).
//! - `avio.rs`: custom `AVIOContext` read / seek callbacks for every [`crate::Source`]
//!   (files too, so no FFmpeg protocol parsing of user paths and `--disable-network`
//!   costs nothing).
//! - `check.rs`: the load-time self-test (an in-memory WAV and a 2-frame rawvideo AVI).
//!
//! Rules for every `extern "C"` callback: the body is wrapped in
//! `std::panic::catch_unwind(AssertUnwindSafe(..))` and a panic becomes `AVERROR_EXTERNAL`
//! (a panic unwinding into C aborts the process). Every `unsafe` block carries a
//! `// SAFETY:` comment (`clippy::undocumented_unsafe_blocks` is forbidden crate-wide).
//! Limits are set before `avformat_open_input` (`probesize`, `max_analyze_duration`,
//! `max_streams`, `max_pixels`, `err_detect`): file contents are untrusted.

/// Every FFmpeg function the crate binds (decode, encode, mux, hardware), by name, so
/// the loader can report exactly what a build lacks. Bound by V1a; the encode / mux names
/// are listed now so wave 2 and wave 4 extend nothing here.
#[allow(dead_code)] // read by the loader (V1a)
pub(crate) const SYMBOLS: &[&str] = &[
    // avutil
    "avutil_version",
    "avutil_configuration",
    "av_log_set_level",
    "av_strerror",
    "av_malloc",
    "av_free",
    "av_freep",
    "av_dict_get",
    "av_dict_set",
    "av_dict_count",
    "av_dict_free",
    "av_opt_set",
    "av_opt_set_int",
    "av_opt_set_q",
    "av_opt_get",
    "av_opt_get_int",
    "av_opt_set_chlayout",
    "av_opt_get_chlayout",
    "av_frame_alloc",
    "av_frame_free",
    "av_frame_unref",
    "av_frame_get_buffer",
    "av_frame_make_writable",
    "av_frame_side_data_get",
    "av_image_get_buffer_size",
    "av_image_copy_to_buffer",
    "av_get_pix_fmt_name",
    "av_get_sample_fmt_name",
    "av_get_bytes_per_sample",
    "av_sample_fmt_is_planar",
    "av_samples_get_buffer_size",
    "av_channel_layout_describe",
    "av_channel_layout_default",
    "av_channel_layout_uninit",
    "av_channel_layout_copy",
    "av_rescale_q",
    "av_rescale_q_rnd",
    "av_display_rotation_get",
    "av_color_range_name",
    "av_color_primaries_name",
    "av_color_transfer_name",
    "av_color_space_name",
    "av_hwdevice_ctx_create",
    "av_hwframe_transfer_data",
    "av_buffer_ref",
    "av_buffer_unref",
    // swresample
    "swresample_version",
    "swr_alloc",
    "swr_alloc_set_opts2",
    "swr_init",
    "swr_free",
    "swr_convert",
    "swr_get_delay",
    "swr_get_out_samples",
    // swscale
    "swscale_version",
    "sws_getContext",
    "sws_freeContext",
    "sws_scale",
    // avcodec
    "avcodec_version",
    "avcodec_configuration",
    "avcodec_find_decoder",
    "avcodec_find_decoder_by_name",
    "avcodec_find_encoder",
    "avcodec_find_encoder_by_name",
    "avcodec_alloc_context3",
    "avcodec_free_context",
    "avcodec_open2",
    "avcodec_close",
    "avcodec_parameters_alloc",
    "avcodec_parameters_free",
    "avcodec_parameters_copy",
    "avcodec_parameters_to_context",
    "avcodec_parameters_from_context",
    "avcodec_send_packet",
    "avcodec_receive_frame",
    "avcodec_send_frame",
    "avcodec_receive_packet",
    "avcodec_flush_buffers",
    "avcodec_get_name",
    "avcodec_profile_name",
    "avcodec_descriptor_get",
    "avcodec_descriptor_next",
    "avcodec_get_hw_config",
    "av_codec_iterate",
    "av_codec_is_decoder",
    "av_codec_is_encoder",
    "av_packet_alloc",
    "av_packet_free",
    "av_packet_unref",
    "av_packet_ref",
    "av_packet_rescale_ts",
    "av_packet_side_data_get",
    // avformat
    "avformat_version",
    "avformat_configuration",
    "avformat_alloc_context",
    "avformat_free_context",
    "avformat_open_input",
    "avformat_close_input",
    "avformat_find_stream_info",
    "av_find_best_stream",
    "av_read_frame",
    "av_seek_frame",
    "avformat_seek_file",
    "avformat_flush",
    "av_guess_frame_rate",
    "av_guess_sample_aspect_ratio",
    "avio_alloc_context",
    "avio_context_free",
    "avio_open2",
    "avio_closep",
    "avio_flush",
    "avformat_alloc_output_context2",
    "avformat_new_stream",
    "av_guess_format",
    "avformat_write_header",
    "av_interleaved_write_frame",
    "av_write_frame",
    "av_write_trailer",
    "av_dump_format",
];

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
    }
}
