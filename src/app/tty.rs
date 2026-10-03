//! THE terminal door of `ae app`: raw mode, the alternate screen and the
//! window size of this process's own terminal, through rustix's SAFE termios
//! (no libc, no `unsafe`). clippy.toml denies every `rustix::termios` entry
//! point outside this file.
//!
//! The terminal is put back on every way out ae controls: [`Tty`]'s `Drop`,
//! and a panic hook that runs before the release profile's `panic = "abort"`.
//! A SIGKILL or SIGTERM leaves it raw; in tmux the pane goes with the process.
//! The screen sequences go to a CLONE of stdout's descriptor, never through
//! `std::io::stdout()`: a panic on another thread while the caller holds the
//! stdout lock would otherwise wait on that lock forever.

use std::fs::File;
use std::io::Write as _;
use std::os::fd::{AsFd as _, OwnedFd};

use rustix::termios::{self, OptionalActions, OutputModes, Termios};

/// Into the app's screen: alternate screen, cursor hidden, bracketed paste on.
const ENTER: &str = "\x1b[?1049h\x1b[?25l\x1b[?2004h\x1b[2J";
/// Back out: attributes reset, bracketed paste off, cursor shown, main screen.
const LEAVE: &str = "\x1b[0m\x1b[?2004l\x1b[?25h\x1b[?1049l";

/// This process's terminal in raw mode, until dropped.
pub(crate) struct Tty {
    fd: OwnedFd,
    screen: File,
    saved: Termios,
}

impl Tty {
    /// Raw mode on stdin's terminal, the app's screen on stdout. `Err` names
    /// why there is none.
    #[allow(
        clippy::disallowed_methods,
        reason = "THE terminal door: raw mode and its restore for ae app (clippy.toml)"
    )]
    pub(crate) fn start() -> Result<Self, String> {
        let fd = std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .map_err(|err| format!("stdin cannot be held ({err})"))?;
        let screen = std::io::stdout()
            .as_fd()
            .try_clone_to_owned()
            .map(File::from)
            .map_err(|err| format!("stdout cannot be held ({err})"))?;
        let saved = termios::tcgetattr(&fd).map_err(|err| format!("no terminal mode ({err})"))?;
        let mut raw = saved.clone();
        raw.make_raw();
        // Raw input, cooked output: a newline still returns the carriage.
        raw.output_modes.insert(OutputModes::OPOST);
        termios::tcsetattr(&fd, OptionalActions::Now, &raw)
            .map_err(|err| format!("raw mode refused ({err})"))?;
        let hook = (fd.try_clone(), screen.try_clone(), saved.clone());
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let (Ok(fd), Ok(screen), saved) = &hook {
                restore(fd, screen, saved);
            }
            previous(info);
        }));
        let _ = (&screen).write_all(ENTER.as_bytes());
        Ok(Self { fd, screen, saved })
    }

    /// The window size, columns then rows; `None` when the terminal will not say.
    #[allow(
        clippy::disallowed_methods,
        reason = "THE terminal door: the window size ae app draws in (clippy.toml)"
    )]
    pub(crate) fn size(&self) -> Option<(u16, u16)> {
        let size = termios::tcgetwinsize(&self.fd).ok()?;
        (size.ws_col > 0 && size.ws_row > 0).then_some((size.ws_col, size.ws_row))
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        restore(&self.fd, &self.screen, &self.saved);
    }
}

/// The main screen and the mode the terminal had before.
#[allow(
    clippy::disallowed_methods,
    reason = "THE terminal door: the saved mode put back (clippy.toml)"
)]
fn restore(fd: &OwnedFd, mut screen: &File, saved: &Termios) {
    let _ = screen.write_all(LEAVE.as_bytes());
    let _ = termios::tcsetattr(fd, OptionalActions::Now, saved);
}
