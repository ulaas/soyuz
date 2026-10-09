//! Startup placeholder window (roadmap R5, `[appearance] startup_placeholder`): a native
//! window with the theme background and the logo, shown within a few tens of milliseconds
//! of process start while the GPU renderer initialises, and closed after the real window
//! has presented its first frame. With `[performance] cpu_first_frame` it also shows the
//! real UI rendered on the CPU ([`Splash::present`]) and reports pointer input
//! ([`Splash::take_input`]) until the GPU window takes over. The raw user32 / GDI calls
//! live in `sys/windows/splash.rs`; elsewhere this is a no-op.

/// Where the placeholder goes: the rectangle the real window will occupy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    /// Centered on the primary monitor with this inner size in points (eframe's
    /// `centered` placement).
    Centered { width: f32, height: f32 },
    /// The work area of the primary monitor (the window restores maximized).
    Maximized,
    /// At this position and inner size in points (`SOYUZ_WINDOW_POS`, e.g. off-screen).
    At { x: f32, y: f32, width: f32, height: f32 },
}

/// Pointer input over a placeholder that shows UI frames (client pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    Move {
        x: i32,
        y: i32,
    },
    /// `button`: 0 = primary, 1 = secondary, 2 = middle.
    Button {
        x: i32,
        y: i32,
        button: u8,
        down: bool,
    },
    /// Wheel notches (positive = away from the user).
    Wheel {
        notches: f32,
    },
}

/// An open placeholder; [`Splash::close`] it once the real window is visible.
pub struct Splash {
    #[cfg(windows)]
    inner: crate::sys::windows::splash::Splash,
}

impl Splash {
    /// Show the placeholder: `bg` is the window background (RGB), `logo` an RGBA picture
    /// (straight alpha) drawn centered; `painted` runs right after the first paint.
    /// `frames`: UI frames will be presented, so the placeholder takes the real window's
    /// exact client rectangle (otherwise it is inset by a few pixels).
    pub fn open(
        place: Place,
        bg: [u8; 3],
        logo: Option<(Vec<u8>, u32, u32)>,
        painted: Box<dyn FnOnce() + Send>,
        frames: bool,
    ) -> Option<Splash> {
        #[cfg(windows)]
        {
            use crate::sys::windows::splash as s;
            let place = match place {
                Place::Centered { width, height } => s::Place::Centered { width, height },
                Place::Maximized => s::Place::Maximized,
                Place::At { x, y, width, height } => s::Place::At { x, y, width, height },
            };
            s::open(place, bg, logo, painted, frames).map(|inner| Splash { inner })
        }
        #[cfg(not(windows))]
        {
            let _ = (place, bg, logo, painted, frames);
            None
        }
    }

    /// Client size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        #[cfg(windows)]
        {
            (self.inner.size.0.max(0) as u32, self.inner.size.1.max(0) as u32)
        }
        #[cfg(not(windows))]
        (0, 0)
    }

    /// Scale factor of the monitor (DPI / 96).
    pub fn scale(&self) -> f32 {
        #[cfg(windows)]
        {
            self.inner.scale
        }
        #[cfg(not(windows))]
        1.0
    }

    /// Show a full client-area frame (`w × h` BGRA, top-down) instead of the logo;
    /// `painted` runs (on the placeholder thread) once it has been drawn.
    pub fn present(&self, bgra: Vec<u8>, w: u32, h: u32, painted: Option<Box<dyn FnOnce() + Send>>) {
        #[cfg(windows)]
        self.inner.present(bgra, w as i32, h as i32, painted);
        #[cfg(not(windows))]
        let _ = (bgra, w, h, painted);
    }

    /// Pointer input received since the last call (frames mode).
    pub fn take_input(&self) -> Vec<Input> {
        #[cfg(windows)]
        {
            self.inner.take_input()
        }
        #[cfg(not(windows))]
        Vec::new()
    }

    /// Close the placeholder (any thread; idempotent).
    pub fn close(&self) {
        #[cfg(windows)]
        self.inner.close();
    }
}
