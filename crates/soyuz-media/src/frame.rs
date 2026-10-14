//! One decoded picture ([`grab_frame`]): poster frames and thumbnails.

use crate::info::Source;
use crate::{MediaError, Runtime};

/// Which frame, and how big.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameRequest {
    /// Seconds from the start.
    pub at: f64,
    /// Longest edge of the result (the frame is scaled down to fit, never up).
    pub max_px: u32,
    /// `true`: the first frame at or after `at` (decode from the previous keyframe);
    /// `false`: the nearest keyframe before `at` (`skip_frame = nonkey`, one decoder
    /// thread; what thumbnails use).
    pub exact: bool,
}

impl FrameRequest {
    /// A keyframe near `at`, fitted into `max_px` (thumbnails, posters).
    pub fn near(at: f64, max_px: u32) -> FrameRequest {
        FrameRequest { at, max_px, exact: false }
    }
}

/// Decode one frame of the first video stream of `src` to RGBA, rotated as the stream's
/// display matrix says and fitted into `req.max_px`. `stale` is polled between packets;
/// `true` stops the work with [`MediaError::Stale`].
///
/// V0 stub: [`MediaError::NotLoaded`].
pub fn grab_frame(
    rt: &Runtime,
    src: &Source,
    req: FrameRequest,
    stale: &dyn Fn() -> bool,
) -> Result<image::RgbaImage, MediaError> {
    let _ = (rt, src, req, stale);
    Err(MediaError::NotLoaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_helpers() {
        let r = FrameRequest::near(1.5, 256);
        assert_eq!(r, FrameRequest { at: 1.5, max_px: 256, exact: false });
    }
}
