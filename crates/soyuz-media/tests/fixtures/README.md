# soyuz-media test fixtures

Five tiny files (under 10 KB each) for what the in-code writers (`soyuz_media::fixtures`:
WAV, raw / Motion-JPEG AVI) cannot express: a display-matrix rotation, HDR colour
signalling with chapters, an attached picture, a codec with hardware decoders (MPEG-2)
and 10-bit PQ video. All content is synthetic (FFmpeg's
`testsrc2`, `sine` and `color` sources); nothing third-party is embedded.

Made 2026-10-15 with the FFmpeg 9.0 development runtime (`ffmpeg.exe` of the BtbN
`lgpl-shared` build, `Lavf63.1.102`). Commands, run in an empty folder:

```sh
# rotated.mp4: 1 s, 64x48 mpeg4 at 10 fps + mono AAC 22.05 kHz, display matrix 90°
# counter-clockwise (= 270° clockwise for display), title and language tags.
ffmpeg -y -f lavfi -i "testsrc2=size=64x48:rate=10:duration=1" \
       -f lavfi -i "sine=frequency=440:sample_rate=22050:duration=1" \
       -map 0:v -map 1:a -c:v mpeg4 -q:v 8 -c:a aac -b:a 24k -ac 1 \
       -metadata title="Soyuz rotated" -metadata:s:a:0 language=eng plain.mp4
ffmpeg -y -display_rotation 90 -i plain.mp4 -c copy -metadata title="Soyuz rotated" rotated.mp4

# hdr-chapters.mkv: 0.5 s, 64x48 at 10 fps, BT.2020 / PQ (SMPTE ST 2084) colour
# signalling in the container, two chapters and a title (meta.txt below).
ffmpeg -y -f lavfi -i "testsrc2=size=64x48:rate=10:duration=0.5" -i meta.txt \
       -map 0:v -map_metadata 1 \
       -vf "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc:range=tv" \
       -c:v mpeg4 -q:v 8 -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc -color_range tv \
       hdr-chapters.mkv

# cover.flac: 0.3 s mono 8 kHz FLAC with a 16x16 red PNG attached picture and tags.
ffmpeg -y -f lavfi -i "sine=frequency=330:sample_rate=8000:duration=0.3" \
       -f lavfi -i "color=c=red:size=16x16:rate=1:duration=1" \
       -map 0:a -map 1:v -frames:v 1 -c:a flac -c:v png -disposition:v attached_pic \
       -metadata artist=Soyuz -metadata title=Cover cover.flac

# hw-mpeg2.mkv (V4b): 1 s, 128x96 MPEG-2 at 10 fps, no colour tags; MPEG-2 has
# D3D11VA / VAAPI / VideoToolbox decoders and an LGPL encoder.
ffmpeg -y -f lavfi -i "testsrc2=size=128x96:rate=10:duration=1" \
       -c:v mpeg2video -q:v 31 -g 5 -bf 0 hw-mpeg2.mkv

# pq10-vp9.webm (V4b): 0.5 s, 64x48 VP9 profile 2 (yuv420p10le), BT.2020 / PQ.
ffmpeg -y -f lavfi -i "testsrc2=size=64x48:rate=10:duration=0.5" \
       -vf "format=yuv420p10le,setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc:range=tv" \
       -c:v libvpx-vp9 -profile:v 2 -crf 40 -b:v 0 -deadline realtime \
       -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc -color_range tv pq10-vp9.webm
```

`meta.txt` (ffmetadata):

```
;FFMETADATA1
title=Soyuz HDR
[CHAPTER]
TIMEBASE=1/1000
START=0
END=250
title=Intro
[CHAPTER]
TIMEBASE=1/1000
START=250
END=500
title=Outro
```

The `setparams` filter is what carries the colour tags into the container (the encoder
options alone do not survive `mpeg4`).

| file | bytes | checked by |
|---|---|---|
| `rotated.mp4` | 6 347 | `rotated_fixture_probe_and_frame` (rotation 270, 48x64 frame, AAC reader) |
| `hdr-chapters.mkv` | 1 901 | `hdr_and_chapters_fixture` (`Hdr::Hdr10`, colour names, two chapters) |
| `cover.flac` | 9 486 | `cover_art_fixture` (PNG cover bytes, attached-pic stream, tags) |
| `hw-mpeg2.mkv` | 4 713 | `tests/hw_yuv.rs` (hardware decoding or its software fallback, NV12 from a device) |
| `pq10-vp9.webm` | 1 730 | `tests/hw_yuv.rs` (10-bit planes, PQ / BT.2020 colour, YUV downscale) |
