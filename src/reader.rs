//! `_reader [--client <name>]` — the pane reader toggle.
//!
//! One reader per window: a DETACHED pane above the source showing a FROZEN
//! snapshot of the source's screen and history in copy mode, so the keyboard
//! never leaves the source's input box and the mouse wheel scrolls the reader.
//! The same call closes it. The no-select mode-table wheel map that makes the
//! reader mouse-scrollable belongs to the server-global input map
//! ([`crate::session_tmux::status_bindings_argv`]), asserted by a launch and
//! an upgrade; a `pane-mode-changed` hook stamped on the reader closes it when
//! `q` or Escape ends its mode instead of leaving a spare shell.
//!
//! A status binding runs this with `--client <name>`; a shell inside a pane
//! may run it with `$TMUX_PANE` instead. The toggle never guesses: an
//! unstamped pane is not a reader, and a reader is found only by its stamp in
//! the calling pane's own window.

use std::io::Write;

use crate::inventory::ServerId;
use crate::session_tmux::{Op, argv, interpret_pane_id};
use crate::tmux::OptionScope;
use crate::transport;

/// The reader pane's share of its source's cell.
const READER_SIZE: &str = "60%";

/// The pane option naming the source a reader pane shows.
pub(crate) const READER_SOURCE_OPTION: &str = "@ae_reader_src";

const USAGE: &str = "usage: ae _reader [--client <name>]";

/// The reader verb: parse, resolve, toggle.
pub(crate) fn run(
    tail: &[String],
    _out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let client = match tail {
        [] => None,
        [flag, name] if flag == "--client" => Some(name.clone()),
        _ => {
            writeln!(err, "{USAGE}")?;
            return Ok(crate::entry::EXIT_USAGE);
        }
    };
    let declared = crate::doors::declared_server(crate::shape::current());
    let Some(server) = crate::doors::launch_target(declared.as_ref()) else {
        writeln!(
            err,
            "ae: the tmux server pair is ambiguous, so ae cannot place a reader."
        )?;
        return Ok(crate::entry::EXIT_FAILED);
    };
    let Some(source) = source_pane(&server, client.as_deref()) else {
        writeln!(
            err,
            "ae: no calling pane to read: a status binding supplies --client, a shell its TMUX_PANE."
        )?;
        return Ok(crate::entry::EXIT_FAILED);
    };
    let Some(session) = session_of(&server, &source) else {
        writeln!(err, "ae: pane {source} is not on this tmux server.")?;
        return Ok(crate::entry::EXIT_FAILED);
    };
    let Some(panes) = transport::observe_window_panes(&server, &session) else {
        writeln!(err, "ae: tmux did not answer the pane listing.")?;
        return Ok(crate::entry::EXIT_FAILED);
    };
    let Some(window) = panes
        .iter()
        .find(|pane| pane.pane_id == source)
        .map(|pane| pane.window_id.clone())
    else {
        writeln!(err, "ae: pane {source} is not in a window ae can read.")?;
        return Ok(crate::entry::EXIT_FAILED);
    };
    // The window's reader, whatever source it was opened for. The SAME source
    // toggles it off; a different LIVE source retargets the one reader; a
    // source that is gone (the reader outlived it) also closes — the calling
    // pane is then the reader itself, and the toggle never guesses a new one.
    if let Some(existing) = panes
        .iter()
        .find(|pane| pane.window_id == window && pane.reader_src.is_some())
    {
        let stamp = existing.reader_src.clone().unwrap_or_default();
        let source_live = panes.iter().any(|pane| pane.pane_id == stamp);
        if stamp == source || !source_live {
            return Ok(if transport::kill_pane(&server, &existing.pane_id) {
                0
            } else {
                writeln!(
                    err,
                    "ae: tmux refused to close the reader {}.",
                    existing.pane_id
                )?;
                crate::entry::EXIT_FAILED
            });
        }
        if !transport::kill_pane(&server, &existing.pane_id) {
            writeln!(
                err,
                "ae: tmux refused to close the reader {} before retargeting it.",
                existing.pane_id
            )?;
            return Ok(crate::entry::EXIT_FAILED);
        }
    }
    if open(&server, &source).is_none() {
        writeln!(err, "ae: tmux refused to open a reader for {source}.")?;
        return Ok(crate::entry::EXIT_FAILED);
    }
    Ok(0)
}

/// Split the reader above `source`, stamp it, enter the snapshot, then stamp
/// the close hook — in that order: the stamp is written BEFORE the mode so a
/// crash between the two leaves a findable pane, and the hook AFTER the entry
/// so its first fire is not the entry itself.
fn open(server: &ServerId, source: &str) -> Option<String> {
    let (succeeded, stdout) = transport::run_tmux_op(&argv(
        server,
        &Op::SplitReader {
            source,
            size: READER_SIZE,
        },
    ));
    let reader = interpret_pane_id(succeeded, &stdout)?;
    let _ = transport::publish_option(
        server,
        OptionScope::Pane,
        &reader,
        READER_SOURCE_OPTION,
        source,
    );
    let _ = transport::run_tmux_op(&argv(
        server,
        &Op::CopyModeFrom {
            source,
            target: &reader,
        },
    ));
    let _ = transport::run_tmux_op(&argv(server, &Op::SetReaderCloseHook { reader: &reader }));
    Some(reader)
}

/// The source pane: the explicit client's current pane, else `$TMUX_PANE`.
fn source_pane(server: &ServerId, client: Option<&str>) -> Option<String> {
    let Some(client) = client else {
        return crate::doors::calling_pane_id();
    };
    transport::observe_clients(server)?
        .into_iter()
        .find(|observed| observed.name == client)
        .map(|observed| observed.pane)
}

/// The session a pane belongs to, through the one server-wide listing that
/// already reads it.
fn session_of(server: &ServerId, pane: &str) -> Option<String> {
    transport::observe_fleet_panes(server)?
        .into_iter()
        .find(|observed| observed.pane == pane)
        .map(|observed| observed.session)
}
