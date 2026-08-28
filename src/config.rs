//! The three `[workspace]` values compact resolves from config — and nothing else.
//!
//! ae config is INI-style; the frozen `parse_config`/`get_config` pair (in the bash
//! `ae`) reads every section and key. Compact needs exactly three of them —
//! `[workspace].main`, `[workspace].workers`, `[workspace].purge_agent_history` —
//! layering a global file under an origin-local one (the LAST value wins, as
//! `get_config` does). This reads those three, with the same per-line grammar the
//! frozen parser uses, and deliberately nothing more: compact needs three keys, so it
//! reads three keys. It is not a general config framework and must not grow into one.

use std::path::{Path, PathBuf};

/// The `[workspace]` values compact resolves. `main`/`workers` are `None` when the
/// key never appeared (an empty-but-present value is `Some("")`, which the caller
/// distinguishes). `purge_agent_history` is the resolved boolean.
#[derive(Debug)]
pub(crate) struct Workspace {
    pub(crate) main: Option<String>,
    pub(crate) workers: Option<String>,
    pub(crate) purge_agent_history: bool,
}

/// Read `[workspace].{main,workers,purge_agent_history}`, layering `local` over
/// `global` so the LAST file to set a key wins — matching `get_config`'s "last match
/// wins, local overrides global". `purge_agent_history` is true iff the resolved value
/// is one of `true|1|yes|on`, exactly as `_end_effective_purge` decides it.
///
/// ABSENCE VS UNREADABLE. Absence is expressed by passing `None`; a `Some(path)` is a
/// SELECTED file the caller has decided to read, so if it cannot be read or decoded
/// (permission, non-UTF-8, a directory, or gone) the read FAILS closed — `Err(path)` —
/// rather than being silently treated as empty. Reading a `purge_agent_history = true`
/// config as empty was the purge-bypass hazard; the caller turns this `Err` into a
/// refusal.
pub(crate) fn read_workspace(
    global: Option<&Path>,
    local: Option<&Path>,
) -> Result<Workspace, PathBuf> {
    let mut main = None;
    let mut workers = None;
    let mut purge = None;
    for file in [global, local].into_iter().flatten() {
        if apply_file(file, &mut main, &mut workers, &mut purge).is_err() {
            return Err(file.to_owned());
        }
    }
    Ok(Workspace {
        main,
        workers,
        purge_agent_history: matches!(purge.as_deref(), Some("true" | "1" | "yes" | "on")),
    })
}

/// Overlay one SELECTED config file's `[workspace]` keys onto the accumulators. Any
/// key it sets overrides an earlier file's — later files win. `Err(())` when the file
/// cannot be read or decoded (the caller selected it, so this is a hard failure, not
/// "contributes nothing").
fn apply_file(
    file: &Path,
    main: &mut Option<String>,
    workers: &mut Option<String>,
    purge: &mut Option<String>,
) -> Result<(), ()> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: reads the INI config the frozen parse_config reads — see clippy.toml"
    )]
    let read = std::fs::read_to_string(file);
    let text = read.map_err(|_| ())?;
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = section_header(line) {
            section = name;
            continue;
        }
        if section != "workspace" {
            continue;
        }
        if let Some((key, value)) = parse_entry(line) {
            match key {
                "main" => *main = Some(value),
                "workers" => *workers = Some(value),
                "purge_agent_history" => *purge = Some(value),
                _ => {}
            }
        }
    }
    Ok(())
}

/// A `[section]` header line → the section name. The frozen grammar is
/// `^\[([a-zA-Z_-]+)\]$`: a non-empty run of ASCII letters, `_`, and `-`.
fn section_header(line: &str) -> Option<String> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    if !inner.is_empty()
        && inner
            .bytes()
            .all(|b| b.is_ascii_alphabetic() || b == b'_' || b == b'-')
    {
        Some(inner.to_owned())
    } else {
        None
    }
}

/// A `key = value` line → `(key, value)`. Mirrors the frozen parser: a `key = "..."`
/// keeps the bytes between the outer quotes verbatim (comments and `#` inside are
/// kept); an unquoted value strips a trailing `#comment` then trailing whitespace and
/// must be non-empty. The key grammar is `^[a-zA-Z_][a-zA-Z0-9_-]*`.
fn parse_entry(line: &str) -> Option<(&str, String)> {
    let eq = line.find('=')?;
    let key = line[..eq].trim_end();
    if !is_config_key(key) {
        return None;
    }
    let rhs = line[eq + 1..].trim_start();
    // The line is already whole-line-trimmed, so a fully-quoted value ends at the
    // final byte. A quote that opens but does not close falls through to the unquoted
    // branch — exactly as the frozen parser's second regex then matches it.
    if let Some(rest) = rhs.strip_prefix('"')
        && let Some(inner) = rest.strip_suffix('"')
    {
        return Some((key, inner.to_owned()));
    }
    let val = match rhs.find('#') {
        Some(hash) => &rhs[..hash],
        None => rhs,
    };
    let val = val.trim_end();
    if val.is_empty() {
        return None;
    }
    Some((key, val.to_owned()))
}

/// The frozen key grammar `^[a-zA-Z_][a-zA-Z0-9_-]*`.
fn is_config_key(s: &str) -> bool {
    let mut bytes = s.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_alphabetic() || b == b'_' => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Write `text` to a fresh temp file and return its path (kept alive by the
    /// returned `NamedTemp`).
    struct NamedTemp(std::path::PathBuf);
    impl NamedTemp {
        fn new(tag: &str, text: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ae-config-{tag}-{}-{}",
                std::process::id(),
                tag
            ));
            let mut f = std::fs::File::create(&path).expect("temp config");
            f.write_all(text.as_bytes()).expect("write config");
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for NamedTemp {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn extracts_workspace_main_workers_and_purge() {
        let c = NamedTemp::new(
            "basic",
            "[agents]\ncl = \"claude\"\n[workspace]\nmain = cl\nworkers = a, b\npurge_agent_history = true\n",
        );
        let w = read_workspace(Some(c.path()), None).expect("readable config");
        assert_eq!(w.main.as_deref(), Some("cl"));
        assert_eq!(w.workers.as_deref(), Some("a, b"));
        assert!(w.purge_agent_history);
    }

    #[test]
    fn keys_outside_workspace_are_ignored() {
        let c = NamedTemp::new(
            "sections",
            "[agents]\nmain = not-this\n[prompt]\nworkers = nor-this\n",
        );
        let w = read_workspace(Some(c.path()), None).expect("readable config");
        assert_eq!(w.main, None);
        assert_eq!(w.workers, None);
        assert!(!w.purge_agent_history);
    }

    #[test]
    fn a_quoted_value_keeps_its_inner_bytes_and_hash() {
        let c = NamedTemp::new("quoted", "[workspace]\nmain = \"cl #1\"\n");
        let w = read_workspace(Some(c.path()), None).expect("readable config");
        assert_eq!(w.main.as_deref(), Some("cl #1"));
    }

    #[test]
    fn an_unquoted_value_strips_a_trailing_comment() {
        let c = NamedTemp::new("comment", "[workspace]\nmain = cl   # the main\n");
        let w = read_workspace(Some(c.path()), None).expect("readable config");
        assert_eq!(w.main.as_deref(), Some("cl"));
    }

    #[test]
    fn local_overrides_global_last_wins() {
        let g = NamedTemp::new("g", "[workspace]\nmain = global\nworkers = gw\n");
        let l = NamedTemp::new("l", "[workspace]\nmain = local\n");
        let w = read_workspace(Some(g.path()), Some(l.path())).expect("readable config");
        // local set main → local wins; local left workers → global's survives.
        assert_eq!(w.main.as_deref(), Some("local"));
        assert_eq!(w.workers.as_deref(), Some("gw"));
    }

    #[test]
    fn absence_is_none_but_a_selected_unreadable_file_refuses() {
        // Absence is expressed by passing `None`, not by a path that fails to read.
        let w = read_workspace(None, None).expect("no files is empty, not an error");
        assert_eq!(w.main, None);
        assert!(!w.purge_agent_history);
        // A file the caller SELECTED (Some) that cannot be read fails closed — this is
        // the purge-bypass guard: a present-but-unreadable config must never read as
        // empty. `Err` carries the offending path.
        let missing = Path::new("/no/such/config");
        assert_eq!(
            read_workspace(Some(missing), None).unwrap_err(),
            missing.to_path_buf(),
        );
    }

    #[test]
    fn a_present_but_unreadable_config_refuses() {
        // A real regular file whose bytes cannot be read (here: not valid UTF-8) must
        // refuse, not silently contribute nothing — the config carried purge=true.
        let c = NamedTemp::new("unreadable", "");
        std::fs::write(c.path(), [0xff, 0xfe, 0x00, 0x9c]).expect("write non-utf8");
        assert_eq!(
            read_workspace(Some(c.path()), None).unwrap_err(),
            c.path().to_path_buf(),
        );
    }

    #[test]
    fn purge_truthy_and_falsey_values() {
        for v in ["true", "1", "yes", "on"] {
            let c = NamedTemp::new("pt", &format!("[workspace]\npurge_agent_history = {v}\n"));
            assert!(
                read_workspace(Some(c.path()), None)
                    .expect("readable config")
                    .purge_agent_history,
                "'{v}' is truthy"
            );
        }
        for v in ["false", "0", "no", "off", "TRUE", "maybe"] {
            let c = NamedTemp::new("pf", &format!("[workspace]\npurge_agent_history = {v}\n"));
            assert!(
                !read_workspace(Some(c.path()), None)
                    .expect("readable config")
                    .purge_agent_history,
                "'{v}' is not truthy"
            );
        }
    }
}
