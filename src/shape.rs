//! What this binary IS — and therefore whose environment it trusts.
//!
//! Slice Z3 deletes `ae-entry`. The public `ae` is a symlink to THIS binary, so
//! the two things the wrapper decided from its own file name are decided here
//! from [`std::env::current_exe`]:
//!
//! * **INSTALLED** — the resolved executable is `<HOME>/.ae/versions/<V>/ae-core`,
//!   a directory the canonical installer published. ae state is `<HOME>/.ae` and
//!   nothing an inherited variable says can move it; the version directory is
//!   validated before any effect.
//! * **CHECKOUT** — anywhere else. `AE_HOME`, `CONFIG_FILE` and the
//!   `AE_TMUX_SERVER` pair are HONOURED. That is the `ae-dev` namespace and the
//!   two bash suites, which are its only callers.
//!
//! # Why the position and not the health
//!
//! C51's lesson, ported: the wrapper once keyed "am I a bundle" off whether its
//! sibling core looked usable, so a bundle whose core was missing silently
//! degraded into checkout semantics *at the public boundary*. Shape is decided
//! by WHERE this binary sits and nothing else. A binary sitting in a version
//! directory that does not validate is a BROKEN INSTALL — a refusal — never a
//! checkout.
//!
//! # What is validated, and what is not
//!
//! The installer publishes exactly three members and one manifest naming two of
//! them ([`MANIFEST`]). This module proves the STRUCTURE on every invocation:
//! the members are regular non-symlink files, the manifest parses, and it names
//! exactly the two members it is supposed to. It does NOT re-hash them, and the
//! omission is deliberate on two counts: the core has no SHA-256 primitive and
//! adding a dependency for one is a supply-chain decision nobody made, and
//! hashing a ~2.4 MB binary on every invocation would be paid by every helper
//! call in every live session. Content integrity is proven ONCE, by `install`,
//! which verifies the published checksums before it extracts anything.
//!
//! MODE IS NOT VALIDATED HERE, and that is a ruling rather than an oversight:
//! the version directory and its members are published read-only (0555/0444),
//! but a WRITABLE core is a `doctor` WARN (slice Z3 contract (e)). A refusal on
//! mode would make that warning unreachable — `doctor` could never run to print
//! it — and would brick every helper in a live session over a stray `chmod`.

use std::path::{Path, PathBuf};

/// The installer's manifest: the file it writes beside the members it covers.
pub const MANIFEST: &str = "SHA256SUMS";

/// The core member — this binary, under the name the installer publishes it as.
pub const CORE: &str = "ae-core";

/// The installer member: the immutable sibling `upgrade` execs.
pub const INSTALLER: &str = "install";

/// The directory a version directory sits in, under ae's home.
pub const VERSIONS: &str = "versions";

/// What this binary is, and whose environment it therefore trusts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    /// Published by the canonical installer: `<home>/versions/<version>/ae-core`.
    Installed {
        /// `<HOME>/.ae` — where ae state lives, and not negotiable.
        home: PathBuf,
        /// The version directory this binary was published into.
        version_dir: PathBuf,
        /// That directory's name, which must be the version this binary reports.
        version: String,
    },
    /// Anything else: a build in a checkout, run by `ae-dev` or the suites.
    Checkout,
}

impl Shape {
    /// Whether inherited `AE_HOME` / `CONFIG_FILE` / `AE_TMUX_SERVER*` are read.
    #[must_use]
    pub fn honours_environment(&self) -> bool {
        matches!(self, Self::Checkout)
    }
}

/// Classify `exe` — an already-resolved `current_exe()` — against `home`, the
/// value of `$HOME`.
///
/// POSITIONAL AND TOTAL: `<home>/.ae/versions/<anything>/ae-core` is installed,
/// everything else is a checkout. The directory's NAME is not checked here —
/// a wrong name is a broken install that must refuse, and refusing is
/// [`validate`]'s job, not classification's.
#[must_use]
pub fn classify(exe: &Path, home: Option<&Path>) -> Shape {
    let (Some(home), Some(file)) = (home, exe.file_name()) else {
        return Shape::Checkout;
    };
    if file != CORE {
        return Shape::Checkout;
    }
    let Some(version_dir) = exe.parent() else {
        return Shape::Checkout;
    };
    let Some(versions) = version_dir.parent() else {
        return Shape::Checkout;
    };
    let ae_home = home.join(".ae");
    if versions != ae_home.join(VERSIONS) {
        return Shape::Checkout;
    }
    let Some(version) = version_dir.file_name().and_then(|name| name.to_str()) else {
        return Shape::Checkout;
    };
    Shape::Installed {
        home: ae_home,
        version_dir: version_dir.to_path_buf(),
        version: version.to_owned(),
    }
}

/// Why an installed version directory was refused.
///
/// One line each, and each names the repair. `ae upgrade` runs ahead of this
/// gate precisely so a refusal here still has a way out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broken(pub String);

impl std::fmt::Display for Broken {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "ae: {}\nae: run 'ae upgrade' to reinstall.", self.0)
    }
}

/// The manifest as the installer writes it: `<64 hex><SP><SP><name>` lines,
/// one per covered member, LF-terminated, bare basenames, no other content.
///
/// Returns the names in file order, or the offending line.
///
/// # Errors
///
/// The first line that is not exactly that shape.
pub fn parse_manifest(text: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    // A trailing LF is required, so the split leaves one empty final element and
    // any other empty element is a blank line — which the format does not have.
    let Some(body) = text.strip_suffix('\n') else {
        return Err("the manifest does not end in a newline".to_owned());
    };
    for line in body.split('\n') {
        let Some((digest, name)) = line.split_once("  ") else {
            return Err(format!("malformed manifest line: {line}"));
        };
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("malformed manifest line: {line}"));
        }
        if digest.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(format!("malformed manifest line: {line}"));
        }
        if name.is_empty() || name.contains('/') || name.contains(' ') {
            return Err(format!("malformed manifest line: {line}"));
        }
        names.push(name.to_owned());
    }
    Ok(names)
}

/// Whether `names` is exactly the covered set, in any order.
///
/// The manifest does not cover itself, so two members and no more.
#[must_use]
pub fn manifest_covers_members(names: &[String]) -> bool {
    let mut sorted: Vec<&str> = names.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    sorted.len() == names.len() && sorted == [CORE, INSTALLER]
}

/// One member's on-disk classification, as [`validate`] needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Member {
    /// A regular file that is not a symlink — the only acceptable answer.
    Regular,
    /// Present, but a link, a directory, or a device.
    NotRegular,
    /// Not there at all.
    Absent,
}

/// What [`validate`] needs to look at, so the ruling itself stays pure.
pub trait VersionDir {
    /// How `name` in the version directory classifies.
    fn member(&self, name: &str) -> Member;
    /// The manifest's bytes as text, or `None` when it could not be read.
    fn manifest(&self) -> Option<String>;
}

/// Prove the version directory is the one `install` published, or say what is
/// wrong with it.
///
/// Runs BEFORE any effect and refuses the whole invocation — the same place
/// the wrapper's pair gate stood, and for the same reason: a core nobody can
/// vouch for must not create a session, deliver a message, or write a byte of
/// state.
///
/// # Errors
///
/// [`Broken`], one line naming what failed and how to repair it.
pub fn validate(dir: &impl VersionDir, version: &str, reports: &str) -> Result<(), Broken> {
    if version != reports {
        return Err(Broken(format!(
            "this core is installed as version {version} but reports {reports}"
        )));
    }
    for name in [CORE, INSTALLER, MANIFEST] {
        match dir.member(name) {
            Member::Regular => {}
            Member::Absent => {
                return Err(Broken(format!("the installed version is missing {name}")));
            }
            Member::NotRegular => {
                return Err(Broken(format!(
                    "the installed {name} is not a regular file"
                )));
            }
        }
    }
    let Some(text) = dir.manifest() else {
        return Err(Broken(format!("the installed {MANIFEST} cannot be read")));
    };
    let names = parse_manifest(&text).map_err(|why| Broken(format!("{MANIFEST}: {why}")))?;
    if !manifest_covers_members(&names) {
        return Err(Broken(format!(
            "{MANIFEST} does not name exactly {CORE} and {INSTALLER}"
        )));
    }
    Ok(())
}

/// The real version directory: the three doors this module opens on disk.
///
/// `symlink_metadata`, never `metadata` — a member that is a symlink to a
/// mutable file outside the immutable directory passes every follow-test and is
/// then executed as if it were ours. That is C51/C67 restated, and it is why
/// the classification lstats first.
pub struct OnDisk<'a>(pub &'a Path);

impl VersionDir for OnDisk<'_> {
    fn member(&self, name: &str) -> Member {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: an immutable bundle member is proven REGULAR and NON-SYMLINK before it is trusted, so the classification must be the non-following one — see clippy.toml"
        )]
        let probe = std::fs::symlink_metadata(self.0.join(name));
        match probe {
            Ok(meta) if meta.file_type().is_file() => Member::Regular,
            Ok(_) => Member::NotRegular,
            Err(_) => Member::Absent,
        }
    }

    fn manifest(&self) -> Option<String> {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the installer's manifest is what proves this version directory is the one it published — see clippy.toml"
        )]
        let read = std::fs::read_to_string(self.0.join(MANIFEST));
        read.ok()
    }
}

/// This process's resolved executable, or `None` when the OS will not say.
///
/// `current_exe`, deliberately, and NOT `argv[0]`: the two answer different
/// questions and slice Z2 pinned the difference. `argv[0]` says which SESSION
/// HELPER this is ([`crate::shim`]); `current_exe` says which BINARY is
/// running, which is the only thing that can decide the shape.
///
/// RESOLVED HERE, not by the OS. `current_exe` is only as resolved as the
/// platform makes it: Linux answers from `/proc/self/exe`, which is the real
/// file, while macOS answers the path this process was EXEC'D BY — the symlink,
/// not its target. Slice Z3 makes that difference decide the product: `ae` is
/// `~/.local/bin/ae`, a link, and every session helper is another link, so on
/// macOS an unresolved answer names a file called `ae` or `send` and the
/// positional test below sees a CHECKOUT for every installed invocation on the
/// machine. Canonicalising here is what makes the two platforms answer the same
/// question.
#[must_use]
pub fn resolved_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the shape is positional, and macOS hands back the link it was exec'd by rather than the file it reached"
    )]
    let resolved = std::fs::canonicalize(&exe);
    Some(resolved.unwrap_or(exe))
}

/// This process's shape, resolved once.
///
/// Cached because it cannot change mid-run and because every reader of ae's
/// state root asks for it — a second classification would be a second answer to
/// "whose environment does this binary trust".
#[must_use]
pub fn current() -> &'static Shape {
    static CELL: std::sync::OnceLock<Shape> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        // BOTH SIDES CANONICAL, or the comparison is of SPELLINGS and not of
        // positions. `current_exe()` resolves every link on the way to this
        // binary; `$HOME` resolves nothing, and one directory reached by two
        // names is ordinary — `/tmp` and `/var` are `/private/...` on macOS,
        // and a `/home` mounted through a link is the same shape on Linux.
        // A home spelled the other way classifies a PUBLISHED core as a
        // checkout, which hands an inherited `AE_HOME` the state root the
        // install owns: exactly the C51 degradation at the public boundary
        // that this module's positional rule exists to refuse.
        let home = crate::doors::home().map(|home| canonical_home(&home));
        classify(
            resolved_exe().unwrap_or_default().as_path(),
            home.as_deref(),
        )
    })
}

/// `$HOME` resolved the way `current_exe()` resolves, or unchanged when the OS
/// will not say — an unresolvable home is a checkout, which is the safe answer.
fn canonical_home(home: &Path) -> PathBuf {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the shape is positional, so the position has to be read the same way on both sides of the comparison"
    )]
    let resolved = std::fs::canonicalize(home);
    resolved.unwrap_or_else(|_| home.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        core: Member,
        installer: Member,
        manifest_member: Member,
        text: Option<String>,
    }

    impl Fake {
        fn good() -> Self {
            Self {
                core: Member::Regular,
                installer: Member::Regular,
                manifest_member: Member::Regular,
                text: Some(format!("{0}  {CORE}\n{0}  {INSTALLER}\n", "a".repeat(64))),
            }
        }
    }

    impl VersionDir for Fake {
        fn member(&self, name: &str) -> Member {
            match name {
                CORE => self.core,
                INSTALLER => self.installer,
                _ => self.manifest_member,
            }
        }
        fn manifest(&self) -> Option<String> {
            self.text.clone()
        }
    }

    #[test]
    fn a_core_in_a_version_directory_under_the_home_is_installed() {
        let shape = classify(
            Path::new("/u/me/.ae/versions/2026.9.1/ae-core"),
            Some(Path::new("/u/me")),
        );
        assert_eq!(
            shape,
            Shape::Installed {
                home: PathBuf::from("/u/me/.ae"),
                version_dir: PathBuf::from("/u/me/.ae/versions/2026.9.1"),
                version: "2026.9.1".to_owned(),
            }
        );
        assert!(!shape.honours_environment());
    }

    #[test]
    fn everything_else_is_a_checkout() {
        for exe in [
            // A checkout build.
            "/w/ae/target/debug/ae",
            // The right name in the wrong place.
            "/opt/ae/ae-core",
            // Another user's install.
            "/u/you/.ae/versions/2026.9.1/ae-core",
            // The version directory itself, without the member name.
            "/u/me/.ae/versions/2026.9.1",
            // One level too deep.
            "/u/me/.ae/versions/2026.9.1/sub/ae-core",
        ] {
            assert_eq!(
                classify(Path::new(exe), Some(Path::new("/u/me"))),
                Shape::Checkout,
                "{exe}"
            );
        }
        assert_eq!(
            classify(Path::new("/u/me/.ae/versions/2026.9.1/ae-core"), None),
            Shape::Checkout,
            "no HOME to compare against"
        );
    }

    #[test]
    fn the_manifest_is_the_installers_two_lines_and_nothing_else() {
        let digest = "0".repeat(64);
        let text = format!("{digest}  {CORE}\n{digest}  {INSTALLER}\n");
        let names = parse_manifest(&text).expect("the installer's own spelling");
        assert_eq!(names, [CORE, INSTALLER]);
        assert!(manifest_covers_members(&names));
    }

    #[test]
    fn a_manifest_that_is_not_that_shape_is_refused() {
        let digest = "0".repeat(64);
        for (text, why) in [
            (
                format!("{digest}  {CORE}\n"),
                "one line does not cover both",
            ),
            (format!("{digest} {CORE}\n"), "one space, not two"),
            (format!("{digest}  {CORE}"), "no trailing newline"),
            (format!("{digest}  {CORE}\n\n"), "a blank line"),
            (format!("{}  {CORE}\n", "0".repeat(63)), "short digest"),
            (format!("{}  {CORE}\n", "A".repeat(64)), "uppercase digest"),
            (format!("{digest}  ./{CORE}\n"), "a path, not a basename"),
            (format!("{digest}  {CORE}\n{digest}  {CORE}\n"), "twice"),
            (
                format!("{digest}  {CORE}\n{digest}  {INSTALLER}\n{digest}  {MANIFEST}\n"),
                "a third member",
            ),
        ] {
            let refused = match parse_manifest(&text) {
                Err(_) => true,
                Ok(names) => !manifest_covers_members(&names),
            };
            assert!(refused, "{why}: {text:?}");
        }
    }

    #[test]
    fn a_well_formed_version_directory_validates() {
        assert_eq!(validate(&Fake::good(), "2026.9.1", "2026.9.1"), Ok(()));
    }

    #[test]
    fn a_version_directory_that_does_not_match_the_binary_refuses() {
        let broken = validate(&Fake::good(), "2026.9.1", "2026.9.2").expect_err("a mismatch");
        assert!(broken.0.contains("2026.9.1"), "{broken:?}");
        assert!(broken.to_string().contains("ae upgrade"), "{broken:?}");
    }

    #[test]
    fn a_missing_or_linked_member_refuses_before_anything_else() {
        let mut absent = Fake::good();
        absent.installer = Member::Absent;
        assert!(
            validate(&absent, "1.1.1", "1.1.1")
                .expect_err("a missing installer")
                .0
                .contains(INSTALLER)
        );

        let mut linked = Fake::good();
        linked.manifest_member = Member::NotRegular;
        assert!(
            validate(&linked, "1.1.1", "1.1.1")
                .expect_err("a linked manifest")
                .0
                .contains("not a regular file")
        );
    }

    #[test]
    fn a_tampered_manifest_refuses() {
        let mut tampered = Fake::good();
        tampered.text = Some("nonsense\n".to_owned());
        assert!(
            validate(&tampered, "1.1.1", "1.1.1")
                .expect_err("a tampered manifest")
                .0
                .contains(MANIFEST)
        );

        let mut short = Fake::good();
        short.text = Some(format!("{}  {CORE}\n", "b".repeat(64)));
        assert!(
            validate(&short, "1.1.1", "1.1.1")
                .expect_err("a manifest naming one member")
                .0
                .contains("exactly")
        );

        let mut unreadable = Fake::good();
        unreadable.text = None;
        assert!(
            validate(&unreadable, "1.1.1", "1.1.1")
                .expect_err("an unreadable manifest")
                .0
                .contains("cannot be read")
        );
    }

    /// A home reached through a symlink is the SAME home, and an install under
    /// it is INSTALLED. Without this, `$HOME=/tmp/x` against a `current_exe()`
    /// of `/private/tmp/x/.ae/versions/V/ae-core` classifies as a checkout, and
    /// the public boundary quietly starts honouring an inherited `AE_HOME`.
    #[test]
    fn a_home_reached_through_a_link_is_still_installed() {
        let root = std::env::temp_dir().join(format!("ae-shape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let real = root.join("real");
        std::fs::create_dir_all(&real).expect("scratch");
        let link = root.join("link");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        let exe = std::fs::canonicalize(&real)
            .expect("canonical")
            .join(".ae")
            .join(VERSIONS)
            .join("9.9.9")
            .join(CORE);

        assert_eq!(
            classify(&exe, Some(link.as_path())),
            Shape::Checkout,
            "the raw spelling cannot match a resolved exe"
        );
        assert!(
            matches!(
                classify(&exe, Some(canonical_home(&link).as_path())),
                Shape::Installed { ref version, .. } if version == "9.9.9"
            ),
            "the resolved spelling must"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
