//! Where the suite's scratch roots live, and how the Rust and lane reapers
//! bind a registered fixture to its owner's process instance.

#![allow(
    clippy::disallowed_methods,
    reason = "test scratch lives on the real filesystem; the boundary is about PRODUCT code"
)]

use std::path::{Component, Path, PathBuf};

/// The longest socket path tmux can bind: `sun_path` is 104 bytes on macOS,
/// NUL included.
pub(crate) const SOCKET_PATH_MAX: usize = 103;

/// `AE_TEST_TMPDIR`, else `/tmp` — never `$TMPDIR`, which macOS spells 49 bytes
/// long: a `-L` socket two levels beneath it outgrows [`SOCKET_PATH_MAX`].
pub(crate) fn base() -> PathBuf {
    std::env::var_os("AE_TEST_TMPDIR")
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
}

/// Seven characters encode the full start time, without its redundant weekday.
/// Match `short_start` in the lane. The short spelling leaves room for sockets
/// even when a fixture runs a second test process under its own scratch root.
fn short_start(identity: &str) -> Option<String> {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    if !matches!(
        identity.as_bytes(),
        [b'A'..=b'Z', b'a'..=b'z', b'a'..=b'z', ..]
    ) || !(17..=18).contains(&identity.len())
        || !identity.as_bytes().get(6..)?.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let clock = identity.len() - 10;
    let month = MONTHS
        .iter()
        .position(|month| identity.get(3..6) == Some(month))?;
    let day = identity.get(6..clock)?.parse::<usize>().ok()?;
    let hour = identity.get(clock..clock + 2)?.parse::<usize>().ok()?;
    let minute = identity.get(clock + 2..clock + 4)?.parse::<usize>().ok()?;
    let second = identity.get(clock + 4..clock + 6)?.parse::<usize>().ok()?;
    let year = identity.get(clock + 6..)?.parse::<usize>().ok()?;
    [year / 62, year % 62, month, day, hour, minute, second]
        .into_iter()
        .map(|index| DIGITS.get(index).copied().map(char::from))
        .collect()
}

pub(crate) fn owner_dir_name(pid: u32, identity: &str) -> Option<String> {
    Some(format!("ae{pid}.{}", short_start(identity)?))
}

pub(crate) fn own_dir() -> &'static Path {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let base = base();
        let Some(identity) = super::parity::capture::raw::own_identity(&base) else {
            panic!("scratch allocation needs the process start identity");
        };
        let Some(name) = owner_dir_name(std::process::id(), identity) else {
            panic!("invalid process start timestamp: {identity:?}");
        };
        base.join(name)
    })
}

pub(crate) enum ReapRoot<'a> {
    Owned(&'a Path),
    Legacy(&'a Path),
    External,
    Protected,
}

/// Match `owned_root` in the lane. Only a plain suite namespace with the
/// registered PID and start time may be reaped. The caller additionally
/// requires an absent PID for unqualified legacy scratch. An unmatched
/// namespace is protected from both deletion and socket cleanup.
pub(crate) fn registered_root<'a>(
    owner: u32,
    recorded: Option<&str>,
    target: &'a Path,
) -> ReapRoot<'a> {
    let base = base();
    let relative = target
        .strip_prefix(&base)
        .ok()
        .or_else(|| target.strip_prefix(std::fs::canonicalize(&base).ok()?).ok());
    let Some(relative) = relative else {
        return ReapRoot::External;
    };
    let mut components = relative.components();
    let Some(Component::Normal(name)) = components.next() else {
        return ReapRoot::External;
    };
    let Some(name) = name.to_str() else {
        return ReapRoot::External;
    };
    let pid = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let legacy = name.strip_prefix("ae-it-").is_some_and(pid);
    let qualified = name
        .strip_prefix("ae")
        .and_then(|name| name.split_once('.'))
        .is_some_and(|(owner, stamp)| {
            pid(owner) && stamp.len() == 7 && stamp.bytes().all(|byte| byte.is_ascii_alphanumeric())
        });
    if !legacy && !qualified {
        return ReapRoot::External;
    }
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return ReapRoot::Protected;
    }
    let Some(parent) = target.parent() else {
        return ReapRoot::Protected;
    };
    let plain = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir());
    let gone = std::fs::symlink_metadata(target)
        .is_err_and(|why| why.kind() == std::io::ErrorKind::NotFound);
    if !plain(parent) || !(gone || plain(target)) {
        return ReapRoot::Protected;
    }
    if recorded
        .and_then(|identity| owner_dir_name(owner, identity))
        .as_deref()
        == Some(name)
    {
        ReapRoot::Owned(parent)
    } else if name == format!("ae-it-{owner}") {
        ReapRoot::Legacy(parent)
    } else {
        ReapRoot::Protected
    }
}
