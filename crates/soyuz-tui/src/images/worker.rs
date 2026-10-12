//! Graphics-protocol encoding (sixel, kitty, iTerm2) off the UI thread: one worker
//! thread per UI thread, started on first use, wakes the frame loop after each image.

use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use image::{DynamicImage, RgbaImage};
use ratatui::layout::Size;
use ratatui_image::Resize;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use soyuz_appcore::host::Wake;

use super::Key;

/// One image to encode.
pub struct Job {
    pub key: Key,
    pub img: Arc<RgbaImage>,
    /// Pixel size to scale to (fits `cells` at the picker's font size).
    pub px: (u32, u32),
    pub cells: Size,
    pub picker: Picker,
}

/// An encoded image (`None`: the encoder failed; drawn as nothing).
pub struct Done {
    pub key: Key,
    pub proto: Option<Protocol>,
}

/// Scale and encode `job` (blocking; the worker thread and tests).
pub fn encode(job: &Job) -> Option<Protocol> {
    let (w, h) = (job.px.0.max(1), job.px.1.max(1));
    let img = if job.img.dimensions() == (w, h) {
        (*job.img).clone()
    } else {
        image::imageops::resize(&*job.img, w, h, image::imageops::FilterType::Triangle)
    };
    match job.picker.new_protocol(DynamicImage::ImageRgba8(img), job.cells, Resize::Fit(None)) {
        Ok(p) => Some(p),
        Err(e) => {
            log::warn!("tui: cannot encode an image: {e}");
            None
        }
    }
}

/// The worker thread's channels.
pub struct Worker {
    tx: Sender<Job>,
    rx: Receiver<Done>,
}

impl Worker {
    /// Start the thread; `wake` asks the frame loop for a frame after each result.
    pub fn spawn(wake: Wake) -> Option<Worker> {
        let (tx, jobs) = crossbeam_channel::unbounded::<Job>();
        let (done, rx) = crossbeam_channel::unbounded::<Done>();
        let spawned = std::thread::Builder::new().name("soyuz-tui-images".into()).spawn(move || {
            for job in jobs {
                let proto = encode(&job);
                if done.send(Done { key: job.key, proto }).is_err() {
                    break;
                }
                wake();
            }
        });
        match spawned {
            Ok(_) => Some(Worker { tx, rx }),
            Err(e) => {
                log::warn!("tui: cannot start the image encoder: {e}");
                None
            }
        }
    }

    /// Queue `job`; gives it back when the thread is gone.
    pub fn send(&self, job: Job) -> Result<(), Job> {
        self.tx.send(job).map_err(|e| e.into_inner())
    }

    /// Results so far.
    pub fn drain(&self) -> Vec<Done> {
        self.rx.try_iter().collect()
    }

    /// Wait up to `timeout` for one result (tests).
    pub fn wait(&self, timeout: std::time::Duration) -> Option<Done> {
        self.rx.recv_timeout(timeout).ok()
    }
}
