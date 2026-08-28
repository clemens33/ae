//! `ae compact`'s freeze/resolve step, in the pinned core.
//!
//! `_compact-freeze <session-dir> [--keep-history]` resolves everything the compact
//! boundary is authorized against — BEFORE anything is messaged, stopped or archived —
//! and emits it as one frozen tuple. It is PURE READ-ONLY: it reads meta and config
//! and resolves paths, and writes nothing.
//!
//! CLEAN CUT. compact is local-mode only, and a session ae cannot cleanly classify —
//! a managed mode, an unknown mode, no origin, an unresolvable origin, an unreadable
//! config, or NO VALID SESSION ID — is refused with a clear reason rather than
//! emulated or migrated on the fly. In particular a session with no parseable
//! `session_id` is unsupported old state: it is refused with a refresh/migrate
//! instruction, never minted a new id (the frozen bash minted one; the clean cut does
//! not).
//!
//! The tuple's ten `0x1f`-separated fields, in order: `name`, `uuid`, `uuid_origin`,
//! `mode`, `origin` (the recorded path, verified to be a directory — not
//! canonicalized), `config`, `purge` (`true`/`false`), `archive_path`, `main_ref`
//! (`alias:name`), `roster` (`main=<alias> workers=<a,b|->`).

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::archive::{ConfigNode, classify_config_node};
use crate::meta;
use crate::state::EXIT_FAILED;

/// `_compact-freeze` core entry. Emits the frozen tuple on `out` and returns `0`, or
/// writes a clear one-line refusal to `err` and returns [`EXIT_FAILED`]. Never
/// mutates anything.
#[allow(
    clippy::too_many_lines,
    reason = "a linear resolve-or-refuse sequence; each step's refusal reads best beside the check that raises it"
)]
pub(crate) fn freeze(
    dir: &Path,
    keep_history: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();

    let Ok(bytes) = meta::read_bytes(dir) else {
        writeln!(
            err,
            "compact: no ae session state for '{name}' — nothing to compact."
        )?;
        return Ok(EXIT_FAILED);
    };

    // Mode: local only. A managed or unclassifiable mode is refused, not emulated.
    let mode = meta_str(&bytes, "mode").unwrap_or_default();
    match mode.as_str() {
        "local" => {}
        "git" | "full" => {
            writeln!(
                err,
                "compact: '{name}' is {mode} mode; compact is local-mode only. Use: ae end {name}, then start a new session yourself."
            )?;
            return Ok(EXIT_FAILED);
        }
        "" => {
            writeln!(
                err,
                "compact: session '{name}' records no mode — refusing to compact a session ae cannot classify."
            )?;
            return Ok(EXIT_FAILED);
        }
        other => {
            writeln!(
                err,
                "compact: session '{name}' records an unknown mode '{other}'."
            )?;
            return Ok(EXIT_FAILED);
        }
    }

    // Origin: recorded, and it must resolve to a directory. The RAW path is kept, not
    // a canonicalized one — the fresh session `cd`s into it and symlinks resolve then;
    // `metadata` (a tracked capability door) both proves it exists and follows a
    // symlink to its target the way the launch will. A harmless divergence from the
    // frozen bash, which normalized the path via `_canonical_dir`.
    let origin = meta_str(&bytes, "origin").unwrap_or_default();
    if origin.is_empty() {
        writeln!(
            err,
            "compact: session '{name}' records no origin — the fresh session would have nowhere to start."
        )?;
        return Ok(EXIT_FAILED);
    }
    if !dir_exists(&origin) {
        writeln!(
            err,
            "compact: session '{name}' records origin '{origin}', which does not resolve to a directory."
        )?;
        return Ok(EXIT_FAILED);
    }

    // Config: the recorded config (if any) layered UNDER the origin's local `.ae/config`
    // — the same two-layer lookup `_compact_config_roster`/`_end_effective_purge` use
    // for exactly these keys. Every SELECTED path is CLASSIFIED (`classify_config_node`:
    // stat + one lstat, neither opening the node) before it is read, so a FIFO or device
    // can never reach `read_to_string` and block the process. Only a confirmed regular
    // file is handed to the reader, which then still refuses on an unreadable or non-UTF-8
    // file (the purge-bypass guard). `config` is emitted RAW in the tuple regardless.
    let config = meta_str(&bytes, "config").unwrap_or_default();

    // The recorded global config, when present and not /dev/null, is REQUIRED to be a
    // regular file. Absent, a non-regular node, or an existence that cannot be proven
    // (permission/I/O) all refuse rather than being guessed at — and none are opened.
    let global_cfg = if config.is_empty() || config == "/dev/null" {
        None
    } else {
        match classify_config_node(Path::new(&config)) {
            ConfigNode::Regular => Some(PathBuf::from(&config)),
            ConfigNode::Absent | ConfigNode::Other => {
                writeln!(
                    err,
                    "compact: session '{name}' records config '{config}', which is not a readable regular file (absent, a directory/FIFO/special node, or unreadable). The fresh session's roster comes from that config; compact will not guess it."
                )?;
                return Ok(EXIT_FAILED);
            }
        }
    };

    // The origin's local `.ae/config` is OPTIONAL only when TRULY ABSENT (lstat NotFound).
    // A present non-regular node, OR an error that cannot prove absence (permission/I/O —
    // e.g. an untraversable `.ae`), must REFUSE — never silently fall back to the global
    // as if the local override were not there.
    let local_cfg_path = Path::new(&origin).join(".ae").join("config");
    let local_cfg = match classify_config_node(&local_cfg_path) {
        ConfigNode::Absent => None,
        ConfigNode::Regular => Some(local_cfg_path),
        ConfigNode::Other => {
            writeln!(
                err,
                "compact: session '{name}' has a local .ae/config that exists but is not a readable regular file (a directory/FIFO/special node, or unreadable); refusing rather than silently ignoring it."
            )?;
            return Ok(EXIT_FAILED);
        }
    };

    // Both selected paths are now confirmed regular files, so the read cannot block; a
    // decode failure (non-UTF-8) or a permission error still refuses here.
    let workspace = match crate::config::read_workspace(global_cfg.as_deref(), local_cfg.as_deref())
    {
        Ok(w) => w,
        Err(path) => {
            writeln!(
                err,
                "compact: session '{name}' records config '{}', which cannot be read. The fresh session's roster comes from that config; compact will not guess it.",
                path.display()
            )?;
            return Ok(EXIT_FAILED);
        }
    };

    // Session id: a valid recorded UUID. CLEAN CUT — no minting; a session with none
    // is unsupported old state, refused with a refresh/migrate instruction.
    let raw_uuid = meta_str(&bytes, "session_id").unwrap_or_default();
    let uuid = crate::archive::canonical_uuid(&raw_uuid);
    if uuid.is_empty() {
        writeln!(
            err,
            "compact: session '{name}' records no valid session id — refresh or migrate the session, then retry."
        )?;
        return Ok(EXIT_FAILED);
    }
    let uuid_origin = meta_str(&bytes, "session_id_origin").unwrap_or_else(|| "session".to_owned());

    // compact keeps the archive and the history by definition; a config that opts into
    // purge is a contradiction the human resolves explicitly with --keep-history.
    let purge = !keep_history && workspace.purge_agent_history;
    if purge {
        writeln!(
            err,
            "compact: session '{name}' has purge_agent_history enabled, which contradicts compact. To proceed: ae compact --keep-history {name}."
        )?;
        return Ok(EXIT_FAILED);
    }

    // The main agent to hand over from — its `alias:name` ref, taken from the TYPED
    // roster grammar (SC-405c), not string-sliced. A malformed `agent.main` (`cl`,
    // `cl:`, `:main`, empty) never becomes a roster entry, so it is refused HERE rather
    // than emitted as a broken handover ref that P3.7b would then try to deliver to.
    let meta_text = String::from_utf8_lossy(&bytes);
    let parsed = meta::Meta::parse(&meta_text);
    let Some(main_ref) = parsed
        .roster()
        .iter()
        .find(|entry| entry.slot == "main")
        .map(meta::RosterEntry::reference)
    else {
        writeln!(
            err,
            "compact: session '{name}' records no valid main agent (alias:name) to hand over from."
        )?;
        return Ok(EXIT_FAILED);
    };

    // The roster the fresh session is PROMISED to start. main is required.
    let Some(roster_main) = workspace.main.as_deref().filter(|m| !m.is_empty()) else {
        writeln!(
            err,
            "compact: the recorded config names no [workspace] main — the fresh session would have no agent."
        )?;
        return Ok(EXIT_FAILED);
    };
    let roster_workers = workspace
        .workers
        .as_deref()
        .filter(|w| !w.is_empty())
        .unwrap_or("-");
    let roster = format!("main={roster_main} workers={roster_workers}");

    let Some(root) = crate::state_root() else {
        writeln!(err, "compact: cannot resolve the ae state root.")?;
        return Ok(EXIT_FAILED);
    };
    let archive_path = root.join("archive").join(&uuid);

    let fields = [
        name,
        uuid,
        uuid_origin,
        mode,
        origin,
        config,
        // Always false: compact keeps the history, and a purge config was already
        // refused above (or overridden by --keep-history), so the boundary that
        // consumes this tuple never sees purge=true.
        "false".to_owned(),
        archive_path.to_string_lossy().into_owned(),
        main_ref,
        roster,
    ];
    // Framing guard: the tuple is ONE `0x1f`-separated line. A field carrying the
    // separator byte would forge extra fields, and a newline would split the record —
    // both silently corrupt what the boundary parses back (the TSV-framing hazard, one
    // separator up). Refuse rather than emit a tuple that does not round-trip.
    if let Some(bad) = fields
        .iter()
        .find(|f| f.contains('\u{1f}') || f.contains('\n'))
    {
        writeln!(
            err,
            "compact: a resolved value contains a control byte (U+001F or newline) that would corrupt the frozen tuple: {bad:?}"
        )?;
        return Ok(EXIT_FAILED);
    }
    writeln!(out, "{}", fields.join("\u{1f}"))?;
    Ok(0)
}

/// A meta value as an owned lossy string, or `None` when the key is absent.
fn meta_str(bytes: &[u8], key: &str) -> Option<String> {
    meta::first_value(bytes, key).map(|value| String::from_utf8_lossy(value).into_owned())
}

/// Whether `path` resolves (following symlinks) to a directory — the origin's
/// existence-and-kind gate. `metadata` is a tracked capability door.
fn dir_exists(path: &str) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: proves the recorded origin exists and is a directory before it becomes the fresh session's cwd — see clippy.toml"
    )]
    let meta = std::fs::metadata(path);
    meta.is_ok_and(|m| m.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp session dir under a scratch root that doubles as `AE_HOME`.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ae-compact-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch");
            Self(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const UUID: &str = "11111111-1111-1111-1111-111111111111";

    /// Build a session dir with `meta` lines and a `[workspace]` config, returning the
    /// session dir. `origin` is the scratch root (a real, canonicalizable dir).
    fn session(s: &Scratch, extra_meta: &str, config_body: Option<&str>) -> PathBuf {
        let dir = s.0.join("sess");
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = s.0.join("config");
        let config_line = if let Some(body) = config_body {
            std::fs::write(&config_path, body).unwrap();
            format!("config={}\n", config_path.display())
        } else {
            String::new()
        };
        let meta = format!(
            "session_id={UUID}\nmode=local\norigin={}\nagent.main=cl:main:{UUID}\n{config_line}{extra_meta}",
            s.0.display()
        );
        std::fs::write(dir.join("meta"), meta).unwrap();
        dir
    }

    fn run(dir: &Path, keep_history: bool) -> (u8, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = freeze(dir, keep_history, &mut out, &mut err).expect("freeze ran");
        (
            code,
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        )
    }

    #[test]
    fn a_local_session_emits_the_ten_field_tuple() {
        let s = Scratch::new("ok");
        let dir = session(&s, "", Some("[workspace]\nmain = cl\nworkers = a, b\n"));
        let (code, out, err) = run(&dir, false);
        assert_eq!(code, 0, "{err}");
        let fields: Vec<&str> = out.trim_end().split('\u{1f}').collect();
        assert_eq!(fields.len(), 10, "ten fields: {out:?}");
        assert_eq!(fields[0], "sess");
        assert_eq!(fields[1], UUID);
        assert_eq!(fields[3], "local");
        assert_eq!(fields[6], "false", "purge false without a purge config");
        assert_eq!(fields[8], "cl:main");
        assert_eq!(fields[9], "main=cl workers=a, b");
    }

    #[test]
    fn a_managed_mode_is_refused_local_only() {
        let s = Scratch::new("git");
        let dir = session(&s, "", Some("[workspace]\nmain = cl\n"));
        std::fs::write(
            dir.join("meta"),
            format!(
                "session_id={UUID}\nmode=git\norigin={}\nagent.main=cl:main\n",
                s.0.display()
            ),
        )
        .unwrap();
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(err.contains("local-mode only"), "{err}");
    }

    #[test]
    fn no_valid_session_id_is_refused_with_refresh_migrate() {
        let s = Scratch::new("nouuid");
        let dir = session(&s, "", Some("[workspace]\nmain = cl\n"));
        std::fs::write(
            dir.join("meta"),
            format!(
                "session_id=not-a-uuid\nmode=local\norigin={}\nagent.main=cl:main\n",
                s.0.display()
            ),
        )
        .unwrap();
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(
            err.contains("no valid session id") && err.contains("refresh"),
            "{err}"
        );
    }

    #[test]
    fn a_missing_origin_is_refused() {
        let s = Scratch::new("noorigin");
        let dir = s.0.join("sess");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta"),
            format!("session_id={UUID}\nmode=local\nagent.main=cl:main\n"),
        )
        .unwrap();
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(err.contains("records no origin"), "{err}");
    }

    #[test]
    fn an_unresolvable_origin_is_refused() {
        let s = Scratch::new("badorigin");
        let dir = s.0.join("sess");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta"),
            format!(
                "session_id={UUID}\nmode=local\norigin=/no/such/place/xyz\nagent.main=cl:main\n"
            ),
        )
        .unwrap();
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(err.contains("does not resolve"), "{err}");
    }

    #[test]
    fn no_main_agent_is_refused() {
        let s = Scratch::new("nomain");
        let dir = s.0.join("sess");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta"),
            format!("session_id={UUID}\nmode=local\norigin={}\n", s.0.display()),
        )
        .unwrap();
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(err.contains("no valid main agent"), "{err}");
    }

    #[test]
    fn a_config_naming_no_workspace_main_is_refused() {
        let s = Scratch::new("nowsmain");
        let dir = session(&s, "", Some("[agents]\ncl = claude\n"));
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(err.contains("names no [workspace] main"), "{err}");
    }

    #[test]
    fn a_purge_config_is_refused_unless_keep_history() {
        let s = Scratch::new("purge");
        let dir = session(
            &s,
            "",
            Some("[workspace]\nmain = cl\npurge_agent_history = true\n"),
        );
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(
            err.contains("purge_agent_history") && err.contains("--keep-history"),
            "{err}"
        );
        // keep_history overrides the contradiction and succeeds.
        let (code2, out2, err2) = run(&dir, true);
        assert_eq!(code2, 0, "{err2}");
        assert_eq!(out2.trim_end().split('\u{1f}').nth(6), Some("false"));
    }

    #[test]
    fn a_present_but_undecodable_config_is_refused() {
        // The purge-bypass regression: a config that is a REAL regular file (so the old
        // is_file gate passed) but whose bytes cannot be decoded must refuse — not read
        // as empty and let a purge=true setting slip through. Non-UTF-8 stands in for
        // "present but unreadable".
        let s = Scratch::new("badcfg");
        let dir = s.0.join("sess");
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = s.0.join("bad-config");
        std::fs::write(&cfg, [0xff, 0xfe, 0x00, 0x9c]).unwrap();
        std::fs::write(
            dir.join("meta"),
            format!(
                "session_id={UUID}\nmode=local\norigin={}\nagent.main=cl:main\nconfig={}\n",
                s.0.display(),
                cfg.display()
            ),
        )
        .unwrap();
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(err.contains("cannot be read"), "{err}");
    }

    #[test]
    fn a_field_carrying_the_separator_byte_is_refused() {
        // A resolved value with a 0x1f would forge extra tuple fields; the framing guard
        // refuses rather than emit a tuple that does not round-trip. Here the config's
        // workers value smuggles one in.
        let s = Scratch::new("framing");
        let dir = session(&s, "", Some("[workspace]\nmain = cl\nworkers = a\u{1f}b\n"));
        let (code, out, err) = run(&dir, false);
        assert_eq!(code, 1, "{err}");
        assert!(
            err.contains("U+001F") || err.contains("control byte"),
            "{err}"
        );
        assert!(out.is_empty(), "no tuple emitted on refusal: {out:?}");
    }

    #[test]
    fn a_malformed_agent_main_is_refused_not_emitted() {
        // `cl`, `cl:`, `:main` each fail the typed roster grammar (SC-405c) and so never
        // become a `main` entry — freeze refuses rather than emitting a broken handover
        // ref P3.7b would try to deliver to. Everything else in the session is valid.
        for bad in ["cl", "cl:", ":main"] {
            let s = Scratch::new("malformedmain");
            let dir = s.0.join("sess");
            std::fs::create_dir_all(&dir).unwrap();
            let cfg = s.0.join("config");
            std::fs::write(&cfg, "[workspace]\nmain = cl\n").unwrap();
            std::fs::write(
                dir.join("meta"),
                format!(
                    "session_id={UUID}\nmode=local\norigin={}\nagent.main={bad}\nconfig={}\n",
                    s.0.display(),
                    cfg.display()
                ),
            )
            .unwrap();
            let (code, out, err) = run(&dir, false);
            assert_eq!(code, 1, "agent.main={bad:?}: {err}");
            assert!(
                err.contains("no valid main agent"),
                "agent.main={bad:?}: {err}"
            );
            assert!(
                out.is_empty(),
                "agent.main={bad:?} emitted a tuple: {out:?}"
            );
        }
    }

    // The FIFO global regression lives in the black-box `tests/it/compact.rs`: creating
    // a FIFO needs `mkfifo(1)`, and `std::process::Command` is a crate-wide disallowed
    // type whose only sanctioned doors are in the it-target (`crate::cli::mkfifo`). A
    // unit test here would open a new Command door in `src/` and fail the capability
    // self-test. The DIRECTORY-as-local case below needs no such fixture.

    #[test]
    fn a_present_nonregular_local_config_is_refused_not_ignored() {
        // The origin's local `.ae/config` exists but is a DIRECTORY (a present, non-
        // regular node). It must refuse — not silently fall back to the valid global as
        // if the local overlay were absent.
        let s = Scratch::new("nonreglocal");
        std::fs::create_dir_all(s.0.join(".ae").join("config")).unwrap();
        let dir = session(&s, "", Some("[workspace]\nmain = cl\n"));
        let (code, out, err) = run(&dir, false);
        assert_eq!(code, 1, "{err}");
        assert!(
            err.contains("local .ae/config") && err.contains("not a readable regular file"),
            "{err}"
        );
        assert!(out.is_empty(), "no tuple emitted on refusal: {out:?}");
    }

    #[test]
    fn an_untraversable_local_config_is_refused_not_ignored() {
        // The local override EXISTS but its parent `.ae` is untraversable, so lstat on it
        // fails with a permission error — NOT NotFound. Existence cannot be proven, so it
        // must refuse rather than silently use the (valid) global config. Regression for
        // the absent-vs-can't-prove-absence conflation.
        use std::os::unix::fs::PermissionsExt as _;
        let s = Scratch::new("untraversable");
        let dotae = s.0.join(".ae");
        std::fs::create_dir_all(&dotae).unwrap();
        std::fs::write(dotae.join("config"), "[workspace]\nmain = local\n").unwrap();
        std::fs::set_permissions(&dotae, std::fs::Permissions::from_mode(0o000)).unwrap();
        // If this process can still traverse `.ae` (e.g. it runs as root), the premise
        // does not hold — restore and skip rather than assert a false negative.
        let denied = std::fs::symlink_metadata(dotae.join("config")).is_err();
        if !denied {
            let _ = std::fs::set_permissions(&dotae, std::fs::Permissions::from_mode(0o755));
            eprintln!("`.ae` traversal not denied (root?); skipping untraversable regression");
            return;
        }
        let dir = session(&s, "", Some("[workspace]\nmain = cl\n"));
        let (code, out, err) = run(&dir, false);
        // Restore before asserting so the scratch dir can always be cleaned up on drop.
        let _ = std::fs::set_permissions(&dotae, std::fs::Permissions::from_mode(0o755));
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("local .ae/config"), "{err}");
        assert!(out.is_empty(), "no tuple emitted on refusal: {out:?}");
    }
}
