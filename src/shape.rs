//! What this binary IS — and therefore whose environment it trusts.
//!
//! Slice Z3 deletes `ae-entry`. The public `ae` is a symlink to THIS binary, so
//! the two things the wrapper decided from its own file name are decided here
//! from [`std::env::current_exe`]:
//!
//! * **INSTALLED** — the resolved executable is `<root>/.ae/versions/<V>/ae-core`,
//!   a directory the canonical installer published. ae state is `<root>/.ae`,
//!   READ OFF THE EXECUTABLE'S OWN PATH, and nothing an inherited variable says
//!   can move it; the version directory is validated before any effect.
//! * **DISPLACED** — the same position, but `$HOME` names a different root.
//!   A refusal naming both, never a demotion.
//! * **CHECKOUT** — anywhere else. `AE_HOME`, `CONFIG_FILE` and the
//!   `AE_TMUX_SERVER` pair are HONOURED. That is the `ae-dev` namespace and the
//!   two bash suites, which are its only callers.

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
    /// Published by the installer, but run against a `$HOME` that names a
    /// different root.
    Displaced {
        /// `<root>/.ae`, derived from where this binary SITS.
        home: PathBuf,
        /// The inherited `$HOME`, which names somewhere else.
        declared: PathBuf,
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

    /// The state root this binary's own POSITION derives, if it has one.
    #[must_use]
    pub fn published_home(&self) -> Option<&Path> {
        match self {
            Self::Installed { home, .. } | Self::Displaced { home, .. } => Some(home),
            Self::Checkout => None,
        }
    }
}

/// Classify `exe` — an already-resolved `current_exe()` — against `home`, the
/// value of `$HOME`.
#[must_use]
pub fn classify(exe: &Path, home: Option<&Path>) -> Shape {
    let Some((ae_home, version_dir, version)) = published_position(exe) else {
        return Shape::Checkout;
    };
    if let Some(declared) = home
        && declared.join(".ae") != ae_home
    {
        return Shape::Displaced {
            home: ae_home,
            declared: declared.to_path_buf(),
        };
    }
    Shape::Installed {
        home: ae_home,
        version_dir,
        version,
    }
}

/// The `<root>/.ae`, the version directory and its name, as `exe`'s own path
/// spells them — or `None` when this binary is not sitting in one.
fn published_position(exe: &Path) -> Option<(PathBuf, PathBuf, String)> {
    if exe.file_name()? != CORE {
        return None;
    }
    let version_dir = exe.parent()?;
    let versions = version_dir.parent()?;
    if versions.file_name()? != VERSIONS {
        return None;
    }
    let ae_home = versions.parent()?;
    if ae_home.file_name()? != ".ae" {
        return None;
    }
    let version = version_dir.file_name()?.to_str()?;
    Some((
        ae_home.to_path_buf(),
        version_dir.to_path_buf(),
        version.to_owned(),
    ))
}

/// The refusal a [`Shape::Displaced`] carries: two lines naming BOTH roots.
#[must_use]
pub fn displaced_refusal(home: &Path, declared: &Path) -> String {
    format!(
        "ae: this core is published under {} but HOME names {}\n\
         ae: an installed ae never adopts a foreign state root — run the ae published under that HOME, or correct HOME.",
        home.display(),
        declared.display()
    )
}

/// Why an installed version directory was refused.
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
#[must_use]
pub fn current() -> &'static Shape {
    static CELL: std::sync::OnceLock<Shape> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        // BOTH SIDES CANONICAL, or the comparison is of SPELLINGS and not of
        // positions.
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
            // The two words without the `.ae` that makes them ae's.
            "/opt/versions/2026.9.1/ae-core",
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
    }

    /// The position is the whole answer: an absent `$HOME` disagrees with
    /// nothing, so a published core is still installed and still answers from
    /// its own directory.
    #[test]
    fn a_published_core_with_no_home_to_compare_is_still_installed() {
        assert_eq!(
            classify(Path::new("/u/me/.ae/versions/2026.9.1/ae-core"), None),
            Shape::Installed {
                home: PathBuf::from("/u/me/.ae"),
                version_dir: PathBuf::from("/u/me/.ae/versions/2026.9.1"),
                version: "2026.9.1".to_owned(),
            }
        );
    }

    /// **B2.**
    #[test]
    fn a_published_core_against_a_foreign_home_is_displaced_and_honours_nothing() {
        let shape = classify(
            Path::new("/u/you/.ae/versions/2026.9.1/ae-core"),
            Some(Path::new("/u/me")),
        );
        assert_eq!(
            shape,
            Shape::Displaced {
                home: PathBuf::from("/u/you/.ae"),
                declared: PathBuf::from("/u/me"),
            }
        );
        assert!(
            !shape.honours_environment(),
            "a displaced core must not adopt the environment it is refusing over"
        );
        assert_eq!(
            shape.published_home(),
            Some(Path::new("/u/you/.ae")),
            "the position, never the inherited home"
        );
        let refusal = displaced_refusal(Path::new("/u/you/.ae"), Path::new("/u/me"));
        assert!(refusal.contains("/u/you/.ae"), "{refusal}");
        assert!(refusal.contains("/u/me"), "{refusal}");
        assert!(
            !refusal.contains("ae upgrade"),
            "upgrade is not the repair for a foreign HOME: {refusal}"
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
    /// it is INSTALLED.
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

        assert!(
            matches!(
                classify(&exe, Some(link.as_path())),
                Shape::Displaced { .. }
            ),
            "the raw spelling cannot match a resolved exe — and the position is \
             still an install, so the mismatch is a refusal rather than a checkout"
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
