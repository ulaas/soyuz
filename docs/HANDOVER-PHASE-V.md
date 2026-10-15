# Phase V handover: FFmpeg media (status 2026-10-15, complete)

Phase V moves every media task in Soyuz (thumbnails, previews, video playback, audio,
media tools) to the FFmpeg shared libraries, loaded at run time and shipped with the app.
**All four waves are done** (branch `phase-v`, merged into `main`). What is left is the first
CI run and the checks on real Linux / macOS hardware (see [Open risks](#open-risks)). The CI jobs that build FFmpeg run on every push to `main` / `phase-*`; check
their results against [First CI run](#first-ci-run).

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
| V2a | `fdba642` | `Player`: demux / video / audio threads, audio-clock master (wall clock without audio), coalesced accurate seeks, late frames dropped, loop, volume / mute, `SetOutputSize`; `Drop` waits ≤ `JOIN_TIMEOUT` (500 ms) and abandons (leaks) contexts of a stuck thread. |
| V2b | `8ffe12f`, `7ccc275` | FFmpeg audio in previews (`[media] audio_backend`, Symphonia fallback), AudioReader-fed playback with live volume / mute, `services/media_player.rs` (`RodioOutput`, ring sink, device-side clock), `models/player.rs` (`PlayerModel`, `app.ui.player`), Settings rows for the player keys. |
| V2c | `aa97e1d` | Transcoder (`transcode::run` / `plan`: remux, trim, convert, extract audio, save frame; `.part` + rename; cancel between packets), 14 presets incl. hardware H.264 / HEVC, VP8 / VP9 + Opus, ProRes, MPEG-4, FLAC / AAC / Opus / WAV / ALAC; `sys/encode.rs`, `sys/mux.rs` over our own write AVIO. |
| Actions | `546c4da` | Shared `Player*` actions (play / pause, seek, volume, mute) in `KeyContext::Player` and `Media*` actions (convert, extract audio, trim, save frame) in FileOps; palette, locales, KEYBOARD.md. |
| V3a | `fa268c7` | GUI player in the preview pane and Quick Look (`ui/preview/video.rs`): off-thread open (`PlayerModel::open_in_background` + `poll`), one reused texture per player, loop in the model, `KeyContext::Player` while the player has focus, `player_smoke` test; frame budget worst 2.9 ms. |
| V3b | `aea41a0`, `b930f5b` | TUI player (`soyuz-tui/src/player.rs`): frames through the terminal image path at `[media] tui_fps`, status line with text progress bar, volume, mute, loop, mouse seek, screen-reader words; headless tests and snapshots; Settings row for `tui_fps`. |
| V3c | `22963b3` | Media tools as jobs: `JobKind::Media` (serde `"media"`), `OpKind::Media`, overwrite through `<stem>.soyuz-new.<ext>` plus the trash, identity-stamped undo, journal resume; dialog model (`models/media_tools.rs`, off-thread probe, dry-run plan), GUI and TUI dialogs, context-menu entries, `[media] tools` row. |
| Fix | `e91d83b` | `PlayerModel::sync_target(None)` closes the player (a missing pending key counted as "same key"); the TUI reads the model's loop toggle. |
| V4c code | `4fb86db` | In-app FFmpeg runtime download (`services/updates/ffmpeg_runtime.rs`: own matcher on the `ffmpeg-runtime-<ver>` tag, `SHA256SUMS.txt`, streamed hash, staged replace of `<config dir>/ffmpeg`, reload without restart; Settings button and palette action, hidden in Flatpak / MSIX and where no runtime is published); the app updater never picks runtime assets; the Linux in-place updater installs `lib/soyuz/*` and the FFmpeg licence texts and removes old-major libraries. |
| V4a | `ac0e38e` | AI / MCP tools `media_info` (Read) and `media_convert` / `media_extract_audio` / `media_trim` / `media_frame` (Write: dry-run plans of `JobKind::Media` jobs with conflict policy and scope checks); Rhai `media_info` / `convert` / `extract_audio` / `trim` / `save_frame`; `[media] ffmpeg` / `tools` gates; crash sentinel on the app and MCP workers. |
| V4b | `b8a230d` | Hardware decoding (`hw.rs`, `sys/hwdec.rs`: D3D11VA / VAAPI / VideoToolbox through `hw_device_ctx` (offset 560 in every major, checked at run time against the `hwaccel_flags` AVOption), transfer to NV12 / P010, software fallback also mid-stream) and the GPU YUV path (`yuv.rs`, `ui/preview/video_gpu.{rs,wgsl}`: BT.601 / 709 / 2020, range, PQ / HLG tone mapping; `[media] gpu_yuv`, off by default). |
| Gaps | `e253cad` | `[media] threads` reaches the transcoder; resumed media jobs keep their tool label; chapters / tags / cover art / rotation kept where the container allows; pure-Rust faststart (`[media] faststart`). Details below. |
| Docs | `c16f267`, `de57719` | USER_GUIDE "Media", ACCESSIBILITY, UIA audit states `video` / `mediatools`, demo video (`demo-fixture.ps1 -Ffmpeg`), shots `video-player.png` / `media-tools.png`; media-tools text fields named, player Mute / Loop report their toggle state (audit re-run: 19 checks, 0 failed). |
| Player | `47f621f` | 150 ms audio pre-roll before the clock runs (engine and rodio ring), packet queues bounded by bytes (video 32 MiB, audio 4 MiB), `AudioReader` is `Send`, hidden `test-seams` feature with a mid-stream hardware-fallback test; `request_seek` race fixed (interrupts set before the target is published). |

Gates at the end of Phase V on Windows: fmt, clippy (`-D warnings`; `soyuz-media` also for
the Linux and macOS targets), `cargo deny`, unsafe allowlist, 2,423 workspace tests, and
1,693 `soyuz-media` / `soyuz-appcore` / `soyuz-jobs` / `soyuz-tui` / `soyuz-ai` / `soyuz-mcp` /
`soyuz-script` tests against FFmpeg 9.0. UIA audit media states: 19 checks, 0 failed.
Release frame-budget tour: worst 5.7 ms, p95 1.2 ms.

Converter gap pass (branch `vgaps`) details:

- `[media] threads` reaches the transcoder: `JobOptions::media` (`soyuz_jobs::MediaOptions`,
  serde default, kept by the crash journal) is filled from `[media]` on submit
  (`JobsService::submit` / `reconfigure`) and handed to `soyuz_media::run_with` as
  `RunOptions`; decoders and video encoders get `threads` (0 = `auto`).
- A resumed media job keeps its tool in the title ("Trimming …", not "Converting …"):
  `OpKind::of_kind` reads the tool from the `MediaOp`, and `services/crash.rs` tracks resumed
  media jobs with `media_tools::job_meta` (tests: `a_resumed_trim_keeps_its_op_label_and_options`,
  `resumed_media_jobs_keep_their_tool_in_the_title`). Script and AI submissions use
  `media_tools::job_meta` too; AI jobs also take the configured `JobOptions::media`.
- Transcoder fidelity (`tests/fidelity.rs`, on the committed fixtures):
  - **Chapters** are written for MP4 / M4A / MOV, Matroska / WebM, Ogg, FLAC and MP3
    (`mux::container_keeps_chapters`) with all their tags, moved to the output's time zero
    and clipped to a trim window; other containers get a plan / run warning.
  - **Tags**: container and stream tags are copied as before; Ogg (stream tags only) gets
    the container tags merged into each audio stream.
  - **Cover art** (attached pictures) is copied wherever the container has a slot
    (`mux::container_holds_cover`: MP4 / M4A / MOV `covr`, MP3 `APIC`, FLAC `PICTURE`,
    Matroska as an attachment, since its muxer would write a one-frame video track); its
    one packet is written once at time 0, whatever the trim window. Ogg / Opus, WAV, WebM
    and the rest drop it quietly (FFmpeg's Ogg muxer writes no `METADATA_BLOCK_PICTURE`).
  - **Rotation** (display matrix) is kept by MP4 / M4A / MOV and Matroska / WebM, copied or
    re-encoded (`mux::container_keeps_rotation`). AVI, MPEG-TS, Ogg and the rest have no
    field for it: the picture stays as coded and the plan / run warn ("the rotation (270
    degrees) is lost"). Rotating the pixels instead would need a filter graph (not bound).
  - **Faststart** for MP4 / M4A / MOV: FFmpeg's `movflags=+faststart` re-opens the output
    by URL, which our write AVIO cannot offer, so `faststart::relocate` (plain Rust) moves
    the `moov` in front of the `mdat` after the trailer: in place, same file length, every
    `stco` / `co64` offset patched; fragmented files, a `moov` over 256 MiB or a 32-bit
    offset that would overflow leave the file as written. New key `[media] faststart`
    (true), Settings row "Web-ready MP4".

## How the pieces fit

```
soyuz-app (egui) / soyuz-tui (ratatui)
        │  Settings media group, preview card, About
soyuz-appcore  services/media.rs   runtime(), status(), configure(), retry(), crash sentinel
               services/updates/ffmpeg_runtime.rs   in-app runtime download
               models/player.rs, models/media_tools.rs, services/media_player.rs (rodio)
               services/thumb_video.rs, preview_video.rs, preview_rich.rs (video facts, posters)
               file_class.rs        one video / audio extension table (wider when a runtime is loaded)
        │
soyuz-media    lib.rs     Runtime::get(dirs) / loaded(), search_dirs(), lib_file_names(), Majors, Versions
               info.rs    probe() → MediaInfo { streams, chapters, tags, cover }
               frame.rs   grab_frame() → RgbaImage (rotation applied, fitted into max_px)
               audio.rs   AudioReader (f32 interleaved, seek), peaks()
               player.rs  Player (threads, clock, pre-roll, seek; frames → FrameSink, audio → AudioSink)
               hw.rs, yuv.rs   hardware device choice / fallback; YUV frames and colour maths
               transcode.rs, presets.rs, mux.rs   run / plan, 14 presets, container tables
               faststart.rs  moov before mdat after the trailer (plain Rust)
               sys/       the only unsafe code: loader, symbols, layout, avio, demux, codec,
                          hwdec, scale, resample, encode, mux, check (self-test)
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

`ffmpeg` (true), `ffmpeg_dir` (""), `threads` (0 = auto), `hw_decode` ("auto"), `gpu_yuv` (false),
`audio_backend` ("auto"), `player` (true), `autoplay` (false), `loop` (false), `volume`
(80), `muted` (false), `seek_step_secs` (5.0), `max_px` (1280), `tui_fps` (8),
`poster_secs` (0.0 = a fraction of the duration, as before), `tools` (true), `faststart` (true).

Every key is read and has a GUI row in Settings → Preview → Media (the TUI settings page is
generated from the schema). `threads` is used by the player and the media tools.
`[preview] video` (was `video_ffmpeg`) turns video previews on or off.

## Working on it locally

- **Libraries for development**: any LGPL shared build of FFmpeg 9.0 works, e.g. BtbN's
  `ffmpeg-n9.0-latest-win64-lgpl-shared-9.0.zip` (development only; never ship it: its
  statically linked dependencies have no published sources). Its `include/` folder has the
  headers for checking struct offsets; its `ffmpeg.exe` can make test files.
- **Run the media tests against real libraries**:
  `SOYUZ_FFMPEG_DIR=<folder with the DLLs> SOYUZ_MEDIA_TESTS=1 cargo test -p soyuz-media -p soyuz-appcore -p soyuz-jobs -p soyuz-tui -p soyuz-ai -p soyuz-mcp -p soyuz-script`.
  `SOYUZ_MEDIA_HW=1` makes `tests/hw_yuv.rs` fail when no hardware device opens (for runs on
  a GPU machine).
  With `SOYUZ_MEDIA_TESTS=1` a missing runtime is a failure; without it the with-libs
  tests skip. Plain `cargo test --workspace` needs no libraries.
- **Inspect a file**: `SOYUZ_FFMPEG_DIR=<folder> cargo run -p soyuz-media --example probe -- [--json] <file>...`.
- **Test fixtures**: `crates/soyuz-media/tests/fixtures/` (small files, commands in its
  README) plus the writers in `soyuz_media::fixtures` (WAV, I420 / MJPEG AVI).
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

## After Phase V

Not in scope, candidates for later: subtitles, audio-track switching, SVT-AV1 encoding,
process isolation for decoding, reading HDR mastering metadata (PQ assumes a 1000-nit
peak), a compact volume control in the narrow preview pane, toggle state for the other
icon toggles (only the player's Mute / Loop report "not pressed"), hiding (not only
refusing) the AI write tools when `[media] tools = false`, and the crash sentinel for
Rhai's `media_info`. `sys::Demuxer<'a>` is `Send` even when it borrows a non-`Sync`
`stale` closure: unused that way today, documented in `sys/mod.rs`; tighten the bound
before a demuxer crosses threads.

## Open risks

- The first CI run builds FFmpeg for the first time; expect one or two fixes in the build
  script.
- The 7.1 and 8.x layout tables are verified against headers only until `media-v7` and the
  Ubuntu leg run.
- Hardware decoding ran only on Windows (D3D11VA); VAAPI and VideoToolbox are untested on
  real devices (CI exercises the software fallback). Untagged HD is treated as BT.709 by the
  GPU path and as BT.601 by swscale, so mis-tagged clips differ slightly between the paths.
- Linux and macOS cross-clippy of `soyuz-app` cannot run on the Windows dev machine
  (`ring` needs a C toolchain); CI covers it.
- In-process decoding has no process isolation: the limits, own AVIO and the crash sentinel
  reduce the risk; a malicious file that crashes FFmpeg still takes the app down once.
- Patents: H.264 / HEVC / AAC decoders may be patent-encumbered in some countries; Soyuz
  ships no patent licence (USER_GUIDE, "Codecs and patents").
