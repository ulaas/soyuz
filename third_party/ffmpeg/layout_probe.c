/*
 * layout_probe: prints offsetof() of every FFmpeg struct field soyuz-media reads, as JSON,
 * so CI can compare the Rust `#[repr(C)]` prefix structs (crates/soyuz-media/src/sys/
 * layout.rs) with the headers of the exact library build in use. One line per field:
 * {"struct":"AVFrame","field":"pts","offset":168}, then the five library majors.
 *
 * Build (headers only, no linking):
 *   cc -I<ffmpeg include dir> -o layout_probe third_party/ffmpeg/layout_probe.c
 * Run:  ./layout_probe > layout.json
 *
 * Compile-only in Phase V0; V1a adds the Rust side (`cargo test -p soyuz-media -- layout`
 * reads layout.json through SOYUZ_FFMPEG_LAYOUT=<file>). The field list mirrors
 * plan-v §1.1: nothing of AVFrame past decode_error_flags, nothing of AVStream past
 * attached_pic, AVCodecContext only hw_device_ctx (V4b). Fields that exist in one major
 * only are guarded by the FF_API_* macros of that major's version_major.h.
 */
#include <stddef.h>
#include <stdio.h>

#include <libavcodec/avcodec.h>
#include <libavcodec/codec_par.h>
#include <libavcodec/packet.h>
#include <libavcodec/version.h>
#include <libavcodec/version_major.h>
#include <libavformat/avformat.h>
#include <libavformat/version.h>
#include <libavformat/version_major.h>
#include <libavutil/avutil.h>
#include <libavutil/frame.h>
#include <libavutil/version.h>
#include <libswresample/version.h>
#include <libswscale/version.h>

#define OFF(type, field) printf("{\"struct\":\"" #type "\",\"field\":\"" #field "\",\"offset\":%zu}\n", offsetof(type, field))

int main(void) {
    /* AVPacket: identical on 7.1 / 8.x / 9.0. */
    OFF(AVPacket, buf);
    OFF(AVPacket, pts);
    OFF(AVPacket, dts);
    OFF(AVPacket, data);
    OFF(AVPacket, size);
    OFF(AVPacket, stream_index);
    OFF(AVPacket, flags);
    OFF(AVPacket, side_data);
    OFF(AVPacket, side_data_elems);
    OFF(AVPacket, duration);
    OFF(AVPacket, pos);
    OFF(AVPacket, opaque);
    OFF(AVPacket, opaque_ref);
    OFF(AVPacket, time_base);

    /* AVFrame: the stable prefix (7.1 inserts key_frame, interlaced_frame, top_field_first,
     * palette_has_changed and pkt_pos; pkt_size after decode_error_flags is never read). */
    OFF(AVFrame, data);
    OFF(AVFrame, linesize);
    OFF(AVFrame, extended_data);
    OFF(AVFrame, width);
    OFF(AVFrame, height);
    OFF(AVFrame, nb_samples);
    OFF(AVFrame, format);
#if FF_API_FRAME_KEY
    OFF(AVFrame, key_frame);
#endif
    OFF(AVFrame, pict_type);
    OFF(AVFrame, sample_aspect_ratio);
    OFF(AVFrame, pts);
    OFF(AVFrame, pkt_dts);
    OFF(AVFrame, time_base);
    OFF(AVFrame, quality);
    OFF(AVFrame, opaque);
    OFF(AVFrame, repeat_pict);
#if FF_API_INTERLACED_FRAME
    OFF(AVFrame, interlaced_frame);
    OFF(AVFrame, top_field_first);
#endif
#if FF_API_PALETTE_HAS_CHANGED
    OFF(AVFrame, palette_has_changed);
#endif
    OFF(AVFrame, sample_rate);
    OFF(AVFrame, buf);
    OFF(AVFrame, extended_buf);
    OFF(AVFrame, nb_extended_buf);
    OFF(AVFrame, side_data);
    OFF(AVFrame, nb_side_data);
    OFF(AVFrame, flags);
    OFF(AVFrame, color_range);
    OFF(AVFrame, color_primaries);
    OFF(AVFrame, color_trc);
    OFF(AVFrame, colorspace);
    OFF(AVFrame, chroma_location);
    OFF(AVFrame, best_effort_timestamp);
#if FF_API_FRAME_PKT
    OFF(AVFrame, pkt_pos);
#endif
    OFF(AVFrame, metadata);
    OFF(AVFrame, decode_error_flags);
#if FF_API_FRAME_PKT
    OFF(AVFrame, pkt_size);
#endif
    OFF(AVFrame, hw_frames_ctx);
    OFF(AVFrame, ch_layout);
    OFF(AVFrame, duration);

    /* AVStream: read up to attached_pic (7.1 has deprecated side-data fields after it). */
    OFF(AVStream, av_class);
    OFF(AVStream, index);
    OFF(AVStream, id);
    OFF(AVStream, codecpar);
    OFF(AVStream, priv_data);
    OFF(AVStream, time_base);
    OFF(AVStream, start_time);
    OFF(AVStream, duration);
    OFF(AVStream, nb_frames);
    OFF(AVStream, disposition);
    OFF(AVStream, discard);
    OFF(AVStream, sample_aspect_ratio);
    OFF(AVStream, metadata);
    OFF(AVStream, avg_frame_rate);
    OFF(AVStream, attached_pic);
    OFF(AVStream, event_flags);
    OFF(AVStream, r_frame_rate);
    OFF(AVStream, pts_wrap_bits);

    /* AVFormatContext: the prefix through interrupt_callback. */
    OFF(AVFormatContext, av_class);
    OFF(AVFormatContext, iformat);
    OFF(AVFormatContext, oformat);
    OFF(AVFormatContext, priv_data);
    OFF(AVFormatContext, pb);
    OFF(AVFormatContext, ctx_flags);
    OFF(AVFormatContext, nb_streams);
    OFF(AVFormatContext, streams);
    OFF(AVFormatContext, nb_stream_groups);
    OFF(AVFormatContext, stream_groups);
    OFF(AVFormatContext, nb_chapters);
    OFF(AVFormatContext, chapters);
    OFF(AVFormatContext, url);
    OFF(AVFormatContext, start_time);
    OFF(AVFormatContext, duration);
    OFF(AVFormatContext, bit_rate);
    OFF(AVFormatContext, packet_size);
    OFF(AVFormatContext, max_delay);
    OFF(AVFormatContext, flags);
    OFF(AVFormatContext, probesize);
    OFF(AVFormatContext, max_analyze_duration);
    OFF(AVFormatContext, key);
    OFF(AVFormatContext, keylen);
    OFF(AVFormatContext, nb_programs);
    OFF(AVFormatContext, programs);
    OFF(AVFormatContext, video_codec_id);
    OFF(AVFormatContext, audio_codec_id);
    OFF(AVFormatContext, subtitle_codec_id);
    OFF(AVFormatContext, data_codec_id);
    OFF(AVFormatContext, metadata);
    OFF(AVFormatContext, start_time_realtime);
    OFF(AVFormatContext, fps_probe_size);
    OFF(AVFormatContext, error_recognition);
    OFF(AVFormatContext, interrupt_callback);

    /* AVChapter (chapters[]): small and stable. */
    OFF(AVChapter, id);
    OFF(AVChapter, time_base);
    OFF(AVChapter, start);
    OFF(AVChapter, end);
    OFF(AVChapter, metadata);

    /* AVCodecParameters: 8.x / 9.0 append alpha_mode at the end. */
    OFF(AVCodecParameters, codec_type);
    OFF(AVCodecParameters, codec_id);
    OFF(AVCodecParameters, codec_tag);
    OFF(AVCodecParameters, extradata);
    OFF(AVCodecParameters, extradata_size);
    OFF(AVCodecParameters, coded_side_data);
    OFF(AVCodecParameters, nb_coded_side_data);
    OFF(AVCodecParameters, format);
    OFF(AVCodecParameters, bit_rate);
    OFF(AVCodecParameters, bits_per_coded_sample);
    OFF(AVCodecParameters, bits_per_raw_sample);
    OFF(AVCodecParameters, profile);
    OFF(AVCodecParameters, level);
    OFF(AVCodecParameters, width);
    OFF(AVCodecParameters, height);
    OFF(AVCodecParameters, sample_aspect_ratio);
    OFF(AVCodecParameters, framerate);
    OFF(AVCodecParameters, field_order);
    OFF(AVCodecParameters, color_range);
    OFF(AVCodecParameters, color_primaries);
    OFF(AVCodecParameters, color_trc);
    OFF(AVCodecParameters, color_space);
    OFF(AVCodecParameters, chroma_location);
    OFF(AVCodecParameters, video_delay);
    OFF(AVCodecParameters, ch_layout);
    OFF(AVCodecParameters, sample_rate);
    OFF(AVCodecParameters, block_align);
    OFF(AVCodecParameters, frame_size);
    OFF(AVCodecParameters, initial_padding);
    OFF(AVCodecParameters, trailing_padding);
    OFF(AVCodecParameters, seek_preroll);

    /* AVCodecContext: never read as a struct except hw_device_ctx (V4b). */
    OFF(AVCodecContext, hw_device_ctx);

    /* AVPacketSideData / AVFrameSideData: type, data, size. */
    OFF(AVPacketSideData, data);
    OFF(AVPacketSideData, size);
    OFF(AVPacketSideData, type);
    OFF(AVFrameSideData, type);
    OFF(AVFrameSideData, data);
    OFF(AVFrameSideData, size);
    OFF(AVFrameSideData, metadata);
    OFF(AVFrameSideData, buf);

    printf("{\"majors\":{\"avutil\":%d,\"swresample\":%d,\"swscale\":%d,\"avcodec\":%d,\"avformat\":%d}}\n",
           LIBAVUTIL_VERSION_MAJOR, LIBSWRESAMPLE_VERSION_MAJOR, LIBSWSCALE_VERSION_MAJOR, LIBAVCODEC_VERSION_MAJOR,
           LIBAVFORMAT_VERSION_MAJOR);
    return 0;
}
