//! A PTY wrapper that lets the raw shell output be observed before alacritty's parser
//! sees it. alacritty_terminal's `EventLoop` reads the PTY itself and its parser drops
//! OSC 7 (the cwd that fish, `vte.sh`, oh-my-zsh, … report), so [`TeePty`] sits between
//! the two: it forwards everything to the real [`Pty`] and, in `read`, feeds the bytes
//! to an [`Osc7Scanner`]. A reported directory updates the shared cwd immediately; the
//! title hook keeps working, and whichever report is processed last wins (the tee sees
//! a chunk before the parser does, so within one read an OSC 7 is applied first).
//!
//! Works for both PTY flavours (openpty on unix, ConPTY on Windows): everything goes
//! through alacritty's `EventedReadWrite` trait. The only `unsafe` in the crate is the
//! forwarding of the `unsafe fn register` contract below.

use std::io::{self, Read};
use std::sync::Arc;

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Pty};
use polling::{Event, PollMode, Poller};

use crate::backend::Shared;
use crate::osc7::Osc7Scanner;

/// The real PTY plus an OSC 7 scanner on its output.
pub(crate) struct TeePty {
    inner: Pty,
    scanner: Osc7Scanner,
    shared: Arc<Shared>,
}

impl TeePty {
    pub fn new(inner: Pty, shared: Arc<Shared>) -> TeePty {
        TeePty { inner, scanner: Osc7Scanner::new(None), shared }
    }
}

/// The event loop reads through `reader()`, which hands out the wrapper itself: each
/// read goes to the inner PTY reader, and whatever came back is scanned. Errors
/// (including `WouldBlock`) pass through unchanged.
impl Read for TeePty {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.reader().read(buf)?;
        if let Some(dir) = self.scanner.feed(&buf[..n]) {
            self.shared.report_cwd(dir);
        }
        Ok(n)
    }
}

impl EventedReadWrite for TeePty {
    type Reader = TeePty;
    type Writer = <Pty as EventedReadWrite>::Writer;

    /// # Safety
    ///
    /// Same contract as the inner `register`: the registered sources must outlive their
    /// registration in `poll`.
    unsafe fn register(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        // SAFETY: the sources being registered are owned by `self.inner`, which lives
        // exactly as long as this wrapper; the caller (alacritty's event loop) upholds
        // the outlive-the-registration contract for `self`, and therefore for `inner`.
        // The event loop deregisters before dropping the PTY, as it would unwrapped.
        unsafe { self.inner.register(poll, interest, mode) }
    }

    fn reregister(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        self.inner.reregister(poll, interest, mode)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.inner.deregister(poll)
    }

    fn reader(&mut self) -> &mut TeePty {
        self
    }

    fn writer(&mut self) -> &mut Self::Writer {
        self.inner.writer()
    }
}

impl EventedPty for TeePty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.inner.next_child_event()
    }
}

impl OnResize for TeePty {
    fn on_resize(&mut self, window_size: WindowSize) {
        self.inner.on_resize(window_size);
    }
}
