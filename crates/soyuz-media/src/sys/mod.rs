//! The FFmpeg FFI: the only `unsafe` code of the crate (allow-listed in
//! `scripts/unsafe-allowlist.txt`; CI compares the list with the tree).
//!
//! - `loader.rs`: finds and loads the five libraries in dependency order (`avutil`,
//!   `swresample`, `swscale`, `avcodec`, `avformat`), each by absolute path. Windows uses
//!   `libloading::os::windows::Library::load_with_flags` with
//!   `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS` explicitly
//!   (`Library::new` passes no flags); our Linux build links with `-Wl,-rpath,'$ORIGIN'`
//!   and the macOS dylibs carry `@rpath` install names plus `@loader_path`. Checks the
//!   version functions against [`crate::Majors::SUPPORTED`] before any struct is read,
//!   binds the symbols and runs the self-test. Libraries are never unloaded.
//! - `symbols.rs`: one `extern "C"` function pointer per entry of [`SYMBOLS`], bound
//!   with `Library::get`. A missing symbol is [`crate::LoadError::Broken`].
//! - `layout.rs`: `#[repr(C)]` *prefix* structs (`AVFrame7` / `AVFrame8`, `AVStream`,
//!   `AVFormatContext`, …) and `core::mem::offset_of!` tables; checked against
//!   `third_party/ffmpeg/layout_probe.c` in CI. `AVFrame` is never read past
//!   `decode_error_flags` (7.1 has `pkt_size` after it); `AVStream` only up to
//!   `attached_pic` (7.1 has deprecated side-data fields after it); `AVCodecContext` is
//!   never read as a struct (AVOptions, plus the one `hw_device_ctx` offset in V4b).
//! - `types.rs`: constants (`AVMEDIA_TYPE_*`, `AVERROR_*`, flags) and opaque handles.
//! - `avio.rs`: custom `AVIOContext` read / seek callbacks for every [`crate::Source`]
//!   (files too, so no FFmpeg protocol parsing of user paths and `--disable-network`
//!   costs nothing); `stale` is polled on every read.
//! - `demux.rs`, `codec.rs`, `scale.rs`, `resample.rs`: RAII wrappers (`Demuxer`,
//!   `Packet`, `Decoder`, `Frame`, `Scaler`, `Resampler`) with safe methods; the safe
//!   modules of the crate use only these.
//! - `check.rs`: the load-time self-test (an in-memory WAV and a 2-frame rawvideo AVI
//!   from `crate::fixtures`).
//!
//! Rules for every `extern "C"` callback: the body is wrapped in
//! `std::panic::catch_unwind(AssertUnwindSafe(..))` and a panic becomes `AVERROR_EXTERNAL`
//! (a panic unwinding into C aborts the process). Every `unsafe` block carries a
//! `// SAFETY:` comment (`clippy::undocumented_unsafe_blocks` is forbidden crate-wide).
//! Limits are set before `avformat_open_input` (`probesize`, `analyzeduration`,
//! `max_streams`) and on every decoder (`max_pixels`, `err_detect`): file contents are
//! untrusted.

pub(crate) mod avio;
pub(crate) mod check;
pub(crate) mod codec;
pub(crate) mod demux;
pub(crate) mod layout;
pub(crate) mod loader;
pub(crate) mod resample;
pub(crate) mod scale;
pub(crate) mod symbols;
pub(crate) mod types;

use std::ffi::{CStr, CString, c_char, c_int, c_void};

pub(crate) use loader::{Av, cstr};

use crate::MediaError;

/// `MediaError::Av` with FFmpeg's text for `code`.
pub(crate) fn av_err(av: &Av, code: c_int) -> MediaError {
    let mut buf = [0 as c_char; 128];
    // SAFETY: `buf` is a writable buffer of the given size; `av_strerror` NUL-terminates.
    unsafe { (av.f.av_strerror)(code, buf.as_mut_ptr(), buf.len()) };
    // SAFETY: NUL-terminated by `av_strerror` (or all zeros).
    let what = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned();
    MediaError::Av { code, what }
}

/// Map a negative return value to an error.
pub(crate) fn ok(av: &Av, code: c_int) -> Result<c_int, MediaError> {
    if code < 0 { Err(av_err(av, code)) } else { Ok(code) }
}

/// A C string of `s` (interior NULs dropped).
pub(crate) fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).expect("no NUL left")
}

/// `av_opt_set(obj, name, val, 0)`.
pub(crate) fn opt_set(av: &Av, obj: *mut c_void, name: &str, val: &str) -> Result<(), MediaError> {
    let (n, v) = (cstring(name), cstring(val));
    // SAFETY: `obj` is an AVOptions-enabled object (a context) the caller owns.
    ok(av, unsafe { (av.f.av_opt_set)(obj, n.as_ptr(), v.as_ptr(), 0) }).map(drop)
}

/// `av_opt_set_int(obj, name, val, 0)`.
pub(crate) fn opt_set_int(av: &Av, obj: *mut c_void, name: &str, val: i64) -> Result<(), MediaError> {
    let n = cstring(name);
    // SAFETY: as in `opt_set`.
    ok(av, unsafe { (av.f.av_opt_set_int)(obj, n.as_ptr(), val, 0) }).map(drop)
}

/// `av_opt_set_q(obj, name, val, 0)`.
pub(crate) fn opt_set_q(av: &Av, obj: *mut c_void, name: &str, val: layout::AVRational) -> Result<(), MediaError> {
    let n = cstring(name);
    // SAFETY: as in `opt_set`.
    ok(av, unsafe { (av.f.av_opt_set_q)(obj, n.as_ptr(), val, 0) }).map(drop)
}

/// Every `(key, value)` of an `AVDictionary`, in order.
pub(crate) fn dict_entries(av: &Av, dict: *const types::AVDictionary) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if dict.is_null() {
        return out;
    }
    let mut prev: *const layout::AVDictionaryEntry = std::ptr::null();
    loop {
        // SAFETY: `dict` is a live dictionary owned by a context the caller holds; the
        // iteration protocol of `av_dict_get` with an empty key and IGNORE_SUFFIX.
        let e = unsafe { (av.f.av_dict_get)(dict, c"".as_ptr(), prev, types::AV_DICT_IGNORE_SUFFIX) };
        if e.is_null() {
            return out;
        }
        // SAFETY: a non-null entry has NUL-terminated key and value.
        let (k, v) = unsafe { (cstr((*e).key), cstr((*e).value)) };
        out.push((k.unwrap_or_default(), v.unwrap_or_default()));
        prev = e;
    }
}

/// One dictionary value by (case-insensitive) key.
pub(crate) fn dict_get(av: &Av, dict: *const types::AVDictionary, key: &str) -> Option<String> {
    if dict.is_null() {
        return None;
    }
    let k = cstring(key);
    // SAFETY: as in `dict_entries`.
    let e = unsafe { (av.f.av_dict_get)(dict, k.as_ptr(), std::ptr::null(), 0) };
    if e.is_null() {
        return None;
    }
    // SAFETY: a non-null entry has a NUL-terminated value.
    unsafe { cstr((*e).value) }
}
