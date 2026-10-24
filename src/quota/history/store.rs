//! The history's filesystem door. Never follows data links or writes another session.
#![allow(
    clippy::disallowed_methods,
    reason = "bounded own-session history writer and read-only fleet door"
)]
use super::{Report, Row, Skipped, fold, parse, render, uuid};
use crate::time::Timestamp;
use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, File, Metadata, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

const FILE_CAP: u64 = 16 * 1024 * 1024;
const FLEET_CAP: u64 = 64 * 1024 * 1024;
// O_NONBLOCK | O_NOFOLLOW. macOS SDK sys/fcntl.h; Linux uapi:
// https://github.com/torvalds/linux/blob/master/include/uapi/asm-generic/fcntl.h
// arm64 overrides NOFOLLOW: https://github.com/torvalds/linux/blob/master/arch/arm64/include/uapi/asm/fcntl.h
#[cfg(target_os = "macos")]
const FLAGS: i32 = 0x0004 | 0x0100;
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const FLAGS: i32 = 0o4000 | 0o100_000;
#[cfg(all(target_os = "linux", not(target_arch = "aarch64")))]
const FLAGS: i32 = 0o4000 | 0o400_000;
fn refused() -> io::Error {
    io::Error::other("quota history node, identity or bound refused")
}
fn directory(path: &Path, private: bool) -> io::Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || (private && m.mode() & 0o077 != 0) {
        return Err(refused());
    }
    Ok(())
}
fn same(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino()
}
fn open(path: &Path, write: bool, create: bool, private: bool) -> io::Result<File> {
    let before = match fs::symlink_metadata(path) {
        Ok(m) => Some(m),
        Err(e) if create && e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    if before.as_ref().is_some_and(|m| {
        !m.is_file() || m.mode() & 0o400 == 0 || (private && m.mode() & 0o077 != 0)
    }) {
        return Err(refused());
    }
    let file = OpenOptions::new()
        .read(true)
        .append(write)
        .create(create)
        .mode(0o600)
        .custom_flags(FLAGS)
        .open(path)?;
    let held = file.metadata()?;
    let named = fs::symlink_metadata(path)?;
    if !held.is_file()
        || !named.is_file()
        || !same(&held, &named)
        || before.as_ref().is_some_and(|m| !same(m, &held))
        || (private && held.mode() & 0o077 != 0)
    {
        return Err(refused());
    }
    Ok(file)
}
fn session_id(dir: &Path) -> io::Result<String> {
    directory(dir, false)?;
    let file = open(&dir.join(crate::store::META), false, false, false)?;
    let mut text = String::new();
    file.take(1_048_577).read_to_string(&mut text)?;
    if text.len() > 1_048_576 {
        return Err(refused());
    }
    let mut ids = text
        .lines()
        .filter_map(|line| line.strip_prefix("session_id="));
    let id = ids.next().filter(|id| uuid(id)).ok_or_else(refused)?;
    if ids.next().is_some() {
        return Err(refused());
    }
    Ok(id.to_owned())
}
fn date(at: i64) -> String {
    Timestamp::from_epoch(at)
        .to_string()
        .get(..10)
        .unwrap_or_default()
        .to_owned()
}
fn date_name(name: &str) -> Option<&str> {
    let day = name.strip_suffix(".jsonl")?;
    (day.len() == 10 && Timestamp::parse(&format!("{day}T00:00:00Z")).is_some()).then_some(day)
}
fn entries(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let entries = fs::read_dir(dir)?
        .take(129)
        .map(|e| e.map(|e| e.path()))
        .collect::<io::Result<Vec<_>>>()?;
    if entries.len() > 128 {
        return Err(refused());
    }
    Ok(entries)
}
fn write_once(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    if file.write(bytes)? == bytes.len() {
        Ok(())
    } else {
        Err(io::ErrorKind::WriteZero.into())
    }
}
fn needs_seal(file: &mut File, len: u64) -> io::Result<bool> {
    let mut last = *b"\n";
    if len > 0 {
        file.seek(SeekFrom::End(-1))?;
        file.read_exact(&mut last)?;
    }
    Ok(last[0] != b'\n')
}
/// Append a bounded batch under the session's nonblocking private lock.
/// # Errors
/// Unsafe nodes, foreign incarnation, permissions, bounds, lock contention or I/O.
pub fn append(session_dir: &Path, rows: &[Row], now: i64) -> io::Result<()> {
    if rows.len() > 256 {
        return Err(io::Error::other("quota history batch exceeds 256 rows"));
    }
    if now <= 0 {
        return Err(refused());
    }
    directory(session_dir.parent().ok_or_else(refused)?, false)?;
    let id = session_id(session_dir)?;
    let mut batch: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::new();
    let mut bytes = 0;
    for row in rows {
        let (_, incarnation, at, _) = row.identity();
        let line = render(row);
        if incarnation != id || parse(&line).is_err() {
            return Err(refused());
        }
        bytes += line.len() + 1;
        if bytes > 1_048_576 {
            return Err(io::Error::other("quota history batch exceeds 1 MiB"));
        }
        let mut line = line.into_bytes();
        line.push(b'\n');
        batch
            .entry(format!("{}.jsonl", date(at)))
            .or_default()
            .push(line);
    }
    if rows.is_empty() {
        return Ok(());
    }
    let dir = session_dir.join("quota-history");
    match DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e),
    }
    directory(&dir, true)?;
    entries(&dir)?;
    let lock = open(&dir.join(".lock"), true, true, true)?;
    lock.try_lock().map_err(|e| match e {
        fs::TryLockError::WouldBlock => io::ErrorKind::WouldBlock.into(),
        fs::TryLockError::Error(e) => e,
    })?;
    let paths = entries(&dir)?;
    let oldest = date(now.saturating_sub(7 * 86_400));
    for path in &paths {
        let Some(day) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(date_name)
        else {
            continue;
        };
        let m = fs::symlink_metadata(path)?;
        if !m.is_file() {
            return Err(refused());
        }
        if day < oldest.as_str() {
            fs::remove_file(path)?;
        }
    }
    let kept = entries(&dir)?;
    let additions = batch
        .keys()
        .filter(|n| {
            !kept
                .iter()
                .any(|p| p.file_name().is_some_and(|f| f == n.as_str()))
        })
        .count();
    if kept.len() + additions > 128 {
        return Err(refused());
    }
    // Preflight every date before any row write. A cap never lands half a batch.
    let mut files = Vec::new();
    for (name, lines) in batch {
        if date_name(&name).is_none() {
            return Err(refused());
        }
        let mut file = open(&dir.join(name), true, true, true)?;
        let len = file.metadata()?.len();
        let seal = needs_seal(&mut file, len)?;
        let need: usize = lines.iter().map(Vec::len).sum();
        if len
            .saturating_add(u64::try_from(need).map_err(|_| refused())?)
            .saturating_add(u64::from(seal))
            > FILE_CAP
        {
            return Err(refused());
        }
        files.push((file, seal, lines));
    }
    for (mut file, seal, lines) in files {
        if seal {
            write_once(&mut file, b"\n")?;
        }
        for line in lines {
            write_once(&mut file, &line)?;
        }
    }
    Ok(())
}
fn valid_clock(row: &Row, now: i64) -> bool {
    let at = row.identity().2;
    let day = at.div_euclid(86_400);
    at < now.saturating_add(super::super::FUTURE_SKEW_SECS)
        && day >= now.div_euclid(86_400).saturating_sub(7)
        && day <= now.div_euclid(86_400)
        && match row {
            Row::Sample(s) => s.observed_at < at.saturating_add(super::super::FUTURE_SKEW_SECS),
            Row::Workload(w) => w.observed_at.is_none_or(|t| t <= at),
        }
}
fn read_file(
    path: &Path,
    id: Option<&str>,
    now: i64,
    budget: &mut u64,
    skipped: &mut Skipped,
) -> Vec<Row> {
    let Ok(m) = fs::symlink_metadata(path) else {
        skipped.unreadable += 1;
        return Vec::new();
    };
    if m.file_type().is_symlink() {
        skipped.link += 1;
        return Vec::new();
    }
    if !m.is_file() {
        skipped.nonregular += 1;
        return Vec::new();
    }
    if m.len() > FILE_CAP {
        skipped.oversize += 1;
        return Vec::new();
    }
    let Some(id) = id else {
        skipped.unreadable += 1;
        return Vec::new();
    };
    if m.len() > *budget {
        skipped.truncated += 1;
        return Vec::new();
    }
    let read = (|| {
        let file = open(path, false, false, true)?;
        let mut body = Vec::new();
        file.take(FILE_CAP.min(*budget) + 1)
            .read_to_end(&mut body)?;
        Ok::<_, io::Error>(body)
    })();
    let Ok(body) = read else {
        skipped.unreadable += 1;
        return Vec::new();
    };
    if body.len() as u64 > FILE_CAP {
        skipped.oversize += 1;
        return Vec::new();
    }
    if body.len() as u64 > *budget {
        skipped.truncated += 1;
        return Vec::new();
    }
    *budget = budget.saturating_sub(body.len() as u64);
    let mut rows = Vec::new();
    let mut foreign = false;
    let mut lines = body.split(|b| *b == b'\n');
    while let Some(line) = lines.next() {
        if lines.clone().next().is_none() {
            if !line.is_empty() {
                skipped.partial += 1;
            }
            break;
        }
        match std::str::from_utf8(line).ok().and_then(|s| parse(s).ok()) {
            Some(row) => {
                if row.identity().1 != id {
                    foreign = true;
                }
                if valid_clock(&row, now) {
                    rows.push(row);
                } else {
                    skipped.damaged += 1;
                }
            }
            None => skipped.damaged += 1,
        }
    }
    if foreign {
        skipped.foreign += 1;
        rows.clear();
    }
    rows
}
/// Read retained fleet evidence without creating, locking or mutating a file.
/// Bounds and per-file refusals are visible in `Report::skipped`.
#[must_use]
pub fn read_fleet(state_root: &Path, now: i64) -> Report {
    let mut skipped = Skipped::default();
    let mut rows = Vec::new();
    let mut budget = FLEET_CAP;
    let sessions = state_root.join("sessions");
    if directory(state_root, false).is_err() || directory(&sessions, false).is_err() {
        return Report::default();
    }
    let Ok(dirs) = fs::read_dir(sessions) else {
        return Report::default();
    };
    let mut dirs = dirs
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .take(129)
        .map(|e| e.path())
        .collect::<Vec<_>>();
    if dirs.len() > 128 {
        skipped.truncated += 1;
        dirs.truncate(128);
    }
    dirs.sort();
    for dir in dirs {
        let history = dir.join("quota-history");
        if directory(&history, true).is_err() {
            match fs::symlink_metadata(&history) {
                Ok(m) if m.file_type().is_symlink() => skipped.link += 1,
                Ok(m) if !m.is_dir() => skipped.nonregular += 1,
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                _ => skipped.unreadable += 1,
            }
            continue;
        }
        let id = session_id(&dir).ok();
        let Ok(paths) = entries(&history) else {
            skipped.truncated += 1;
            continue;
        };
        let oldest = date(now.saturating_sub(7 * 86_400));
        let today = date(now);
        let mut files: Vec<_> = paths
            .into_iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .and_then(date_name)
                    .is_some_and(|d| d >= oldest.as_str() && d <= today.as_str())
            })
            .collect();
        files.sort_by(|a, b| b.cmp(a));
        for file in files.into_iter().take(8) {
            let found = read_file(&file, id.as_deref(), now, &mut budget, &mut skipped);
            let room = 65_536 - rows.len();
            if found.len() > room {
                skipped.truncated += 1;
            }
            rows.extend(found.into_iter().take(room));
        }
    }
    let mut report = fold(rows);
    report.skipped = skipped;
    report
}

#[cfg(test)]
mod tests {
    struct Dir(std::path::PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn spelled_open_flags_refuse_a_symlink() {
        use std::os::unix::fs::OpenOptionsExt;
        let root = std::env::temp_dir().join(format!("ae-qh-flags-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let _guard = Dir(root.clone());
        let target = root.join("target");
        std::fs::write(&target, "x").unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(super::FLAGS)
                .open(&link)
                .is_err()
        );
    }
}
