//! One bounded image fetch, shared by the two input surfaces.
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

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
