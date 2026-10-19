//! Paste a picture from the system clipboard (a screenshot, an image copied in a browser
//! or an editor) into a folder as a PNG file (`[behaviour] paste_images`).
//!
//! The paste reads the picture itself when the system clipboard holds no files
//! (`services::os_clipboard::with_paste`: Windows `PNG` / `CF_DIBV5` / `CF_DIB` through
//! raw user32, Linux an `image/*` type of the native selection; [`from_clipboard`]), so an
//! image never wins over copied files. A frontend whose toolkit already delivers image
//! pastes can hand one over with [`offer`] in the frame its Ctrl+V arrives; [`take`] gives
//! it to the paste only while fresh, so an image that came with a key press handled
//! elsewhere does not turn up on a later paste.
//!
//! Encoding runs on a worker thread into a temporary file; the file then goes into the
//! folder through a `Create` job copying it (`CreateKind::File { template }`), so the
//! paste shows in the activity panel, follows the conflict policy and can be undone. The
//! new file is selected and its name opened for editing, like a new file.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use soyuz_jobs::{CreateKind, JobKind};

use crate::core::AppCore;
use crate::models::toasts::Toast;
use crate::services::jobs::{AfterJob, JobMeta, OpKind};
use crate::services::mailbox::{Postman, UiMsg};
use crate::state::pane::PaneId;

/// An offered image older than this is not used (its key press was handled elsewhere).
const FRESH: Duration = Duration::from_secs(2);
/// Temporary PNGs older than this are removed when the next image is pasted.
const KEEP_TEMP: Duration = Duration::from_secs(60 * 60);

/// Straight-alpha RGBA rows, produced on the worker thread (the frontend's own pixel
/// format is converted there, not on the UI thread).
pub type Pixels = Box<dyn FnOnce() -> Vec<u8> + Send>;

/// An image from the clipboard, waiting for this frame's Paste.
pub struct PastedImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
    at: Instant,
}

impl std::fmt::Debug for PastedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PastedImage").field("width", &self.width).field("height", &self.height).finish()
    }
}

/// The frontend got an image paste (`width` × `height`, `pixels` straight-alpha RGBA).
pub fn offer(app: &mut AppCore, width: u32, height: u32, pixels: Pixels) {
    if width == 0 || height == 0 {
        return;
    }
    app.fileops.pasted_image = Some(PastedImage { width, height, pixels, at: Instant::now() });
}

/// The image offered for this paste, if it is fresh and `[behaviour] paste_images` is on.
pub fn take(app: &mut AppCore) -> Option<PastedImage> {
    let img = app.fileops.pasted_image.take()?;
    (app.config.behaviour.paste_images && img.at.elapsed() < FRESH).then_some(img)
}

/// The picture read from the system clipboard by the paste itself (decoded on the
/// encoder thread), when `[behaviour] paste_images` is on.
pub fn from_clipboard(app: &AppCore, img: crate::services::os_clipboard::ClipImage) -> Option<PastedImage> {
    app.config.behaviour.paste_images.then(|| PastedImage {
        width: img.width,
        height: img.height,
        pixels: Box::new(move || img.decode()),
        at: Instant::now(),
    })
}

/// The file name for an image pasted at `now` (`Pasted image 2026-10-16 143012.png`).
pub fn file_name(now: chrono::DateTime<chrono::Local>) -> String {
    let stamp = now.format("%Y-%m-%d %H%M%S").to_string();
    format!("{}.png", tr!("ops-pasted-image-name", stamp = stamp))
}

/// Save `img` into `dest`: encode on a worker, then [`UiMsg::CreateFrom`] submits the job.
pub fn save(app: &mut AppCore, img: PastedImage, dest: PathBuf) {
    let pane = app.ws.pane();
    let names: Vec<&str> = if pane.location.as_fs_path() == Some(dest.as_path()) {
        pane.entries.iter().map(|e| e.name.as_str()).collect()
    } else {
        Vec::new()
    };
    let name = super::names::unique_in(&names, &file_name(chrono::Local::now()), super::names::case_insensitive_fs());
    let job = PasteJob { pane: pane.id, path: dest.join(name), postman: app.services.mailbox.postman() };
    let spawned = std::thread::Builder::new().name("soyuz-paste-image".into()).spawn(move || job.run(img));
    if let Err(e) = spawned {
        log::warn!("cannot spawn the image encoder: {e}");
    }
}

struct PasteJob {
    pane: PaneId,
    path: PathBuf,
    postman: Postman,
}

impl PasteJob {
    fn run(self, img: PastedImage) {
        let dir = temp_dir();
        sweep(&dir);
        let template = dir.join(temp_name());
        let encoded = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|()| encode(img, &template));
        let msg = match encoded {
            Ok(()) => UiMsg::CreateFrom { pane: self.pane, path: self.path, template },
            Err(e) => UiMsg::Notice(Toast::error(tr!("ops-paste-image-failed")).body(e)),
        };
        self.postman.send(msg);
    }
}

/// Write `img` as a PNG to `out`.
fn encode(img: PastedImage, out: &Path) -> Result<(), String> {
    let PastedImage { width, height, pixels, .. } = img;
    let buf = image::RgbaImage::from_raw(width, height, pixels()).ok_or_else(|| "bad image size".to_string())?;
    buf.save_with_format(out, image::ImageFormat::Png).map_err(|e| e.to_string())
}

fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("soyuz-pasted-images")
}

fn temp_name() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{}-{}.png", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

/// Remove temporary PNGs older than [`KEEP_TEMP`] (their jobs finished long ago).
fn sweep(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|a| a > KEEP_TEMP));
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// [`UiMsg::CreateFrom`]: copy the encoded `template` to `path` as a new file, then
/// select it with its name open for editing.
pub fn create_from(app: &mut AppCore, pane: PaneId, path: PathBuf, template: PathBuf) {
    let kind = JobKind::Create { path: path.clone(), kind: CreateKind::File { template: Some(template) } };
    let spec = app.fileops.jobs.spec(kind);
    let meta = JobMeta::new(OpKind::NewFile, std::slice::from_ref(&path), path.parent().map(Path::to_path_buf))
        .after(AfterJob::SelectAndRename { pane, path })
        .quiet();
    app.fileops.jobs.submit(spec, meta);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    #[test]
    fn pasted_images_are_named_by_date_and_time() {
        let at = chrono::Local.with_ymd_and_hms(2026, 10, 16, 14, 30, 12).unwrap();
        assert_eq!(file_name(at), "Pasted image 2026-10-16 143012.png");
    }
}
