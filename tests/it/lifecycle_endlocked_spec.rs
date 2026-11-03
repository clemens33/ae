//! Characterisation pins for `end_locked` (slice endlocked, #300 step 3):
//! the observable behaviour of `_end` at 73c6b309, frozen before the
//! refactor extracts pure decisions and named steps from it.
//!
//! Oracle: the CURRENT observable behaviour — exit code, stdout, stderr,
//! ordered tmux argv, ordered git children (`GIT_TRACE2`), session-dir and
//! archive effects, worktree and remote effects. Bug-shaped behaviour is
//! pinned as-is and named in the test that pins it; nothing here asks for a
//! fix. Where a group is already pinned elsewhere this file adds only the
//! exact text, order, or effect the older pin does not carry:
//! P1 lock `lifecycle.rs` held-lock test, P6 boot-proof trio (substring
//! level there, exact here), P8 reprove cells (table-tested in `end.rs`),
//! P11 retry-no-network (`lifecycle_transact_spec.rs`, final lines here).
//!
//! Trace pins run `_end -f` under the launchx perl tmux wrapper on the
//! rig's private socket; hook envs refuse one argv or rewrite meta at the
//! kill, as `buildx` does. No live session is ever a target: every server
//! here is the rig's own.

use super::super::cli::{OwnedScratch, Runner, ae, bounded, git_in};
use super::super::session_launch::launchx_trace_command;
use super::guard_spec::{end, half_ended, keepalive, obstruct_teardown, put, tree};
use super::retain_spec::managed;
use super::*;

// ---- shared rig helpers -----------------------------------------------------

/// Rewrite one `key=value` row of the rig's live meta (`None` drops it).
fn set_row(rig: &Rig, key: &str, value: Option<&str>) {
    let path = rig.dir.join("meta");
    let text = std::fs::read_to_string(&path).expect("rig meta");
    let prefix = format!("{key}=");
    let mut rows: Vec<String> = text
        .lines()
        .filter(|line| !line.starts_with(&prefix))
        .map(str::to_owned)
        .collect();
    if let Some(value) = value {
        rows.push(format!("{prefix}{value}"));
    }
    std::fs::write(&path, rows.join("\n") + "\n").expect("rewritten meta");
}

fn meta_bytes(rig: &Rig) -> Vec<u8> {
    std::fs::read(rig.dir.join("meta")).expect("rig meta bytes")
}

/// The `$<n>` id of rig tmux session `name`, empty when absent.
fn session_id(rig: &Rig, name: &str) -> String {
    let (_, listed) = rig.tmux(&["list-sessions", "-F", "#{session_id} #{session_name}"]);
    listed
        .lines()
        .find_map(|line| line.split_once(' ').filter(|(_, n)| *n == name))
        .map_or_else(String::new, |(id, _)| id.to_owned())
}

fn events_text(path: &Path) -> String {
    std::fs::read_to_string(path.join("events.jsonl")).unwrap_or_default()
}

/// Rows of an archived meta as (key, value) lookups.
fn archive_row(rig: &Rig, key: &str) -> String {
    let text = std::fs::read_to_string(rig.archive().join("meta")).expect("archive meta");
    let prefix = format!("{key}=");
    text.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or_default()
        .to_owned()
}

// ---- traced end runs ---------------------------------------------------------

const HOOK: &str = r#"
my $boundary = $ENV{AE_ENDX_BOUNDARY} // '';
if ($boundary ne '' && (grep { $_ eq 'kill-session' } @ARGV)
    && (grep { $_ eq "=$ENV{AE_ENDX_TARGET}" } @ARGV)) {
    use Fcntl qw(:flock);
    open(my $lock, '>>', $ENV{AE_ENDX_LOCK}) or die "boundary lock: $!";
    my $held = flock($lock, LOCK_EX | LOCK_NB) ? 0 : 1;
    my $archive = -e $ENV{AE_ENDX_ARCHIVE} ? 1 : 0;
    my $meta = -f $ENV{AE_ENDX_META} ? 1 : 0;
    my $entries = 0;
    if (opendir(my $root, $ENV{AE_ENDX_ARCHIVE_ROOT})) {
        $entries = scalar grep { $_ ne '.' && $_ ne '..' } readdir($root);
        closedir($root);
    }
    open(my $facts, '>', $boundary) or die "boundary facts: $!";
    print $facts "held=$held archive=$archive meta=$meta entries=$entries\n";
    close($facts);
    close($lock);
}
if (($ENV{AE_ENDX_FAIL_SWITCH} // '') eq '1' && grep { $_ eq 'switch-client' } @ARGV) {
    exit 1;
}
my $fail_kill = $ENV{AE_ENDX_FAIL_KILL} // '';
if ($fail_kill ne '' && (grep { $_ eq 'kill-session' } @ARGV)
    && (grep { index($_, $fail_kill) >= 0 } @ARGV)) {
    exit 1;
}
my $drift = $ENV{AE_ENDX_DRIFT_META} // '';
if ($drift ne '' && grep { $_ eq 'kill-session' } @ARGV) {
    open(my $in, '<', $drift) or die "drift read: $!";
    local $/; my $body = <$in>; close($in);
    $body =~ s/^origin=.*$/origin=\/drifted\/elsewhere/m;
    open(my $out, '>', $drift) or die "drift write: $!";
    print $out $body; close($out);
}
"#;

/// One `_end` against the rig under the launchx argv wrapper; the trace
/// rows come back as argv vectors in call order.
fn end_traced(
    rig: &Rig,
    args: &[&str],
    extra: &[(&str, &str)],
) -> (Option<i32>, String, String, Vec<Vec<String>>) {
    let mut cmd = ae();
    cmd.env("AE_HOME", &rig.home);
    cmd.env_remove("TMUX");
    cmd.env_remove("TMUX_PANE");
    for (key, value) in extra {
        cmd.env(key, value);
    }
    for arg in args {
        cmd.arg(arg);
    }
    let log = launchx_trace_command(&mut cmd, &rig.home);
    install_hook(&rig.home);
    let (code, out, err) = run_runner(cmd);
    let text = std::fs::read_to_string(&log).expect("tmux trace");
    let rows = text
        .lines()
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect();
    (code, out, err, rows)
}

fn install_hook(home: &Path) {
    let wrapper = home.join("trace-bin/tmux");
    let original = std::fs::read_to_string(&wrapper).expect("trace wrapper");
    let marker = "exec {$ENV{AE_LAUNCHX_REAL_TMUX}}";
    assert_eq!(original.matches(marker).count(), 1, "one real-tmux exec");
    std::fs::write(
        &wrapper,
        original.replace(marker, &format!("{HOOK}\n{marker}")),
    )
    .expect("hooked wrapper");
}

fn run_runner(mut cmd: Runner) -> (Option<i32>, String, String) {
    let out = bounded(
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the ae binary should run"),
        Duration::from_secs(30),
    )
    .expect("the core returned");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Index of the first trace row carrying `verb` (and `needle` in any arg).
fn at(rows: &[Vec<String>], verb: &str, needle: &str) -> Option<usize> {
    rows.iter().position(|row| {
        row.iter().any(|arg| arg == verb)
            && (needle.is_empty() || row.iter().any(|arg| arg.contains(needle)))
    })
}

fn count(rows: &[Vec<String>], verb: &str, needle: &str) -> usize {
    rows.iter()
        .filter(|row| {
            row.iter().any(|arg| arg == verb)
                && (needle.is_empty() || row.iter().any(|arg| arg.contains(needle)))
        })
        .count()
}

fn trace_dump(rows: &[Vec<String>]) -> String {
    rows.iter()
        .map(|row| row.join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---- P4: live local end order ----------------------------------------------

/// P1/P4: observe the actual lifecycle lock and disk state at the target
/// kill boundary. Final-state assertions alone cannot prove this order.
#[test]
fn endlocked_kill_holds_the_lock_before_any_archive_and_leaves_it_reusable() {
    let rig = Rig::new("elockorder");
    let target = session_id(&rig, &rig.name);
    assert!(!target.is_empty(), "a precise target id");
    let lock = rig
        .home
        .join("sessions")
        .join(format!(".lifecycle.{}.lock", rig.name));
    let facts = rig.home.join("kill-boundary");
    let (code, out, err, _) = end_traced(
        &rig,
        &["_end", "-f", &rig.name],
        &[
            ("AE_ENDX_BOUNDARY", &facts.display().to_string()),
            ("AE_ENDX_TARGET", &target),
            ("AE_ENDX_LOCK", &lock.display().to_string()),
            ("AE_ENDX_ARCHIVE", &rig.archive().display().to_string()),
            (
                "AE_ENDX_ARCHIVE_ROOT",
                &rig.home.join("archive").display().to_string(),
            ),
            ("AE_ENDX_META", &rig.dir.join("meta").display().to_string()),
        ],
    );
    assert_eq!(
        std::fs::read_to_string(facts).expect("boundary observed"),
        "held=1 archive=0 meta=1 entries=0\n"
    );
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(exists(&rig.archive().join("meta")), "published after kill");
    assert!(!exists(&rig.dir), "unregistered after publication");
    let released = std::fs::OpenOptions::new()
        .append(true)
        .open(lock)
        .expect("lock inode");
    assert!(released.try_lock().is_ok(), "lock file remains reusable");
}

/// P4: a live local end with an attached client proves, in argv order:
/// sight, fleet read, client read, switch, then the kill of the target id,
/// then the verify listing. The client lands on the survivor and the
/// archive records the not-managed sentinel facts.
#[test]
fn endlocked_live_local_hands_off_then_kills_in_order() {
    let rig = Rig::new("elive");
    keepalive(&rig);
    let target = session_id(&rig, &rig.name);
    assert!(!target.is_empty(), "the live target has an id");
    let (_host, client) = rig.attach_client();
    let (code, out, err, rows) = end_traced(&rig, &["_end", "-f", &rig.name], &[]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    let dump = trace_dump(&rows);
    let sight = at(&rows, "list-sessions", "#{session_created}").expect("sight listing");
    let fleet = at(&rows, "list-sessions", "").expect("a sessions listing");
    let clients = at(&rows, "list-clients", "").expect("client read before the kill");
    let switch = at(&rows, "switch-client", "keepalive").expect("the handoff switch");
    let kill = at(&rows, "kill-session", &target).expect("the kill of the target id");
    let verify = rows
        .iter()
        .rposition(|row| row.iter().any(|arg| arg == "list-sessions"))
        .expect("the verify listing");
    assert!(sight < switch && fleet <= switch, "{dump}");
    assert!(clients < switch && switch < kill && kill < verify, "{dump}");
    let (_, now) = rig.tmux(&["list-clients", "-F", "#{client_name} #{session_name}"]);
    assert!(
        now.lines()
            .any(|line| line == format!("{client} keepalive")),
        "the client was handed to the survivor: {now}"
    );
    assert!(exists(&rig.archive().join("meta")), "archive published");
    assert!(!exists(&rig.dir), "live state removed after the archive");
    assert_eq!(archive_row(&rig, "git_push_outcome"), "not-managed");
    assert_eq!(archive_row(&rig, "git_push_ref"), "-");
    assert_eq!(archive_row(&rig, "preserved_work_dir"), "-");
    assert!(
        out.contains(&format!("Archived {UUID}"))
            && out.contains(&format!("Cleaned up local session {}", rig.name))
            && out.contains(&format!("Ended local session {}", rig.name)),
        "{out}"
    );
    assert_eq!(err, migrate_note(&rig), "only the migrate note is reported");
}

/// The stderr line every rig end carries: the hand-built meta is placed at
/// `meta_version=2` by its `schema=2` row and stepped to 4 before the end
/// reads it.
fn migrate_note(rig: &Rig) -> String {
    format!(
        "note: session {:?} was migrated from meta_version=2 to 4.\n",
        rig.name
    )
}

/// I9: with no other session to hand to, the fleet is still read on the way
/// to the kill but no client read or switch is ever issued.
#[test]
fn endlocked_solo_live_end_reads_no_clients() {
    let rig = Rig::new("esolo");
    let target = session_id(&rig, &rig.name);
    let (code, out, err, rows) = end_traced(&rig, &["_end", "-f", &rig.name], &[]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    let dump = trace_dump(&rows);
    assert!(at(&rows, "kill-session", &target).is_some(), "{dump}");
    assert_eq!(count(&rows, "list-clients", ""), 0, "{dump}");
    assert_eq!(count(&rows, "switch-client", ""), 0, "{dump}");
    // Sight lists once (a live hit pays no verify before the kill), the
    // fleet read and the post-kill verify are further listings.
    assert!(count(&rows, "list-sessions", "") >= 3, "{dump}");
}

// ---- P5: positive Absent and orphan pane views ------------------------------

/// Stop the rig session out from under ae, leaving its state behind.
fn stop_outside(rig: &Rig) {
    assert!(
        rig.tmux(&["kill-session", "-t", &format!("={}", rig.name)])
            .0,
        "the session stops outside ae"
    );
    assert!(!rig.session_is_live(), "the session is gone");
}

/// P5: an absent session ends with no kill at all; the orphan-view listing
/// is paid because views could still hold its windows.
#[test]
fn endlocked_absent_without_views_ends_with_no_kill() {
    let rig = Rig::new("eabsent");
    keepalive(&rig);
    stop_outside(&rig);
    let (code, out, err, rows) = end_traced(&rig, &["_end", "-f", &rig.name], &[]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    let dump = trace_dump(&rows);
    assert_eq!(count(&rows, "kill-session", ""), 0, "{dump}");
    assert!(
        at(&rows, "list-sessions", "@ae_view_uuid").is_some(),
        "the orphan listing was read: {dump}"
    );
    // A miss pays both the identity listing and the strict verify listing.
    assert!(count(&rows, "list-sessions", "") >= 2, "{dump}");
    assert!(!exists(&rig.dir), "the stopped state is gone");
    assert!(exists(&rig.archive().join("meta")), "archived first");
}

/// Plant an orphan pane view still recorded against the rig session uuid.
fn orphan_view(rig: &Rig) -> String {
    let view = "_aeview-el1";
    assert!(
        rig.tmux(&["new-session", "-d", "-s", view, "sh"]).0,
        "the orphan view starts"
    );
    assert!(
        rig.tmux(&[
            "set-option",
            "-t",
            &format!("={view}:"),
            "@ae_view_uuid",
            UUID
        ])
        .0,
        "the view is recorded against the session"
    );
    view.to_owned()
}

/// P5/I10: the orphan view is KILLED by the end (an effect, not a refusal)
/// and the end then proceeds.
#[test]
fn endlocked_absent_orphan_view_is_killed_and_the_end_proceeds() {
    let rig = Rig::new("eorphan");
    keepalive(&rig);
    let view = orphan_view(&rig);
    stop_outside(&rig);
    let view_id = session_id(&rig, &view);
    let (code, out, err, rows) = end_traced(&rig, &["_end", "-f", &rig.name], &[]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(
        at(&rows, "kill-session", &view_id).is_some(),
        "the view was killed by id: {}",
        trace_dump(&rows)
    );
    assert_eq!(session_id(&rig, &view), "", "the view is gone");
    assert!(!exists(&rig.dir), "the end completed");
}

/// P5/I10: a view that survives its kill refuses the end with the held
/// text; the session state and the view both survive.
#[test]
fn endlocked_absent_view_that_survives_its_kill_refuses() {
    let rig = Rig::new("eheld");
    keepalive(&rig);
    let view = orphan_view(&rig);
    stop_outside(&rig);
    let view_id = session_id(&rig, &view);
    let (code, out, err, _rows) = end_traced(
        &rig,
        &["_end", "-f", &rig.name],
        &[("AE_ENDX_FAIL_KILL", &view_id)],
    );
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    let want = format!(
        "{}Error: session '{}' is gone, but pane view session(s) {view_id} still hold its windows, so its agents still run — state preserved. Retry 'ae end {}'.\n  Nothing was stopped and nothing was deleted.\n1 session(s) failed to end. See errors above.\n",
        migrate_note(&rig),
        rig.name,
        rig.name
    );
    assert_eq!(err, want, "the exact held-view refusal");
    assert!(out.is_empty(), "nothing was reported done: {out}");
    assert!(exists(&rig.dir.join("meta")), "the state survives");
    assert!(!exists(&rig.archive()), "no archive was published");
    assert_ne!(session_id(&rig, &view), "", "the view still holds on");
}

// ---- P6: positive Unknown, boot proof (exact text) --------------------------

fn boot_end(tag: &str, proof: bool, flag: bool) -> (Rig, Option<i32>, String, String) {
    let rig = Rig::new(tag);
    let last_live = immortal_fixture(&rig);
    let boot = if proof {
        last_live + 3600
    } else {
        last_live - 3600
    }
    .to_string();
    let mut args = vec!["_end", "-f"];
    if flag {
        args.push("--assume-stopped");
    }
    args.push(&rig.name);
    let run = rig.run_with_env(&[("AE_TEST_BOOT_TIME", &boot)], &args);
    (rig, run.0, run.1, run.2)
}

fn unreachable_refusal(rig: &Rig) -> String {
    format!(
        "{}Error: cannot verify session '{}' state (its recorded tmux server is unreachable) — state preserved.\n1 session(s) failed to end. See errors above.\n",
        migrate_note(rig),
        rig.name
    )
}

/// P6: `--assume-stopped` alone never crosses an unreachable recorded
/// server, and the boot proof is not even the refusal text — one line.
#[test]
fn endlocked_unknown_with_flag_without_boot_proof_refuses_exactly() {
    let (rig, code, out, err) = boot_end("eboot1", false, true);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "{out}");
    assert_eq!(err, unreachable_refusal(&rig));
    assert!(exists(&rig.dir.join("meta")), "state preserved");
    assert!(!exists(&rig.archive()), "no archive");
}

/// P6: the boot proof alone, without the human acknowledgement, refuses
/// with the identical line.
#[test]
fn endlocked_unknown_with_boot_proof_without_flag_refuses_exactly() {
    let (rig, code, out, err) = boot_end("eboot2", true, false);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "{out}");
    assert_eq!(err, unreachable_refusal(&rig));
    assert!(exists(&rig.dir.join("meta")), "state preserved");
}

/// P6: flag AND proof proceed, announcing both, on stdout first.
#[test]
fn endlocked_unknown_with_flag_and_boot_proof_proceeds_announced() {
    let (rig, code, out, err) = boot_end("eboot3", true, true);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert_eq!(
        out.lines().next(),
        Some(
            "Proceeding on explicit --assume-stopped acknowledgement and the boot-time proof (this session's own last sign of life predates the host boot, so nothing can still hold it)."
        ),
        "{out}"
    );
    assert_eq!(err, migrate_note(&rig));
    assert!(exists(&rig.archive().join("meta")), "archived");
    assert!(!exists(&rig.dir), "removed after archive");
}

// ---- P2/P3: preflight before migrate ----------------------------------------

/// An unowned symlink in the session dir makes the retained preflight fail.
fn plant_bad_node(rig: &Rig) -> PathBuf {
    let link = rig.dir.join("planted-link");
    std::os::unix::fs::symlink("/etc/hosts", &link).expect("an unowned symlink");
    link
}

fn preflight_why(link: &Path) -> String {
    format!(
        "archive: cannot capture {} (unowned symlink); remove it, then ae end",
        link.display()
    )
}

/// P2: with no archive node yet, the unadorned reason is the refusal, and
/// it lands BEFORE migration writes anything — meta bytes untouched, the
/// live session untouched.
#[test]
fn endlocked_preflight_refuses_before_migrate_with_plain_reason() {
    let rig = Rig::new("epre1");
    let link = plant_bad_node(&rig);
    let before = meta_bytes(&rig);
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert_eq!(
        err,
        format!(
            "Error: {}\n1 session(s) failed to end. See errors above.\n",
            preflight_why(&link)
        )
    );
    assert_eq!(meta_bytes(&rig), before, "migration never ran");
    assert!(rig.session_is_live(), "nothing was stopped");
    assert!(!exists(&rig.archive()), "no archive");
}

/// P2: when the archive target node already exists, the same preflight
/// failure is reported as an archive/live mismatch naming both paths.
#[test]
fn endlocked_preflight_with_archive_node_reports_the_mismatch() {
    let rig = Rig::new("epre2");
    let link = plant_bad_node(&rig);
    std::fs::create_dir_all(rig.archive()).expect("a standing archive node");
    let before = meta_bytes(&rig);
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    let want = format!(
        "Error: archive {} cannot match live directory {}: {}. NOTHING was deleted. Move the whole STOPPED live directory to a new private recovery directory outside {}, then restart under a fresh name.\n1 session(s) failed to end. See errors above.\n",
        rig.archive().display(),
        rig.dir.display(),
        preflight_why(&link),
        rig.home.join("sessions").display()
    );
    assert_eq!(err, want);
    assert_eq!(meta_bytes(&rig), before, "migration never ran");
    assert!(rig.session_is_live(), "nothing was stopped");
}

/// P3: an unblocked end reports the migration on stderr — the only stderr
/// line of a clean rig end — and then ends normally.
#[test]
fn endlocked_migrate_note_precedes_a_normal_end_and_is_archived() {
    let rig = Rig::new("emig");
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert_eq!(err, migrate_note(&rig));
    assert_eq!(archive_row(&rig, "source_session_id"), UUID);
    assert!(!exists(&rig.dir), "the end completed");
}

// ---- P9/P8: pre-stop refusals -------------------------------------------------

/// P9: an archive node with no complete strict archive fails the retry
/// inspection BEFORE anything is stopped.
#[test]
fn endlocked_incomplete_archive_node_refuses_the_inspection() {
    let rig = Rig::new("einsp");
    std::fs::create_dir_all(rig.archive()).expect("a bare archive node");
    let (code, out, err, rows) = end_traced(&rig, &["_end", "-f", &rig.name], &[]);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    let want = format!(
        "{}Error: archive {} cannot match live directory {}: no complete strict End archive. NOTHING was deleted. Move the whole STOPPED live directory to a new private recovery directory outside {}, then restart under a fresh name.\n1 session(s) failed to end. See errors above.\n",
        migrate_note(&rig),
        rig.archive().display(),
        rig.dir.display(),
        rig.home.join("sessions").display()
    );
    assert_eq!(err, want);
    assert_eq!(count(&rows, "kill-session", ""), 0, "{}", trace_dump(&rows));
    assert!(rig.session_is_live(), "the refusal precedes the stop");
    assert!(exists(&rig.dir.join("meta")), "state preserved");
}

/// P8: a session whose recorded id is not a UUID cannot be archived, and
/// the refusal lands before the stop with both lines exact.
#[test]
fn endlocked_unarchivable_session_refuses_before_the_stop() {
    let rig = Rig::new("eunavail");
    set_row(&rig, "session_id", Some("not-a-uuid"));
    let (code, out, err, rows) = end_traced(&rig, &["_end", "-f", &rig.name], &[]);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    let want = format!(
        "{}Error: session '{}' cannot be archived — its session_id (not-a-uuid) is not a UUID, and ae will not overwrite a value it cannot parse.\n  Nothing was stopped and NOTHING was deleted; ae does not delete a session it could not capture.\n1 session(s) failed to end. See errors above.\n",
        migrate_note(&rig),
        rig.name
    );
    assert_eq!(err, want);
    assert_eq!(count(&rows, "kill-session", ""), 0, "{}", trace_dump(&rows));
    assert!(rig.session_is_live(), "nothing was stopped");
}

// ---- P7: no positive record, the enumerable sweep ----------------------------

/// A recordless session in a scratch state root whose `TMUX_TMPDIR` the
/// test owns, so the sweep enumerates exactly what the test plants there.
struct SweepRig {
    scratch: Scratch,
    dir: PathBuf,
    name: String,
}

impl SweepRig {
    fn new(tag: &str, name: &str) -> Self {
        let scratch = Scratch::new(OwnedScratch::root("lc", tag).keep());
        let home = scratch.home.clone();
        let dir = home.join("sessions").join(name);
        put(&dir, "notes.txt", b"the only copy");
        let text = format!(
            "session={name}\nsession_id={UUID}\nwork_dir={}\nmode=local\nschema=2\n",
            home.display()
        );
        put(&dir, "meta", text.as_bytes());
        std::fs::create_dir_all(home.join("tmux")).expect("a private tmux dir");
        std::fs::write(home.join("config"), "").expect("a config");
        Self {
            scratch,
            dir,
            name: name.to_owned(),
        }
    }

    fn archive(&self) -> PathBuf {
        self.scratch.home.join("archive").join(UUID)
    }

    /// The socket directory the sweep will enumerate for this root.
    fn enum_dir(&self) -> PathBuf {
        use std::os::unix::fs::MetadataExt as _;
        let uid = std::fs::metadata(&self.scratch.home)
            .expect("the state root")
            .uid();
        self.scratch.home.join("tmux").join(format!("tmux-{uid}"))
    }

    /// Start a bare tmux server on one enumerable socket holding `session`.
    fn plant_server(&self, label: &str, session: &str) -> PathBuf {
        let dir = self.enum_dir();
        std::fs::create_dir_all(&dir).expect("the enumerable dir");
        let socket = dir.join(label);
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(socket.clone()),
        ));
        args.extend(
            ["-f", "/dev/null", "new-session", "-d", "-s", session, "sh"].map(str::to_owned),
        );
        assert!(
            super::super::phase2::run_tmux(&args, &self.scratch.home).0,
            "the planted enumerable server starts"
        );
        socket
    }

    fn kill_server(&self, socket: &Path) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(socket.to_path_buf()),
        ));
        args.push("kill-server".to_owned());
        super::super::phase2::run_tmux(&args, &self.scratch.home);
    }

    fn end(&self, flag: bool) -> (Option<i32>, String, String, Vec<Vec<String>>) {
        let home = &self.scratch.home;
        let mut cmd = ae();
        cmd.env("AE_HOME", home)
            .env("CONFIG_FILE", home.join("config"))
            .env("TMUX_TMPDIR", home.join("tmux"))
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", home.join("tmux").join("s"))
            .env("AE_NO_AUTOSTART", "1")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .arg("_end")
            .arg("-f");
        if flag {
            cmd.arg("--assume-stopped");
        }
        cmd.arg(&self.name);
        let log = launchx_trace_command(&mut cmd, home);
        install_hook(home);
        let (code, out, err) = run_runner(cmd);
        let text = std::fs::read_to_string(&log).expect("tmux trace");
        let rows = text
            .lines()
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect();
        (code, out, err, rows)
    }

    fn note(&self) -> String {
        format!(
            "note: session {:?} was migrated from meta_version=2 to 4.\n",
            self.name
        )
    }
}

/// P7: sweep Clear without the acknowledgement refuses with three lines;
/// the sweep itself makes no tmux call when nothing is enumerable.
#[test]
fn endlocked_recordless_clear_sweep_without_flag_refuses() {
    let rig = SweepRig::new("esw1", "swclear");
    let (code, out, err, rows) = rig.end(false);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    let want = format!(
        "{}Error: session '{}' has no positive server record (pre-fix or ambiguous meta) — it may be live somewhere ae cannot enumerate.\n  Resolve: 'ae doctor --refresh {}', or acknowledge it is stopped: ae end --assume-stopped {}\n  State preserved.\n1 session(s) failed to end. See errors above.\n",
        rig.note(),
        rig.name,
        rig.name,
        rig.name
    );
    assert_eq!(err, want);
    assert!(out.is_empty(), "{out}");
    assert!(rows.is_empty(), "no tmux was asked: {}", trace_dump(&rows));
    assert!(exists(&rig.dir.join("notes.txt")), "nothing deleted");
    assert!(!exists(&rig.archive()), "no archive");
}

/// P7: the same Clear sweep with the acknowledgement proceeds, announcing
/// the enumerable proof, and still issues no kill — there is no server.
#[test]
fn endlocked_recordless_clear_sweep_with_flag_proceeds_without_a_kill() {
    let rig = SweepRig::new("esw2", "swclear");
    let (code, out, err, rows) = rig.end(true);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert_eq!(
        out.lines().next(),
        Some(
            "Proceeding on explicit --assume-stopped acknowledgement (not live on any enumerable server, default included)."
        ),
        "{out}"
    );
    assert_eq!(count(&rows, "kill-session", ""), 0, "{}", trace_dump(&rows));
    assert!(!exists(&rig.dir), "the state is gone");
    assert!(exists(&rig.archive().join("meta")), "archived first");
}

/// P7: a name LIVE on one enumerable server refuses even WITH the
/// acknowledgement — the sweep answer outranks the flag.
#[test]
fn endlocked_recordless_live_on_an_enumerable_server_refuses_despite_the_flag() {
    let rig = SweepRig::new("esw3", "swlive");
    let socket = rig.plant_server("enum1", "swlive");
    let (code, out, err, _rows) = rig.end(true);
    rig.kill_server(&socket);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    let want = format!(
        "{}Error: session '{}' has no positive server record AND is LIVE on enumerable server 'enum1'.\n  Establish ownership first: 'ae doctor --refresh {}'.\n  State preserved.\n1 session(s) failed to end. See errors above.\n",
        rig.note(),
        rig.name,
        rig.name
    );
    assert_eq!(err, want);
    assert!(exists(&rig.dir.join("notes.txt")), "state preserved");
}

/// P7: an enumerable socket that will not answer refuses — the target
/// could be hiding behind it — and names the socket. The unanswerable
/// socket is a live server behind a mode-000 socket file.
#[test]
fn endlocked_recordless_unanswerable_enumerable_server_refuses() {
    use std::os::unix::fs::PermissionsExt as _;
    let rig = SweepRig::new("esw4", "swmute");
    let socket = rig.plant_server("enum1", "someoneelse");
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o000))
        .expect("the socket goes mute");
    let (code, out, err, _rows) = rig.end(true);
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o700))
        .expect("the socket speaks again");
    rig.kill_server(&socket);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    let want = format!(
        "{}Error: cannot verify enumerable tmux server 'enum1' (socket exists but did not answer) — session '{}' could be hiding behind it.\n  Fix that server (permissions/staleness), then retry. State preserved.\n1 session(s) failed to end. See errors above.\n",
        rig.note(),
        rig.name
    );
    assert_eq!(err, want);
    assert!(exists(&rig.dir.join("notes.txt")), "state preserved");
}

// ---- P10/P11: handoff failure records and retry lines ------------------------

fn warning_line(rig: &Rig) -> String {
    format!(
        "Warning: client handoff failed: 1 client(s) could not switch from '{}' to 'keepalive'",
        rig.name
    )
}

/// P10 fresh Keep: a failed handoff records its end-result row into the
/// session journal before the finish (kept observable here by a teardown
/// that refuses after the archive published), and warns on stderr.
#[test]
fn endlocked_fresh_handoff_failure_records_its_result_and_warns() {
    let rig = Rig::new("ehand1");
    keepalive(&rig);
    let _client = rig.attach_client();
    let tomb = obstruct_teardown(&rig);
    put(&rig.dir, "memo.tsv", b"x\tremember\n");
    let (code, out, err, _rows) = end_traced(
        &rig,
        &["_end", "-f", &rig.name],
        &[("AE_ENDX_FAIL_SWITCH", "1")],
    );
    std::fs::remove_file(&tomb).expect("the obstruction cleared");
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(err.contains(&warning_line(&rig)), "{err}");
    let events = events_text(&rig.dir);
    assert!(
        events.contains("end-result") && events.contains("client handoff failed"),
        "the failed handoff was journaled: {events}"
    );
    assert!(!rig.session_is_live(), "the session was still stopped");
    assert!(exists(&rig.archive().join("meta")), "the archive published");
}

/// P10/I3: when the kill itself fails after a failed handoff, the note is
/// DISCARDED — no Warning, no journal row — and the refusal is the
/// still-alive line alone.
#[test]
fn endlocked_failed_handoff_is_discarded_when_the_kill_fails() {
    let rig = Rig::new("ehand2");
    keepalive(&rig);
    let _client = rig.attach_client();
    let target = session_id(&rig, &rig.name);
    let (code, out, err, _rows) = end_traced(
        &rig,
        &["_end", "-f", &rig.name],
        &[("AE_ENDX_FAIL_SWITCH", "1"), ("AE_ENDX_FAIL_KILL", &target)],
    );
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    let want = format!(
        "{}Error: could not kill session '{}' — still alive; state preserved. Retry 'ae end {}'.\n1 session(s) failed to end. See errors above.\n",
        migrate_note(&rig),
        rig.name,
        rig.name
    );
    assert_eq!(err, want);
    assert!(
        !events_text(&rig.dir).contains("client handoff failed"),
        "no row was recorded for the discarded note"
    );
    assert!(rig.session_is_live(), "the session survives");
    assert!(exists(&rig.dir.join("meta")), "the state survives");
}

/// P10 retry: on a retry whose strict archive already stands, the same
/// failed handoff WARNS but journals nothing — the emission is decided
/// against an already-completed archive. The session is live again on its
/// recorded server so the handoff has something to fail at.
#[test]
fn endlocked_retry_handoff_failure_warns_but_journals_nothing() {
    let (rig, _wt, _proj) = managed("ehand3");
    let _first = half_ended(&rig);
    assert!(
        rig.tmux(&["new-session", "-d", "-s", &rig.name, "sh"]).0,
        "the session is live again for the retry"
    );
    let _client = rig.attach_client();
    let (code, out, err, _rows) = end_traced(
        &rig,
        &["_end", "-f", &rig.name],
        &[
            ("AE_ENDX_FAIL_SWITCH", "1"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ],
    );
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(err.contains(&warning_line(&rig)), "{err}");
    assert!(!exists(&rig.dir), "the retry converged");
    let events = events_text(&rig.archive());
    assert!(
        !events.contains("client handoff failed"),
        "nothing was appended after the strict publish: {events}"
    );
    assert_eq!(
        out.lines().last(),
        Some(format!("Ended {}", rig.name)).as_deref()
    );
}

/// P11: a local retry reuses its archive and still reports the LOCAL final
/// line — the local branch outranks the retry branch.
#[test]
fn endlocked_a_local_retry_reports_the_local_final_line() {
    let rig = Rig::new("eretry1");
    let first = half_ended(&rig);
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert_eq!(
        out.lines().last(),
        Some(format!("Ended local session {}", rig.name)).as_deref()
    );
    assert_eq!(tree(&rig.archive()), first, "the archive is reused");
    assert!(!exists(&rig.dir), "unregistered");
}

/// P11: a git retry runs no commit or push again and reports the plain
/// final line. (The no-network half is pinned with `GIT_TRACE2` in
/// `lifecycle_transact_spec.rs`; here the spoken lines are pinned.)
#[test]
fn endlocked_a_git_retry_reports_the_plain_final_line() {
    let (rig, _wt, _proj) = managed("eretry2");
    let _first = half_ended(&rig);
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(!out.contains("Committing changes"), "{out}");
    assert!(!out.contains("Pushing to origin"), "{out}");
    assert_eq!(
        out.lines().last(),
        Some(format!("Ended {}", rig.name)).as_deref()
    );
}

// ---- P13: finish failures after the stop --------------------------------------

/// P13: meta identity that changes while the session is being killed is
/// refused at the finish, after the stop — stopped, kept, unarchived.
#[test]
fn endlocked_identity_drift_at_the_kill_refuses_the_finish() {
    let rig = Rig::new("edrift");
    let meta = rig.dir.join("meta");
    let meta_text = meta.display().to_string();
    let (code, out, err, _rows) = end_traced(
        &rig,
        &["_end", "-f", &rig.name],
        &[("AE_ENDX_DRIFT_META", &meta_text)],
    );
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(
        err.contains("operation identity changed before the holds")
            && err.contains(&rig.archive().display().to_string())
            && err.contains(&rig.dir.display().to_string()),
        "{err}"
    );
    assert!(!rig.session_is_live(), "the session was stopped first");
    assert!(exists(&rig.dir.join("meta")), "nothing was deleted");
    assert!(!exists(&rig.archive()), "nothing was published");
    assert!(!out.contains("Ended "), "no success was reported: {out}");
}

/// The standing publish claim that makes `finish` refuse after git ran.
fn standing_claim(rig: &Rig) {
    let claim = rig.home.join("archive").join(format!(".publishing.{UUID}"));
    std::fs::create_dir_all(&claim).expect("the standing claim dir");
    std::fs::write(claim.join("partial"), b"another publisher").expect("the standing claim");
}

/// P12/P13 order: the push lands BEFORE the finish is attempted — proven
/// by a finish that refuses on a standing claim after the remote ref
/// already moved.
#[test]
fn endlocked_the_push_lands_before_a_refused_finish() {
    let (rig, wt, _proj) = managed("eorder");
    put(&wt, "work.txt", b"session work\n");
    standing_claim(&rig);
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    let branch = format!("refs/heads/ae/{}", rig.name);
    let remote = rig.home.join("remote.git");
    let pushed = git_in(&remote, &["rev-parse", &branch]);
    let head = git_in(&wt, &["rev-parse", "HEAD"]);
    assert_eq!(
        pushed.trim(),
        head.trim(),
        "the session commit reached the remote before the finish refused"
    );
    assert!(!rig.session_is_live(), "the session was stopped");
    assert!(exists(&rig.dir.join("meta")), "the state is kept");
    assert!(!exists(&rig.archive()), "no archive was published");
}

// ---- P12: the git branch -------------------------------------------------------

fn end_git(rig: &Rig) -> (Option<i32>, String, String) {
    end(rig)
}

fn stdout_has_line(out: &str, line: &str) {
    assert!(
        out.lines().any(|l| l == line),
        "missing line {line:?} in: {out}"
    );
}

/// P12: pending work is committed with the end's subject, then pushed to
/// the session branch — exact spoken lines, remote ref, archive facts,
/// and the lossless worktree removed.
#[test]
fn endlocked_pending_work_is_committed_then_pushed() {
    let (rig, wt, _proj) = managed("egpush");
    put(&wt, "a.txt", b"changed\n");
    put(&wt, "work.txt", b"session work\n");
    let (code, out, err) = end_git(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    stdout_has_line(&out, &format!("Committing changes in {}...", rig.name));
    stdout_has_line(&out, &format!("Pushing to origin/ae/{}...", rig.name));
    stdout_has_line(&out, "  2 file(s) changed");
    stdout_has_line(&out, &format!("Pushed to origin/ae/{}", rig.name));
    assert_eq!(
        out.lines().last(),
        Some(format!("Ended {}", rig.name)).as_deref()
    );
    let remote = rig.home.join("remote.git");
    let subject = git_in(
        &remote,
        &[
            "log",
            "-1",
            "--format=%s",
            &format!("refs/heads/ae/{}", rig.name),
        ],
    );
    assert_eq!(subject.trim(), format!("ae: end session {}", rig.name));
    assert_eq!(archive_row(&rig, "git_push_outcome"), "pushed");
    assert_eq!(
        archive_row(&rig, "git_push_ref"),
        format!("origin/ae/{}", rig.name)
    );
    assert_eq!(archive_row(&rig, "preserved_work_dir"), "-");
    assert!(!exists(&wt), "the lossless worktree was removed");
    assert!(!exists(&rig.dir), "the state was removed after archive");
}

/// P12/I11: a clean tree with an unpushed commit skips the commit entirely
/// — no Committing line, no stamp — and still pushes.
#[test]
fn endlocked_a_clean_unpushed_commit_pushes_without_committing() {
    let (rig, wt, _proj) = managed("egclean");
    put(&wt, "ready.txt", b"ready\n");
    git_in(&wt, &["add", "-A"]);
    git_in(&wt, &["commit", "-q", "-m", "ready"]);
    let (code, out, err) = end_git(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(!out.contains("Committing changes"), "{out}");
    stdout_has_line(&out, &format!("Pushing to origin/ae/{}...", rig.name));
    stdout_has_line(&out, "  1 file(s) changed");
    stdout_has_line(&out, &format!("Pushed to origin/ae/{}", rig.name));
    let remote = rig.home.join("remote.git");
    let subject = git_in(
        &remote,
        &[
            "log",
            "-1",
            "--format=%s",
            &format!("refs/heads/ae/{}", rig.name),
        ],
    );
    assert_eq!(subject.trim(), "ready", "no end commit was invented");
}

/// P12: HEAD already on a remote says so, records already-reachable with
/// the "-" push ref, and the provably lossless worktree is removed.
#[test]
fn endlocked_an_already_reachable_head_pushes_nothing() {
    let (rig, wt, proj) = managed("egreach");
    let branch = format!("ae/{}", rig.name);
    git_in(&proj, &["push", "-q", "origin", &branch]);
    let (code, out, err) = end_git(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    stdout_has_line(&out, "No new commits to push.");
    assert!(!out.contains("Pushing to origin"), "{out}");
    assert_eq!(archive_row(&rig, "git_push_outcome"), "already-reachable");
    assert_eq!(archive_row(&rig, "git_push_ref"), "-");
    assert_eq!(archive_row(&rig, "preserved_work_dir"), "-");
    assert!(!exists(&wt), "the worktree was removed");
}

/// P12: a push that fails leaves the session STOPPED, the commit safe in
/// the worktree, and prints the exact retry block (bug-shaped: the retry
/// path is unquoted — pinned as-is).
#[test]
fn endlocked_a_failed_push_keeps_the_commit_and_prints_the_retry() {
    let (rig, wt, proj) = managed("egpfail");
    git_in(
        &proj,
        &["remote", "set-url", "origin", "/nonexistent/ae-origin.git"],
    );
    put(&wt, "work.txt", b"session work\n");
    let (code, out, err) = end_git(&rig);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    let want = format!(
        "{}Error: push failed. The session is STOPPED; nothing was deleted.\n  The commit is safe locally — ae state and the working tree are preserved.\n  Working directory: {}\n  To retry the push: cd {} && git push -u origin HEAD:refs/heads/ae/{}\n  Then finish up:    ae end {}\n  Or pick the session back up: ae {}\n1 session(s) failed to end. See errors above.\n",
        migrate_note(&rig),
        wt.display(),
        wt.display(),
        rig.name,
        rig.name,
        rig.name
    );
    assert_eq!(err, want);
    assert!(!rig.session_is_live(), "the session was stopped first");
    assert!(exists(&rig.dir.join("meta")), "the state is kept");
    assert!(!exists(&rig.archive()), "no archive");
    let subject = git_in(&wt, &["log", "-1", "--format=%s", "HEAD"]);
    assert_eq!(subject.trim(), format!("ae: end session {}", rig.name));
}

/// P12: a commit that fails likewise stops first and deletes nothing,
/// with the exact five-line block; HEAD never moved.
#[test]
fn endlocked_a_failed_commit_stops_first_and_deletes_nothing() {
    use std::os::unix::fs::PermissionsExt as _;
    let (rig, wt, proj) = managed("egcfail");
    let hook = proj.join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").expect("a refusing hook");
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
        .expect("the hook is executable");
    let head_before = git_in(&wt, &["rev-parse", "HEAD"]);
    put(&wt, "work.txt", b"session work\n");
    let (code, out, err) = end_git(&rig);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    let want = format!(
        "{}Error: commit failed. The session is STOPPED; nothing was deleted.\n  Your work, ae state and agent conversation files are all preserved.\n  Working directory: {}\n  Fix the commit there, then re-run: ae end {}\n  Or pick the session back up: ae {}\n1 session(s) failed to end. See errors above.\n",
        migrate_note(&rig),
        wt.display(),
        rig.name,
        rig.name
    );
    assert_eq!(err, want);
    stdout_has_line(&out, &format!("Committing changes in {}...", rig.name));
    assert!(!rig.session_is_live(), "the session was stopped first");
    assert!(exists(&rig.dir.join("meta")), "the state is kept");
    assert_eq!(git_in(&wt, &["rev-parse", "HEAD"]), head_before);
}

/// P12: with no origin at all the committed worktree is preserved and the
/// archive records no-origin beside its path.
#[test]
fn endlocked_no_origin_preserves_the_committed_worktree() {
    let (rig, wt, proj) = managed("egnone");
    git_in(&proj, &["remote", "remove", "origin"]);
    put(&wt, "work.txt", b"session work\n");
    let (code, out, err) = end_git(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    stdout_has_line(&out, &format!("Committing changes in {}...", rig.name));
    stdout_has_line(
        &out,
        &format!(
            "Working directory preserved: {} (no origin remote to push to).",
            wt.display()
        ),
    );
    let tail: Vec<&str> = out.lines().rev().take(2).collect();
    assert_eq!(tail[0], format!("  Directory preserved: {}", wt.display()));
    assert_eq!(
        tail[1],
        format!("Ended {} — working directory preserved.", rig.name)
    );
    assert_eq!(archive_row(&rig, "git_push_outcome"), "no-origin");
    assert_eq!(
        archive_row(&rig, "preserved_work_dir"),
        wt.display().to_string()
    );
    assert!(exists(&wt.join("work.txt")), "the worktree is kept");
}

/// P12: ignored output the push would not carry keeps the worktree with
/// the nonlossless reason, after the reachable line.
#[test]
fn endlocked_ignored_output_keeps_the_worktree() {
    let (rig, wt, proj) = managed("egkeep");
    let branch = format!("ae/{}", rig.name);
    git_in(&proj, &["push", "-q", "origin", &branch]);
    put(&wt, "target/out.bin", b"ignored output");
    let (code, out, err) = end_git(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    stdout_has_line(&out, "No new commits to push.");
    stdout_has_line(
        &out,
        &format!(
            "  Working directory preserved: {} (untracked or ignored data remains, or its absence is unproven).",
            wt.display()
        ),
    );
    assert_eq!(archive_row(&rig, "git_push_outcome"), "already-reachable");
    assert_eq!(
        archive_row(&rig, "preserved_work_dir"),
        wt.display().to_string()
    );
    assert!(exists(&wt.join("target/out.bin")), "the output survives");
}

/// P12: legacy session state inside the worktree preserves it on the
/// legacy reason (the git proof is never run) and is named again when the
/// cleanup keeps it.
#[test]
fn endlocked_legacy_state_keeps_the_worktree() {
    let (rig, wt, proj) = managed("eglegacy");
    let branch = format!("ae/{}", rig.name);
    git_in(&proj, &["push", "-q", "origin", &branch]);
    put(&wt, &format!(".ae/{}/state", rig.name), b"legacy");
    let (code, out, err) = end_git(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    stdout_has_line(
        &out,
        &format!(
            "  Working directory preserved: {} (legacy session state or a non-directory/unreadable .ae path remains).",
            wt.display()
        ),
    );
    stdout_has_line(
        &out,
        &format!(
            "  Legacy session state preserved: {}",
            wt.join(".ae").join(&rig.name).display()
        ),
    );
    assert!(exists(&wt), "the worktree is kept");
}

/// P12 bug-shaped (pinned, not fixed): a workdir that is not a git repo
/// refuses BEFORE the stop, naming the FALLBACK worktree path when the
/// recorded dir is gone — and says "still running" of a session already
/// observed absent.
#[test]
fn endlocked_a_missing_workdir_refuses_before_any_stop() {
    let (rig, wt, _proj) = managed("egmiss");
    stop_outside(&rig);
    std::fs::remove_dir_all(&wt).expect("the managed worktree is gone too");
    set_row(
        &rig,
        "work_dir",
        Some(&rig.home.join("nowhere").display().to_string()),
    );
    let (code, out, err, rows) = end_traced(
        &rig,
        &["_end", "-f", &rig.name],
        &[
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ],
    );
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    let want = format!(
        "{}Error: working directory is not a git repo — cannot preserve work.\n  Nothing was stopped and nothing was deleted; the session is still running.\n  Working directory: {}\n1 session(s) failed to end. See errors above.\n",
        migrate_note(&rig),
        wt.display()
    );
    assert_eq!(err, want);
    assert_eq!(count(&rows, "kill-session", ""), 0, "{}", trace_dump(&rows));
    assert!(exists(&rig.dir.join("meta")), "the state is kept");
}

/// B1 bug-shaped (pinned, not fixed): an UNKNOWN mode falls through to
/// the git treatment — reachable line and all — while its cleanup takes
/// the local tail, so the worktree is left standing. The branch is chosen
/// by exclusion from local, never by naming git/full.
#[test]
fn endlocked_an_unknown_mode_falls_through_to_git() {
    let (rig, wt, proj) = managed("egmode");
    let branch = format!("ae/{}", rig.name);
    git_in(&proj, &["push", "-q", "origin", &branch]);
    set_row(&rig, "mode", Some("weird"));
    let (code, out, err) = end_git(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    stdout_has_line(&out, "No new commits to push.");
    stdout_has_line(&out, &format!("Cleaned up local session {}", rig.name));
    assert_eq!(
        out.lines().last(),
        Some(format!("Ended {}", rig.name)).as_deref()
    );
    assert_eq!(archive_row(&rig, "git_push_outcome"), "already-reachable");
    assert!(!exists(&rig.dir), "the state was removed by the tail");
    assert!(exists(&wt), "the worktree was left standing");
}

/// P10 bug-shaped (pinned, not fixed): an empty mode takes the LOCAL
/// branch in `end_locked`, but the teardown proves mode `local`
/// byte-exact and refuses the empty spelling — so the end stops and
/// archives, then fails, and can never converge.
#[test]
fn endlocked_an_empty_mode_is_local() {
    let rig = Rig::new("eemode");
    set_row(&rig, "mode", Some(""));
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    let want = format!(
        "{}teardown: '{}' does not prove mode 'local' (meta records '', compared byte-exact) — refusing (only local teardown is this entry's).\nError: the session state for '{}' was not fully removed — see the teardown diagnostic above; the session is NOT ended.\n1 session(s) failed to end. See errors above.\n",
        migrate_note(&rig),
        rig.name,
        rig.name
    );
    assert_eq!(err, want);
    assert!(!rig.session_is_live(), "the session was stopped");
    assert!(exists(&rig.archive().join("meta")), "the archive published");
    assert!(
        exists(&rig.dir.join("meta")),
        "the state could not be removed"
    );
    assert!(!out.contains("Ended "), "no success was reported: {out}");
}

/// P10: a local end never pays the git-repo precondition — a recorded
/// work dir that is simply gone does not matter.
#[test]
fn endlocked_a_local_end_ignores_a_missing_workdir() {
    let rig = Rig::new("enowd");
    set_row(
        &rig,
        "work_dir",
        Some(&rig.home.join("gone").display().to_string()),
    );
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert_eq!(
        out.lines().last(),
        Some(format!("Ended local session {}", rig.name)).as_deref()
    );
}

/// P10 purge: no archive is written and the exact no-archive line names
/// the id that never had one.
#[test]
fn endlocked_a_purged_local_end_writes_no_archive() {
    let rig = Rig::new("epurged");
    let (code, out, err) = rig.run(&["_end", "-f", "--purge-history", &rig.name]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    stdout_has_line(
        &out,
        &format!("No archive written (--purge-history); none existed for {UUID}."),
    );
    assert_eq!(
        out.lines().last(),
        Some(format!("Ended local session {}", rig.name)).as_deref()
    );
    assert!(!exists(&rig.archive()), "no archive");
    assert!(!exists(&rig.dir), "the state is gone");
}
