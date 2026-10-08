//! Startup placeholder window (roadmap R5, `[appearance] startup_placeholder`): a native
//! window with the theme background and the logo, shown within a few tens of milliseconds
//! of process start while the GPU renderer initialises, and closed after the real window
//! has presented its first frame. The raw user32 / GDI calls live in
//! `sys/windows/splash.rs`; elsewhere this is a no-op.

/// Where the placeholder goes: the rectangle the real window will occupy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    /// Centered on the primary monitor with this inner size in points (eframe's
    /// `centered` placement).
    Centered { width: f32, height: f32 },
    /// The work area of the primary monitor (the window restores maximized).
    Maximized,
}

/// An open placeholder; [`Splash::close`] it once the real window is visible.
pub struct Splash {
    #[cfg(windows)]
    inner: crate::sys::windows::splash::Splash,
}

impl Splash {
    /// Show the placeholder: `bg` is the window background (RGB), `logo` an RGBA picture
    /// (straight alpha) drawn centered; `painted` runs right after the first paint.
    pub fn open(
        place: Place,
        bg: [u8; 3],
        logo: Option<(Vec<u8>, u32, u32)>,
        painted: Box<dyn FnOnce() + Send>,
    ) -> Option<Splash> {
        #[cfg(windows)]
        {
            use crate::sys::windows::splash as s;
            let place = match place {
                Place::Centered { width, height } => s::Place::Centered { width, height },
                Place::Maximized => s::Place::Maximized,
            };
            s::open(place, bg, logo, painted).map(|inner| Splash { inner })
        }
        #[cfg(not(windows))]
        {
            let _ = (place, bg, logo, painted);
            None
        }
    }

    /// Close the placeholder (any thread; idempotent).
    pub fn close(&self) {
        #[cfg(windows)]
        self.inner.close();
    }
}
