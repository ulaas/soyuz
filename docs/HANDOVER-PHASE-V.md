# Phase V handover: FFmpeg media (status 2026-10-15)

Phase V moves every media task in Soyuz (thumbnails, previews, video playback, audio,
media tools) to the FFmpeg shared libraries, loaded at run time and shipped with the app.
**Wave 1 is done and merged into `main`; waves 2–4 are not started.** The CI jobs that
build FFmpeg have not run yet: the first push is their first test (see
[First CI run](#first-ci-run)).

## Decisions (owner, 2026-10-14)

| Topic | Decision |
|---|---|
| Loading | Run time, like pdfium. Soyuz works without the libraries: our own container parser (`preview_media`) gives metadata, Symphonia / rodio keep playing audio. |
| Unsafe code | One new allowlisted folder, `crates/soyuz-media/src/sys/*`. Every `allow(unsafe_code)` file is listed in `scripts/unsafe-allowlist.txt`, checked by CI (`no-shell32` job). |
| FFmpeg version | **9.0** bundled (avcodec 63, avformat 63, avutil 61, swscale 10, swresample 7). Linux system libraries of 8.x and 7.1 are accepted too. |
| Library source | Our own LGPL build in CI (`scripts/ffmpeg-build.sh`): FFmpeg 9.0.2 + dav1d. No GPL, no nonfree, no version3. |
| Encoders | FFmpeg's own (aac, flac, alac, pcm, mjpeg, png, mpeg4, prores_ks), libopus, libvpx; hardware H.264 / HEVC via NVENC, AMF, QSV (Windows), VideoToolbox (macOS), VAAPI (Linux). SVT-AV1 later. Software H.264 / HEVC encoding is impossible under the LGPL (x264 / x265 are GPL). |
| macOS | Universal libraries (arm64 + x86_64, lipo). The signed app has the `com.apple.security.cs.disable-library-validation` entitlement (`installer/macos/entitlements.plist`). |
| `ffmpeg` command line | Removed. The old `[preview] video_ffmpeg` / `ffmpeg_path` keys are migrated (schema 2 → 3). |
| Distribution | Inside every artifact, and as a separate GitHub release `ffmpeg-runtime-<ffver>` with the source tarballs, `BUILD.txt`, `SOURCES.txt` and `LICENSE.md`, never on the app release. |
| Scope guard | Subtitles, audio-track switching and HDR tone mapping are out of waves 1–3. V4b (hardware decoding, GPU YUV path) is optional: only if the CPU path misses the 16 ms frame budget. |

## What is done

| Step | Commit | Contents |
|---|---|---|
| V0 | `f7eb3be` | `soyuz-media` crate (API as stubs), `[media]` config + schema 3 migration, `KeyContext::Player`, update check only picks `soyuz-<version>-` assets, unsafe allowlist + CI step, FFmpeg pins and `layout_probe.c`. |
| V1a | `e740520`, `1686180` | `soyuz-media` core: loader, 116 bound symbols, struct layouts per major, load-time self-test, own AVIO, `probe`, `grab_frame`, `AudioReader`, `peaks`, tests with and without libraries. |
| V1b | `b825784` | FFmpeg build script, CI jobs `ffmpeg` / `media` / `media-v7`, release packaging, installers, LGPL notices, `docs/RELEASING.md` "FFmpeg runtime". |
| V1c | `78aa723` | `services/media.rs` (load, status, crash sentinel), posters / thumbnails / metadata through `soyuz-media`, CLI removed, one extension table, Settings media group, About notice, TUI video thumbnails. |
| Fixes | `49d82c9`, `37daeff` | Layout probe also prints `AVFrameSideData.buf` and the `AVCodec` prefix. |

Gates at `37daeff` on Windows: fmt, clippy (`-D warnings`, also for the Linux and macOS
targets on `soyuz-media` / `soyuz-core`), `cargo deny`, unsafe allowlist, 2,263 workspace
tests, and 778 `soyuz-media` / `soyuz-appcore` / `soyuz-jobs` tests against FFmpeg 9.0.

## How the pieces fit

```
soyuz-app (egui) / soyuz-tui (ratatui)
        │  Settings media group, preview card, About
soyuz-appcore  services/media.rs   runtime(), status(), configure(), retry(), crash sentinel
               services/thumb_video.rs, preview_video.rs, preview_rich.rs (video facts, posters)
               file_class.rs        one video / audio extension table (wider when a runtime is loaded)
        │
soyuz-media    lib.rs     Runtime::get(dirs) / loaded(), search_dirs(), lib_file_names(), Majors, Versions
               info.rs    probe() → MediaInfo { streams, chapters, tags, cover }
               frame.rs   grab_frame() → RgbaImage (rotation applied, fitted into max_px)
               audio.rs   AudioReader (f32 interleaved, seek), peaks()
               player.rs, transcode.rs, presets.rs   STUBS for waves 2–3
               sys/       the only unsafe code: loader, symbols, layout, avio, demux, codec,
                          scale, resample, check (self-test)
        │
avcodec-63 / avformat-63 / avutil-61 / swscale-10 / swresample-7   (or 8.x / 7.1 on Linux)
```

Key properties of `soyuz-media`:

- **Struct access** goes through `#[repr(C)]` prefix structs per major with `offset_of!`
  (`sys/layout.rs`). Only `AVFrame` differs between 7.1 and 8.x / 9.0. `AVCodecContext`
  is never read as a struct: decoders are set up with `avcodec_parameters_to_context` and
  AVOptions. `sys::layout::tests::expected_offsets` pins the hand-computed offsets;
  `layout_json` compares them with `layout_probe.c` output when `SOYUZ_FFMPEG_LAYOUT` is set.
- **Loading**: by absolute path in the order avutil → swresample → swscale → avcodec →
  avformat; Windows uses `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS`.
  The version set must match one supported row (`Majors::SUPPORTED`). A self-test decodes an
  in-memory WAV and a 2-frame AVI before the runtime is used. Libraries are never unloaded.
  The first success is kept for the process; a failure is retried when the folders or the
  first library file change.
- **Search order** (`search_dirs`): `SOYUZ_FFMPEG_DIR` → `[media] ffmpeg_dir` → the exe's
  folder (macOS also `../Frameworks`, Linux also `../lib/soyuz`, `../lib64/soyuz`) → Flatpak
  `codecs-extra/lib` → `<config dir>/ffmpeg` → Linux system folders → the loader's own path.
- **Untrusted input**: every source, files included, is read through our own `AVIOContext`
  (no URL protocols; the build has `--disable-network`); callbacks run in `catch_unwind`;
  `stale` is checked on every read and between packets; limits on open (probesize 16 MiB,
  analyzeduration 5 s, max_streams 64) and on every decoder (max_pixels 64 MP).
- **Crash sentinel** (`services/media.rs`): a file being decoded automatically is recorded
  in `<config dir>/media-inflight`; if the process dies, that file is skipped by thumbnails
  and the preview pane at the next start until the user opens it in Quick Look.

## Config `[media]`

`ffmpeg` (true), `ffmpeg_dir` (""), `threads` (0 = auto), `hw_decode` ("auto"),
`audio_backend` ("auto"), `player` (true), `autoplay` (false), `loop` (false), `volume`
(80), `muted` (false), `seek_step_secs` (5.0), `max_px` (1280), `tui_fps` (8),
`poster_secs` (0.0 = a fraction of the duration, as before), `tools` (true).

Read today: `ffmpeg`, `ffmpeg_dir`, `threads`, `max_px`, `poster_secs` (these have GUI rows
in Settings → Preview → Media). The others are read by waves 2–3; add their GUI rows in the
step that starts reading them (no dead settings). `[preview] video` (was `video_ffmpeg`)
turns video previews on or off.

## Working on it locally

- **Libraries for development**: any LGPL shared build of FFmpeg 9.0 works, e.g. BtbN's
  `ffmpeg-n9.0-latest-win64-lgpl-shared-9.0.zip` (development only; never ship it: its
  statically linked dependencies have no published sources). Its `include/` folder has the
  headers for checking struct offsets; its `ffmpeg.exe` can make test files.
- **Run the media tests against real libraries**:
  `SOYUZ_FFMPEG_DIR=<folder with the DLLs> SOYUZ_MEDIA_TESTS=1 cargo test -p soyuz-media -p soyuz-appcore -p soyuz-jobs`.
  With `SOYUZ_MEDIA_TESTS=1` a missing runtime is a failure; without it the with-libs
  tests skip. Plain `cargo test --workspace` needs no libraries.
- **Inspect a file**: `SOYUZ_FFMPEG_DIR=<folder> cargo run -p soyuz-media --example probe -- [--json] <file>...`.
- **Test fixtures**: `crates/soyuz-media/tests/fixtures/` (three tiny files, commands in
  its README) plus the writers in `soyuz_media::fixtures` (WAV, I420 / MJPEG AVI).
- **Builds**: `scripts/ffmpeg-build.sh <linux64|win64|macos-arm64|macos-x86_64|macos-universal> <out-dir> [--dry-run]`
  reads `third_party/ffmpeg/pins.env`; it runs on Linux / macOS (Windows DLLs are
  cross-built with mingw on Linux). `docs/RELEASING.md` "FFmpeg runtime" covers pin updates
  and LGPL compliance.

## First CI run

The workflows run on pushes to `main` and `phase-*`. Expected on the first run:

1. **`ffmpeg`** (three targets, 10–20 min each on a cache miss): builds FFmpeg 9.0.2 and
   uploads `ffmpeg-<target>` artifacts plus `ffmpeg-sources`. Each job log shows
   `BUILD.txt` (configure line, DLL import list). Most likely failure: the Windows
   DLL-import gate flags `libstdc++-6.dll` / `libwinpthread-1.dll` from the libvpl C++
   build: adjust the linker flags in `settings()` of the build script.
2. **`media`** (Windows, Ubuntu, macOS): compares the struct layouts with the probe, then
   runs the media tests against the built libraries.
3. **`media-v7`** (`debian:trixie`, distro FFmpeg 7.1): the first real test of the 7.1
   layout table, which so far is verified against the headers only.
4. Re-run once with the `rebuild_ffmpeg` input; the next run should restore from the cache.
5. First `v*` tag after this: the app draft has no `ffmpeg-*` asset; release
   `ffmpeg-runtime-9.0.2` exists and is not marked latest; `rpm -qp --requires` shows no
   `libav*`; the Linux tarball has `lib/soyuz/libavcodec.so.63`; the signed macOS job shows
   the entitlement.

Not verifiable locally (only in CI): the real FFmpeg builds, WiX harvesting of the DLLs,
`gh release create --latest=false`, cargo-deb globs, the `actions/cache` paths.

## Remaining waves

Up to three agents in parallel, one crate or module set each, one compile at a time.

| Step | Owns | Notes |
|---|---|---|
| **V2a** player engine | `soyuz-media/src/{player,clock,queue}.rs` | Demux thread → video / audio decode threads → bounded queues; audio clock master (wall clock without audio); accurate seek; frames through the `FrameSink` trait. If a decode thread does not stop within the bounded join, **leak** its contexts, never free them. Tests with a null audio output. |
| **V2b** FFmpeg audio + player model | `services/preview_audio.rs`, `preview_rich.rs` (audio), `ui/preview/rich.rs::audio`, new `services/media_player.rs` (rodio `AudioOutput`), new `models/player.rs`, locales | `[media] audio_backend` picks FFmpeg or Symphonia. Count `played_frames` inside the rodio `Source` pull (the cpal callback); never block that thread; underruns play silence and advance the clock. Opus / WMA / APE / AC-3 start playing here. |
| **V2c** transcoder + presets | `soyuz-media/src/{transcode,presets,mux}.rs` | Encoders set up through `AVCodecParameters` (there is no `sample_fmt` AVOption). `AVPacket.pos` is often -1: progress falls back to pts / duration. Hardware encoders: probe by opening once (`Runtime::has_encoder` already does). Output to `<dest>.part`, then rename. |
| **V3a** GUI player | `ui/preview/video.rs`, `ui/quicklook.rs`, keymap defaults (`KeyContext::Player`), `actions.rs` (`Action::Media*`), palette, locales, frame-budget tour step | Update one texture per player (`TextureHandle::set`), not a new texture per frame. Controls get accessible names. |
| **V3b** TUI player | `soyuz-tui` `view/preview.rs` (video), `view/quicklook.rs`, `input/preview.rs`, TUI locales, snapshots | Frames at `[media] tui_fps` through the terminal image path; audio-only and a text progress bar when images are off. |
| **V3c** media tools as jobs | `soyuz-jobs/src/{kinds/media.rs, job.rs, kinds/mod.rs, history.rs}`, `models/media_tools.rs`, `models/dialogs.rs`, `ops/fileops.rs`, `services/jobs.rs`, GUI + TUI dialogs | New `JobKind::Media` (its serde name is persisted), `OpKind::Media`; every exhaustive match site (`job.rs` brief / touched_roots / default_label, `kinds/mod.rs` run_kind, `journal.rs`, `jobs_rows.rs`, `ui/panels/activity.rs`); undo via `UndoRecord::Copied { dst }` + identity stamp; priority Low by default. Regenerate KEYBOARD / CONFIG after merging V3a, then V3c. |
| **V4a** AI / MCP / Rhai | `soyuz-ai` tool catalog, `soyuz-script` API, docs | `media_info` (Read) and `media_convert` / `media_extract_audio` / `media_trim` / `media_frame` (Write, dry-run plans → approval → job). |
| **V4b** (optional) | `soyuz-media/src/hw.rs`, `hw_device_ctx` offset, `ui/preview/video_gpu.rs` | Only if the CPU path misses the frame budget. |
| **V4c** polish | `services/updates.rs` (runtime download with its own matcher on the `ffmpeg-runtime-*` tag), Settings button, USER_GUIDE, ACCESSIBILITY, UIA audit state, screenshots | The Linux in-place updater must also extract `lib/soyuz/*`. |

## Open risks

- The first CI run builds FFmpeg for the first time; expect one or two fixes in the build
  script.
- The 7.1 and 8.x layout tables are verified against headers only until `media-v7` and the
  Ubuntu leg run.
- In-process decoding has no process isolation: the limits, own AVIO and the crash sentinel
  reduce the risk; a malicious file that crashes FFmpeg still takes the app down once.
- Patents: H.264 / HEVC / AAC decoders may be patent-encumbered in some countries; Soyuz
  ships no patent licence (USER_GUIDE, "Codecs and patents").
