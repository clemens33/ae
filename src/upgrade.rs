//! `ae upgrade` — fetch the release bundle for this platform and publish it.
//!
//! The one command that must work on a BROKEN install, because it is the thing
//! that repairs one. It runs AHEAD of the version-directory gate
//! ([`crate::shape::validate`]) and reads none of ae's state: everything it
//! needs is its platform, one environment variable, and the network.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The refusal a nonempty argv gets, before anything is fetched.
pub const USAGE: &str = "Usage: AE_VERSION=<calver> ae upgrade";

/// The second line of that refusal: the only supported input is the pin.
pub const USAGE_DETAIL: &str = "ae: upgrade takes no arguments; pin the target with AE_VERSION.";

/// Where release bundles come from.
pub const REPOSITORY: &str = "https://github.com/clemens33/ae";

/// The release manifest naming every tarball of a release.
pub const MANIFEST: &str = "SHA256SUMS";

/// Caps and timeouts.
const TIMEOUT_CONNECT: Duration = Duration::from_secs(15);
const TIMEOUT_RECV_RESPONSE: Duration = Duration::from_mins(1);
const TIMEOUT_GLOBAL: Duration = Duration::from_mins(5);
const MAX_MANIFEST_BYTES: u64 = 1 << 20;
const MAX_ARCHIVE_BYTES: u64 = 64 << 20;

/// The two platforms ae publishes for.
///
/// # Errors
///
/// A platform ae publishes no bundle for.
pub fn platform(os: &str, arch: &str) -> Result<&'static str, String> {
    match (os, arch) {
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("linux", "x86_64") => Ok("linux-x86_64-musl"),
        ("macos", _) => Err("Intel macOS is unsupported; use an Apple Silicon Mac.".to_owned()),
        ("linux", _) => Err("Linux ARM is unsupported; use a Linux x86_64 machine.".to_owned()),
        _ => Err(format!(
            "unsupported platform: {os}/{arch} (only darwin-arm64 and linux-x86_64 are supported)"
        )),
    }
}

/// This machine's platform.
fn current_platform() -> Result<&'static str, String> {
    platform(std::env::consts::OS, std::env::consts::ARCH)
}

/// What `AE_VERSION` asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pin {
    /// Unset: whatever the newest release publishes for this platform.
    Latest,
    /// An exact `CalVer`, with a leading `v` accepted and stripped.
    Exact(String),
}

impl Pin {
    /// Read `AE_VERSION`.
    ///
    /// # Errors
    ///
    /// A value that is not a `CalVer` tag.
    pub fn parse(raw: Option<&str>) -> Result<Self, String> {
        let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Self::Latest);
        };
        let version = raw.strip_prefix('v').unwrap_or(raw);
        if !crate::install::is_version(version) {
            return Err(format!(
                "AE_VERSION must be a CalVer tag such as 2026.9.1 (a leading v is accepted): {raw}"
            ));
        }
        Ok(Self::Exact(version.to_owned()))
    }

    /// The release the download URLs are built from.
    #[must_use]
    pub fn release_ref(&self) -> String {
        match self {
            Self::Latest => "latest".to_owned(),
            Self::Exact(version) => format!("v{version}"),
        }
    }
}

/// `<repository>/releases/<ref>/download/<asset>`.
#[must_use]
pub fn asset_url(release_ref: &str, asset: &str) -> String {
    format!("{REPOSITORY}/releases/{release_ref}/download/{asset}")
}

/// The bundle name for one version and platform.
#[must_use]
pub fn archive_name(version: &str, platform: &str) -> String {
    format!("ae-{version}-{platform}.tar.gz")
}

/// The version the newest release publishes for `platform`, read out of the
/// release manifest.
///
/// # Errors
///
/// No line names a bundle for `platform`.
pub fn latest_version(manifest: &str, platform: &str) -> Result<String, String> {
    let prefix = "ae-";
    let suffix = format!("-{platform}.tar.gz");
    for line in manifest.lines() {
        let Some((_, name)) = line.split_once("  ") else {
            continue;
        };
        let name = name.strip_prefix('*').unwrap_or(name);
        let Some(rest) = name.strip_prefix(prefix) else {
            continue;
        };
        let Some(version) = rest.strip_suffix(&suffix) else {
            continue;
        };
        if crate::install::is_version(version) {
            return Ok(version.to_owned());
        }
    }
    Err(format!("the latest release has no {platform} bundle"))
}

/// The digest the release manifest records for `asset`.
///
/// # Errors
///
/// No entry, or one that is not 64 lowercase hex digits.
pub fn expected_digest(manifest: &str, asset: &str) -> Result<String, String> {
    for line in manifest.lines() {
        let Some((digest, name)) = line.split_once("  ") else {
            continue;
        };
        if name.strip_prefix('*').unwrap_or(name) != asset {
            continue;
        }
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            break;
        }
        return Ok(digest.to_ascii_lowercase());
    }
    Err(format!(
        "checksum entry for {asset} is missing or malformed"
    ))
}

/// The entries a bundle tarball may carry, and nothing else.
///
/// # Errors
///
/// The first entry that is not one of the four.
pub fn entries_ok(listing: &str, root: &str) -> Result<(), String> {
    for entry in listing.lines().filter(|line| !line.is_empty()) {
        let allowed = [
            root.to_owned(),
            format!("{root}/"),
            format!("{root}/{}", crate::shape::CORE),
            format!("{root}/{}", crate::shape::INSTALLER),
            format!("{root}/{}", crate::shape::MANIFEST),
        ];
        if !allowed.iter().any(|name| name == entry) {
            return Err(format!("archive contains an unexpected path: {entry}"));
        }
    }
    Ok(())
}

// ─── the locked client ───────────────────────────────────────────────────

/// The SECOND `ureq::Agent` construction site in this crate, and the difference
/// from [`crate::telegram`]'s is one setting.
fn agent() -> ureq::Agent {
    let crypto = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .unversioned_rustls_crypto_provider(crypto)
        .build();
    let config = ureq::Agent::config_builder()
        .https_only(true)
        .proxy(None)
        .max_redirects(3)
        .timeout_connect(Some(TIMEOUT_CONNECT))
        .timeout_recv_response(Some(TIMEOUT_RECV_RESPONSE))
        .timeout_global(Some(TIMEOUT_GLOBAL))
        .tls_config(tls)
        .build();
    ureq::Agent::new_with_config(config)
}

/// GET `url`, refusing a body larger than `cap`.
fn fetch(agent: &ureq::Agent, url: &str, cap: u64) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    let mut response = agent
        .get(url)
        .call()
        .map_err(|_| format!("could not fetch {url}"))?;
    let mut body = Vec::new();
    let read = response
        .body_mut()
        .as_reader()
        .take(cap + 1)
        .read_to_end(&mut body)
        .map_err(|_| format!("could not read {url}"))?;
    if read as u64 > cap {
        return Err(format!("{url} is larger than this installer accepts"));
    }
    Ok(body)
}

// ─── extraction ──────────────────────────────────────────────────────────

/// A PRODUCT CROSSING of `clippy.toml`'s `Command` deny — the archive door.
fn tar(args: &[&str]) -> std::io::Result<std::process::Output> {
    #[allow(
        clippy::disallowed_types,
        reason = "upgrade's archive door: `tar` lists and extracts the downloaded bundle, and the listing is proven before the extraction runs — a tar crate would be a dependency for one call"
    )]
    let mut command = std::process::Command::new("tar");
    command.args(args).output()
}

/// A directory this process owns, removed when it is dropped.
struct Scratch(PathBuf);

impl Scratch {
    /// Create one under the system temp directory.
    fn new() -> Result<Self, String> {
        let base = std::env::temp_dir();
        for attempt in 0..64_u32 {
            let path = base.join(format!("ae-upgrade.{}.{attempt}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(why) if why.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(why) => return Err(format!("could not create a scratch directory: {why}")),
            }
        }
        Err("could not create a scratch directory".to_owned())
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Through the chokepoint: an extracted bundle root is 0555, and a 0555
        // directory refuses the unlink of its own entries.
        let _ = crate::install::remove_private_tree(&self.0);
    }
}

// ─── the command ─────────────────────────────────────────────────────────

/// `ae upgrade` — refuse an argv, fetch, verify, publish.
///
/// # Errors
///
/// Propagates a write failure on the caller's streams.
pub fn run(
    tail: &[String],
    out: &mut impl std::io::Write,
    err: &mut impl std::io::Write,
) -> crate::Result<u8> {
    if !tail.is_empty() {
        writeln!(err, "{USAGE}")?;
        writeln!(err, "{USAGE_DETAIL}")?;
        err.flush()?;
        return Ok(crate::entry::EXIT_USAGE);
    }
    let Some(home) = crate::doors::home() else {
        writeln!(err, "ae: HOME is not set, so there is nowhere to install.")?;
        err.flush()?;
        return Ok(crate::entry::EXIT_USAGE);
    };
    // BEFORE THE DOWNLOAD AND BEFORE ANY MUTATION. A publish is `$HOME`-pinned
    // end to end, and it is no longer only a file copy: it migrates, repoints
    // and relinks every session under `$HOME/.ae` and then deletes version
    // directories there. A checkout run whose state root is somewhere else —
    // `ae-dev` is the whole point of that door — would therefore reach straight
    // past its own namespace into the real fleet. That was documented and not
    // prevented, which is not good enough now that a publish writes to every
    // session it finds.
    if let Some(escape) = namespace_escape(&home) {
        writeln!(err, "{escape}")?;
        err.flush()?;
        return Ok(crate::entry::EXIT_USAGE);
    }
    match upgrade(&home, out) {
        Ok(published) => {
            for note in &published.notes {
                writeln!(out, "ae: {note}")?;
            }
            writeln!(
                out,
                "ae: installed {} under {}",
                published.version,
                published.version_dir.display()
            )?;
            out.flush()?;
            Ok(0)
        }
        Err(why) => {
            out.flush()?;
            writeln!(err, "ae: {why}")?;
            err.flush()?;
            Ok(crate::entry::EXIT_FAILED)
        }
    }
}

/// The refusal a checkout run earns when its state root is not the one a
/// publish would write to, or `None` when the two agree.
///
/// Only the CHECKOUT shape can differ: an installed ae ignores `AE_HOME`
/// outright, and the bootstrap runs the bundle's own core with a plain `$HOME`.
/// So this refuses exactly the case where an operator pointed ae at a second
/// namespace and then asked it to upgrade — and it names both roots, because
/// the whole confusion is that they are not the same.
fn namespace_escape(home: &Path) -> Option<String> {
    let shape = crate::shape::current();
    if !shape.honours_environment() {
        return None;
    }
    let pinned = home.join(".ae");
    let effective = crate::doors::state_root(shape)?;
    if effective == pinned {
        return None;
    }
    Some(format!(
        "ae: refusing to upgrade — this is a checkout build whose state root is {}, \
         but a publish always writes to {} and would migrate, repoint and prune the \
         sessions THERE.\nae: run the installed ae ({}) to upgrade it, or unset AE_HOME.",
        effective.display(),
        pinned.display(),
        home.join(".local").join("bin").join("ae").display()
    ))
}

/// Download, verify, extract, publish.
fn upgrade(
    home: &Path,
    out: &mut impl std::io::Write,
) -> Result<crate::install::Published, String> {
    let platform = current_platform()?;
    let pin = Pin::parse(crate::doors::target_version().as_deref())?;
    let release_ref = pin.release_ref();
    let agent = agent();

    let manifest_url = asset_url(&release_ref, MANIFEST);
    let manifest_bytes = fetch(&agent, &manifest_url, MAX_MANIFEST_BYTES)?;
    let manifest = String::from_utf8(manifest_bytes)
        .map_err(|_| format!("{manifest_url} is not a text manifest"))?;
    let version = match &pin {
        Pin::Latest => latest_version(&manifest, platform)?,
        Pin::Exact(version) => version.clone(),
    };
    let archive = archive_name(&version, platform);
    let expected = expected_digest(&manifest, &archive)?;

    let _ = writeln!(out, "ae upgrade: fetching {archive}");
    let _ = out.flush();
    let archive_bytes = fetch(
        &agent,
        &asset_url(&release_ref, &archive),
        MAX_ARCHIVE_BYTES,
    )?;
    if crate::install::sha256_hex(&archive_bytes) != expected {
        return Err(format!(
            "checksum mismatch for {archive}; nothing was installed"
        ));
    }

    let scratch = Scratch::new()?;
    let archive_path = scratch.path().join(&archive);
    std::fs::write(&archive_path, &archive_bytes)
        .map_err(|why| format!("could not stage {archive}: {why}"))?;
    let root = format!("ae-{version}-{platform}");
    extract(&archive_path, scratch.path(), &root)?;
    delegate(&scratch.path().join(&root), home, &version)
}

/// Hand the extracted bundle to ITS OWN core, exactly as the bootstrap does.
///
/// THE PUBLISH BELONGS TO THE NEW CORE, not to this one. A publish is no longer
/// a file copy: it steps every session's meta through the migration chain. The
/// steps for versions N..M live in the core being INSTALLED — this process only
/// knows the chain as of its own release, so an in-process `install_from` would
/// migrate tomorrow's sessions with yesterday's rules, and on the first real
/// schema change would simply have no step to run.
///
/// The core is trusted the same way `install` trusts it: the archive was
/// checksummed against the release manifest before extraction, the listing was
/// proved to name nothing but its own members, and the core re-verifies every
/// member's digest before it publishes anything. Running it is also not a new
/// capability — [`crate::install::verify`] already runs a bundle core to ask
/// its version.
///
/// A core too old to know `_install` would fail here; none ever was, because
/// `_install` predates `upgrade`.
fn delegate(root: &Path, home: &Path, version: &str) -> Result<crate::install::Published, String> {
    let core = root.join(crate::shape::CORE);
    let out =
        run_core(&core, root, home).map_err(|why| format!("could not run the new core: {why}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        // The new core's own refusal, verbatim: it named the session, the
        // journal or the digest that stopped it, and rewording that here would
        // lose the only thing the operator can act on.
        let said = stderr.trim();
        return Err(if said.is_empty() {
            format!("the new core refused the install ({})", out.status)
        } else {
            said.to_owned()
        });
    }
    // Its notes are the operator's, and the caller prints them. The `ae: `
    // prefix is this process's own convention, added when they are printed.
    let notes = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("ae: "))
        .filter(|line| !line.starts_with("installed "))
        .map(ToOwned::to_owned)
        .collect();
    Ok(crate::install::Published {
        version_dir: home.join(".ae").join(crate::shape::VERSIONS).join(version),
        version: version.to_owned(),
        notes,
    })
}

/// A PRODUCT CROSSING of `clippy.toml`'s `Command` deny — the handover door.
fn run_core(core: &Path, root: &Path, home: &Path) -> std::io::Result<std::process::Output> {
    #[allow(
        clippy::disallowed_types,
        reason = "upgrade's handover door: the digest-verified new core performs its own publish, because the migration chain that publish runs belongs to the version being installed"
    )]
    let mut command = std::process::Command::new(core);
    command
        .arg(crate::cli::INSTALL)
        .arg("--from")
        .arg(root)
        .env("HOME", home)
        .output()
}

/// List, prove, then unpack.
fn extract(archive: &Path, into: &Path, root: &str) -> Result<(), String> {
    let archive = archive.to_string_lossy().into_owned();
    let into_text = into.to_string_lossy().into_owned();
    let listing = tar(&["-tzf", &archive]).map_err(|why| format!("could not run tar: {why}"))?;
    if !listing.status.success() {
        return Err("the archive is not a readable gzip tarball".to_owned());
    }
    entries_ok(&String::from_utf8_lossy(&listing.stdout), root)?;
    let unpacked = tar(&["-xzf", &archive, "-C", &into_text])
        .map_err(|why| format!("could not run tar: {why}"))?;
    if !unpacked.status.success() {
        return Err("archive extraction failed".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_published_platforms_have_a_bundle_name() {
        assert_eq!(platform("macos", "aarch64"), Ok("darwin-arm64"));
        assert_eq!(platform("linux", "x86_64"), Ok("linux-x86_64-musl"));
        for (os, arch) in [
            ("macos", "x86_64"),
            ("linux", "aarch64"),
            ("windows", "x86_64"),
        ] {
            assert!(platform(os, arch).is_err(), "accepted {os}/{arch}");
        }
    }

    #[test]
    fn an_argv_is_a_usage_error_before_anything_is_fetched() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&["2026.9.1".to_owned()], &mut out, &mut err).expect("writes");
        assert_eq!(code, crate::entry::EXIT_USAGE);
        assert!(out.is_empty(), "nothing is fetched");
        let text = String::from_utf8(err).expect("utf8");
        assert!(text.contains(USAGE), "{text}");
        assert!(text.contains("AE_VERSION"), "{text}");
    }

    #[test]
    fn the_pin_accepts_a_calver_with_or_without_its_v_and_nothing_else() {
        assert_eq!(Pin::parse(None), Ok(Pin::Latest));
        assert_eq!(Pin::parse(Some("")), Ok(Pin::Latest));
        assert_eq!(
            Pin::parse(Some("v2026.9.1")),
            Ok(Pin::Exact("2026.9.1".to_owned()))
        );
        assert_eq!(
            Pin::parse(Some("2026.9.1")),
            Ok(Pin::Exact("2026.9.1".to_owned()))
        );
        assert!(Pin::parse(Some("bad")).is_err());
        assert_eq!(Pin::Latest.release_ref(), "latest");
        assert_eq!(Pin::Exact("2026.9.1".to_owned()).release_ref(), "v2026.9.1");
    }

    #[test]
    fn the_release_manifest_answers_which_version_and_which_digest() {
        let manifest = format!(
            "{0}  ae-2026.9.1-darwin-arm64.tar.gz\n{1}  ae-2026.9.1-linux-x86_64-musl.tar.gz\n",
            "a".repeat(64),
            "b".repeat(64)
        );
        assert_eq!(
            latest_version(&manifest, "darwin-arm64"),
            Ok("2026.9.1".to_owned())
        );
        assert!(latest_version(&manifest, "linux-aarch64-musl").is_err());
        assert_eq!(
            expected_digest(&manifest, "ae-2026.9.1-linux-x86_64-musl.tar.gz"),
            Ok("b".repeat(64))
        );
        assert!(expected_digest(&manifest, "ae-2026.9.2-darwin-arm64.tar.gz").is_err());
        // A malformed digest is missing, not accepted: a short hex string would
        // otherwise be compared against a real one and always disagree, which
        // reads as tampering rather than as a broken manifest.
        assert!(
            expected_digest(
                "dead  ae-1.2.3-darwin-arm64.tar.gz\n",
                "ae-1.2.3-darwin-arm64.tar.gz"
            )
            .is_err()
        );
    }

    #[test]
    fn an_archive_that_names_anything_but_its_own_members_is_refused() {
        let root = "ae-2026.9.1-darwin-arm64";
        assert!(
            entries_ok(
                &format!("{root}/\n{root}/ae-core\n{root}/install\n{root}/SHA256SUMS\n"),
                root
            )
            .is_ok()
        );
        for hostile in [
            format!("{root}/\n{root}/../../etc/passwd\n"),
            format!("{root}/\n/etc/passwd\n"),
            format!("{root}/\n{root}/extra\n"),
            "other/\nother/ae-core\n".to_owned(),
        ] {
            assert!(
                entries_ok(&hostile, root).is_err(),
                "accepted a hostile listing: {hostile}"
            );
        }
    }
}
