//! Quiet installed-only automatic release checks.
//!
//! Hooks only call [`schedule`]. The detached `_autoupgrade` child owns policy,
//! cadence, networking and the outer upgrade lock; foreground commands never
//! wait for any of them.

use std::io::{Read as _, Write as _};
use std::path::Path;
use std::time::Duration;

/// The advisory lock shared by manual upgrade and automatic checks.
pub const LOCK_FILE: &str = ".ae-upgrade.lock";
/// The bounded last-attempt record.
pub const CHECK_FILE: &str = "upgrade.check";
/// The bounded background diagnostic log.
pub const LOG_FILE: &str = "upgrade.log";
/// Successful/current checks run at most this often.
pub const CADENCE_SECS: i64 = 15 * 60;
/// Failed or interrupted checks back off this long.
pub const BACKOFF_SECS: i64 = 60 * 60;
/// Manual upgrade waits for another manual/automatic operation to finish.
pub(crate) const MANUAL_LOCK_WAIT: Duration = Duration::from_mins(1);

const CHECK_FORMAT: &str = "1";
const MAX_CHECK_BYTES: u64 = 64 << 10;
const MAX_LOG_BYTES: usize = 64 << 10;
const MAX_LOG_DETAIL_CHARS: usize = 8 << 10;

/// The global `[workspace] auto_upgrade` ruling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Policy {
    Enabled,
    Disabled,
    Invalid(String),
}

/// Read only the global config. A project-local overlay never gets a vote over
/// machine-wide software updates.
#[must_use]
pub fn policy(global: &Path) -> Policy {
    match crate::config::read_global_workspace_key(global, "auto_upgrade") {
        Err(why) => Policy::Invalid(why),
        Ok(value) => match value.as_deref().map(str::trim) {
            None | Some("on") => Policy::Enabled,
            Some("off") => Policy::Disabled,
            Some(other) => Policy::Invalid(other.to_owned()),
        },
    }
}

/// The small closed vocabulary persisted in `upgrade.check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckResult {
    Started,
    Current,
    Installed,
    FailedManifest,
    FailedArchive,
    FailedInstall,
}

impl CheckResult {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Current => "current",
            Self::Installed => "installed",
            Self::FailedManifest => "failed-manifest",
            Self::FailedArchive => "failed-archive",
            Self::FailedInstall => "failed-install",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "started" => Some(Self::Started),
            "current" => Some(Self::Current),
            "installed" => Some(Self::Installed),
            "failed-manifest" => Some(Self::FailedManifest),
            "failed-archive" => Some(Self::FailedArchive),
            "failed-install" => Some(Self::FailedInstall),
            _ => None,
        }
    }

    const fn uses_backoff(self) -> bool {
        matches!(
            self,
            Self::Started | Self::FailedManifest | Self::FailedArchive | Self::FailedInstall
        )
    }
}

/// One bounded attempt record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub attempted_at: i64,
    pub seen: Option<String>,
    pub result: CheckResult,
}

impl Check {
    fn render(&self) -> String {
        format!(
            "format={CHECK_FORMAT}\nattempted_at={}\nseen={}\nresult={}\n",
            self.attempted_at,
            self.seen.as_deref().unwrap_or(""),
            self.result.as_str()
        )
    }
}

/// Parse the hostile persisted record. The format is deliberately fixed-order
/// and closed: duplicates, unknown fields, control text and partial writes all
/// fail as one malformed state that the scheduler may safely replace.
///
/// # Errors
///
/// A field is missing, malformed, unknown, or outside the accepted vocabulary.
pub fn parse_check(bytes: &[u8]) -> Result<Check, String> {
    if bytes.len() as u64 > MAX_CHECK_BYTES {
        return Err("upgrade.check exceeds 64 KiB".to_owned());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "upgrade.check is not UTF-8".to_owned())?;
    let Some(text) = text.strip_suffix('\n') else {
        return Err("upgrade.check has no final newline".to_owned());
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let [format, attempted_at, seen, result] = lines.as_slice() else {
        return Err("upgrade.check has the wrong field count".to_owned());
    };
    if *format != format!("format={CHECK_FORMAT}") {
        return Err("upgrade.check has an unsupported format".to_owned());
    }
    let attempted_at = attempted_at
        .strip_prefix("attempted_at=")
        .ok_or_else(|| "upgrade.check has no attempted_at".to_owned())?
        .parse::<i64>()
        .map_err(|_| "upgrade.check attempted_at is invalid".to_owned())?;
    if attempted_at < 0 {
        return Err("upgrade.check attempted_at is negative".to_owned());
    }
    let seen = seen
        .strip_prefix("seen=")
        .ok_or_else(|| "upgrade.check has no seen version".to_owned())?;
    let seen = if seen.is_empty() {
        None
    } else if crate::install::is_version(seen) {
        Some(seen.to_owned())
    } else {
        return Err("upgrade.check seen version is invalid".to_owned());
    };
    let result = result
        .strip_prefix("result=")
        .and_then(CheckResult::parse)
        .ok_or_else(|| "upgrade.check result is invalid".to_owned())?;
    Ok(Check {
        attempted_at,
        seen,
        result,
    })
}

/// A status reader distinguishes absent, malformed and usable records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckState {
    Missing,
    Malformed(String),
    Valid(Check),
}

/// One read-only status row for `version` and `doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusRow {
    pub(crate) detail: String,
    pub(crate) warning: bool,
}

/// Automatic-upgrade policy and last-attempt state. Reading it never schedules
/// work and checkout/displaced shapes never inspect an installed state root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Status {
    pub(crate) policy: StatusRow,
    pub(crate) check: StatusRow,
}

/// Read the current bounded attempt state.
#[must_use]
pub fn check_state(ae_home: &Path) -> CheckState {
    match door::read_bounded(&ae_home.join(CHECK_FILE), MAX_CHECK_BYTES) {
        Ok(None) => CheckState::Missing,
        Ok(Some(bytes)) => match parse_check(&bytes) {
            Ok(check) => CheckState::Valid(check),
            Err(why) => CheckState::Malformed(why),
        },
        Err(why) => CheckState::Malformed(why.to_string()),
    }
}

/// Whether an attempt is due at `now`. A future stamp (wall-clock rollback)
/// waits rather than hot-looping; malformed state is replaced by the one child
/// that wins the outer lock and stamps before networking.
#[must_use]
pub fn due(state: &CheckState, now: i64) -> bool {
    let CheckState::Valid(check) = state else {
        return true;
    };
    let Some(elapsed) = now.checked_sub(check.attempted_at) else {
        return false;
    };
    if elapsed < 0 {
        return false;
    }
    let wait = if check.result.uses_backoff() {
        BACKOFF_SECS
    } else {
        CADENCE_SECS
    };
    elapsed >= wait
}

/// Read status without triggering a check.
#[must_use]
pub(crate) fn status(shape: &crate::shape::Shape) -> Status {
    status_at(shape, crate::time::Timestamp::now().epoch())
}

fn status_at(shape: &crate::shape::Shape, now: i64) -> Status {
    let home = match shape {
        crate::shape::Shape::Installed { home, .. } => home,
        crate::shape::Shape::Checkout => {
            return unavailable_status("unavailable (checkout builds never auto-upgrade)");
        }
        crate::shape::Shape::Displaced { .. } => {
            return unavailable_status("unavailable (HOME does not name this installed core)");
        }
    };
    let policy_value = policy(&home.join("config"));
    let enabled = policy_value == Policy::Enabled;
    let policy = match policy_value {
        Policy::Enabled => StatusRow {
            detail: "on (global policy; default when absent)".to_owned(),
            warning: false,
        },
        Policy::Disabled => StatusRow {
            detail: "off (global policy)".to_owned(),
            warning: false,
        },
        Policy::Invalid(value) => StatusRow {
            detail: format!("invalid global auto_upgrade value: {value:?}"),
            warning: true,
        },
    };
    let check = match check_state(home) {
        CheckState::Missing => StatusRow {
            detail: "never checked".to_owned(),
            warning: false,
        },
        CheckState::Malformed(why) => StatusRow {
            detail: format!("invalid upgrade.check: {why}"),
            warning: true,
        },
        CheckState::Valid(check) => {
            let failed = check.result.uses_backoff();
            let stale = enabled && !failed && due(&CheckState::Valid(check.clone()), now);
            StatusRow {
                detail: format!(
                    "{}: {}; seen {}",
                    crate::time::Timestamp::from_epoch(check.attempted_at),
                    check.result.as_str(),
                    check.seen.as_deref().unwrap_or("none")
                ) + if stale { "; stale" } else { "" },
                warning: failed || stale,
            }
        }
    };
    Status { policy, check }
}

fn unavailable_status(reason: &str) -> Status {
    Status {
        policy: StatusRow {
            detail: reason.to_owned(),
            warning: false,
        },
        check: StatusRow {
            detail: "not read for this binary shape".to_owned(),
            warning: false,
        },
    }
}

/// Take the outer upgrade/check lock. A zero wait is the automatic try-lock.
pub(crate) fn lock(ae_home: &Path, wait: Duration) -> std::io::Result<std::fs::File> {
    crate::store::lock(&ae_home.join(LOCK_FILE), wait)
}

/// A `nohup` argv minted only for the automatic checker. No caller can add a
/// shell fragment, body argument, version pin or alternate internal command.
pub(crate) struct DetachedArgv(Vec<String>);

impl DetachedArgv {
    pub(crate) fn as_args(&self) -> &[String] {
        &self.0
    }
}

fn detached_argv(ae_home: &Path) -> Option<DetachedArgv> {
    let home = ae_home.parent()?;
    let command = home.join(".local").join("bin").join("ae");
    Some(DetachedArgv(vec![
        command.to_string_lossy().into_owned(),
        crate::cli::AUTOUPGRADE.to_owned(),
    ]))
}

/// Nonblocking hook. It reads enough local state to avoid needless children,
/// then delegates every authoritative check to the detached child.
pub fn schedule() {
    let shape = crate::shape::current();
    let _ = schedule_with(
        shape,
        crate::doors::no_autostart(),
        crate::time::Timestamp::now().epoch(),
        crate::transport::run_autoupgrade_detached,
    );
}

fn schedule_with(
    shape: &crate::shape::Shape,
    no_autostart: bool,
    now: i64,
    spawn: impl FnOnce(&DetachedArgv) -> bool,
) -> bool {
    let crate::shape::Shape::Installed {
        home,
        version_dir,
        version,
    } = shape
    else {
        return false;
    };
    if no_autostart
        || policy(&home.join("config")) != Policy::Enabled
        || !due(&check_state(home), now)
        || crate::shape::validate(&crate::shape::OnDisk(version_dir), version, crate::VERSION)
            .is_err()
    {
        return false;
    }
    detached_argv(home).is_some_and(|argv| spawn(&argv))
}

struct Claim {
    _held: std::fs::File,
    check: Check,
}

fn claim(
    ae_home: &Path,
    now: i64,
    suppressed: impl FnOnce() -> bool,
) -> Result<Option<Claim>, String> {
    let Ok(held) = lock(ae_home, Duration::ZERO) else {
        return Ok(None);
    };
    // Both are authoritative only inside exclusion: N scheduled children can
    // race here, but only one may stamp and therefore reach its manifest GET.
    if suppressed() || policy(&ae_home.join("config")) != Policy::Enabled {
        return Ok(None);
    }
    if !due(&check_state(ae_home), now) {
        return Ok(None);
    }
    let check = Check {
        attempted_at: now,
        seen: None,
        result: CheckResult::Started,
    };
    write_check(ae_home, &check)?;
    Ok(Some(Claim { _held: held, check }))
}

/// `_autoupgrade`: silent background check and optional publication.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one linear background transaction: lock, stamp, discover, compare, prepare and delegate; splitting it would hide the stage-to-diagnostic mapping"
)]
pub fn run(tail: &[String]) -> u8 {
    if !tail.is_empty() {
        return crate::entry::EXIT_USAGE;
    }
    let crate::shape::Shape::Installed { home: ae_home, .. } = crate::shape::current() else {
        return 0;
    };
    if crate::doors::no_autostart() || policy(&ae_home.join("config")) != Policy::Enabled {
        return 0;
    }
    let now = crate::time::Timestamp::now().epoch();
    let mut claim = match claim(ae_home, now, crate::doors::no_autostart) {
        Ok(Some(claim)) => claim,
        Ok(None) => return 0,
        Err(why) => {
            append_log(ae_home, now, "failed", "state", &why);
            return crate::entry::EXIT_FAILED;
        }
    };
    let check = &mut claim.check;

    let Some(home) = ae_home.parent() else {
        finish_failure(
            ae_home,
            check,
            CheckResult::FailedInstall,
            "pointer",
            "installed ae home has no parent",
        );
        return crate::entry::EXIT_FAILED;
    };
    let paths = match crate::install::fixed_paths(home) {
        Ok(paths) => paths,
        Err(why) => {
            finish_failure(ae_home, check, CheckResult::FailedInstall, "pointer", &why);
            return crate::entry::EXIT_FAILED;
        }
    };
    let current = match crate::install::current_public_version(&paths) {
        Ok(version) => version,
        Err(why) => {
            finish_failure(ae_home, check, CheckResult::FailedInstall, "pointer", &why);
            return crate::entry::EXIT_FAILED;
        }
    };
    let candidate = match crate::upgrade::discover_automatic() {
        Ok(candidate) => candidate,
        Err(why) => {
            finish_failure(
                ae_home,
                check,
                CheckResult::FailedManifest,
                "manifest",
                &why,
            );
            return crate::entry::EXIT_FAILED;
        }
    };
    check.seen = Some(candidate.version().to_owned());
    if crate::install::compare_versions(candidate.version(), &current)
        != Some(std::cmp::Ordering::Greater)
    {
        check.result = CheckResult::Current;
        finish(ae_home, check, "current", "manifest", "no newer release");
        return 0;
    }
    let prepared = match crate::upgrade::prepare(candidate) {
        Ok(prepared) => prepared,
        Err(why) => {
            finish_failure(ae_home, check, CheckResult::FailedArchive, "archive", &why);
            return crate::entry::EXIT_FAILED;
        }
    };
    match crate::upgrade::delegate_automatic(&prepared, home) {
        Ok(crate::upgrade::AutomaticInstall::Published(published)) => {
            check.result = CheckResult::Installed;
            let detail = if published.notes.is_empty() {
                format!("installed {}", published.version)
            } else {
                format!(
                    "installed {}; {}",
                    published.version,
                    published.notes.join("; ")
                )
            };
            finish(ae_home, check, "installed", "install", &detail);
            0
        }
        Ok(crate::upgrade::AutomaticInstall::Superseded { candidate, current }) => {
            check.result = CheckResult::Current;
            finish(
                ae_home,
                check,
                "current",
                "install",
                &format!("candidate {candidate} superseded by {current}"),
            );
            0
        }
        Err(why) => {
            finish_failure(ae_home, check, CheckResult::FailedInstall, "install", &why);
            crate::entry::EXIT_FAILED
        }
    }
}

fn finish_failure(ae_home: &Path, check: &mut Check, result: CheckResult, stage: &str, why: &str) {
    check.result = result;
    finish(ae_home, check, "failed", stage, why);
}

fn finish(ae_home: &Path, check: &Check, outcome: &str, stage: &str, detail: &str) {
    let stored = write_check(ae_home, check);
    append_log(ae_home, check.attempted_at, outcome, stage, detail);
    if let Err(why) = stored {
        append_log(ae_home, check.attempted_at, "failed", "state", &why);
    }
}

fn write_check(ae_home: &Path, check: &Check) -> Result<(), String> {
    door::replace(&ae_home.join(CHECK_FILE), check.render().as_bytes()).map_err(|why| {
        format!(
            "could not write {}: {why}",
            ae_home.join(CHECK_FILE).display()
        )
    })
}

fn append_log(ae_home: &Path, epoch: i64, outcome: &str, stage: &str, detail: &str) {
    let path = ae_home.join(LOG_FILE);
    let detail = one_line(detail);
    let line = format!("{epoch}\t{outcome}\t{stage}\t{detail}\n");
    let mut bytes = match door::read_bounded(&path, MAX_LOG_BYTES as u64) {
        Ok(Some(bytes)) => bytes,
        Ok(None) | Err(_) => Vec::new(),
    };
    bytes.extend_from_slice(line.as_bytes());
    if bytes.len() > MAX_LOG_BYTES {
        let keep_from = bytes.len() - MAX_LOG_BYTES;
        let line_start = bytes[keep_from..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |at| keep_from + at + 1);
        bytes.drain(..line_start);
    }
    let _ = door::replace(&path, &bytes);
}

fn one_line(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take(MAX_LOG_DETAIL_CHARS)
        .collect()
}

mod door {
    use super::*;

    pub(super) fn read_bounded(path: &Path, cap: u64) -> std::io::Result<Option<Vec<u8>>> {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: automatic-upgrade state is hostile persisted input and must be classified without following a symlink"
        )]
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(why) => return Err(why),
        };
        if !meta.file_type().is_file() || meta.len() > cap {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "state is not a bounded regular file",
            ));
        }
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: bounded automatic-upgrade state is read only after its non-symlink regular-file gate"
        )]
        let mut file = std::fs::File::open(path)?;
        let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
        std::io::Read::by_ref(&mut file)
            .take(cap + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > cap {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "state exceeds its size bound",
            ));
        }
        Ok(Some(bytes))
    }

    pub(super) fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt as _;

        #[allow(
            clippy::disallowed_methods,
            reason = "a door: automatic-upgrade state is replaced only when absent or a regular non-symlink file"
        )]
        match std::fs::symlink_metadata(path) {
            Ok(meta) if !meta.file_type().is_file() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "refusing to replace non-regular state",
                ));
            }
            Ok(_) => {}
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {}
            Err(why) => return Err(why),
        }
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("state");
        let temp = path.with_file_name(format!(".{name}.{}", std::process::id()));
        let _ = std::fs::remove_file(&temp);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        if let Err(why) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            let _ = std::fs::remove_file(&temp);
            return Err(why);
        }
        std::fs::rename(&temp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&temp);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ae-autoupgrade-{tag}-{}", std::process::id()))
    }

    fn installed(home: &Path) -> crate::shape::Shape {
        crate::shape::Shape::Installed {
            home: home.to_owned(),
            version_dir: home.join("versions").join(crate::VERSION),
            version: crate::VERSION.to_owned(),
        }
    }

    #[test]
    fn global_policy_defaults_on_and_accepts_only_on_or_off() {
        let root = root("policy");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture");
        let global = root.join("config");
        assert_eq!(policy(&global), Policy::Enabled, "absent defaults on");
        std::fs::write(&global, "[workspace]\nauto_upgrade = off\n").expect("config");
        assert_eq!(policy(&global), Policy::Disabled);
        std::fs::write(&global, "[workspace]\nauto_upgrade = yes\n").expect("config");
        assert_eq!(policy(&global), Policy::Invalid("yes".to_owned()));
        std::fs::write(&global, "[workspace]\nauto_upgrade =\n").expect("config");
        assert!(matches!(policy(&global), Policy::Invalid(_)));
        std::fs::write(&global, "[workspace]\nauto_upgrade on\n").expect("config");
        assert!(matches!(policy(&global), Policy::Invalid(_)));
        std::fs::write(&global, "[workspace]\nauto_upgrade = \"\"\n").expect("config");
        assert_eq!(policy(&global), Policy::Invalid(String::new()));
        std::fs::write(
            &global,
            "[workspace]\nauto_upgrade = off\n[workspace]\nauto_upgrade = on\n",
        )
        .expect("config");
        assert_eq!(policy(&global), Policy::Enabled, "last global value wins");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test proves a read-only status operation schedules and writes nothing"
    )]
    fn status_names_policy_result_and_malformed_state_without_scheduling() {
        let root = root("status");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture");
        let shape = installed(&root);

        let missing = status(&shape);
        assert_eq!(
            missing.policy.detail,
            "on (global policy; default when absent)"
        );
        assert_eq!(missing.check.detail, "never checked");
        assert!(!root.join(LOCK_FILE).exists(), "status never schedules");

        std::fs::write(root.join("config"), "[workspace]\nauto_upgrade =\n").expect("config");
        let invalid = status(&shape);
        assert!(invalid.policy.warning);
        assert!(invalid.policy.detail.contains("invalid"));

        std::fs::write(root.join(CHECK_FILE), b"hostile\n").expect("state");
        let malformed = status(&shape);
        assert!(malformed.check.warning);
        assert!(malformed.check.detail.contains("invalid upgrade.check"));

        std::fs::write(root.join("config"), "[workspace]\nauto_upgrade = on\n").expect("config");
        write_check(
            &root,
            &Check {
                attempted_at: 1_789_000_000,
                seen: Some("2026.10.2".to_owned()),
                result: CheckResult::Installed,
            },
        )
        .expect("state");
        let finished = status_at(&shape, 1_789_000_000);
        assert_eq!(
            finished.check.detail,
            "2026-09-10T00:26:40Z: installed; seen 2026.10.2"
        );
        assert!(!finished.check.warning);

        let stale = status_at(&shape, 1_789_000_000 + CADENCE_SECS);
        assert!(stale.check.warning);
        assert!(stale.check.detail.ends_with("; stale"), "{:?}", stale.check);

        std::fs::write(root.join("config"), "[workspace]\nauto_upgrade = off\n").expect("config");
        let disabled = status_at(&shape, 1_789_000_000 + CADENCE_SECS);
        assert!(!disabled.check.warning);
        assert!(!disabled.check.detail.ends_with("; stale"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test proves ineligible scheduling paths create none of the updater artifacts"
    )]
    fn checkout_disabled_and_no_autostart_scheduling_have_zero_effects() {
        let root = root("eligibility");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture");
        let mut spawned = 0;

        assert!(!schedule_with(
            &crate::shape::Shape::Checkout,
            false,
            1_000,
            |_| {
                spawned += 1;
                true
            }
        ));
        std::fs::write(root.join("config"), "[workspace]\nauto_upgrade = off\n").expect("config");
        assert!(!schedule_with(&installed(&root), false, 1_000, |_| {
            spawned += 1;
            true
        }));
        std::fs::write(root.join("config"), "[workspace]\nauto_upgrade =\n").expect("config");
        assert!(!schedule_with(&installed(&root), false, 1_000, |_| {
            spawned += 1;
            true
        }));
        std::fs::write(root.join("config"), "[workspace]\nauto_upgrade = on\n").expect("config");
        assert!(!schedule_with(&installed(&root), true, 1_000, |_| {
            spawned += 1;
            true
        }));

        assert_eq!(spawned, 0);
        assert!(!root.join(CHECK_FILE).exists());
        assert!(!root.join(LOG_FILE).exists());
        assert!(!root.join(LOCK_FILE).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_a_structurally_valid_install_reaches_the_detached_spawn() {
        let root = root("validated");
        let _ = std::fs::remove_dir_all(&root);
        let version = root.join("versions").join(crate::VERSION);
        std::fs::create_dir_all(&version).expect("version dir");
        std::fs::write(root.join("config"), "[workspace]\nauto_upgrade = on\n").expect("config");
        let mut spawned = 0;
        assert!(!schedule_with(&installed(&root), false, 1_000, |_| {
            spawned += 1;
            true
        }));
        for member in [crate::shape::CORE, crate::shape::INSTALLER] {
            std::fs::write(version.join(member), member).expect("member");
        }
        std::fs::write(
            version.join(crate::shape::MANIFEST),
            format!(
                "{0}  {1}\n{0}  {2}\n",
                "a".repeat(64),
                crate::shape::CORE,
                crate::shape::INSTALLER
            ),
        )
        .expect("manifest");
        assert!(schedule_with(&installed(&root), false, 1_000, |_| {
            spawned += 1;
            true
        }));
        assert_eq!(spawned, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn many_claimants_reach_one_manifest_fetch() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Barrier};

        const CLAIMANTS: usize = 16;
        let root = Arc::new(root("claimants"));
        let _ = std::fs::remove_dir_all(root.as_ref());
        std::fs::create_dir_all(root.as_ref()).expect("fixture");
        let start = Arc::new(Barrier::new(CLAIMANTS));
        let arrived = Arc::new(AtomicUsize::new(0));
        let fetches = Arc::new(AtomicUsize::new(0));
        let installs = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();
        for _ in 0..CLAIMANTS {
            let root = Arc::clone(&root);
            let start = Arc::clone(&start);
            let arrived = Arc::clone(&arrived);
            let fetches = Arc::clone(&fetches);
            let installs = Arc::clone(&installs);
            workers.push(std::thread::spawn(move || {
                start.wait();
                let claimed = claim(&root, 1_000, || false).expect("claim");
                arrived.fetch_add(1, Ordering::SeqCst);
                if let Some(mut claimed) = claimed {
                    while arrived.load(Ordering::SeqCst) != CLAIMANTS {
                        std::thread::yield_now();
                    }
                    // This is the injected manifest-fetch seam. The winner
                    // keeps exclusion until every competing try-lock returned.
                    fetches.fetch_add(1, Ordering::SeqCst);
                    installs.fetch_add(1, Ordering::SeqCst);
                    claimed.check.result = CheckResult::Current;
                    finish(&root, &claimed.check, "current", "manifest", "test");
                }
            }));
        }
        for worker in workers {
            worker.join().expect("claimant");
        }
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
        assert_eq!(installs.load(Ordering::SeqCst), 1);
        assert!(matches!(
            check_state(&root),
            CheckState::Valid(Check {
                result: CheckResult::Current,
                ..
            })
        ));
        let _ = std::fs::remove_dir_all(root.as_ref());
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test observes a temporary detached child's completion marker without adding a process door"
    )]
    fn detached_checker_returns_before_its_child_and_the_child_completes() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = root("detached");
        let _ = std::fs::remove_dir_all(&root);
        let ae_home = root.join(".ae");
        let bin = root.join(".local").join("bin");
        std::fs::create_dir_all(&ae_home).expect("ae home");
        std::fs::create_dir_all(&bin).expect("bin");
        let marker = root.join("finished");
        let pid_file = root.join("pid");
        let program = bin.join("ae");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\ntest \"$1\" = \"{}\" || exit 9\nprintf %s \"$$\" > '{}'\nsleep 1\nprintf done > '{}'\n",
                crate::cli::AUTOUPGRADE,
                pid_file.display(),
                marker.display()
            ),
        )
        .expect("program");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .expect("executable");
        let argv = detached_argv(&ae_home).expect("sealed argv");

        let started = std::time::Instant::now();
        assert!(crate::transport::run_autoupgrade_detached(&argv));
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "foreground waited for child"
        );
        for _ in 0..40 {
            if marker.is_file() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            std::fs::read_to_string(&marker).ok().as_deref(),
            Some("done")
        );
        let pid = std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|word| word.parse::<u32>().ok())
            .expect("child pid");
        for _ in 0..40 {
            if crate::procs::snapshot().is_some_and(|rows| rows.iter().all(|row| row.pid != pid)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            crate::procs::snapshot().is_some_and(|rows| rows.iter().all(|row| row.pid != pid)),
            "completed detached child was not reaped while its parent remained alive"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn persisted_state_is_strict_and_round_trips() {
        let check = Check {
            attempted_at: 1_789_000_000,
            seen: Some("2026.10.2".to_owned()),
            result: CheckResult::Installed,
        };
        assert_eq!(parse_check(check.render().as_bytes()), Ok(check));
        for bad in [
            b"".as_slice(),
            b"format=1\nattempted_at=1\nseen=bad\nresult=current\n".as_slice(),
            b"format=1\nattempted_at=-1\nseen=\nresult=current\n".as_slice(),
            b"format=1\nattempted_at=1\nseen=\nresult=unknown\n".as_slice(),
            b"format=1\nattempted_at=1\nseen=\nresult=current\nextra=x\n".as_slice(),
            b"format=1\nattempted_at=1\nseen=\nresult=current".as_slice(),
        ] {
            assert!(parse_check(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn cadence_backoff_and_clock_rollback_never_hot_loop() {
        let at = 10_000;
        let state = |result| {
            CheckState::Valid(Check {
                attempted_at: at,
                seen: None,
                result,
            })
        };
        assert!(!due(&state(CheckResult::Current), at + CADENCE_SECS - 1));
        assert!(due(&state(CheckResult::Current), at + CADENCE_SECS));
        assert!(!due(
            &state(CheckResult::FailedManifest),
            at + BACKOFF_SECS - 1
        ));
        assert!(due(&state(CheckResult::FailedManifest), at + BACKOFF_SECS));
        assert!(!due(&state(CheckResult::Started), at - 1));
        assert!(due(&CheckState::Missing, at));
        assert!(due(&CheckState::Malformed("bad".to_owned()), at));
    }

    #[test]
    fn detached_argv_is_public_pointer_plus_one_fixed_word() {
        let argv = detached_argv(Path::new("/u/me/.ae")).expect("parent");
        assert_eq!(
            argv.as_args(),
            ["/u/me/.local/bin/ae", crate::cli::AUTOUPGRADE],
            "no shell, body or version pin"
        );
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test proves the advisory lock file persists after the lock is released"
    )]
    fn outer_lock_is_advisory_and_automatic_claimants_do_not_wait() {
        let root = std::env::temp_dir().join(format!("ae-autoupgrade-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture");
        let held = lock(&root, Duration::ZERO).expect("first claimant");
        let refused = lock(&root, Duration::ZERO).expect_err("held lock must refuse");
        assert_eq!(refused.kind(), std::io::ErrorKind::WouldBlock);
        assert!(root.join(LOCK_FILE).is_file(), "lock file persists");
        drop(held);
        assert!(
            lock(&root, Duration::ZERO).is_ok(),
            "released lock is reusable"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test inspects its private log fixture and planted symlink target"
    )]
    fn diagnostic_log_is_bounded_keeps_the_latest_outcome_and_refuses_a_symlink() {
        use std::os::unix::fs::symlink;

        let root = root("log");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture");
        for epoch in 0..100 {
            append_log(&root, epoch, "failed", "install", &"x".repeat(10_000));
        }
        let bytes = std::fs::read(root.join(LOG_FILE)).expect("log");
        assert!(bytes.len() <= MAX_LOG_BYTES, "{}", bytes.len());
        assert!(
            String::from_utf8_lossy(&bytes).contains("99\tfailed\tinstall\t"),
            "latest outcome was discarded"
        );

        let external = root.join("external");
        std::fs::write(&external, "untouched").expect("external");
        std::fs::remove_file(root.join(LOG_FILE)).expect("old log");
        symlink(&external, root.join(LOG_FILE)).expect("planted link");
        append_log(&root, 101, "failed", "state", "must not follow");
        assert_eq!(
            std::fs::read_to_string(&external).ok().as_deref(),
            Some("untouched")
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
