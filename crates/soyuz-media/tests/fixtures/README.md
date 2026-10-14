# soyuz-media test fixtures

Three tiny files (under 10 KB each) for what the in-code writers (`soyuz_media::fixtures`:
WAV, raw / Motion-JPEG AVI) cannot express: a display-matrix rotation, HDR colour
signalling with chapters, and an attached picture. All content is synthetic (FFmpeg's
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
