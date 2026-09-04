//! `ae upgrade` — fetch the release bundle for this platform and publish it.
//!
//! The one command that must work on a BROKEN install, because it is the thing
//! that repairs one. It runs AHEAD of the version-directory gate
//! ([`crate::shape::validate`]) and reads none of ae's state: everything it
//! needs is its platform, one environment variable, and the network.
//!
//! # Slice Z4 removed the handover
//!
//! It used to `exec` the immutable sibling `install`, because the installer's
//! logic lived in bash and a core could not publish itself. That logic is
//! [`crate::install`] now, so this module downloads, verifies and calls
//! [`crate::install::publish`] IN PROCESS. The sibling `install` is still a
//! bundle member and still published — it is what a human runs to bootstrap a
//! machine — but `ae upgrade` no longer needs it to exist to work.
//!
//! # `AE_VERSION` is the pin, and it is scoped to this word
//!
//! Nothing else in ae reads it, and [`crate::transport`]'s spawn door removes it
//! from every child, so an operator pin cannot freeze into the tmux server a
//! launch creates and silently pin an upgrade months later.
//!
//! # Egress
//!
//! The same posture as [`crate::telegram`], with ONE deliberate difference.
//! There is no secret in these URLs — a release asset is public — and GitHub
//! answers a release download with a 302 to its object store, so a finite
//! redirect budget is correct here where `max_redirects(0)` is correct there.
//! `https_only(true)` still means a redirect cannot downgrade to cleartext, and
//! the payload is proven by SHA-256 against a manifest fetched over the same
//! locked agent, so a redirect that lands somewhere unexpected produces a
//! checksum refusal rather than an install.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The refusal a nonempty argv gets, before anything is fetched.
pub const USAGE: &str = "Usage: AE_VERSION=<calver> ae upgrade";

/// The second line of that refusal: the only supported input is the pin.
pub const USAGE_DETAIL: &str = "ae: upgrade takes no arguments; pin the target with AE_VERSION.";

/// Where release bundles come from. A constant, not a door: an operator-named
/// repository is a code-execution surface with a friendly name, and ae has
/// exactly one origin.
pub const REPOSITORY: &str = "https://github.com/clemens33/ae";

/// The release manifest naming every tarball of a release.
pub const MANIFEST: &str = "SHA256SUMS";

/// Caps and timeouts. A repair path may not hang, and it may not read an
/// unbounded body into memory: a bundle is a couple of megabytes, and the cap is
/// generous against that rather than against what a hostile server might send.
const TIMEOUT_CONNECT: Duration = Duration::from_secs(15);
const TIMEOUT_RECV_RESPONSE: Duration = Duration::from_mins(1);
const TIMEOUT_GLOBAL: Duration = Duration::from_mins(5);
const MAX_MANIFEST_BYTES: u64 = 1 << 20;
const MAX_ARCHIVE_BYTES: u64 = 64 << 20;

/// The two platforms ae publishes for.
///
/// Named rather than derived from `uname` strings: the bundle name is built
/// from this word, so a platform ae does not publish must refuse here rather
/// than 404 later.
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
/// The manifest names tarballs, one line per asset. The first line whose name is
/// this platform's bundle answers; a release with no bundle for this platform is
/// a refusal rather than a 404 on a guessed name.
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
/// Checked BEFORE extraction, against `tar -tzf`: an archive that names a path
/// outside its own root — `../`, an absolute path, a fourth member — is refused
/// rather than unpacked and then inspected.
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
///
/// * `proxy(None)` — the default is `Proxy::try_from_env()`. An upgrade is the
///   highest-authority path ae has; an exported `HTTPS_PROXY` must not choose
///   which bytes it publishes.
/// * `https_only(true)` — the default is `false`. A redirect must not be able to
///   downgrade the transfer to cleartext.
/// * `max_redirects(3)` — telegram uses 0 because its URL carries the bot token.
///   Here there is no secret to leak and GitHub answers a release download with
///   a 302 to its object store, so a finite budget is what makes the feature
///   work at all. What keeps it honest is the digest: a redirect that lands
///   somewhere unexpected produces a checksum refusal, not an install.
/// * finite `timeout_*` — a repair path may not hang.
/// * an explicit `ring` provider — ureq's docs reserve the right to change which
///   provider its rustls feature selects, and a static musl binary's provider is
///   not a detail to discover in production.
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
///
/// The error is the URL and a CLASS, never `ureq::Error`'s own text: it can
/// quote a URI, and while these carry no secret, one redacted vocabulary across
/// the crate is worth more than one exception.
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

/// THE FIFTH PRODUCT CROSSING of `clippy.toml`'s `Command` deny.
///
/// `tar`, twice: once to LIST an archive and once to extract it. It is a
/// deliberate non-dependency — the alternative is a tar-and-gzip crate pair in a
/// binary whose whole dependency posture is "std owns every byte we write", for
/// one call on one command. `tar` is on every machine ae runs on, and the
/// listing pass ([`entries_ok`]) is what makes running it safe: nothing is
/// unpacked until every entry has been proven to sit under the expected root.
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
    match upgrade(&home, out) {
        Ok(published) => {
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
    crate::install::install_from(&scratch.path().join(&root), home)
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
