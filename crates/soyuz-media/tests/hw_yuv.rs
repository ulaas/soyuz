//! Hardware decoding and the YUV frame path (V4b) against a real FFmpeg runtime
//! (`SOYUZ_FFMPEG_DIR`, gated like `tests/player.rs`).
//!
//! CI has no GPU decoders: every test passes with the software fallback, and the
//! hardware path is checked whenever a device opens (D3D11VA / VAAPI / VideoToolbox with
//! an MPEG-2 clip). `SOYUZ_MEDIA_HW=1` makes a missing device a failure (manual runs on a
//! machine with a GPU).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use soyuz_media::{
    DIR_ENV, FramePixels, FrameSink, HwPref, Player, PlayerOptions, PlayerState, Runtime, Source, Transfer, YuvFrame,
    YuvImage, YuvLayout, YuvMatrix, fixtures, search_dirs,
};

fn runtime() -> Option<Arc<Runtime>> {
    let strict = std::env::var("SOYUZ_MEDIA_TESTS").is_ok_and(|v| v == "1");
    let dirs: Vec<PathBuf> = match std::env::var_os(DIR_ENV) {
        Some(d) => vec![PathBuf::from(d)],
        None => search_dirs("", &std::env::temp_dir().join("soyuz-media-tests-config")),
    };
    match Runtime::get(&dirs) {
        Ok(rt) => Some(rt),
        Err(e) if strict => panic!("SOYUZ_MEDIA_TESTS=1 but no FFmpeg runtime loads: {e}"),
        Err(e) => {
            eprintln!("skipped: no FFmpeg runtime ({e}); set {DIR_ENV} to a folder with the libraries");
            None
        }
    }
}

macro_rules! rt {
    () => {
        match runtime() {
            Some(r) => r,
            None => return,
        }
    };
}

fn fixture(name: &str) -> Source {
    Source::File(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name))
}

/// What a sink saw: RGBA pictures (size) and YUV pictures (owned copies).
#[derive(Default)]
struct Seen {
    rgba: Vec<(u32, u32, Vec<u8>)>,
    yuv: Vec<YuvImage>,
}

#[derive(Clone, Default)]
struct Sink {
    yuv: bool,
    seen: Arc<Mutex<Seen>>,
}

impl FrameSink for Sink {
    fn present(&mut self, f: FramePixels<'_>) {
        self.seen.lock().unwrap().rgba.push((f.width, f.height, f.rgba.to_vec()));
    }

    fn wants_yuv(&self, _w: u32, _h: u32) -> bool {
        self.yuv
    }

    fn present_yuv(&mut self, f: YuvFrame<'_>) {
        assert!(self.yuv, "YUV only when asked for");
        self.seen.lock().unwrap().yuv.push(YuvImage::from_frame(&f).expect("planes cover the picture"));
    }
}

fn play(rt: &Arc<Runtime>, src: Source, hw: HwPref, sink: &Sink, max_px: u32) -> (Player, PlayerState) {
    let opts = PlayerOptions { max_px, autoplay: true, hw, ..Default::default() };
    let p = Player::open(Arc::clone(rt), src, opts, Box::new(sink.clone()), None, Arc::new(|| {})).expect("open");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = p.state();
        assert!(s.error.is_none(), "{s:?}");
        if s.ended {
            return (p, s);
        }
        assert!(Instant::now() < deadline, "did not end: {s:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn hardware_decoding_plays_or_falls_back_silently() {
    let rt = rt!();
    // Auto: the platform's device when the machine has one, software otherwise; either
    // way every frame arrives.
    let sink = Sink::default();
    let (p, s) = play(&rt, fixture("hw-mpeg2.mkv"), HwPref::Auto, &sink, 4096);
    let n = sink.seen.lock().unwrap().rgba.len();
    assert!(n >= 8, "{n} frames");
    assert!(sink.seen.lock().unwrap().rgba.iter().all(|(w, h, _)| (*w, *h) == (128, 96)));
    match &s.hw {
        Some(api) => eprintln!("hardware path exercised: {api}"),
        None if std::env::var("SOYUZ_MEDIA_HW").is_ok_and(|v| v == "1") => {
            panic!("SOYUZ_MEDIA_HW=1 but no device decoded")
        }
        None => eprintln!("no hardware decoder on this machine: software fallback checked"),
    }
    drop(p);
    // Off: software, never a device.
    let sink = Sink::default();
    let (_p, s) = play(&rt, fixture("hw-mpeg2.mkv"), HwPref::Off, &sink, 4096);
    assert_eq!(s.hw, None);
    assert!(sink.seen.lock().unwrap().rgba.len() >= 8);
    // Another platform's API cannot open here: software, same pictures.
    let foreign = if cfg!(target_os = "linux") { HwPref::D3d11va } else { HwPref::Vaapi };
    let sink = Sink::default();
    let (_p, s) = play(&rt, fixture("hw-mpeg2.mkv"), foreign, &sink, 4096);
    assert_eq!(s.hw, None);
    assert!(sink.seen.lock().unwrap().rgba.len() >= 8);
    // A codec without a hardware configuration (Motion JPEG) decodes in software.
    let frames: Vec<Vec<u8>> = (0..5).map(|_| fixtures::jpeg_solid(32, 16, [200, 40, 40])).collect();
    let clip = fixtures::avi(32, 16, 25, fixtures::AviCodec::Mjpeg, &frames);
    let sink = Sink::default();
    let (_p, s) = play(&rt, Source::Mem(Arc::from(clip.into_boxed_slice())), HwPref::Auto, &sink, 4096);
    assert_eq!(s.hw, None);
}

#[test]
fn yuv_sink_gets_planes_that_match_swscale() {
    let rt = rt!();
    // Raw I420 (yuv420p, no colour tags: limited-range BT.601 for SD), BT.601 red.
    let frame: Vec<u8> = [vec![81u8; 32 * 16], vec![90u8; 16 * 8], vec![240u8; 16 * 8]].concat();
    let clip = fixtures::avi(32, 16, 25, fixtures::AviCodec::I420, &vec![frame; 4]);
    let src = || Source::Mem(Arc::from(clip.clone().into_boxed_slice()));
    let yuv = Sink { yuv: true, ..Default::default() };
    play(&rt, src(), HwPref::Off, &yuv, 4096);
    let rgba = Sink::default();
    play(&rt, src(), HwPref::Off, &rgba, 4096);
    let y = yuv.seen.lock().unwrap();
    let r = rgba.seen.lock().unwrap();
    assert!(y.rgba.is_empty(), "every picture went as planes");
    let img = &y.yuv[0];
    assert_eq!((img.width, img.height, img.layout, img.bytes, img.depth), (32, 16, YuvLayout::I420, 1, 8));
    assert_eq!(img.display, (32, 16));
    assert!(!img.color.full_range && img.color.matrix == YuvMatrix::Bt601, "{:?}", img.color);
    assert_eq!(img.color.transfer, Transfer::Sdr);
    // The CPU reference of the GPU conversion agrees with swscale (BT.601 limited).
    let ours = img.to_rgba();
    let (w, _, theirs) = &r.rgba[0];
    let at = ((8 * *w + 16) * 4) as usize;
    for c in 0..3 {
        let d = (i32::from(ours[at + c]) - i32::from(theirs[at + c])).abs();
        assert!(d <= 3, "channel {c}: ours {:?} swscale {:?}", &ours[at..at + 4], &theirs[at..at + 4]);
    }
    assert!(ours[at] > 245 && ours[at + 1] < 10 && ours[at + 2] < 10, "red: {:?}", &ours[at..at + 4]);
    drop((y, r));
    // A 4:2:2 / 4:4:4 picture (Motion JPEG from the `image` encoder) has no YUV form
    // here: it still arrives, as RGBA.
    let frames: Vec<Vec<u8>> = (0..3).map(|_| fixtures::jpeg_solid(32, 16, [220, 30, 30])).collect();
    let mj = fixtures::avi(32, 16, 25, fixtures::AviCodec::Mjpeg, &frames);
    let sink = Sink { yuv: true, ..Default::default() };
    play(&rt, Source::Mem(Arc::from(mj.into_boxed_slice())), HwPref::Off, &sink, 4096);
    let seen = sink.seen.lock().unwrap();
    assert!(seen.yuv.len() + seen.rgba.len() >= 3);
}

#[test]
fn hdr_frames_carry_ten_bit_planes_and_their_transfer() {
    let rt = rt!();
    let yuv = Sink { yuv: true, ..Default::default() };
    play(&rt, fixture("pq10-vp9.webm"), HwPref::Off, &yuv, 4096);
    let seen = yuv.seen.lock().unwrap();
    let img = seen.yuv.first().expect("a YUV picture");
    assert_eq!(
        (img.width, img.height, img.layout, img.bytes, img.depth, img.msb),
        (64, 48, YuvLayout::I420, 2, 10, false)
    );
    assert_eq!(img.color.transfer, Transfer::Pq);
    assert_eq!(img.color.matrix, YuvMatrix::Bt2020);
    assert!(img.color.bt2020_primaries && !img.color.full_range);
    // Tone-mapped, nothing is black or blown out over the whole test pattern.
    let px = img.to_rgba();
    let mean = px.iter().step_by(4).map(|v| u32::from(*v)).sum::<u32>() / (64 * 48);
    assert!((20..250).contains(&mean), "mean red {mean}");
    // Larger than the output size: scaled down in YUV (10-bit kept for HDR).
    let small = Sink { yuv: true, ..Default::default() };
    play(&rt, fixture("pq10-vp9.webm"), HwPref::Off, &small, 16);
    let seen = small.seen.lock().unwrap();
    let img = seen.yuv.first().expect("a YUV picture");
    assert_eq!((img.width, img.height, img.depth), (16, 12, 10));
    assert_eq!(img.color.transfer, Transfer::Pq);
}

/// With a device: hardware frames are copied to system memory and handed over as NV12
/// (the transfer format of D3D11VA / VAAPI / VideoToolbox for 8-bit content).
#[test]
fn hardware_frames_reach_the_yuv_sink_as_nv12() {
    let rt = rt!();
    let yuv = Sink { yuv: true, ..Default::default() };
    let (_p, s) = play(&rt, fixture("hw-mpeg2.mkv"), HwPref::Auto, &yuv, 4096);
    let seen = yuv.seen.lock().unwrap();
    let img = seen.yuv.first().expect("a YUV picture");
    assert_eq!((img.width, img.height), (128, 96));
    match s.hw {
        Some(_) => assert_eq!(img.layout, YuvLayout::Nv12, "{:?}", img.layout),
        None => {
            eprintln!("skipped the NV12 check: no hardware decoder on this machine");
            assert_eq!(img.layout, YuvLayout::I420);
        }
    }
    assert!(!img.color.full_range);
    assert_eq!(img.color.matrix, YuvMatrix::Bt601, "SD without colour tags: BT.601");
}
