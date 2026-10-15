//! Hardware video decoding (V4b, `[media] hw_decode`).
//!
//! [`open_video_decoder`] tries the devices [`HwPref`] allows, in order: Direct3D 11
//! (D3D11VA) on Windows, VAAPI on Linux, VideoToolbox on macOS for `Auto`, the named one
//! for an explicit choice, none for `Off`. A device is used when the codec has a
//! hardware configuration for it (`avcodec_get_hw_config`), the device opens
//! (`av_hwdevice_ctx_create`, cached per process: a machine without the driver is
//! asked once) and the decoder opens with it attached; any failure falls through to the
//! next candidate and finally to the software decoder, silently (logged at debug level).
//!
//! Frames come back in the hardware format ([`HwActive::pix_fmt`]); the player copies
//! them to system memory (`av_hwframe_transfer_data`, NV12 / P010) before scaling or
//! handing the planes to a YUV sink. FFmpeg itself falls back to software formats when
//! its hwaccel refuses a stream (profile, size); a decoding error while the device is
//! active makes the player reopen the decoder in software at the current position.

use std::ffi::c_int;
use std::sync::{Arc, Mutex};

use crate::MediaError;
use crate::decode::{Decoder, HwDevice, hw_pix_fmt};
use crate::player::HwPref;
use crate::sys::Av;
use crate::sys::layout::{AVCodecParameters, AVRational};
use crate::sys::types::{AV_HWDEVICE_TYPE_D3D11VA, AV_HWDEVICE_TYPE_VAAPI, AV_HWDEVICE_TYPE_VIDEOTOOLBOX, AVCodec};

/// A hardware decoding API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HwApi {
    D3d11va,
    Vaapi,
    Videotoolbox,
}

impl HwApi {
    /// The name shown in the player status (`PlayerState::hw`) and logs.
    pub fn name(self) -> &'static str {
        match self {
            HwApi::D3d11va => "d3d11va",
            HwApi::Vaapi => "vaapi",
            HwApi::Videotoolbox => "videotoolbox",
        }
    }

    fn device_type(self) -> c_int {
        match self {
            HwApi::D3d11va => AV_HWDEVICE_TYPE_D3D11VA,
            HwApi::Vaapi => AV_HWDEVICE_TYPE_VAAPI,
            HwApi::Videotoolbox => AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
        }
    }

    /// The platform's API for `Auto`.
    pub fn native() -> Option<HwApi> {
        if cfg!(windows) {
            Some(HwApi::D3d11va)
        } else if cfg!(target_os = "macos") {
            Some(HwApi::Videotoolbox)
        } else if cfg!(target_os = "linux") {
            Some(HwApi::Vaapi)
        } else {
            None
        }
    }
}

/// The APIs to try for a preference, in order.
pub fn candidates(pref: HwPref) -> Vec<HwApi> {
    match pref {
        HwPref::Auto => HwApi::native().into_iter().collect(),
        HwPref::Off => Vec::new(),
        HwPref::D3d11va => vec![HwApi::D3d11va],
        HwPref::Vaapi => vec![HwApi::Vaapi],
        HwPref::Videotoolbox => vec![HwApi::Videotoolbox],
    }
}

/// The decoder opened with a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HwActive {
    pub api: HwApi,
    /// The hardware pixel format frames come in (`AV_PIX_FMT_D3D11`, `_VAAPI`, …).
    pub pix_fmt: c_int,
}

/// Devices opened so far (`None`: opening failed; not retried in this process).
static DEVICES: Mutex<Vec<(HwApi, Option<Arc<HwDevice>>)>> = Mutex::new(Vec::new());

fn device(av: &'static Av, api: HwApi) -> Option<Arc<HwDevice>> {
    let mut cache = DEVICES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, d)) = cache.iter().find(|(a, _)| *a == api) {
        return d.clone();
    }
    let d = match HwDevice::create(av, api.device_type()) {
        Ok(d) => Some(Arc::new(d)),
        Err(e) => {
            log::info!("soyuz-media: no {} device ({e}); decoding in software", api.name());
            None
        }
    };
    cache.push((api, d.clone()));
    d
}

/// Open a video decoder, with a hardware device when `pref` allows one that works for
/// this codec; software otherwise.
pub(crate) fn open_video_decoder(
    av: &'static Av,
    codec: *const AVCodec,
    par: *const AVCodecParameters,
    time_base: AVRational,
    options: &[(&str, &str)],
    pref: HwPref,
) -> Result<(Decoder, Option<HwActive>), MediaError> {
    for api in candidates(pref) {
        let Some(pix_fmt) = hw_pix_fmt(av, codec, api.device_type()) else { continue };
        let Some(dev) = device(av, api) else { continue };
        match Decoder::open_hw(av, codec, par, time_base, options, &dev) {
            Ok(d) => return Ok((d, Some(HwActive { api, pix_fmt }))),
            Err(e) => log::debug!("soyuz-media: {} decoder did not open ({e}); trying software", api.name()),
        }
    }
    Ok((Decoder::open(av, codec, par, time_base, options)?, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_pick_the_platform_api() {
        assert!(candidates(HwPref::Off).is_empty());
        assert_eq!(candidates(HwPref::Vaapi), vec![HwApi::Vaapi]);
        assert_eq!(candidates(HwPref::D3d11va), vec![HwApi::D3d11va]);
        assert_eq!(candidates(HwPref::Videotoolbox), vec![HwApi::Videotoolbox]);
        let auto = candidates(HwPref::Auto);
        if cfg!(windows) {
            assert_eq!(auto, vec![HwApi::D3d11va]);
        } else if cfg!(target_os = "macos") {
            assert_eq!(auto, vec![HwApi::Videotoolbox]);
        } else if cfg!(target_os = "linux") {
            assert_eq!(auto, vec![HwApi::Vaapi]);
        }
        assert_eq!(HwApi::D3d11va.name(), "d3d11va");
        assert_eq!(HwApi::D3d11va.device_type(), 7);
        assert_eq!(HwApi::Vaapi.device_type(), 3);
        assert_eq!(HwApi::Videotoolbox.device_type(), 6);
    }
}
