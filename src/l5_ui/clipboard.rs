//! Clipboard text writes and bounded image reads for both input surfaces.
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::console::open;
use crate::inventory::ServerId;
use crate::{board::terminal_text, transport};

pub(crate) const EMPTY_DRAFT: &str = "nothing to copy: the draft is empty";

/// The most UTF-8 bytes written to a terminal clipboard outside tmux.
const TEXT_MAX: usize = 65536;

/// How a copy names the text it wrote.
pub(crate) fn lines_word(text: &str) -> String {
    match text.lines().count() {
        1 => "1 line".to_owned(),
        lines => format!("{lines} lines"),
    }
}

/// Keep the text for the next terminal write if it fits whole.
pub(crate) fn stage_text(slot: &mut Option<String>, text: &str, count: &str) -> String {
    if text.len() > TEXT_MAX {
        let size = text.len();
        return format!("not copied: {size} bytes is over the {TEXT_MAX}-byte clipboard limit");
    }
    *slot = Some(text.to_owned());
    format!("sent {count} to the terminal clipboard - unconfirmed")
}

/// Load the tmux paste buffer, also asking tmux to copy to the terminal when
/// exactly one client is the newest viewer of `pane`.
pub(crate) fn ship_tmux(
    server: &ServerId,
    pane: &str,
    surface: &str,
    text: &str,
    count: &str,
) -> String {
    let viewer = match transport::observe_clients(server) {
        None => Err("the tmux clients did not answer".to_owned()),
        Some(clients) => open::copy_client(&clients, pane).map_err(|why| why.why_for(surface)),
    }
    .and_then(|name| {
        open::client_name(&name)
            .then_some(name)
            .ok_or_else(|| "the client name failed its grammar".to_owned())
    });
    match (
        transport::copy_buffer(server, viewer.as_deref().ok(), text.as_bytes()),
        viewer,
    ) {
        (Err(why), _) => format!("copy failed: {}", terminal_text(&why)),
        (Ok(()), Ok(_)) => format!(
            "copied {count} to the tmux buffer; terminal clipboard if your terminal allows it"
        ),
        (Ok(()), Err(why)) => format!("copied {count} to the tmux buffer only - {why}"),
    }
}

/// An OSC 52 clipboard write. Base64 keeps payload bytes inert.
pub(crate) fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// `bytes` as RFC 4648 base64, padded.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let word = (group.iter()).fold(0_u32, |word, byte| (word << 8) | u32::from(*byte));
        let word = word << (8 * (3 - group.len()));
        for place in 0..=group.len() {
            let digit = usize::try_from((word >> (18 - 6 * place)) & 63).unwrap_or(0);
            out.push(char::from(DIGITS[digit]));
        }
        out.extend(std::iter::repeat_n('=', 3 - group.len()));
    }
    out
}
type Answer = Result<PathBuf, String>;
pub(crate) type Job<T> = Option<(T, Receiver<Answer>)>;

pub(crate) fn start<T>(job: &mut Job<T>, token: T, dir: PathBuf) {
    if job.is_some() {
        return;
    }
    let (send, recv) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(fetch(&dir));
    });
    *job = Some((token, recv));
}

/// Consume once; stale entries leave the inert image and change no draft.
pub(crate) fn poll<T: PartialEq>(job: &mut Job<T>, token: Option<&T>) -> Option<Answer> {
    let (bound, recv) = job.as_ref()?;
    let answer = match recv.try_recv() {
        Ok(answer) => answer,
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => Err("clipboard tool failed".to_owned()),
    };
    let current = Some(bound) == token;
    *job = None;
    current.then_some(answer)
}

pub(crate) fn finish(
    input: &mut crate::console::input::Input,
    answer: Answer,
) -> Result<(), String> {
    let path = answer?;
    let inserted = input.image_path(&path.to_string_lossy());
    if inserted.is_err() {
        let _ = std::fs::remove_file(path);
    }
    inserted
}

/// Remove the images a draft saved and no longer holds a token of. A send
/// transfers an image to the history instead, and a missing file is no error.
pub(crate) fn sweep(input: &mut crate::console::input::Input) {
    for path in input.take_released() {
        let _ = std::fs::remove_file(path);
    }
}

fn fetch(dir: &Path) -> Answer {
    let bytes = crate::transport::run_clipboard_image()?;
    if bytes.is_empty() {
        return Err("no image on the clipboard".to_owned());
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("clipboard image is not a PNG".to_owned());
    }
    let messages = image_dir(dir, &crate::doors::cwd())?;
    let name = format!(
        "img-{}-{:016x}.png",
        crate::time::Timestamp::now().epoch(),
        crate::entropy()
    );
    let path = messages.join(name);
    std::fs::create_dir_all(messages)
        .and_then(|()| crate::init::create_exclusive(&path, &bytes, 0o600))
        .map_err(|_| "could not save the clipboard image".to_owned())?;
    Ok(path)
}

fn image_dir(dir: &Path, cwd: &Path) -> Result<PathBuf, String> {
    let messages = cwd.join(dir).join("messages");
    if !messages.is_absolute() {
        return Err("could not save the clipboard image".to_owned());
    }
    messages
        .to_str()
        .ok_or("could not save the clipboard image")?;
    Ok(messages)
}

#[cfg(test)]
mod tests {
    #[test]
    fn clipboard_non_utf8_destination_is_refused_instead_of_inserting_a_wrong_path() {
        use std::os::unix::ffi::OsStringExt as _;
        let invalid =
            std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/image-\xff".to_vec()));
        let relative = std::path::Path::new("relative");
        for dir in [invalid.as_path(), relative] {
            assert_eq!(
                super::image_dir(dir, std::path::Path::new(".")),
                Err("could not save the clipboard image".to_owned())
            );
        }
        assert!(
            super::image_dir(relative, std::path::Path::new("/tmp"))
                .unwrap()
                .is_absolute()
        );
    }
    #[test]
    fn clipboard_completion_consumes_once_and_drops_a_stale_entry() {
        for current in [Some(&1), Some(&2), None] {
            let (send, recv) = std::sync::mpsc::channel();
            let mut job = Some((1, recv));
            assert!(super::poll(&mut job, current).is_none());
            send.send(Ok(std::path::PathBuf::from("/img.png"))).unwrap();
            let result = super::poll(&mut job, current);
            assert_eq!(result.is_some(), current == Some(&1));
            assert!(job.is_none());
            assert!(super::poll(&mut job, current).is_none());
        }
    }
}
