//! #340 P2 pin: a member set that the gate supervisor (the justfile's `REAP`)
//! cannot record ends the run red with a suspect line. A writable entry keeps a
//! fail-closed `lost` mark that the release census refuses even when every process
//! of the run is gone. When no mark can be written either, release stays refused
//! while the supervisor lives, and the supervisor ends only after its own census
//! proved the run empty.

use super::{justfile_variable, read, root};
use crate::parity::Invocation;
use crate::parity::capture::{ExitOutcome, raw};

/// Runs the real supervisor text with a groups directory that turns read-only
/// once the running entry exists, so the next member publish fails. `both`
/// makes the entry itself read-only too, so the lost mark cannot be written.
/// The member ignores TERM, so the cleanup needs its KILL step, and the release
/// census runs once while that member and its supervisor still live.
/// Until `go`, the stand-in cargo blocks in a builtin read on a FIFO, so it has
/// no child a census could catch before the directory turns read-only. It
/// ignores TERM, so the cleanup cannot end it before it records the member.
/// The driver kills only the pids it recorded, each after a start-identity match.
const DRIVER: &str = r#"set -u
dir=$1 arm=$2
mkdir "$dir/groups"
mkfifo "$dir/go"
echo owner-x >"$dir/owner"
identity() { LC_ALL=C ps -o lstart= -p "$1" 2>/dev/null | tr -cd A-Za-z0-9; }
cleanup() {
    local pid stamp
    chmod 755 "$dir/groups" 2>/dev/null
    chmod 644 "$dir"/groups/* 2>/dev/null
    if [[ -f "$dir/owned" ]]; then
        while read -r pid stamp; do
            [[ -n "$stamp" && "$(identity "$pid")" == "$stamp" ]] && kill -KILL "$pid" 2>/dev/null
        done <"$dir/owned"
    fi
    return 0
}
trap cleanup EXIT
AE_GATE_GROUPS="$dir/groups" AE_GATE_OWNER="$dir/owner" AE_GATE_REAP_TERM_SECS=1 AE_GATE_REAP_KILL_SECS=1 \
    perl -e "$AE_LOST_REAP" run bash -c '
        trap "" TERM
        read -r _ <"$0/go"
        bash -c "trap \"\" TERM; exec sleep 300" &
        echo "$! $(LC_ALL=C ps -o lstart= -p $! | tr -cd A-Za-z0-9)" >"$0/member.tmp"
        mv "$0/member.tmp" "$0/member"
        wait' "$dir" >"$dir/run.out" 2>&1 &
sup=$!
echo "$sup $(identity "$sup")" >>"$dir/owned"
for ((tick = 0; tick < 400; tick++)); do
    cat "$dir"/groups/* 2>/dev/null | grep -q '^leader ' && break
    sleep 0.05
done
chmod 555 "$dir/groups"
if [[ "$arm" == both ]]; then chmod 444 "$dir"/groups/*; fi
echo go >"$dir/go"
for ((tick = 0; tick < 400; tick++)); do
    [[ -s "$dir/member" ]] && break
    sleep 0.05
done
cat "$dir/member" >>"$dir/owned"
read -r member stamp <"$dir/member"
during=0
perl -e "$AE_LOST_REAP" check "$dir/groups" owner-x >"$dir/during.out" 2>&1 || during=$?
echo "$during" >"$dir/during-status"
lived=no
if [[ "$(identity "$member")" == "$stamp" ]] && kill -0 "$sup" 2>/dev/null; then lived=yes; fi
echo "$lived" >"$dir/during-lived"
status=0
wait "$sup" || status=$?
echo "$status" >"$dir/run-status"
chmod 755 "$dir/groups"
chmod 644 "$dir"/groups/* 2>/dev/null
: >"$dir/survivors"
while read -r pid stamp; do
    if [[ -n "$stamp" && "$(identity "$pid")" == "$stamp" ]]; then echo "$pid" >>"$dir/survivors"; fi
done <"$dir/owned"
check=0
perl -e "$AE_LOST_REAP" check "$dir/groups" owner-x >"$dir/check.out" 2>&1 || check=$?
echo "$check" >"$dir/check-status"
ls -A "$dir/groups" >"$dir/left"
"#;

struct Outcome {
    during_status: String,
    during_out: String,
    during_lived: String,
    run_status: String,
    run_out: String,
    survivors: String,
    check_status: String,
    check_out: String,
    left: String,
}

/// Runs one bounded bash driver in a fresh scratch case directory with the
/// justfile's `REAP` text in `AE_LOST_REAP` and the justfile's path in
/// `AE_LOST_JUSTFILE`. The scratch root stays alive with the returned handle.
fn drive(driver: &str, tag: &str, arm: &str) -> (crate::cli::OwnedScratch, std::path::PathBuf) {
    let justfile = root().join("justfile");
    let reap = justfile_variable(&read(&justfile), "REAP");
    assert!(
        !reap.is_empty(),
        "the justfile has the REAP supervisor text"
    );
    let base = crate::cli::OwnedScratch::root("gate", &format!("{tag}-{arm}"));
    let dir = base.join("case");
    std::fs::create_dir(&dir).unwrap_or_else(|why| panic!("the case dir: {why}"));
    let timeout = if cfg!(target_os = "macos") {
        "gtimeout"
    } else {
        "timeout"
    };
    let invocation = Invocation::new(timeout)
        .arg("-k")
        .arg("5")
        .arg("60")
        .arg("bash")
        .arg("-c")
        .arg(driver)
        .arg(tag)
        .arg(&dir)
        .arg(arm)
        .env("AE_LOST_REAP", &reap)
        .env("AE_LOST_JUSTFILE", &justfile)
        .env("AE_LANE_MIN_FREE_GIB", "0");
    let out = base.join("driver.out");
    let err = base.join("driver.err");
    let status = raw::run(&invocation, &base, &out, &err)
        .unwrap_or_else(|why| panic!("the bounded {tag} driver: {why}"));
    assert!(
        matches!(status.outcome(), ExitOutcome::Code(0)),
        "driver setup or timeout:\n{}\n{}",
        read(&out),
        read(&err)
    );
    (base, dir)
}

fn lost_case(arm: &str) -> Outcome {
    let (_base, dir) = drive(DRIVER, "reap-lost", arm);
    Outcome {
        during_status: read(&dir.join("during-status")).trim().to_owned(),
        during_out: read(&dir.join("during.out")),
        during_lived: read(&dir.join("during-lived")).trim().to_owned(),
        run_status: read(&dir.join("run-status")).trim().to_owned(),
        run_out: read(&dir.join("run.out")),
        survivors: read(&dir.join("survivors")),
        check_status: read(&dir.join("check-status")).trim().to_owned(),
        check_out: read(&dir.join("check.out")),
        left: read(&dir.join("left")),
    }
}

/// Both arms: release refuses while the member and its supervisor live, the
/// run ends red with the suspect line, and the cleanup reaps every recorded pid.
fn red_and_refused_while_alive(case: &Outcome) {
    assert_eq!(
        case.during_lived, "yes",
        "the census ran while the member and its supervisor lived"
    );
    assert_eq!(
        case.during_status, "1",
        "release refuses while a known member lives: {}",
        case.during_out
    );
    assert!(
        case.during_out.contains(": still running: "),
        "{}",
        case.during_out
    );
    assert_eq!(case.run_status, "1", "the run ends red: {}", case.run_out);
    assert!(
        case.run_out.contains("detached: suspect: ")
            && case.run_out.contains(": a member could not be recorded"),
        "{}",
        case.run_out
    );
    assert_eq!(case.survivors, "", "the cleanup still reaps the run");
}

#[test]
fn a_member_the_entry_cannot_record_leaves_a_lost_mark_that_release_refuses() {
    let case = lost_case("append");
    red_and_refused_while_alive(&case);
    assert!(
        !case.run_out.contains("the lost mark could not be written"),
        "{}",
        case.run_out
    );
    assert_eq!(
        case.check_status, "1",
        "a lost entry whose census is empty still refuses: {}",
        case.check_out
    );
    assert!(
        case.check_out
            .contains(": lost: its supervisor could not record a member"),
        "{}",
        case.check_out
    );
    assert_ne!(case.left.trim(), "", "the release keeps the lost entry");
}

#[test]
fn an_unwritable_lost_mark_keeps_release_refused_until_the_supervisor_proves_empty() {
    let case = lost_case("both");
    red_and_refused_while_alive(&case);
    assert!(
        case.run_out
            .contains(": a member could not be recorded (the lost mark could not be written)"),
        "{}",
        case.run_out
    );
    assert_eq!(
        case.check_status, "0",
        "after the supervisor proved the run empty, release may remove the stale entry: {}",
        case.check_out
    );
    assert_eq!(case.left.trim(), "", "the proved-empty stale entry is gone");
}

/// The real `just lane-lock release` in a scratch git repository whose claim
/// has a registry that exists but cannot be read (`unreadable`) or is a
/// symbolic link (`symlink`). Neither shape may count as an empty registry.
const RELEASE_DRIVER: &str = r#"set -u
dir=$1 arm=$2
git init -q "$dir/repo" || exit 3
cd "$dir/repo" || exit 3
cp "$AE_LOST_JUSTFILE" justfile || exit 3
locks=.local/locks
restore() { chmod 755 "$locks/groups" 2>/dev/null; return 0; }
trap restore EXIT
just lane-lock claim reg-spec >"$dir/claim.out" 2>&1 || { cat "$dir/claim.out"; exit 3; }
case "$arm" in
    unreadable)
        mkdir -p "$locks/groups"
        printf 'x\n' >"$locks/groups/1.Abc"
        chmod 000 "$locks/groups" ;;
    symlink)
        mkdir "$dir/elsewhere"
        printf 'x\n' >"$dir/elsewhere/1.Abc"
        ln -s "$dir/elsewhere" "$locks/groups" ;;
esac
status=0
just lane-lock release reg-spec >"$dir/release.out" 2>&1 || status=$?
restore
echo "$status" >"$dir/release-status"
if [[ -f "$locks/gate/owner" ]]; then echo kept >"$dir/owner-state"; else echo gone >"$dir/owner-state"; fi
cat "$locks/gate/suspect" >"$dir/suspect" 2>/dev/null || : >"$dir/suspect"
"#;

fn release_refuses(arm: &str) {
    let (_base, dir) = drive(RELEASE_DRIVER, "reg-fail", arm);
    let out = read(&dir.join("release.out"));
    assert_ne!(
        read(&dir.join("release-status")).trim(),
        "0",
        "release refuses a {arm} registry: {out}"
    );
    assert_eq!(
        read(&dir.join("owner-state")).trim(),
        "kept",
        "the owner line stays: {out}"
    );
    assert!(
        out.contains("a process this claim started may still run"),
        "{out}"
    );
    assert!(
        read(&dir.join("suspect")).starts_with("suspect: groups: "),
        "the suspect note names the registry: {}",
        read(&dir.join("suspect"))
    );
}

#[test]
fn release_refuses_a_registry_it_cannot_read() {
    release_refuses("unreadable");
}

#[test]
fn release_refuses_a_registry_that_is_a_symbolic_link() {
    release_refuses("symlink");
}

/// The supervisor with an owner file that exists but cannot be read
/// (`unreadable`) or holds no owner line (`empty`). Only an absent owner means
/// an unlocked run; these must stop before any entry or child exists.
const OWNER_DRIVER: &str = r#"set -u
dir=$1 arm=$2
mkdir "$dir/groups" "$dir/gate"
restore() { chmod 644 "$dir/gate/owner" 2>/dev/null; return 0; }
trap restore EXIT
case "$arm" in
    unreadable) echo owner-x >"$dir/gate/owner"; chmod 000 "$dir/gate/owner" ;;
    empty) : >"$dir/gate/owner" ;;
esac
status=0
AE_GATE_GROUPS="$dir/groups" AE_GATE_OWNER="$dir/gate/owner" \
    perl -e "$AE_LOST_REAP" run bash -c 'touch "$0/started"' "$dir" >"$dir/run.out" 2>&1 || status=$?
restore
echo "$status" >"$dir/run-status"
ls -A "$dir/groups" >"$dir/left"
if [[ -e "$dir/started" ]]; then echo yes >"$dir/started-state"; else echo no >"$dir/started-state"; fi
"#;

fn owner_aborts(arm: &str) {
    let (_base, dir) = drive(OWNER_DRIVER, "owner-fail", arm);
    let out = read(&dir.join("run.out"));
    assert_ne!(
        read(&dir.join("run-status")).trim(),
        "0",
        "an {arm} owner ends the run red: {out}"
    );
    assert!(out.contains("detached: ") && out.contains("owner"), "{out}");
    assert_eq!(
        read(&dir.join("started-state")).trim(),
        "no",
        "cargo never starts"
    );
    assert_eq!(read(&dir.join("left")), "", "no entry is left: {out}");
}

#[test]
fn an_owner_file_that_cannot_be_read_stops_the_run_before_cargo() {
    owner_aborts("unreadable");
}

#[test]
fn an_owner_file_without_an_owner_line_stops_the_run_before_cargo() {
    owner_aborts("empty");
}
