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
/// The driver kills only the pids it recorded, each after a start-identity match.
const DRIVER: &str = r#"set -u
dir=$1 arm=$2
mkdir "$dir/groups"
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
        while [[ ! -e "$0/go" ]]; do sleep 0.05; done
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
touch "$dir/go"
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

fn lost_case(arm: &str) -> Outcome {
    let reap = justfile_variable(&read(&root().join("justfile")), "REAP");
    assert!(
        !reap.is_empty(),
        "the justfile has the REAP supervisor text"
    );
    let base = crate::cli::OwnedScratch::root("gate", &format!("reap-lost-{arm}"));
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
        .arg(DRIVER)
        .arg("reap-lost")
        .arg(&dir)
        .arg(arm)
        .env("AE_LOST_REAP", &reap);
    let out = base.join("driver.out");
    let err = base.join("driver.err");
    let status = raw::run(&invocation, &base, &out, &err)
        .unwrap_or_else(|why| panic!("the bounded lost-member driver: {why}"));
    assert!(
        matches!(status.outcome(), ExitOutcome::Code(0)),
        "driver setup or timeout:\n{}\n{}",
        read(&out),
        read(&err)
    );
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
