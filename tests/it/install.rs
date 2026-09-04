//! The installer, black-box, in both halves: `ae _install --from <bundle>`
//! against a real `$HOME`, and — at the end of the file — the bash bootstrap
//! that produces that bundle in the first place.
//!
//! The subject has to be a real process. `_install` publishes into the home
//! `$HOME` derives and runs the bundle's own core to ask its version, so a
//! library test could only be told what a library test already knows. What is
//! asserted here is the PUBLISHED SHAPE — three members, their modes, the
//! command link, the absence of a journal — plus the refusals that must leave
//! that shape untouched.
//!
//! The bundle fixture is the same three members `just bundle` packages, with a
//! manifest written the way both `sha256sum` and `shasum -a 256` write one. A
//! fixture that spelled it differently would prove the installer accepts a shape
//! nothing ships.
//!
//! The bootstrap half runs `install` itself — the repository's file, not a copy
//! — against a `curl` that serves a directory. Same reasoning, one step earlier:
//! what the script owns is a platform, a download, a proof and an exec, and none
//! of those is a thing a library test can be told about.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

/// A `$HOME` to install into, and a bundle directory to install from.
struct Rig {
    scratch: PathBuf,
    home: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let scratch = PathBuf::from(format!("/tmp/aeinstall.{}.{tag}", std::process::id()));
        let _ = remove(&scratch);
        let home = scratch.join("home");
        assert!(std::fs::create_dir_all(&home).is_ok(), "a fixture home");
        Self { scratch, home }
    }

    /// The three members a bundle carries, under `ae-<version>-<platform>`.
    fn bundle(&self, version: &str) -> PathBuf {
        let dir = self.scratch.join(format!("ae-{version}-fixture"));
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a bundle root");
        write_exec(
            &dir.join("ae-core"),
            &format!("#!/bin/sh\necho \"ae {version}\"\n"),
        );
        write_exec(&dir.join("install"), "#!/bin/sh\necho bootstrap\n");
        rewrite_manifest(&dir);
        dir
    }

    fn versions(&self) -> PathBuf {
        self.home.join(".ae").join("versions")
    }

    fn version_dir(&self, version: &str) -> PathBuf {
        self.versions().join(version)
    }

    fn link(&self) -> PathBuf {
        self.home.join(".local").join("bin").join("ae")
    }

    fn journal(&self) -> PathBuf {
        self.home.join(".ae").join(".ae-install.journal")
    }

    /// `ae _install --from <dir>` with `HOME` pointed at the fixture.
    fn install(&self, from: &Path) -> (Option<i32>, String, String) {
        self.run(&["_install", "--from", &from.to_string_lossy()])
    }

    fn run(&self, argv: &[&str]) -> (Option<i32>, String, String) {
        #[allow(
            clippy::disallowed_types,
            reason = "the black-box door: an install is what a real process does to a real HOME"
        )]
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_ae"));
        let out = command
            .env_remove("AE_HOME")
            .env_remove("CONFIG_FILE")
            .env_remove("AE_VERSION")
            .env_remove("TMUX")
            .env("HOME", &self.home)
            .args(argv)
            .output()
            .unwrap_or_else(|why| panic!("the product binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = remove(&self.scratch);
    }
}

fn write_exec(path: &Path, text: &str) {
    let _ = std::fs::remove_file(path);
    assert!(std::fs::write(path, text).is_ok(), "a bundle member");
    assert!(
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).is_ok(),
        "an executable member"
    );
}

/// The manifest, over whatever the two members currently hold.
fn rewrite_manifest(dir: &Path) {
    use std::fmt::Write as _;
    let mut text = String::new();
    for name in ["ae-core", "install"] {
        let bytes = read(&dir.join(name));
        let _ = writeln!(text, "{}  {name}", ae::install::sha256_hex(&bytes));
    }
    let path = dir.join("SHA256SUMS");
    let _ = std::fs::remove_file(&path);
    assert!(std::fs::write(&path, text).is_ok(), "a manifest");
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|why| panic!("{}: {why}", path.display()))
}

fn mode(path: &Path) -> u32 {
    let meta =
        std::fs::symlink_metadata(path).unwrap_or_else(|why| panic!("{}: {why}", path.display()));
    meta.permissions().mode() & 0o7777
}

/// Remove a tree whose members may be 0555 — the mode a publish leaves.
fn remove(path: &Path) -> std::io::Result<()> {
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o755));
            if entry.path().is_dir() {
                let _ = remove(&entry.path());
            }
        }
    }
    std::fs::remove_dir_all(path)
}

/// The target of a symlink that must be there.
fn read_link(path: &Path) -> PathBuf {
    std::fs::read_link(path).unwrap_or_else(|why| panic!("{}: {why}", path.display()))
}

/// Whether anything at all sits at `path` — a dangling link included.
fn present(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

// ─── the published shape ─────────────────────────────────────────────────

#[test]
fn a_verified_bundle_publishes_the_layout_and_the_modes_that_make_it_immutable() {
    let rig = Rig::new("layout");
    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.8.1"));
    assert_eq!(code, Some(0), "install failed: {stdout}{stderr}");

    let dir = rig.version_dir("2026.8.1");
    assert_eq!(mode(&dir.join("ae-core")), 0o555, "the core is immutable");
    assert_eq!(
        mode(&dir.join("install")),
        0o555,
        "the sibling is immutable"
    );
    assert_eq!(mode(&dir.join("SHA256SUMS")), 0o444, "the manifest is 0444");
    assert_eq!(mode(&dir), 0o555, "the version directory is immutable");

    // THE COMMAND LINK IS THE CURRENT POINTER: it names the CORE inside the
    // version directory, with no intermediate pointer to resolve through.
    assert_eq!(
        std::fs::read_link(rig.link()).ok(),
        Some(dir.join("ae-core")),
        "the command link does not name the published core"
    );
    for retired in [".ae/current", ".ae/core"] {
        assert!(
            !present(&rig.home.join(retired)),
            "a retired pointer was published: {retired}"
        );
    }
    assert!(
        !present(&rig.journal()),
        "a completed install left a journal"
    );
    // B21: the installer publishes no config. A config is the launch's to
    // create, and an installer that seeded one would have to be able to put an
    // operator's back.
    assert!(
        !present(&rig.home.join(".ae").join("config")),
        "the installer wrote a config"
    );
}

#[test]
fn the_published_directory_refuses_a_stray_write_through_the_command_link() {
    // The hazard AGENTS.md states in prose, measured: a helper is a symlink to
    // the published core and `>` FOLLOWS a symlink, so a stray redirect would
    // truncate the binary every session on the machine is bound to. A 0555
    // directory is what turns that into EACCES.
    let rig = Rig::new("immutable");
    assert_eq!(rig.install(&rig.bundle("2026.8.1")).0, Some(0));
    let core = rig.version_dir("2026.8.1").join("ae-core");
    let before = read(&core);

    assert!(
        std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(rig.link())
            .is_err(),
        "a redirect through the command link was allowed to write"
    );
    assert!(
        std::fs::write(rig.version_dir("2026.8.1").join("newfile"), "x").is_err(),
        "a new entry inside the version directory was allowed"
    );
    assert_eq!(
        std::fs::read(&core).ok(),
        Some(before),
        "the published core is not byte-identical afterwards"
    );
}

// ─── refusals that write nothing ─────────────────────────────────────────

#[test]
fn a_member_that_disagrees_with_the_bundle_manifest_is_refused_before_any_write() {
    let rig = Rig::new("tamper");
    let bundle = rig.bundle("2026.8.1");
    // The manifest is written first and the member changed after: exactly the
    // shape of a bundle whose bytes were replaced in transit.
    let member = bundle.join("ae-core");
    let mut text = String::from_utf8_lossy(&read(&member)).into_owned();
    text.push_str("# tampered\n");
    write_exec(&member, &text);

    let (code, stdout, stderr) = rig.install(&bundle);
    assert_eq!(code, Some(1), "a tampered bundle installed: {stdout}");
    assert!(
        stderr.contains("checksum mismatch for ae-core"),
        "the refusal does not name the member: {stderr}"
    );
    assert!(
        !present(&rig.versions()),
        "a refused bundle created a versions directory"
    );
    assert!(!present(&rig.link()), "a refused bundle published a link");
}

#[test]
fn a_version_already_installed_with_different_bytes_is_refused() {
    let rig = Rig::new("rebytes");
    let bundle = rig.bundle("2026.8.1");
    assert_eq!(rig.install(&bundle).0, Some(0));
    let published = read(&rig.version_dir("2026.8.1").join("ae-core"));

    // Same version word, different content — the name is a promise about the
    // bytes, so republishing under it is a refusal rather than an overwrite.
    write_exec(
        &bundle.join("install"),
        "#!/bin/sh\necho bootstrap\n# different\n",
    );
    rewrite_manifest(&bundle);
    let (code, _, stderr) = rig.install(&bundle);
    assert_eq!(code, Some(1), "a different build reused a version word");
    assert!(
        stderr.contains("already installed with different bytes"),
        "{stderr}"
    );
    assert_eq!(
        read(&rig.version_dir("2026.8.1").join("ae-core")),
        published,
        "the refusal rewrote the published core"
    );
}

#[test]
fn a_command_destination_that_resolves_into_the_ae_home_is_refused() {
    // B14, and the time-dependence is the point: `~/.local` is a DANGLING link
    // into the home when the installer starts, and creating `~/.ae` is what
    // makes it live. Resolving the ANCESTOR is what catches it.
    let rig = Rig::new("b14");
    let _ = std::fs::remove_dir_all(rig.home.join(".ae"));
    assert!(
        std::os::unix::fs::symlink(rig.home.join(".ae").join("bin"), rig.home.join(".local"))
            .is_ok(),
        "a dangling .local into the home"
    );
    let (code, _, stderr) = rig.install(&rig.bundle("2026.8.1"));
    assert_eq!(code, Some(1), "an aliased destination was accepted");
    assert!(
        stderr.contains("ae home") || stderr.contains("symlink"),
        "the refusal does not name the alias: {stderr}"
    );
    assert!(
        !present(&rig.home.join(".ae").join("bin").join("ae")),
        "a command link was published inside the ae home"
    );
}

#[test]
fn a_directory_where_the_command_link_belongs_is_refused() {
    // F8. A rename onto a directory NESTS instead of replacing and reports
    // success, so the leaf is lstat'ed before anything is published.
    let rig = Rig::new("f8");
    assert!(std::fs::create_dir_all(rig.link()).is_ok(), "a directory");
    let (code, _, stderr) = rig.install(&rig.bundle("2026.8.1"));
    assert_eq!(code, Some(1), "a directory destination was accepted");
    assert!(
        stderr.contains("ae command destination is a directory"),
        "{stderr}"
    );
    assert!(
        rig.link().is_dir() && !rig.link().is_symlink(),
        "the destination stopped being the directory it was"
    );
}

#[test]
fn a_bad_argv_is_a_usage_error_that_touches_nothing() {
    let rig = Rig::new("usage");
    for argv in [
        vec!["_install"],
        vec!["_install", "--from"],
        vec!["_install", "--into", "/tmp"],
    ] {
        let (code, _, stderr) = rig.run(&argv);
        assert_eq!(code, Some(2), "{argv:?} was not a usage error");
        assert!(stderr.contains("_install --from"), "{stderr}");
    }
    assert!(
        !present(&rig.versions()),
        "a usage error published something"
    );
}

// ─── the journal ─────────────────────────────────────────────────────────

#[test]
fn a_hostile_journal_is_refused_and_preserved_for_diagnosis() {
    // The journal is replayed by a LATER process and its replay removes
    // directories and rewrites the command link, so every field is checked
    // against a fact this installer already knows. A record it will not accept
    // is preserved and refuses the run — never half-replayed.
    let ae_home = |rig: &Rig| rig.home.join(".ae");
    for (tag, body) in [
        // A journal from another installer: the retired bash field set.
        (
            "retired-field",
            "format=2\nversion=2026.8.1\nhome=$HOME/.ae\nlink=$HOME/.local/bin/ae\n\
             link_had=0\nlink_old=\nconfig=$HOME/.ae/config\nconfig_had=0\n",
        ),
        // A directory row naming live state, which a replay would rmdir.
        (
            "live-state",
            "format=3\npid=1\nversion=2026.8.1\nhome=$HOME/.ae\nlink=$HOME/.local/bin/ae\n\
             link_had=0\nlink_old=\ncreated_dir=$HOME/.ae/sessions/live\n",
        ),
        // A record for somebody else's command path.
        (
            "foreign-link",
            "format=3\npid=1\nversion=2026.8.1\nhome=$HOME/.ae\nlink=/tmp/elsewhere/ae\n\
             link_had=0\nlink_old=\n",
        ),
    ] {
        let rig = Rig::new(tag);
        assert!(std::fs::create_dir_all(ae_home(&rig)).is_ok());
        let text = body.replace("$HOME", &rig.home.to_string_lossy());
        assert!(std::fs::write(rig.journal(), &text).is_ok(), "a journal");

        let (code, _, stderr) = rig.install(&rig.bundle("2026.8.1"));
        assert_eq!(code, Some(1), "{tag}: a hostile journal was accepted");
        assert!(
            stderr.contains("journal"),
            "{tag}: the refusal does not name the journal: {stderr}"
        );
        assert_eq!(
            std::fs::read_to_string(rig.journal()).ok(),
            Some(text),
            "{tag}: the journal was not preserved verbatim"
        );
        assert!(!present(&rig.link()), "{tag}: a link was published anyway");
    }
}

#[test]
fn an_interrupted_install_is_reversed_by_the_next_run_which_then_completes() {
    // A signal leaves the journal on disk — there is no trap to run, and none
    // is wanted: the next run finds a DEAD pid, replays the record, and starts
    // its own install. The recorded link is restored to what it named before.
    let rig = Rig::new("recover");
    let bundle = rig.bundle("2026.8.1");
    assert_eq!(rig.install(&bundle).0, Some(0));
    let published = read_link(&rig.link());

    // The record a crashed successor would have left: it repointed the link at
    // a version that never landed, and created nothing.
    let stale = rig.version_dir("2026.9.9").join("ae-core");
    assert!(std::fs::remove_file(rig.link()).is_ok());
    assert!(std::os::unix::fs::symlink(&stale, rig.link()).is_ok());
    let text = format!(
        "format=3\npid=2147483646\nversion=2026.9.9\nhome={}\nlink={}\nlink_had=1\nlink_old={}\n",
        rig.home.join(".ae").display(),
        rig.link().display(),
        published.display()
    );
    assert!(
        std::fs::write(rig.journal(), text).is_ok(),
        "a stale journal"
    );

    let (code, stdout, stderr) = rig.install(&bundle);
    assert_eq!(
        code,
        Some(0),
        "the rerun did not complete: {stdout}{stderr}"
    );
    assert_eq!(
        std::fs::read_link(rig.link()).ok(),
        Some(published),
        "the rerun did not put the command link back where the journal said"
    );
    assert!(!present(&rig.journal()), "the rerun left a journal behind");
}

// ─── switching versions ──────────────────────────────────────────────────

#[test]
fn a_second_install_repoints_the_link_and_leaves_the_first_version_intact() {
    // A21. Switching versions is ONE atomic rename of the command link; the
    // version directory it stops naming is not touched, so the previous ae is
    // still there to point back at.
    let rig = Rig::new("switch");
    assert_eq!(rig.install(&rig.bundle("2026.8.1")).0, Some(0));
    assert_eq!(rig.install(&rig.bundle("2026.8.2")).0, Some(0));

    assert_eq!(
        std::fs::read_link(rig.link()).ok(),
        Some(rig.version_dir("2026.8.2").join("ae-core")),
        "the command link was not repointed at the new version"
    );
    for name in ["ae-core", "install", "SHA256SUMS"] {
        assert!(
            rig.version_dir("2026.8.1").join(name).is_file(),
            "the old version lost {name}"
        );
    }
    assert!(!present(&rig.journal()), "a switch left a journal");
}

#[test]
fn reinstalling_the_same_bytes_restates_the_published_modes() {
    // The version directory is the only thing standing between the installed
    // core and a stray write, so a re-install of identical bytes re-asserts the
    // modes rather than assuming them.
    let rig = Rig::new("remode");
    let bundle = rig.bundle("2026.8.1");
    assert_eq!(rig.install(&bundle).0, Some(0));

    let dir = rig.version_dir("2026.8.1");
    assert!(std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).is_ok());
    assert!(
        std::fs::set_permissions(dir.join("ae-core"), std::fs::Permissions::from_mode(0o755))
            .is_ok()
    );

    assert_eq!(rig.install(&bundle).0, Some(0), "a re-install was refused");
    assert_eq!(
        mode(&dir.join("ae-core")),
        0o555,
        "the core stayed writable"
    );
    assert_eq!(mode(&dir), 0o555, "the directory stayed writable");
}

// ─── upgrade ─────────────────────────────────────────────────────────────

#[test]
fn upgrade_refuses_a_bad_pin_before_it_reaches_the_network() {
    // `AE_VERSION` is the target pin and the ONLY input `ae upgrade` takes. A
    // value that is not a CalVer tag is refused where it is read, so a typo
    // never becomes a request for a release that cannot exist.
    let rig = Rig::new("pin");
    #[allow(
        clippy::disallowed_types,
        reason = "the black-box door: upgrade's refusal is a real process's exit status"
    )]
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_ae"));
    let out = command
        .env("HOME", &rig.home)
        .env("AE_VERSION", "not-a-version")
        .arg("upgrade")
        .output()
        .expect("the product binary should run");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("AE_VERSION must be a CalVer tag"),
        "{stderr}"
    );
    assert!(
        !present(&rig.versions()),
        "a refused pin published something"
    );
}

// ─── the bash bootstrap ──────────────────────────────────────────────────

/// The platform `install` resolves for THIS host, spelled the way its `uname`
/// case spells it. ae publishes two bundles and the two CI legs are both of
/// them, so a host outside the pair has no bundle to fetch and nothing here to
/// prove — it is a stated failure rather than a silent skip, because a test
/// that quietly stops running is the failure this project keeps meeting.
const BOOTSTRAP_PLATFORM: &str = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    "darwin-arm64"
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    "linux-x86_64-musl"
} else {
    ""
};

/// The version the fixture release carries. Not the crate's: the bundle core
/// here is a stub, and a version that could be confused with a real one would
/// make a stale assertion look right.
const BOOTSTRAP_VERSION: &str = "2026.9.99";

const RELEASES: &str = "https://github.com/clemens33/ae/releases";

/// How the bootstrap was started. `File` is `bash install`; `Pipe` is the
/// advertised one-liner, where bash reads the script off a pipe it cannot seek.
enum Entry {
    File,
    Pipe,
}

/// One child process, for a subject that has to be one.
#[allow(
    clippy::disallowed_types,
    reason = "the black-box door: a bootstrap that is not a real process is not the subject"
)]
fn command(program: &str) -> std::process::Command {
    std::process::Command::new(program)
}

/// A fixture release, a `curl` that serves it off the disk, and the `$HOME` and
/// `$TMPDIR` a run is confined to.
///
/// NOTHING HERE OPENS A SOCKET. The shim answers from a directory and exits 22
/// — curl's own "not found" — for anything else, so a run that reached past it
/// fails rather than downloading.
struct Bootstrap {
    scratch: PathBuf,
    /// The bundle's own manifest, kept to compare against what the core was
    /// handed: it proves the tree under `--from` is THIS bundle, extracted.
    manifest: Vec<u8>,
}

impl Bootstrap {
    fn new() -> Self {
        let scratch = PathBuf::from(format!("/tmp/aebootstrap.{}", std::process::id()));
        let _ = remove(&scratch);
        let root = scratch.join("bundle").join(bundle_name());
        for dir in [
            &scratch.join("bin"),
            &scratch.join("home"),
            &scratch.join("tmp"),
            &scratch.join("release"),
            &root,
        ] {
            assert!(std::fs::create_dir_all(dir).is_ok(), "a fixture directory");
        }

        // The bundle's three members. The core is a stub that records what it
        // was handed — the real one is what the rest of this file drives — and
        // the sibling is the repository's own `install`, which is the member a
        // release actually ships.
        write_exec(
            &root.join("ae-core"),
            r#"#!/bin/sh
: > "$AE_FIXTURE_REACHED"
for a in "$@"; do printf 'arg %s
' "$a" >> "$AE_FIXTURE_REACHED"; done
printf 'self %s
' "$0" >> "$AE_FIXTURE_REACHED"
root="$(dirname "$0")"
for m in ae-core install SHA256SUMS; do
    [ -f "$root/$m" ] && printf 'member %s
' "$m" >> "$AE_FIXTURE_REACHED"
done
cp "$root/SHA256SUMS" "$AE_FIXTURE_REACHED.manifest"
"#,
        );
        write_exec(
            &root.join("install"),
            &String::from_utf8_lossy(&read(&repository().join("install"))),
        );
        rewrite_manifest(&root);
        let manifest = read(&root.join("SHA256SUMS"));

        // The archive, packed the way `just bundle` packs one: the version
        // directory itself is the single top-level entry.
        let archive = scratch.join("release").join(archive_name());
        let packed = command("tar")
            .env("COPYFILE_DISABLE", "1")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(scratch.join("bundle"))
            .arg(bundle_name())
            .status()
            .unwrap_or_else(|why| panic!("tar should run: {why}"));
        assert!(packed.success(), "the fixture archive should pack");

        // The RELEASE manifest, which is what `curl` serves first. The decoy
        // line is a DIFFERENT platform at a DIFFERENT version on purpose: the
        // scan that picks a version has to filter on the platform, and a filter
        // that stopped working would pick 2026.1.1 and ask for an archive this
        // release does not have.
        let digest = ae::install::sha256_hex(&read(&archive));
        let decoy = if BOOTSTRAP_PLATFORM == "darwin-arm64" {
            "linux-x86_64-musl"
        } else {
            "darwin-arm64"
        };
        let zeros = "0".repeat(64);
        let listing = format!(
            "{zeros}  ae-2026.1.1-{decoy}.tar.gz\n{digest}  {}\n",
            archive_name()
        );
        assert!(
            std::fs::write(scratch.join("release").join("SHA256SUMS"), listing).is_ok(),
            "the release manifest"
        );

        // The curl that never leaves the disk. It selects the URL by scheme
        // rather than by position, so `--retry 3` cannot be mistaken for one.
        write_exec(
            &scratch.join("bin").join("curl"),
            r#"#!/bin/sh
out=""
url=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output) out="$2"; shift 2 ;;
        http*) url="$1"; shift ;;
        *) shift ;;
    esac
done
printf '%s
' "$url" >> "$AE_FIXTURE_LOG"
name="${url##*/}"
[ -f "$AE_FIXTURE_RELEASE/$name" ] || exit 22
cat "$AE_FIXTURE_RELEASE/$name" > "$out"
"#,
        );

        Self { scratch, manifest }
    }

    fn reached(&self) -> PathBuf {
        self.scratch.join("reached")
    }

    fn log(&self) -> PathBuf {
        self.scratch.join("curl.log")
    }

    /// Forget the last run, so the next one's evidence is its own.
    fn reset(&self) {
        for name in ["reached", "reached.manifest", "curl.log"] {
            let _ = std::fs::remove_file(self.scratch.join(name));
        }
    }

    /// Run the REPOSITORY's `install`, with the network shimmed and `$HOME`,
    /// `$TMPDIR` pointed at the fixture.
    fn run(&self, entry: &Entry, pin: Option<&str>) -> (Option<i32>, String) {
        let path = format!(
            "{}:{}",
            self.scratch.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut command = command("bash");
        command
            .env("PATH", path)
            .env("HOME", self.scratch.join("home"))
            .env("TMPDIR", self.scratch.join("tmp"))
            .env("AE_FIXTURE_RELEASE", self.scratch.join("release"))
            .env("AE_FIXTURE_LOG", self.log())
            .env("AE_FIXTURE_REACHED", self.reached());
        match pin {
            Some(version) => command.env("AE_VERSION", version),
            None => command.env_remove("AE_VERSION"),
        };
        let script = repository().join("install");
        let out = match *entry {
            Entry::File => command.arg(&script).output(),
            Entry::Pipe => {
                use std::io::Write as _;
                command.stdin(std::process::Stdio::piped());
                let mut child = command
                    .spawn()
                    .unwrap_or_else(|why| panic!("bash should run: {why}"));
                let text = read(&script);
                let mut stdin = child
                    .stdin
                    .take()
                    .unwrap_or_else(|| panic!("bash should have a piped stdin"));
                // 79 lines fit a pipe buffer many times over, so one write and
                // the close that drops it cannot deadlock against the child.
                assert!(stdin.write_all(&text).is_ok(), "the script should pipe");
                drop(stdin);
                child.wait_with_output()
            }
        }
        .unwrap_or_else(|why| panic!("bash should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// Every line the stub core recorded, in the order it wrote them.
    fn record(&self) -> Vec<String> {
        String::from_utf8_lossy(&read(&self.reached()))
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The URLs the fixture curl was asked for.
    fn urls(&self) -> Vec<String> {
        String::from_utf8_lossy(&read(&self.log()))
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// What every successful run must have done, whatever entry started it.
    fn assert_reached(&self, release: &str) {
        let record = self.record();
        let from = record
            .iter()
            .position(|line| line == "arg --from")
            .and_then(|at| record.get(at + 1))
            .and_then(|line| line.strip_prefix("arg "))
            .unwrap_or_else(|| panic!("no `--from` in the record: {record:?}"))
            .to_owned();

        assert_eq!(
            record.iter().take(3).collect::<Vec<_>>(),
            vec!["arg _install", "arg --from", &format!("arg {from}")],
            "the bootstrap called the core with something other than `_install --from <root>`"
        );
        assert_eq!(
            Path::new(&from).file_name().map(std::ffi::OsStr::to_owned),
            Some(bundle_name().into()),
            "the root handed over is not the bundle's own directory: {from}"
        );
        assert!(
            Path::new(&from).starts_with(self.scratch.join("tmp")),
            "the root came from outside the fixture's own TMPDIR: {from}"
        );
        assert!(
            record.contains(&format!("self {from}/ae-core")),
            "the core that ran is not the one inside the extracted root: {record:?}"
        );
        for member in ["ae-core", "install", "SHA256SUMS"] {
            assert!(
                record.contains(&format!("member {member}")),
                "the extracted root is missing {member}: {record:?}"
            );
        }
        assert_eq!(
            read(&self.scratch.join("reached.manifest")),
            self.manifest,
            "the tree under --from is not the bundle that was verified"
        );

        // The temporary tree is the run's own, and the EXIT trap takes it with
        // it — a bootstrap that left one behind would leak a core per install.
        assert!(!present(Path::new(&from)), "the temporary tree survived");
        // The script publishes NOTHING itself: everything a publication is
        // belongs to the core it hands the bundle to.
        assert!(
            !present(&self.scratch.join("home").join(".ae")),
            "the bootstrap wrote into HOME"
        );
        assert_eq!(
            self.urls(),
            vec![
                format!("{RELEASES}/{release}/download/SHA256SUMS"),
                format!("{RELEASES}/{release}/download/{}", archive_name()),
            ],
            "the bootstrap fetched something other than the manifest and its bundle"
        );
    }
}

impl Drop for Bootstrap {
    fn drop(&mut self) {
        let _ = remove(&self.scratch);
    }
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn bundle_name() -> String {
    format!("ae-{BOOTSTRAP_VERSION}-{BOOTSTRAP_PLATFORM}")
}

fn archive_name() -> String {
    format!("{}.tar.gz", bundle_name())
}

/// The bootstrap, black-box: both entries reach the core with a bundle that was
/// PROVEN first.
///
/// `install` is the only bash file ae still ships and the one file no Rust test
/// ran — the rest of this module starts at `ae _install --from <bundle>`, which
/// is where the bootstrap ENDS. What it owns before that hand-off is the whole
/// of the one-liner's safety: resolve the platform, fetch the release manifest,
/// fetch the archive, prove the archive against that manifest, and only then
/// extract and run. Each of those is asserted here against a `curl` that serves
/// a directory, so the test never opens a socket, and against a `$HOME` and
/// `$TMPDIR` of its own, so it never touches the real one.
///
/// The tamper control at the end is what makes the two runs above mean
/// something: without it a bootstrap that skipped the checksum entirely would
/// pass this test twice over.
#[test]
fn the_bootstrap_proves_the_bundle_then_hands_the_extracted_root_to_the_core() {
    assert!(
        !BOOTSTRAP_PLATFORM.is_empty(),
        "ae publishes bundles for darwin-arm64 and linux-x86_64-musl; this host is neither, \
         so `install` would refuse before doing anything this test could assert"
    );
    let fixture = Bootstrap::new();

    // THE FILE ENTRY, taking the latest release: the version is not given, so
    // the release manifest is what names it.
    let (code, stderr) = fixture.run(&Entry::File, None);
    assert_eq!(code, Some(0), "the file entry failed: {stderr}");
    fixture.assert_reached("latest");

    // THE PIPE ENTRY — the advertised `curl … | bash` — with `AE_VERSION`
    // pinned, which is the other half of the version grammar.
    fixture.reset();
    let (code, stderr) = fixture.run(&Entry::Pipe, Some(BOOTSTRAP_VERSION));
    assert_eq!(code, Some(0), "the pipe entry failed: {stderr}");
    fixture.assert_reached(&format!("v{BOOTSTRAP_VERSION}"));

    // THE CONTROL: an archive whose bytes disagree with the manifest is
    // refused, and the refusal happens BEFORE the extraction — so the core
    // never runs, which is the thing that would matter on a real machine.
    fixture.reset();
    let archive = fixture.scratch.join("release").join(archive_name());
    let mut tampered = read(&archive);
    tampered.push(b'!');
    assert!(std::fs::write(&archive, &tampered).is_ok(), "the tamper");
    let (code, stderr) = fixture.run(&Entry::File, None);
    assert_eq!(
        code,
        Some(1),
        "a tampered archive was not refused: {stderr}"
    );
    assert!(
        stderr.contains("checksum mismatch"),
        "the refusal did not name the checksum: {stderr}"
    );
    assert!(
        !present(&fixture.reached()),
        "the core ran on bytes that failed their checksum"
    );
}
