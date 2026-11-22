//! Independent frozen acceptance for the macOS subset receipt (slice `macreceipt`).
//!
//! Authored before implementation by `macreceipt-rev` (Google).
//! The driver installs this file byte-identically and never edits it; a disputed
//! test goes back to the reviewer with evidence.
//!
//! Contract:
//! - A whole green `just test-macos` on Darwin that starts and ends on the same
//!   clean committed `HEAD` and tree (with unchanged boundary facts) atomically
//!   writes `<main checkout>/.local/gate-receipts/macos-<tree>`.
//! - Fixed 11-line printable-ASCII LF-terminated grammar, max 8192 bytes:
//!   `kind=macos-subset`, `version=1`, `tree`, `head`, `host`, `os=Darwin`,
//!   `arch`, `tmux`, `subset_hash`, `toolchain_pin`, `time`.
//! - `just release-proof` skips `just test-macos` only when `macos-<tree>`
//!   parses strictly, every deciding fact matches the live host, and
//!   `0 <= age <= 86400` s; on match it logs the receipt and age; on any miss,
//!   mismatch, staleness, symlink or malformed file it logs why and runs
//!   `just test-macos`.
//! - A macOS subset receipt never satisfies the hp whole-suite half or
//!   `AE_RELEASE_HP=local`, and a mislabelled whole-suite receipt never
//!   satisfies the Mac subset check.
//! - Filtered, partial, print-only, non-Darwin, failed, dirty or mid-run moved
//!   runs write no macOS subset receipt; a failed run preserves a prior receipt.
#![allow(
    clippy::too_many_lines,
    reason = "independent acceptance fixtures keep the shell fault scenarios together"
)]

use std::collections::BTreeMap;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{read, root, run_in_env};
use crate::cli::OwnedScratch;

const MAC_KEYS: [&str; 11] = [
    "kind",
    "version",
    "tree",
    "head",
    "host",
    "os",
    "arch",
    "tmux",
    "subset_hash",
    "toolchain_pin",
    "time",
];

fn put(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|why| panic!("{}: {why}", parent.display()));
    }
    std::fs::write(path, text).unwrap_or_else(|why| panic!("{}: {why}", path.display()));
}

fn install(path: &Path, body: &str) {
    if path.exists() || path.is_symlink() {
        std::fs::remove_file(path).unwrap_or_else(|why| panic!("{}: {why}", path.display()));
    }
    put(path, body);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|why| panic!("{}: {why}", path.display()));
}

fn find_tool(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(name))
        .find(|file| file.is_file())
        .unwrap_or_else(|| panic!("the fixture needs {name} on PATH"))
}

fn find_any_tool(names: &[&str]) -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    names
        .iter()
        .find_map(|name| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(name))
                .find(|file| file.is_file())
        })
        .unwrap_or_else(|| panic!("the fixture needs one of {names:?} on PATH"))
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = i64::from((month + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn secs_to_stamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    let (hour, min, sec) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

fn stamp_secs(text: &str) -> Option<i64> {
    let stamp = text.strip_suffix('Z')?;
    let (date, time) = stamp.split_once('T')?;
    if date.len() != 10
        || time.len() != 8
        || date.get(4..5) != Some("-")
        || date.get(7..8) != Some("-")
        || time.get(2..3) != Some(":")
        || time.get(5..6) != Some(":")
    {
        return None;
    }
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let (y, m, d) = (date.get(0..4)?, date.get(5..7)?, date.get(8..10)?);
    let (hh, mm, ss) = (time.get(0..2)?, time.get(3..5)?, time.get(6..8)?);
    if !digits(y, 4)
        || !digits(m, 2)
        || !digits(d, 2)
        || !digits(hh, 2)
        || !digits(mm, 2)
        || !digits(ss, 2)
    {
        return None;
    }
    let year: i64 = y.parse().ok()?;
    let month: u32 = m.parse().ok()?;
    let day: u32 = d.parse().ok()?;
    let hour: u32 = hh.parse().ok()?;
    let min: u32 = mm.parse().ok()?;
    let sec: u32 = ss.parse().ok()?;
    if year < 1970 || !(1..=12).contains(&month) || hour > 23 || min > 59 || sec > 60 {
        return None;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let dim = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if !(1..=dim).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + i64::from(hour) * 3600 + i64::from(min) * 60 + i64::from(sec))
}

fn now_epoch() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    i64::try_from(secs).unwrap_or(0)
}

fn parse_macos_receipt(bytes: &[u8]) -> Result<BTreeMap<&'static str, String>, String> {
    if bytes.is_empty() {
        return Err("empty receipt".to_owned());
    }
    if bytes.len() > 8192 {
        return Err("receipt exceeds 8192 bytes".to_owned());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "not UTF-8".to_owned())?;
    if !bytes
        .iter()
        .all(|&b| b == b'\n' || (0x20..=0x7e).contains(&b))
    {
        return Err("bytes outside printable ASCII and LF".to_owned());
    }
    if !text.ends_with('\n') {
        return Err("missing trailing LF".to_owned());
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() != MAC_KEYS.len() {
        return Err(format!(
            "expected {} lines, got {}",
            MAC_KEYS.len(),
            lines.len()
        ));
    }
    let mut map = BTreeMap::new();
    for (index, (line, &expected_key)) in lines.iter().zip(MAC_KEYS.iter()).enumerate() {
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {} is not key=value: {line:?}", index + 1));
        };
        if key != expected_key {
            return Err(format!(
                "line {} has key {key:?}, expected {expected_key:?}",
                index + 1
            ));
        }
        let valid = match expected_key {
            "kind" => value == "macos-subset",
            "version" => value == "1",
            "tree" | "head" => is_hex(value, 40),
            "subset_hash" | "toolchain_pin" => is_hex(value, 64),
            "os" => value == "Darwin",
            "host" | "arch" | "tmux" => !value.is_empty() && value.trim() == value,
            "time" => stamp_secs(value).is_some(),
            _ => false,
        };
        if !valid {
            return Err(format!("invalid value for {expected_key}: {value:?}"));
        }
        map.insert(expected_key, value.to_owned());
    }
    Ok(map)
}

fn replace_field(text: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    let mut replaced = false;
    let out: String = text
        .lines()
        .map(|line| {
            if line.starts_with(&prefix) {
                replaced = true;
                format!("{key}={value}\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    assert!(replaced, "field {key} not found in:\n{text}");
    out
}

fn file_sha256(base: &Path, path: &Path) -> String {
    let tool = find_any_tool(&["shasum", "sha256sum"]);
    let tool_name = tool
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default();
    let path_str = path.to_str().unwrap_or_default();
    let args: Vec<&str> = if tool_name == "shasum" {
        vec!["-a", "256", path_str]
    } else {
        vec![path_str]
    };
    let (code, out, err) = run_in_env(base, base, tool.to_str().unwrap_or_default(), &args, &[]);
    assert_eq!(code, 0, "sha256 of {}: {out}{err}", path.display());
    let hash = out.split_whitespace().next().unwrap_or_default().to_owned();
    assert!(is_hex(&hash, 64), "sha256 output: {out:?}");
    hash
}

fn dir_entries_sorted(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

struct MacFixture {
    base: OwnedScratch,
    tree: PathBuf,
    linked: PathBuf,
    env: Vec<(String, String)>,
    whole_receipt: String,
    hp_reply: String,
}

impl MacFixture {
    fn new(tag: &str, with_linked: bool) -> Self {
        let base = OwnedScratch::root("gate", &format!("macrec-{tag}"));
        let tree = base.join("tree");
        let linked = base.join("linked");
        for dir in [base.join("bin"), base.join("io"), tree.clone()] {
            std::fs::create_dir_all(&dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display()));
        }
        let mut fixture = Self {
            base,
            tree,
            linked,
            env: Vec::new(),
            whole_receipt: String::new(),
            hp_reply: String::new(),
        };
        fixture.stock();
        fixture.init_repo(with_linked);
        fixture.seed_hp_reply();
        fixture
    }

    fn bin(&self) -> PathBuf {
        self.base.join("bin")
    }

    fn stock(&mut self) {
        let pin = read(&root().join("justfile"))
            .lines()
            .find_map(|line| {
                line.strip_prefix("NEXTEST_VERSION := \"")
                    .and_then(|rest| rest.strip_suffix('"'))
            })
            .unwrap_or_else(|| panic!("justfile must pin NEXTEST_VERSION"))
            .to_owned();

        let real_just = find_tool("just");
        let real_git = find_tool("git");
        let real_tmux = find_tool("tmux");
        let real_uname = find_tool("uname");
        let real_timeout = find_any_tool(&["timeout", "gtimeout"]);

        symlink(&real_just, self.bin().join("just"))
            .unwrap_or_else(|why| panic!("symlink just: {why}"));
        symlink(&real_timeout, self.bin().join("timeout"))
            .unwrap_or_else(|why| panic!("symlink timeout: {why}"));
        symlink(&real_timeout, self.bin().join("gtimeout"))
            .unwrap_or_else(|why| panic!("symlink gtimeout: {why}"));

        install(
            &self.bin().join("git"),
            r#"#!/bin/sh
exec "$AE_MAC_REAL_GIT" "$@"
"#,
        );
        install(
            &self.bin().join("tmux"),
            r#"#!/bin/sh
if [ "${1-}" = -V ]; then
    if [ -f "$AE_MAC_TMUX_FILE" ]; then
        cat "$AE_MAC_TMUX_FILE"
    else
        printf 'tmux %s\n' "${AE_MAC_TMUX:-3.7b}"
    fi
    exit 0
fi
exec "$AE_MAC_REAL_TMUX" "$@"
"#,
        );
        install(
            &self.bin().join("uname"),
            r#"#!/bin/sh
case "${1-}" in
    -s) printf '%s\n' "${AE_MAC_OS:-Darwin}"; exit 0 ;;
    -m) printf '%s\n' "${AE_MAC_ARCH:-arm64}"; exit 0 ;;
esac
exec "$AE_MAC_REAL_UNAME" "$@"
"#,
        );
        install(
            &self.bin().join("hostname"),
            r#"#!/bin/sh
printf '%s\n' "${AE_MAC_HOST:-spec-mac}"
"#,
        );
        install(
            &self.bin().join("rustc"),
            r#"#!/bin/sh
case "${1-}" in
    -vV) printf '%s\n' 'rustc 1.97.1 (macrec-spec)' 'host: aarch64-apple-darwin' ;;
    --version) echo 'rustc 1.97.1 (macrec-spec)' ;;
esac
exit 0
"#,
        );
        install(
            &self.bin().join("cargo"),
            r#"#!/bin/sh
case "${1-}:${2-}" in
    fmt:*) echo fmt >>"$AE_MAC_REC"; exit 0 ;;
    clippy:*) echo clippy >>"$AE_MAC_REC"; exit 0 ;;
    check:*) echo check >>"$AE_MAC_REC"; exit 0 ;;
    nextest:--version) printf 'cargo-nextest %s\n' "$AE_MAC_NEXTEST"; exit 0 ;;
    nextest:list) printf '%s\n' 'spec_one: test' 'spec_two: test'; exit 0 ;;
    nextest:run)
        echo nextest >>"$AE_MAC_REC"
        printf '%s\n' "$@" >>"$AE_MAC_NEXT_ARGS"
        case "${AE_MAC_ACT:-}" in
            commit)
                (cd "$AE_MAC_TREE" && echo mid >mid.txt && "$AE_MAC_REAL_GIT" add mid.txt && "$AE_MAC_REAL_GIT" commit -qm mid) || exit 71
                ;;
            restore)
                (cd "$AE_MAC_TREE" && "$AE_MAC_REAL_GIT" checkout -q -- .) || exit 72
                ;;
            flip-tmux)
                printf 'tmux 3.6\n' >"$AE_MAC_TMUX_FILE"
                ;;
        esac
        exit "${AE_MAC_NEXT_EXIT:-0}"
        ;;
    test:--doc) echo doctest >>"$AE_MAC_REC"; exit 0 ;;
esac
exit 0
"#,
        );
        install(&self.bin().join("taplo"), "#!/bin/sh\nexit 0\n");
        install(&self.bin().join("shfmt"), "#!/bin/sh\nexit 0\n");
        install(
            &self.bin().join("shellcheck"),
            "#!/bin/sh\ncase \"${1-}\" in --version) printf '%s\\n' 'ShellCheck - shell script analysis tool' 'version: 0.11.0' ;; esac\nexit 0\n",
        );
        for harness in [
            "claude", "codex", "gemini", "agy", "grok", "muse", "opencode",
        ] {
            install(
                &self.bin().join(harness),
                &format!("#!/bin/sh\necho '{harness} 1.0-spec'\n"),
            );
        }
        install(
            &self.bin().join("ssh"),
            r#"#!/bin/sh
printf '%s\n' "$@" >>"$AE_MAC_SSH_ARGS"
if [ "${AE_MAC_SSH_MUTATE:-}" = dirty ]; then
    printf '\n// dirty during ssh\n' >>"$AE_MAC_TREE/src/lib.rs"
fi
if [ "${AE_MAC_SSH_FAIL:-0}" = 1 ]; then
    echo unavailable >&2
    exit 255
fi
cat "$AE_MAC_REPLY"
"#,
        );

        let path = format!(
            "{}:{}",
            self.bin().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        self.env = vec![
            ("PATH".into(), path),
            ("TMPDIR".into(), self.base.display().to_string()),
            ("AE_TEST_TMPDIR".into(), self.base.display().to_string()),
            ("RUSTC_WRAPPER".into(), String::new()),
            ("CARGO_BUILD_RUSTC_WRAPPER".into(), String::new()),
            ("AE_RELEASE_HP".into(), "spec@hp.invalid".into()),
            ("AE_RELEASE_HP_ROOT".into(), "/tmp/ae-macrec-hp".into()),
            ("AE_MAC_NEXTEST".into(), pin),
            ("AE_MAC_REAL_GIT".into(), real_git.display().to_string()),
            ("AE_MAC_REAL_TMUX".into(), real_tmux.display().to_string()),
            ("AE_MAC_REAL_UNAME".into(), real_uname.display().to_string()),
            ("AE_MAC_TREE".into(), self.tree.display().to_string()),
            (
                "AE_MAC_REC".into(),
                self.base.join("rec").display().to_string(),
            ),
            (
                "AE_MAC_NEXT_ARGS".into(),
                self.base.join("next-args").display().to_string(),
            ),
            (
                "AE_MAC_SSH_ARGS".into(),
                self.base.join("ssh-args").display().to_string(),
            ),
            (
                "AE_MAC_REPLY".into(),
                self.base.join("reply").display().to_string(),
            ),
            (
                "AE_MAC_TMUX_FILE".into(),
                self.base.join("tmux-version").display().to_string(),
            ),
        ];
    }

    fn init_repo(&self, with_linked: bool) {
        for (rel, body) in [
            (
                "Cargo.toml",
                "[package]\nname = \"ae\"\nversion = \"0.0.0\"\n",
            ),
            (
                "Cargo.lock",
                "version = 4\n\n[[package]]\nname = \"ae\"\nversion = \"0.0.0\"\n",
            ),
            ("rust-toolchain.toml", "[toolchain]\nchannel = \"1.97.1\"\n"),
            (
                ".config/nextest.toml",
                "[profile.default]\ntest-threads = 4\n",
            ),
            (".config/macos-subset", "unit\nmodule boundary\n"),
            ("src/lib.rs", ""),
            ("tests/it/main.rs", "mod boundary;\n"),
            (
                "tests/it/boundary.rs",
                "#[test] fn mac() { let _ = cfg!(target_os = \"macos\"); }\n",
            ),
            ("fuzz/Cargo.toml", "[package]\nname = \"ae-fuzz\"\n"),
            (
                "fuzz/Cargo.lock",
                "version = 4\n\n[[package]]\nname = \"ae\"\nversion = \"0.0.0\"\n",
            ),
            (".gitignore", ".local/\n/dist/\n/target/\n"),
        ] {
            put(&self.tree.join(rel), body);
        }
        std::fs::copy(root().join("justfile"), self.tree.join("justfile"))
            .unwrap_or_else(|why| panic!("copy justfile: {why}"));
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "macrec@example.invalid"],
            vec!["config", "user.name", "MacRec"],
            vec!["add", "-A"],
            vec!["commit", "-qm", "fixture"],
        ] {
            let (code, out, err) = self.git(&args);
            assert_eq!(code, 0, "git {args:?}: {out}{err}");
        }
        if with_linked {
            let (code, out, err) = self.git(&[
                "worktree",
                "add",
                "-q",
                self.linked.to_str().unwrap_or_default(),
                "-b",
                "linked",
            ]);
            assert_eq!(code, 0, "git worktree add: {out}{err}");
        }
    }

    fn seed_hp_reply(&mut self) {
        let (code, out, err) = self.just(
            "test",
            &[],
            &[
                ("AE_MAC_HOST", "hp-spec"),
                ("AE_MAC_OS", "Linux"),
                ("AE_MAC_ARCH", "x86_64"),
            ],
        );
        assert_eq!(code, 0, "seed whole-suite receipt: {out}{err}");
        let (_, tree) = self.head_tree();
        let whole_path = self.receipts_dir().join(&tree);
        let whole = read(&whole_path);
        std::fs::remove_file(&whole_path)
            .unwrap_or_else(|why| panic!("remove seed whole receipt: {why}"));
        self.hp_reply = format!("hp-spec\n{whole}");
        self.whole_receipt = whole;
        self.set_hp_reply(&self.hp_reply);
        self.clear_traces();
    }

    fn env_refs<'a>(&'a self, extra: &'a [(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
        self.env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .chain(extra.iter().copied())
            .collect()
    }

    fn git(&self, args: &[&str]) -> (i32, String, String) {
        let env = self.env_refs(&[]);
        run_in_env(&self.base, &self.tree, "git", args, &env)
    }

    fn just_in(
        &self,
        cwd: &Path,
        recipe: &str,
        args: &[&str],
        extra: &[(&str, &str)],
    ) -> (i32, String, String) {
        let env = self.env_refs(extra);
        let justfile = self.tree.join("justfile");
        let mut full = vec![
            "--justfile",
            justfile.to_str().unwrap_or_default(),
            "--working-directory",
            cwd.to_str().unwrap_or_default(),
            recipe,
        ];
        full.extend_from_slice(args);
        run_in_env(&self.base, cwd, "just", &full, &env)
    }

    fn just(&self, recipe: &str, args: &[&str], extra: &[(&str, &str)]) -> (i32, String, String) {
        self.just_in(&self.tree, recipe, args, extra)
    }

    fn head_tree(&self) -> (String, String) {
        let (code, head, err) = self.git(&["rev-parse", "HEAD"]);
        assert_eq!(code, 0, "HEAD: {head}{err}");
        let (code, tree, err) = self.git(&["rev-parse", "HEAD^{tree}"]);
        assert_eq!(code, 0, "tree: {tree}{err}");
        (head.trim().to_owned(), tree.trim().to_owned())
    }

    fn receipts_dir(&self) -> PathBuf {
        self.tree.join(".local/gate-receipts")
    }

    fn mac_receipt_path(&self, tree: &str) -> PathBuf {
        self.receipts_dir().join(format!("macos-{tree}"))
    }

    fn set_hp_reply(&self, text: &str) {
        put(&self.base.join("reply"), text);
    }

    fn clear_traces(&self) {
        for name in ["rec", "next-args", "ssh-args", "tmux-version"] {
            let _ = std::fs::remove_file(self.base.join(name));
        }
    }

    fn recorded(&self) -> Vec<String> {
        std::fs::read_to_string(self.base.join("rec"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn trace(&self, name: &str) -> String {
        std::fs::read_to_string(self.base.join(name)).unwrap_or_default()
    }
}

fn assert_subset_ran_once(fixture: &MacFixture, context: &str, out: &str, err: &str) {
    let nextest_count = fixture
        .recorded()
        .iter()
        .filter(|step| *step == "nextest")
        .count();
    assert_eq!(
        nextest_count,
        1,
        "{context}: Mac subset must run once: recorded={:?}\n{out}{err}",
        fixture.recorded()
    );
    let args = fixture.trace("next-args");
    assert!(
        args.lines().any(|line| line == "-E"),
        "{context}: Mac run must be the filtered subset: {args}\n{out}{err}"
    );
}

#[test]
fn test_macos_writes_a_valid_macos_subset_receipt_in_the_main_checkout() {
    let fixture = MacFixture::new("write", true);
    let (head, tree) = fixture.head_tree();

    // Clean whole test-macos on Darwin writes macos-<tree> and no whole-suite <tree>.
    let (code, out, err) = fixture.just("test-macos", &[], &[]);
    assert_eq!(code, 0, "clean test-macos on Darwin: {out}{err}");
    assert!(
        out.contains("gate-receipt") || err.contains("gate-receipt"),
        "writer must report the receipt: {out}{err}"
    );
    let mac_path = fixture.mac_receipt_path(&tree);
    assert!(
        mac_path.is_file() && !mac_path.is_symlink(),
        "macos-<tree> must be a plain file at {}",
        mac_path.display()
    );
    assert!(
        !fixture.receipts_dir().join(&tree).exists(),
        "test-macos must never write a whole-suite <tree> receipt"
    );
    assert_eq!(
        dir_entries_sorted(&fixture.receipts_dir()),
        vec![format!("macos-{tree}")],
        "one atomic receipt, no temp files left behind"
    );
    let raw = std::fs::read(&mac_path).unwrap_or_else(|why| panic!("read mac receipt: {why}"));
    let fields =
        parse_macos_receipt(&raw).unwrap_or_else(|why| panic!("strict grammar failed: {why}"));
    assert_eq!(fields["kind"], "macos-subset");
    assert_eq!(fields["version"], "1");
    assert_eq!(fields["tree"], tree);
    assert_eq!(fields["head"], head);
    assert_eq!(fields["host"], "spec-mac");
    assert_eq!(fields["os"], "Darwin");
    assert_eq!(fields["arch"], "arm64");
    assert_eq!(fields["tmux"], "tmux 3.7b");
    assert_eq!(
        fields["subset_hash"],
        file_sha256(&fixture.base, &fixture.tree.join(".config/macos-subset"))
    );
    assert_eq!(
        fields["toolchain_pin"],
        file_sha256(&fixture.base, &fixture.tree.join("rust-toolchain.toml"))
    );
    let stamp = stamp_secs(&fields["time"]).unwrap_or(0);
    let delta = now_epoch() - stamp;
    assert!(
        (0..=300).contains(&delta),
        "fresh UTC timestamp expected: stamp={stamp}, delta={delta}"
    );

    // Worktree run writes into the main checkout's .local/gate-receipts, never linked/.local.
    std::fs::remove_dir_all(fixture.receipts_dir())
        .unwrap_or_else(|why| panic!("clear receipts dir: {why}"));
    let (code, out, err) = fixture.just_in(&fixture.linked, "test-macos", &[], &[]);
    assert_eq!(code, 0, "worktree test-macos: {out}{err}");
    assert!(
        mac_path.is_file() && !mac_path.is_symlink(),
        "worktree run must write to main checkout's .local/gate-receipts"
    );
    assert!(
        !fixture.linked.join(".local/gate-receipts").exists(),
        "linked worktree must hold no receipt directory"
    );

    // Non-Darwin (Linux) exits 0 and writes no receipt.
    std::fs::remove_dir_all(fixture.receipts_dir())
        .unwrap_or_else(|why| panic!("clear receipts dir: {why}"));
    let (code, out, err) = fixture.just("test-macos", &[], &[("AE_MAC_OS", "Linux")]);
    assert_eq!(code, 0, "non-Darwin test-macos stays green: {out}{err}");
    assert!(
        dir_entries_sorted(&fixture.receipts_dir()).is_empty(),
        "non-Darwin test-macos must write no receipt: {out}{err}"
    );

    // Unwritable receipts path (regular file) leaves a green test-macos green and notes it.
    put(&fixture.receipts_dir(), "not a directory\n");
    let (code, out, err) = fixture.just("test-macos", &[], &[]);
    assert_eq!(code, 0, "unwritable dir keeps test-macos green: {out}{err}");
    assert!(
        err.contains("receipt"),
        "write failure must be reported on stderr: {err}"
    );
    std::fs::remove_file(fixture.receipts_dir())
        .unwrap_or_else(|why| panic!("remove file blocker: {why}"));

    // Symlinked receipts dir must never be written through and leaves test-macos green.
    let outside = fixture.base.join("outside-receipts");
    std::fs::create_dir_all(&outside).unwrap_or_else(|why| panic!("create outside: {why}"));
    symlink(&outside, fixture.receipts_dir())
        .unwrap_or_else(|why| panic!("symlink receipts dir: {why}"));
    let (code, out, err) = fixture.just("test-macos", &[], &[]);
    assert_eq!(code, 0, "symlinked dir keeps test-macos green: {out}{err}");
    assert!(
        dir_entries_sorted(&outside).is_empty(),
        "writer must never write through a symlinked receipts dir"
    );
}

#[test]
fn filtered_partial_failed_dirty_and_mid_run_moved_test_macos_write_no_receipt() {
    let fixture = MacFixture::new("nowrite", false);
    let (_, tree) = fixture.head_tree();
    let mac_path = fixture.mac_receipt_path(&tree);

    // print=yes writes nothing.
    let (code, out, err) = fixture.just("test-macos", &["print=yes"], &[]);
    assert_eq!(code, 0, "print=yes: {out}{err}");
    assert!(dir_entries_sorted(&fixture.receipts_dir()).is_empty());

    // Extra/filter argument to test-macos refuses and writes nothing.
    let (code, _, _) = fixture.just("test-macos", &["-E", "kind(lib)"], &[]);
    assert_ne!(code, 0, "filtered test-macos arg must refuse");
    assert!(dir_entries_sorted(&fixture.receipts_dir()).is_empty());

    // Filtered/partial lanes never write a macOS receipt.
    for (label, recipe, args) in [
        ("rust-test", "rust-test", &[][..]),
        ("pre", "pre", &[][..]),
        (
            "lane-filtered",
            "_tmux-isolated",
            &["test", "-E", "kind(lib)"][..],
        ),
    ] {
        let (code, out, err) = fixture.just(recipe, args, &[]);
        assert_eq!(code, 0, "{label}: {out}{err}");
        assert!(!mac_path.exists(), "{label} must never write macos-<tree>");
    }

    // Failed fresh test-macos writes nothing.
    let (code, out, err) = fixture.just("test-macos", &[], &[("AE_MAC_NEXT_EXIT", "19")]);
    assert_ne!(code, 0, "red test-macos fails: {out}{err}");
    assert!(!mac_path.exists(), "failed test-macos must write nothing");

    // Dirty tree (unstaged, staged, untracked) writes nothing.
    put(&fixture.tree.join("src/lib.rs"), "// unstaged\n");
    let (code, out, err) = fixture.just("test-macos", &[], &[]);
    assert_eq!(code, 0, "unstaged dirty runs: {out}{err}");
    assert!(!mac_path.exists(), "unstaged dirty writes no receipt");

    assert_eq!(fixture.git(&["add", "src/lib.rs"]).0, 0);
    let (code, out, err) = fixture.just("test-macos", &[], &[]);
    assert_eq!(code, 0, "staged dirty runs: {out}{err}");
    assert!(!mac_path.exists(), "staged dirty writes no receipt");
    assert_eq!(fixture.git(&["reset", "--hard", "-q", "HEAD"]).0, 0);

    put(&fixture.tree.join("loose.txt"), "untracked\n");
    let (code, out, err) = fixture.just("test-macos", &[], &[]);
    assert_eq!(code, 0, "untracked dirty runs: {out}{err}");
    assert!(!mac_path.exists(), "untracked dirty writes no receipt");
    std::fs::remove_file(fixture.tree.join("loose.txt"))
        .unwrap_or_else(|why| panic!("remove untracked: {why}"));

    // Started dirty, cleaned mid-run: writes no receipt.
    put(&fixture.tree.join("src/lib.rs"), "// dirty at start\n");
    let (code, out, err) = fixture.just("test-macos", &[], &[("AE_MAC_ACT", "restore")]);
    assert_eq!(code, 0, "dirty-then-clean runs: {out}{err}");
    assert!(
        !mac_path.exists(),
        "dirty at start and cleaned mid-run must write no receipt: {out}{err}"
    );

    // Boundary fact (tmux -V) changed mid-run: writes no receipt.
    fixture.clear_traces();
    let (code, out, err) = fixture.just("test-macos", &[], &[("AE_MAC_ACT", "flip-tmux")]);
    assert_eq!(code, 0, "mid-run tmux flip stays green: {out}{err}");
    assert!(
        !mac_path.exists(),
        "tmux changing mid-run must write no receipt: {out}{err}"
    );
    fixture.clear_traces();

    // Clean run writes, and a subsequent failed run preserves the prior receipt intact.
    let (code, out, err) = fixture.just("test-macos", &[], &[]);
    assert_eq!(code, 0, "clean control run: {out}{err}");
    let prior = read(&mac_path);
    let (code, out, err) = fixture.just("test-macos", &[], &[("AE_MAC_NEXT_EXIT", "19")]);
    assert_ne!(code, 0, "subsequent red test-macos fails: {out}{err}");
    assert_eq!(
        read(&mac_path),
        prior,
        "failed run must preserve prior macos-<tree> untouched"
    );

    // Started clean, committed mid-run: writes no receipt for either old or new tree.
    let commit_fixture = MacFixture::new("midcommit", false);
    let (code, out, err) = commit_fixture.just("test-macos", &[], &[("AE_MAC_ACT", "commit")]);
    assert_eq!(code, 0, "mid-run commit stays green: {out}{err}");
    assert!(
        dir_entries_sorted(&commit_fixture.receipts_dir()).is_empty(),
        "commit during test-macos must write no receipt: {out}{err}"
    );
}

#[test]
fn a_matching_macos_receipt_skips_the_subset_in_release_proof() {
    let fixture = MacFixture::new("skip", true);
    let (head, tree) = fixture.head_tree();
    let expect = format!("expect={head}:{tree}");

    // Write the receipt from the linked worktree; release-proof from main must skip.
    let (code, out, err) = fixture.just_in(&fixture.linked, "test-macos", &[], &[]);
    assert_eq!(code, 0, "worktree test-macos: {out}{err}");
    fixture.clear_traces();

    let (code, out, err) = fixture.just("release-proof", &[&expect], &[]);
    assert_eq!(code, 0, "matching receipt skips subset: {out}{err}");
    assert!(
        !fixture.trace("ssh-args").is_empty(),
        "hp proof must still run over ssh"
    );
    assert!(
        !fixture.recorded().iter().any(|step| step == "nextest"),
        "matching receipt must skip just test-macos: {:?}",
        fixture.recorded()
    );
    assert!(
        fixture.trace("next-args").is_empty(),
        "no nextest invocation on skip"
    );
    let combined = format!("{out}{err}");
    assert!(
        combined.contains(&format!("macos-{tree}"))
            && (combined.contains("ago") || combined.contains("min") || combined.contains("age")),
        "match output must name the receipt and its age: {combined}"
    );

    // Passing hp=run when both receipts exist also skips test-macos.
    fixture.clear_traces();
    let (code, out, err) = fixture.just("release-proof", &[&expect, "hp=run"], &[]);
    assert_eq!(
        code, 0,
        "hp=run with both receipts skips subset: {out}{err}"
    );
    assert!(
        fixture.trace("next-args").is_empty(),
        "hp=run with existing hp+mac receipts must skip test-macos"
    );

    // Receipt just inside 24 h (23 h 59 m old) still skips.
    let mac_path = fixture.mac_receipt_path(&tree);
    let aged = replace_field(
        &read(&mac_path),
        "time",
        &secs_to_stamp(now_epoch() - 86_340),
    );
    put(&mac_path, &aged);
    fixture.clear_traces();
    let (code, out, err) = fixture.just("release-proof", &[&expect], &[]);
    assert_eq!(code, 0, "23h59m receipt still skips: {out}{err}");
    assert!(
        fixture.trace("next-args").is_empty(),
        "receipt within 24 h must skip test-macos"
    );
}

type MismatchCase<'a> = (&'a str, String, Vec<(&'a str, &'a str)>, &'a [&'a str]);

#[test]
fn each_mismatching_deciding_fact_runs_the_mac_subset_and_explains_why() {
    let fixture = MacFixture::new("mismatch", false);
    let (head, tree) = fixture.head_tree();
    let expect = format!("expect={head}:{tree}");
    let mac_path = fixture.mac_receipt_path(&tree);

    let (code, out, err) = fixture.just("test-macos", &[], &[]);
    assert_eq!(code, 0, "seed macos receipt: {out}{err}");
    let good = read(&mac_path);
    parse_macos_receipt(good.as_bytes()).unwrap_or_else(|why| panic!("good receipt: {why}"));

    let cases: [MismatchCase<'_>; 10] = [
        (
            "tree",
            replace_field(&good, "tree", &"a".repeat(40)),
            vec![],
            &["tree"],
        ),
        (
            "head",
            replace_field(&good, "head", &"b".repeat(40)),
            vec![],
            &["head"],
        ),
        (
            "host",
            replace_field(&good, "host", "other-mac"),
            vec![],
            &["host"],
        ),
        (
            "os-field",
            replace_field(&good, "os", "Linux"),
            vec![],
            &["os", "darwin"],
        ),
        (
            "os-live",
            good.clone(),
            vec![("AE_MAC_OS", "Linux")],
            &["os", "darwin"],
        ),
        (
            "arch",
            good.clone(),
            vec![("AE_MAC_ARCH", "x86_64")],
            &["arch"],
        ),
        (
            "tmux",
            good.clone(),
            vec![("AE_MAC_TMUX", "3.6")],
            &["tmux"],
        ),
        (
            "subset_hash",
            replace_field(&good, "subset_hash", &"c".repeat(64)),
            vec![],
            &["subset_hash", "subset"],
        ),
        (
            "toolchain_pin",
            replace_field(&good, "toolchain_pin", &"d".repeat(64)),
            vec![],
            &["toolchain_pin", "toolchain"],
        ),
        (
            "stale",
            replace_field(&good, "time", &secs_to_stamp(now_epoch() - 86_460)),
            vec![],
            &["stale", "old", "24 h", "age"],
        ),
    ];

    for (label, content, extra, needles) in cases {
        put(&mac_path, &content);
        fixture.clear_traces();
        let (code, out, err) = fixture.just("release-proof", &[&expect], &extra);
        assert_eq!(
            code, 0,
            "{label}: miss must run subset and exit 0: {out}{err}"
        );
        assert_subset_ran_once(&fixture, label, &out, &err);
        let lower = format!("{out}{err}").to_ascii_lowercase();
        assert!(
            needles.iter().any(|needle| lower.contains(needle)),
            "{label}: output must explain why receipt missed (expected one of {needles:?}): {out}{err}"
        );
    }

    // Future timestamp (nonnegative age required) must miss and run the subset.
    put(
        &mac_path,
        &replace_field(&good, "time", &secs_to_stamp(now_epoch() + 120)),
    );
    fixture.clear_traces();
    let (code, out, err) = fixture.just("release-proof", &[&expect], &[]);
    assert_eq!(code, 0, "future receipt runs subset: {out}{err}");
    assert_subset_ran_once(&fixture, "future", &out, &err);
    let lower = format!("{out}{err}").to_ascii_lowercase();
    assert!(
        ["future", "clock", "skew", "age"]
            .iter()
            .any(|needle| lower.contains(needle)),
        "future receipt must explain why it missed: {out}{err}"
    );

    // Off-diagonal HEAD check: empty commit keeps the SAME tree hash while moving HEAD.
    let (code, out, err) = fixture.git(&["commit", "--allow-empty", "-qm", "empty head move"]);
    assert_eq!(code, 0, "empty commit: {out}{err}");
    let (head2, tree2) = fixture.head_tree();
    assert_ne!(head2, head, "empty commit must move HEAD");
    assert_eq!(tree2, tree, "empty commit must keep tree identical");
    fixture.set_hp_reply(&replace_field(&fixture.hp_reply, "head", &head2));
    // mac_path still holds the fresh receipt for (head, tree).
    put(&mac_path, &good);
    fixture.clear_traces();
    let expect2 = format!("expect={head2}:{tree2}");
    let (code, out, err) = fixture.just("release-proof", &[&expect2], &[]);
    assert_eq!(code, 0, "empty commit runs subset: {out}{err}");
    assert_subset_ran_once(&fixture, "empty-commit-head", &out, &err);
    assert!(
        format!("{out}{err}").to_ascii_lowercase().contains("head"),
        "empty-commit miss must name head: {out}{err}"
    );

    // Because the miss above ran a clean test-macos on Darwin at head2, a second
    // release-proof at head2 now skips.
    fixture.clear_traces();
    let (code, out, err) = fixture.just("release-proof", &[&expect2], &[]);
    assert_eq!(code, 0, "fresh receipt after rerun skips: {out}{err}");
    assert!(
        fixture.trace("next-args").is_empty(),
        "freshly rewritten receipt must skip on next release-proof"
    );
}

#[test]
fn malformed_oversized_symlinked_and_mislabelled_macos_receipts_run_the_subset() {
    let fixture = MacFixture::new("hostile", false);
    let (head, tree) = fixture.head_tree();
    let expect = format!("expect={head}:{tree}");
    let mac_path = fixture.mac_receipt_path(&tree);

    let (code, out, err) = fixture.just("test-macos", &[], &[]);
    assert_eq!(code, 0, "seed macos receipt: {out}{err}");
    let good = read(&mac_path);

    let mut non_utf8 = good.as_bytes().to_vec();
    non_utf8.extend_from_slice(&[0xff, 0x00]);
    let swapped = good.replacen(
        &format!("tree={tree}\nhead={head}\n"),
        &format!("head={head}\ntree={tree}\n"),
        1,
    );
    let oversized = replace_field(&good, "tmux", &format!("tmux {}", "x".repeat(9000)));

    let byte_cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("non-utf8", non_utf8),
        ("crlf", good.replace('\n', "\r\n").into_bytes()),
        (
            "blank-line",
            good.replacen("host=", "\nhost=", 1).into_bytes(),
        ),
        (
            "wrong-kind",
            replace_field(&good, "kind", "whole-suite").into_bytes(),
        ),
        (
            "wrong-version",
            replace_field(&good, "version", "2").into_bytes(),
        ),
        ("extra-key", format!("{good}extra=1\n").into_bytes()),
        (
            "duplicate-key",
            good.replacen("host=", "host=spec-mac\nhost=", 1)
                .into_bytes(),
        ),
        ("wrong-order", swapped.into_bytes()),
        (
            "missing-key",
            good.replace("host=spec-mac\n", "").into_bytes(),
        ),
        ("oversized", oversized.into_bytes()),
        (
            "mislabelled-whole-suite",
            fixture.whole_receipt.as_bytes().to_vec(),
        ),
    ];

    for (label, bytes) in byte_cases {
        if mac_path.exists() || mac_path.is_symlink() {
            std::fs::remove_file(&mac_path)
                .unwrap_or_else(|why| panic!("clear mac_path for {label}: {why}"));
        }
        std::fs::write(&mac_path, &bytes).unwrap_or_else(|why| panic!("plant {label}: {why}"));
        fixture.clear_traces();
        let (code, out, err) = fixture.just("release-proof", &[&expect], &[]);
        assert_eq!(code, 0, "{label}: hostile receipt runs subset: {out}{err}");
        assert_subset_ran_once(&fixture, label, &out, &err);
    }

    // Symlinked receipt file pointing at a valid receipt outside .local/gate-receipts.
    let outside = fixture.base.join("outside-valid-mac-receipt");
    put(&outside, &good);
    let _ = std::fs::remove_file(&mac_path);
    symlink(&outside, &mac_path).unwrap_or_else(|why| panic!("symlink receipt file: {why}"));
    fixture.clear_traces();
    let (code, out, err) = fixture.just("release-proof", &[&expect], &[]);
    assert_eq!(code, 0, "symlinked receipt file runs subset: {out}{err}");
    assert_subset_ran_once(&fixture, "symlinked-file", &out, &err);

    // Symlinked .local/gate-receipts directory containing a valid macos-<tree>.
    let outside_dir = fixture.base.join("outside-receipts-dir");
    put(&outside_dir.join(format!("macos-{tree}")), &good);
    std::fs::remove_dir_all(fixture.receipts_dir())
        .unwrap_or_else(|why| panic!("remove receipts dir: {why}"));
    symlink(&outside_dir, fixture.receipts_dir())
        .unwrap_or_else(|why| panic!("symlink receipts dir: {why}"));
    fixture.clear_traces();
    let (code, out, err) = fixture.just("release-proof", &[&expect], &[]);
    assert_eq!(code, 0, "symlinked receipts dir runs subset: {out}{err}");
    assert_subset_ran_once(&fixture, "symlinked-dir", &out, &err);
}

#[test]
fn macos_receipt_never_satisfies_hp_half_or_local_override_and_hp_failure_still_refuses() {
    let fixture = MacFixture::new("isolation", false);
    let (head, tree) = fixture.head_tree();
    let expect = format!("expect={head}:{tree}");
    let mac_path = fixture.mac_receipt_path(&tree);

    let (code, out, err) = fixture.just("test-macos", &[], &[]);
    assert_eq!(code, 0, "seed valid macos receipt: {out}{err}");
    let mac_text = read(&mac_path);

    // Unreachable hp still refuses even with a fresh matching macos-<tree>.
    fixture.clear_traces();
    let (code, out, err) = fixture.just("release-proof", &[&expect], &[("AE_MAC_SSH_FAIL", "1")]);
    assert_ne!(code, 0, "unreachable hp must refuse: {out}{err}");
    assert!(
        format!("{out}{err}").contains("release-proof: refused:"),
        "refusal prefix required: {out}{err}"
    );
    assert!(fixture.trace("next-args").is_empty());

    // A macos-<tree> receipt mislabelled as the hp's whole-suite receipt must refuse.
    fixture.set_hp_reply(&format!(
        "hp-spec\n{}",
        replace_field(&mac_text, "host", "hp-spec")
    ));
    fixture.clear_traces();
    let (code, out, err) = fixture.just("release-proof", &[&expect], &[]);
    assert_ne!(
        code, 0,
        "macos subset receipt on hp must never satisfy hp half: {out}{err}"
    );
    assert!(
        format!("{out}{err}").contains("release-proof: refused:"),
        "{out}{err}"
    );
    assert!(fixture.trace("next-args").is_empty());
    fixture.set_hp_reply(&fixture.hp_reply);

    // AE_RELEASE_HP=local with ONLY macos-<tree> (no whole-suite <tree>) runs whole `just test`.
    assert!(!fixture.receipts_dir().join(&tree).exists());
    fixture.clear_traces();
    let (code, out, err) = fixture.just("release-proof", &[&expect], &[("AE_RELEASE_HP", "local")]);
    assert_eq!(code, 0, "local override runs whole gate: {out}{err}");
    assert!(
        format!("{out}{err}").contains("DEVIATION"),
        "local override prints DEVIATION: {out}{err}"
    );
    assert!(
        fixture.recorded().iter().any(|step| step == "doctest"),
        "local override must run the whole gate (including doctests), not reuse macos-<tree>: {:?}",
        fixture.recorded()
    );
    assert!(
        !fixture.trace("next-args").lines().any(|line| line == "-E"),
        "local override must run unfiltered nextest"
    );

    // Checkout mutation during SSH still refuses even with a fresh matching macos-<tree>.
    fixture.clear_traces();
    let (code, out, err) = fixture.just(
        "release-proof",
        &[&expect],
        &[("AE_MAC_SSH_MUTATE", "dirty")],
    );
    assert_ne!(code, 0, "mutation during ssh must refuse: {out}{err}");
    assert!(
        format!("{out}{err}").contains("release-proof: refused:"),
        "{out}{err}"
    );
    assert!(fixture.trace("next-args").is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn hp_run_reuses_matching_macos_receipt_and_still_runs_hp_leg() {
    let base = OwnedScratch::root("gate", "macrec-par");
    let mac = base.join("mac");
    let hp = base.join("hp");
    let bin = base.join("bin");
    for dir in [&bin, &base.join("io"), &mac] {
        std::fs::create_dir_all(dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display()));
    }
    let pin = read(&root().join("justfile"))
        .lines()
        .find_map(|line| {
            line.strip_prefix("NEXTEST_VERSION := \"")
                .and_then(|rest| rest.strip_suffix('"'))
        })
        .unwrap_or_else(|| panic!("NEXTEST_VERSION pin"))
        .to_owned();

    let real_git = find_tool("git");
    let real_just = find_tool("just");
    let real_tmux = find_tool("tmux");
    let real_uname = find_tool("uname");
    let real_sleep = find_tool("sleep");
    let real_timeout = find_tool("timeout");

    symlink(&real_just, bin.join("just")).unwrap_or_else(|why| panic!("just link: {why}"));
    symlink(&real_timeout, bin.join("timeout")).unwrap_or_else(|why| panic!("timeout link: {why}"));

    install(
        &bin.join("git"),
        r#"#!/usr/bin/env bash
args=()
for arg; do
    if [[ "$arg" == "ssh://spec@hp.invalid$AE_MP_HP" ]]; then arg="$AE_MP_HP"; fi
    args+=("$arg")
done
exec "$AE_MP_REAL_GIT" "${args[@]}"
"#,
    );
    install(
        &bin.join("sleep"),
        "#!/bin/sh\ncase \"$1\" in 30) exec \"$AE_MP_REAL_SLEEP\" 0.02 ;; *) exec \"$AE_MP_REAL_SLEEP\" \"$@\" ;; esac\n",
    );
    install(
        &bin.join("tmux"),
        r#"#!/bin/sh
if [ "${1-}" = -V ]; then
    printf 'tmux %s\n' "${AE_MP_TMUX:-3.7b}"
    exit 0
fi
exec "$AE_MP_REAL_TMUX" "$@"
"#,
    );
    install(
        &bin.join("uname"),
        r#"#!/bin/sh
case "${1-}" in
    -s)
        if [ "$AE_MP_SIDE" = hp ]; then echo Linux; else echo Darwin; fi
        exit 0 ;;
    -m)
        if [ "$AE_MP_SIDE" = hp ]; then echo x86_64; else echo arm64; fi
        exit 0 ;;
esac
exec "$AE_MP_REAL_UNAME" "$@"
"#,
    );
    install(
        &bin.join("hostname"),
        "#!/bin/sh\nprintf 'spec-%s\\n' \"$AE_MP_SIDE\"\n",
    );
    install(
        &bin.join("rustc"),
        "#!/bin/sh\necho 'rustc 1.97.1 (macrec-par)'\n",
    );
    install(
        &bin.join("cargo"),
        r#"#!/bin/sh
case "${1-}:${2-}" in
    nextest:--version) printf 'cargo-nextest %s\n' "$AE_MP_PIN"; exit 0 ;;
    nextest:list) printf '%s\n' 'spec_one: test' 'spec_two: test'; exit 0 ;;
    nextest:run) printf '%s:nextest\n' "$AE_MP_SIDE" >>"$AE_MP_TRACE"; exit 0 ;;
    test:--doc) printf '%s:doctest\n' "$AE_MP_SIDE" >>"$AE_MP_TRACE"; exit 0 ;;
esac
exit 0
"#,
    );
    for tool in [
        "taplo",
        "shfmt",
        "shellcheck",
        "claude",
        "codex",
        "gemini",
        "agy",
        "grok",
        "muse",
        "opencode",
    ] {
        install(&bin.join(tool), "#!/bin/sh\necho 'fixture 1'\n");
    }
    install(
        &bin.join("ssh"),
        r#"#!/usr/bin/env bash
while (($#)); do
    case "$1" in
        -o) shift 2 ;;
        --) shift; break ;;
        -*) shift ;;
        *) break ;;
    esac
done
[[ "${1-}" == spec@hp.invalid ]] || exit 255
shift
export AE_MP_SIDE=hp
cd "$AE_MP_HP" || exit 1
bash -c "$*"
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let env: Vec<(&str, String)> = vec![
        ("PATH", path),
        ("TMPDIR", base.display().to_string()),
        ("AE_TEST_TMPDIR", base.display().to_string()),
        ("RUSTC_WRAPPER", String::new()),
        ("CARGO_BUILD_RUSTC_WRAPPER", String::new()),
        ("AE_RELEASE_HP", "spec@hp.invalid".into()),
        ("AE_RELEASE_HP_ROOT", hp.display().to_string()),
        ("AE_MP_HP", hp.display().to_string()),
        ("AE_MP_PIN", pin),
        ("AE_MP_SIDE", "mac".into()),
        ("AE_MP_TRACE", base.join("trace").display().to_string()),
        ("AE_MP_REAL_GIT", real_git.display().to_string()),
        ("AE_MP_REAL_TMUX", real_tmux.display().to_string()),
        ("AE_MP_REAL_UNAME", real_uname.display().to_string()),
        ("AE_MP_REAL_SLEEP", real_sleep.display().to_string()),
    ];
    let env_refs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();

    for (rel, body) in [
        (
            "Cargo.toml",
            "[package]\nname = \"ae\"\nversion = \"0.0.0\"\n",
        ),
        (
            "Cargo.lock",
            "version = 4\n\n[[package]]\nname = \"ae\"\nversion = \"0.0.0\"\n",
        ),
        ("fuzz/Cargo.toml", "[package]\nname = \"ae-fuzz\"\n"),
        (
            "fuzz/Cargo.lock",
            "version = 4\n\n[[package]]\nname = \"ae\"\nversion = \"0.0.0\"\n",
        ),
        ("src/lib.rs", ""),
        ("tests/it/main.rs", "mod boundary;\n"),
        (
            "tests/it/boundary.rs",
            "#[test] fn mac() { let _ = cfg!(target_os = \"macos\"); }\n",
        ),
        (".config/macos-subset", "unit\nmodule boundary\n"),
        (
            ".config/nextest.toml",
            "[profile.default]\ntest-threads = 1\n",
        ),
        (".gitignore", ".local/\ntarget/\n"),
    ] {
        put(&mac.join(rel), body);
    }
    for name in ["justfile", "rust-toolchain.toml"] {
        std::fs::copy(root().join(name), mac.join(name))
            .unwrap_or_else(|why| panic!("copy {name}: {why}"));
    }
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.name", "Fixture"],
        vec!["config", "user.email", "fixture@example.invalid"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "base"],
    ] {
        let (code, out, err) = run_in_env(&base, &mac, "git", &args, &env_refs);
        assert_eq!(code, 0, "git {args:?}: {out}{err}");
    }
    let (code, out, err) = run_in_env(
        &base,
        &mac,
        "git",
        &[
            "clone",
            "-q",
            "--no-local",
            mac.to_str().unwrap_or_default(),
            hp.to_str().unwrap_or_default(),
        ],
        &env_refs,
    );
    assert_eq!(code, 0, "clone hp: {out}{err}");
    let (code, out, err) = run_in_env(
        &base,
        &hp,
        "git",
        &["checkout", "-q", "--detach", "HEAD"],
        &env_refs,
    );
    assert_eq!(code, 0, "detach hp: {out}{err}");

    let (_, head_out, _) = run_in_env(&base, &mac, "git", &["rev-parse", "HEAD"], &env_refs);
    let (_, tree_out, _) = run_in_env(&base, &mac, "git", &["rev-parse", "HEAD^{tree}"], &env_refs);
    let (head, tree) = (head_out.trim().to_owned(), tree_out.trim().to_owned());
    let expect = format!("expect={head}:{tree}");

    // 1. Pre-populate a matching Mac subset receipt on the Mac side; hp has NO receipt yet.
    let (code, out, err) = run_in_env(&base, &mac, "just", &["test-macos"], &env_refs);
    assert_eq!(code, 0, "seed mac receipt: {out}{err}");
    let mac_receipt = mac
        .join(".local/gate-receipts")
        .join(format!("macos-{tree}"));
    assert!(mac_receipt.is_file(), "mac receipt must exist");
    let hp_receipt = hp.join(".local/gate-receipts").join(&tree);
    assert!(!hp_receipt.exists(), "hp starts with no receipt");

    // 2. release-proof hp=run launches the hp whole leg AND skips Mac test-macos.
    put(&base.join("trace"), "");
    let (code, out, err) = run_in_env(
        &base,
        &mac,
        "just",
        &["release-proof", &expect, "hp=run"],
        &env_refs,
    );
    assert_eq!(code, 0, "hp=run with fresh mac receipt: {out}{err}");
    let trace = read(&base.join("trace"));
    assert!(
        trace.contains("hp:nextest") && trace.contains("hp:doctest"),
        "hp whole suite must still run: {trace}"
    );
    assert!(
        !trace.contains("mac:nextest"),
        "matching mac receipt must skip Mac subset during hp=run: {trace}"
    );
    assert!(
        hp_receipt.is_file(),
        "hp leg must write its whole-suite receipt"
    );
    assert!(
        !hp.join(".local/locks/gate/owner").exists(),
        "hp lane released"
    );
    let combined = format!("{out}{err}");
    assert!(
        combined.contains(&format!("macos-{tree}")),
        "hp=run match must name the reused mac receipt: {combined}"
    );

    // 3. Control flip: remove hp receipt and mismatch tmux in mac receipt -> BOTH legs run.
    std::fs::remove_file(&hp_receipt).unwrap_or_else(|why| panic!("remove hp receipt: {why}"));
    put(
        &mac_receipt,
        &replace_field(&read(&mac_receipt), "tmux", "tmux 3.5"),
    );
    put(&base.join("trace"), "");
    let (code, out, err) = run_in_env(
        &base,
        &mac,
        "just",
        &["release-proof", &expect, "hp=run"],
        &env_refs,
    );
    assert_eq!(code, 0, "hp=run with mismatched mac receipt: {out}{err}");
    let trace = read(&base.join("trace"));
    assert!(
        trace.contains("hp:nextest") && trace.contains("mac:nextest"),
        "mismatched mac receipt must run both legs: {trace}"
    );
}

#[test]
fn docs_and_contract_name_macos_subset_receipt_behaviour() {
    let dev = read(&root().join("docs/development.md"));
    let dev_lower = dev.to_ascii_lowercase();
    assert!(
        dev.contains("macos-<")
            && (dev_lower.contains("kind=macos-subset") || dev_lower.contains("subset_hash")),
        "docs/development.md must document the macos-<tree> subset receipt and its fields"
    );

    let agents = read(&root().join("AGENTS.md"));
    let row = agents
        .lines()
        .find(|line| line.starts_with("| `just release-proof`"))
        .unwrap_or_else(|| panic!("AGENTS.md must have a release-proof lane table row"));
    let row_lower = row.to_ascii_lowercase();
    assert!(
        row_lower.contains("macos-<")
            || row_lower.contains("mac receipt")
            || row_lower.contains("macos receipt")
            || row_lower.contains("subset receipt")
            || row_lower.contains("skip")
            || row_lower.contains("reuse"),
        "AGENTS.md release-proof row must mention reusing/skipping test-macos on a matching receipt: {row}"
    );
}
