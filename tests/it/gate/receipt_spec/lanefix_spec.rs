//! Independent #343 acceptance, frozen before implementation.
//! Lead rulings: Phase 0 c1-c4, permissive foreign runs, worktree ownership,
//! addendum r1 binding cleanup, and name-scoped proof without a nonce.
//! Real recipes, registry and release census decide ownership. Cargo is an
//! external fixture. One date response is fixed to exercise equal owner lines.

use super::*;

const CARGO: &str = r#"#!/bin/sh
if [ "$1:${2-}" != nextest:run ]; then exit 0; fi
stamp=$(LC_ALL=C ps -o lstart= -p $$ | tr -cd A-Za-z0-9)
printf '%s %s\n' "$$" "$stamp" >"$AE_CLAIM_DIR/cargo.tmp"
mv "$AE_CLAIM_DIR/cargo.tmp" "$AE_CLAIM_DIR/cargo"
IFS= read -r finish <"$AE_CLAIM_DIR/finish"
exit 0
"#;

const DATE: &str = r#"#!/bin/sh
if [ "$*" = '-u +%FT%TZ' ]; then
    printf '%s\n' '2030-01-01T00:00:00Z'
    exit 0
fi
exec "$AE_CLAIM_REAL_DATE" "$@"
"#;

const DRIVER: &str = r#"set -u
mode=$1
dir=$AE_CLAIM_DIR
main=$AE_CLAIM_MAIN
linked=$AE_CLAIM_LINKED
locks=$main/.local/locks
top=
identity() { LC_ALL=C ps -o lstart= -p "$1" 2>/dev/null | tr -cd A-Za-z0-9; }
record() {
    local stamp
    stamp=$(identity "$1")
    [[ -z "$stamp" ]] || printf '%s %s\n' "$1" "$stamp" >>"$dir/owned"
}
matched() { [[ -n "$2" && "$(identity "$1")" == "$2" ]]; }
cleanup() {
    local pid stamp
    if [[ -n "$top" && -s "$dir/top" ]]; then
        read -r pid stamp <"$dir/top"
        if matched "$pid" "$stamp"; then kill -TERM "$pid" 2>/dev/null || :; fi
        for ((tick=0; tick<160; tick++)); do
            matched "$pid" "$stamp" || break
            sleep 0.05
        done
    fi
    if [[ -f "$dir/owned" ]]; then
        while read -r pid stamp; do
            if matched "$pid" "$stamp"; then kill -KILL "$pid" 2>/dev/null || :; fi
        done <"$dir/owned"
    fi
    wait 2>/dev/null || :
}
trap cleanup EXIT
trap 'exit 143' TERM HUP
trap 'exit 130' INT
j() { local wd=$1; shift; just -f "$main/justfile" -d "$wd" "$@"; }
claim() { j "$1" lane-lock claim "$2" 0 >>"$dir/claim.out" 2>&1; }
release() {
    release_status=0
    j "$1" lane-lock release "$2" >"$dir/release.out" 2>&1 || release_status=$?
    printf '%s\n' "$release_status" >"$dir/release-status"
}
mkfifo "$dir/finish" || exit 70
claim_name=holder
run_wd=$linked
recipe=_tmux-isolated
proof=unset
case "$mode" in
    free|free-then-claim|proof-free|hp-leg) ;;
    *) claim "$main" holder || exit 71 ;;
esac
if [[ -f "$locks/gate/owner" ]]; then cp "$locks/gate/owner" "$dir/initial-owner"; fi
case "$mode" in
    same|binding-missing|binding-stale|binding-link|same-alias|replace-same|replace-other) run_wd=$main ;;
esac
case "$mode" in
    same-alias) ln -s "$main" "$dir/alias" || exit 72; run_wd=$dir/alias ;;
    foreign-alias) ln -s "$linked" "$dir/alias" || exit 72; run_wd=$dir/alias ;;
    explicit-nested) proof=holder; recipe=rust-test ;;
    proof-free) proof=holder ;;
    proof-other) proof=other ;;
    proof-empty) proof= ;;
    proof-invalid) proof='bad/name' ;;
    malformed-holder)
        cp "$locks/gate/owner" "$dir/valid-owner" || exit 72
        cp "$locks/binding" "$dir/valid-binding" || exit 72
        printf '%s\n' 'bad;holder 2000-01-01T00:00:00Z' >"$locks/gate/owner"
        { cat "$locks/gate/owner"; tail -n 1 "$dir/valid-binding"; } >"$locks/binding" ;;
    binding-missing) rm "$locks/binding" 2>/dev/null || : ;;
    binding-stale) printf 'other 2000-01-01T00:00:00Z\n%s\n' "$linked" >"$locks/binding" ;;
    binding-link)
        cat "$locks/gate/owner" >"$dir/saved-binding"
        printf '%s\n' "$linked" >>"$dir/saved-binding"
        rm "$locks/binding" 2>/dev/null || :
        ln -s "$dir/saved-binding" "$locks/binding" || exit 72 ;;
    release-reclaim)
        release "$main" holder
        cp "$dir/release-status" "$dir/first-release-status"
        if [[ -e "$locks/binding" || -L "$locks/binding" ]]; then echo yes; else echo no; fi >"$dir/first-binding-left"
        claim "$linked" successor || exit 73
        claim_name=successor
        run_wd=$main ;;
esac
if [[ -f "$locks/binding" ]]; then cp "$locks/binding" "$dir/binding-before"; fi
if [[ "$mode" == hp-leg ]]; then
    mkdir -p "$main/.local/release" || exit 73
    head=$(git -C "$main" rev-parse HEAD) || exit 73
    tree=$(git -C "$main" rev-parse 'HEAD^{tree}') || exit 73
    git -C "$main" update-ref "refs/ae-release/$head" "$head" || exit 73
    (eval "$AE_CLAIM_HP_LEG"; hp_leg "$main" "$head" "$tree" holder claim-spec) >"$dir/run.out" 2>&1 &
elif [[ "$proof" == unset ]]; then
    if [[ "$recipe" == rust-test ]]; then j "$run_wd" rust-test >"$dir/run.out" 2>&1 &
    else j "$run_wd" _tmux-isolated test -E 'test(none)' >"$dir/run.out" 2>&1 & fi
else
    if [[ "$recipe" == rust-test ]]; then
        AE_LANE_CLAIM="$proof" j "$run_wd" rust-test >"$dir/run.out" 2>&1 &
    else AE_LANE_CLAIM="$proof" j "$run_wd" _tmux-isolated test -E 'test(none)' >"$dir/run.out" 2>&1 & fi
fi
top=$!
printf '%s %s\n' "$top" "$(identity "$top")" >"$dir/top"
cat "$dir/top" >>"$dir/owned"
for ((tick=0; tick<900; tick++)); do
    [[ -s "$dir/cargo" ]] && break
    kill -0 "$top" 2>/dev/null || break
    sleep 0.05
done
if [[ -s "$dir/cargo" ]]; then
    echo yes >"$dir/started"
    if [[ "$mode" == hp-leg ]]; then cp "$locks/gate/owner" "$dir/initial-owner"; fi
    read -r cargo cargo_stamp <"$dir/cargo"
    cat "$dir/cargo" >>"$dir/owned"
    # Record the real descendant tree for bounded teardown only. Ownership is
    # judged by the unchanged product census, not by this observation.
    LC_ALL=C ps -A -o pid= -o ppid= >"$dir/tree" || exit 74
    awk -v top="$top" '
        { parent[$1]=$2 }
        END { for (p in parent) {
            q=p
            for (n=0; n<128 && q>1; n++) {
                if (q==top) {print p; break}
                q=parent[q]
            }
        }}' "$dir/tree" >"$dir/tree-pids"
    while read -r pid; do record "$pid"; done <"$dir/tree-pids"
    : >"$dir/entries"
    for entry in "$locks"/groups/*; do
        [[ -f "$entry" ]] && cat "$entry" >>"$dir/entries"
    done
    case "$mode" in
        free-then-claim) claim "$main" holder || exit 75 ;;
        malformed-holder)
            cp "$dir/valid-owner" "$locks/gate/owner" || exit 75
            cp "$dir/valid-binding" "$locks/binding" || exit 75 ;;
        replace-other|replace-same)
            # Simulate the administrative recovery in the stated claim-loss
            # case without deleting the old lock or manufacturing an entry.
            mv "$locks/gate" "$dir/prior-gate" || exit 75
            if [[ "$mode" == replace-other ]]; then claim_name=successor; fi
            claim "$linked" "$claim_name" || exit 75
            cp "$locks/gate/owner" "$dir/replacement-owner" ;;
    esac
    if [[ "$mode" != free ]]; then release "$main" "$claim_name"; fi
    : >"$dir/entries-after-release"
    for entry in "$locks"/groups/*; do
        [[ -f "$entry" ]] && cat "$entry" >>"$dir/entries-after-release"
    done
    if matched "$cargo" "$cargo_stamp"; then echo yes; else echo no; fi >"$dir/alive-after-release"
    if [[ -f "$locks/gate/owner" ]]; then cp "$locks/gate/owner" "$dir/owner-after"; fi
    if [[ -f "$locks/gate/suspect" ]]; then cp "$locks/gate/suspect" "$dir/suspect"; fi
    if [[ -f "$locks/binding" ]]; then cp "$locks/binding" "$dir/binding-after"; fi
    if [[ -e "$locks/binding" || -L "$locks/binding" ]]; then echo yes; else echo no; fi >"$dir/binding-left"
    printf 'finish\n' >"$dir/finish"
else
    echo no >"$dir/started"
    : >"$dir/entries"
    for entry in "$locks"/groups/*; do
        [[ -f "$entry" ]] && cat "$entry" >>"$dir/entries"
    done
fi
run_status=0
wait "$top" || run_status=$?
top=
printf '%s\n' "$run_status" >"$dir/run-status"
: >"$dir/entries-after"
for entry in "$locks"/groups/*; do
    [[ -f "$entry" ]] && cat "$entry" >>"$dir/entries-after"
done
cat "$dir/run.out"
[[ ! -f "$dir/release.out" ]] || cat "$dir/release.out"
"#;

struct Case {
    fixture: ReceiptFixture,
    dir: PathBuf,
    evidence: String,
}

impl Case {
    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).unwrap_or_default()
    }

    fn held(&self) {
        assert_eq!(self.file("started").trim(), "yes", "{}", self.evidence);
        assert_eq!(self.file("release-status").trim(), "1", "{}", self.evidence);
        assert!(!self.file("owner-after").is_empty(), "{}", self.evidence);
        assert!(
            self.file("suspect").contains(": still running: "),
            "{}",
            self.evidence
        );
        assert_eq!(
            self.file("alive-after-release").trim(),
            "yes",
            "release only observes the live payload: {}",
            self.evidence
        );
    }

    fn released(&self) {
        assert_eq!(self.file("started").trim(), "yes", "{}", self.evidence);
        assert_eq!(self.file("release-status").trim(), "0", "{}", self.evidence);
        assert_eq!(self.file("owner-after"), "", "{}", self.evidence);
        assert_eq!(self.file("suspect"), "", "{}", self.evidence);
        assert_eq!(self.file("binding-left").trim(), "no", "{}", self.evidence);
        assert_eq!(
            self.file("alive-after-release").trim(),
            "yes",
            "release never signals a foreign payload: {}",
            self.evidence
        );
    }

    fn claim_lines(&self) -> Vec<String> {
        self.claim_lines_in("entries")
    }

    fn claim_lines_in(&self, name: &str) -> Vec<String> {
        self.file(name)
            .lines()
            .filter_map(|line| line.strip_prefix("claim ").map(str::to_owned))
            .collect()
    }
}

fn executable(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(name))
        .find(|file| file.is_file())
        .unwrap_or_else(|| panic!("acceptance needs {name}"))
}

fn case(mode: &str) -> Case {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // The recipe creates another tmux socket below this root. Long scenario
    // names would consume the nested socket's 103-byte macOS path budget.
    let fixture = ReceiptFixture::new(&format!("lf{id:02x}"), true);
    let dir = fixture.base.join("claim-evidence");
    std::fs::create_dir(&dir).expect("claim evidence directory");
    spec_install(&fixture.bin().join("cargo"), CARGO);
    if mode == "replace-same" {
        spec_install(&fixture.bin().join("date"), DATE);
    }
    let timeout = executable(if cfg!(target_os = "macos") {
        "gtimeout"
    } else {
        "timeout"
    });
    if mode == "hp-leg" {
        std::os::unix::fs::symlink(&timeout, fixture.bin().join("timeout"))
            .expect("the hp leg has its real timeout spelling on either host");
    }
    let hp_leg = super::super::justfile_variable(&read(&root().join("justfile")), "HP_LEG");
    let invocation = Invocation::new("nice")
        .arg("-n")
        .arg("10")
        .arg(timeout)
        .arg("-k")
        .arg("5")
        .arg("90")
        .arg("bash")
        .arg("-c")
        .arg(DRIVER)
        .arg("claim-observer")
        .arg(mode)
        .env_cleared()
        .env("AE_LANE_MIN_FREE_GIB", "0")
        .env("AE_LANE_MAX_TEMP_C", "0")
        .env(
            "PATH",
            format!(
                "{}:{}",
                fixture.bin().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("HOME", &*fixture.base)
        .env("TMPDIR", &*fixture.base)
        .env("AE_TEST_TMPDIR", &*fixture.base)
        .env("GIT_CONFIG_GLOBAL", fixture.base.join("gitconfig"))
        .env("AE_CLAIM_DIR", &dir)
        .env("AE_CLAIM_MAIN", &fixture.tree)
        .env("AE_CLAIM_LINKED", &fixture.linked)
        .env("AE_CLAIM_REAL_DATE", executable("date"))
        .env("AE_CLAIM_HP_LEG", hp_leg)
        .env("AE_GATE_REAP_TERM_SECS", "1")
        .env("AE_GATE_REAP_KILL_SECS", "1");
    let (out, err) = (dir.join("observer.out"), dir.join("observer.err"));
    let status =
        raw::run(&invocation, &fixture.tree, &out, &err).expect("the bounded claim observer runs");
    let evidence = format!(
        "{mode}:\n{}\n{}\n{}",
        read(&out),
        read(&err),
        read_optional(&dir.join("run.out"))
    );
    assert!(
        matches!(status.outcome(), ExitOutcome::Code(0)),
        "observer setup or timeout: {evidence}"
    );
    assert_eq!(
        read(&dir.join("run-status")).trim(),
        if mode.starts_with("proof-") { "1" } else { "0" },
        "{evidence}"
    );
    assert_eq!(
        read(&dir.join("entries-after")),
        "",
        "the completed run removes its own entry: {evidence}"
    );
    Case {
        fixture,
        dir,
        evidence,
    }
}

fn read_optional(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn an_unclaimed_linked_worktree_run_never_blocks_or_receives_a_signal_from_the_holder() {
    for mode in ["foreign", "foreign-alias"] {
        let case = case(mode);
        case.released();
        assert_eq!(case.claim_lines(), ["free"], "{}", case.evidence);
        let out = case.file("run.out");
        assert!(out.contains("detached: note:"), "{}", case.evidence);
        assert!(
            out.contains("export AE_LANE_CLAIM=holder"),
            "{}",
            case.evidence
        );
    }
}

#[test]
fn a_foreign_worktree_diagnostic_does_not_offer_an_export_for_a_malformed_holder() {
    let case = case("malformed-holder");
    case.released();
    assert_eq!(case.claim_lines(), ["free"], "{}", case.evidence);
    let out = case.file("run.out");
    assert!(out.contains("detached: note:"), "{}", case.evidence);
    assert!(!out.contains("export AE_LANE_CLAIM="), "{}", case.evidence);
}

#[test]
fn a_claimed_run_and_its_canonical_worktree_alias_keep_the_lock_while_alive() {
    for mode in ["same", "same-alias"] {
        let case = case(mode);
        case.held();
        assert_eq!(
            case.claim_lines(),
            [case.file("initial-owner").trim()],
            "{}",
            case.evidence
        );
        assert_eq!(
            case.file("binding-before"),
            case.file("binding-after"),
            "a refused release preserves the binding: {}",
            case.evidence
        );
        assert_eq!(case.file("binding-left").trim(), "yes", "{}", case.evidence);
    }
}

#[test]
fn an_explicit_cross_worktree_claim_survives_nested_just_and_keeps_the_lock() {
    let case = case("explicit-nested");
    case.held();
    assert_eq!(
        case.claim_lines(),
        [case.file("initial-owner").trim()],
        "{}",
        case.evidence
    );
}

#[test]
fn the_real_hp_leg_keeps_its_claim_when_it_runs_in_the_linked_release_worktree() {
    let case = case("hp-leg");
    case.held();
    assert_eq!(
        case.claim_lines(),
        [case.file("initial-owner").trim()],
        "the hp leg must carry the claim from its main checkout to its linked release worktree: {}",
        case.evidence
    );
}

#[test]
fn an_explicit_claim_that_is_free_different_empty_or_invalid_starts_nothing() {
    for mode in ["proof-free", "proof-other", "proof-empty", "proof-invalid"] {
        let case = case(mode);
        assert_eq!(case.file("started").trim(), "no", "{}", case.evidence);
        assert_eq!(case.file("entries"), "", "{}", case.evidence);
        assert!(
            case.file("run.out").contains("detached: refused:"),
            "{}",
            case.evidence
        );
    }
}

#[test]
fn a_binding_that_is_missing_stale_or_a_symlink_never_frees_a_claimed_run() {
    for mode in ["binding-missing", "binding-stale", "binding-link"] {
        let case = case(mode);
        case.held();
        assert_eq!(
            case.claim_lines(),
            [case.file("initial-owner").trim()],
            "{}",
            case.evidence
        );
    }
}

#[test]
fn a_free_run_stays_free_when_a_claim_starts_after_it() {
    let free = case("free");
    assert_eq!(free.file("started").trim(), "yes", "{}", free.evidence);
    assert_eq!(free.claim_lines(), ["free"], "{}", free.evidence);
    let later = case("free-then-claim");
    later.released();
    assert_eq!(later.claim_lines(), ["free"], "{}", later.evidence);
}

#[test]
fn a_new_claim_never_relabels_a_running_entry_or_signals_its_payload() {
    let case = case("replace-other");
    case.released();
    assert_eq!(
        case.claim_lines(),
        [case.file("initial-owner").trim()],
        "{}",
        case.evidence
    );
    assert_eq!(
        case.claim_lines_in("entries-after-release"),
        case.claim_lines(),
        "the old entry stays attributed to its initial claim after the replacement releases: {}",
        case.evidence
    );
    assert_ne!(case.file("initial-owner"), case.file("replacement-owner"));
}

#[test]
fn a_same_name_same_second_reclaim_conservatively_keeps_the_old_run_in_its_census() {
    let case = case("replace-same");
    case.held();
    assert_eq!(
        case.file("initial-owner"),
        case.file("replacement-owner"),
        "the fixture must exercise identical owner lines: {}",
        case.evidence
    );
    assert_eq!(
        case.claim_lines(),
        [case.file("initial-owner").trim()],
        "{}",
        case.evidence
    );
}

#[test]
fn a_successful_release_removes_its_binding_before_another_worktree_claims() {
    let case = case("release-reclaim");
    case.released();
    assert_eq!(
        case.file("first-release-status").trim(),
        "0",
        "{}",
        case.evidence
    );
    assert_eq!(
        case.file("first-binding-left").trim(),
        "no",
        "{}",
        case.evidence
    );
    assert_eq!(case.claim_lines(), ["free"], "{}", case.evidence);
    assert!(!case.fixture.tree.join(".local/locks/gate").exists());
}
