//! Real recipe orchestration with external command recorders.
//! Pure selection is judged separately in `affected_spec`; the selector recorder
//! supplies protocol decisions only to exercise the shell consumer and routing.

use super::*;

const FILTER: &str = "kind(lib) | kind(bin) | (binary_id(ae::it) & test(/^(doors|floor|gate)::/))";

struct AffectedFixture {
    old: ReceiptFixture,
}

impl AffectedFixture {
    fn new(tag: &str) -> Self {
        let old = ReceiptFixture::new(&format!("affected-{tag}"), false);
        spec_put(
            &old.tree.join("tests/it/main.rs"),
            "mod doors; mod floor; mod gate;\n",
        );
        for name in ["doors", "floor", "gate"] {
            spec_put(
                &old.tree.join(format!("tests/it/{name}.rs")),
                "#[test] fn guard() {}\n",
            );
        }
        for words in [
            vec!["add", "-A"],
            vec!["commit", "-qm", "fixture tree"],
            vec!["branch", "affected-base"],
        ] {
            let (code, out, err) = old.git(&words);
            assert_eq!(code, 0, "fixture git: {out}{err}");
        }
        spec_put(&old.tree.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n");
        for words in [
            vec!["add", "src/lib.rs"],
            vec!["commit", "-qm", "source change"],
        ] {
            let (code, out, err) = old.git(&words);
            assert_eq!(code, 0, "fixture git: {out}{err}");
        }
        let real_just = std::fs::read_link(old.bin().join("just")).expect("real just link");
        let real_git = std::fs::read_link(old.bin().join("git")).expect("real git link");
        let original_cargo =
            std::fs::read_to_string(old.bin().join("cargo")).expect("stock cargo recorder");
        spec_install(&old.bin().join("cargo-original"), &original_cargo);
        spec_put(&old.base.join("real-just"), &real_just.to_string_lossy());
        spec_put(&old.base.join("real-git"), &real_git.to_string_lossy());
        // spec_install unlinks first, so neither real executable symlink is written through.
        spec_install(
            &old.bin().join("cargo"),
            r#"#!/bin/sh
case "$1" in
fmt|clippy|check)
  if [ "${AE_P2_PREREQ_FAIL:-}" = "$1" ]; then echo "$1" >>"$AE_SPEC_REC"; exit 23; fi
  ;;
test)
  case " $* " in
  *' --doc '*) echo doctest >>"$AE_SPEC_REC"; exit "${AE_P2_DOCTEST_EXIT:-0}" ;;
  esac ;;
run)
  case " $* " in
  *' --example affected '*)
    echo selector >>"$AE_SPEC_REC"
    cat >"$HOME/selector-input"
    case "${AE_P2_SELECTION_ACTION:-}" in
    dirty) echo dirt >.affected-dirt ;;
    head) "$(cat "$HOME/real-git")" commit --allow-empty -qm moved ;;
    tree) echo '// changed' >>src/lib.rs; "$(cat "$HOME/real-git")" add src/lib.rs; "$(cat "$HOME/real-git")" commit -qm moved ;;
    esac
    cat "$HOME/selector-output"
    exit "${AE_P2_SELECTOR_EXIT:-0}"
    ;;
  esac ;;
esac
exec "$(dirname "$0")/cargo-original" "$@"
"#,
        );
        spec_install(
            &old.bin().join("just"),
            r#"#!/bin/sh
lane=no
for word in "$@"; do
  if [ "$word" = _tmux-isolated ]; then lane=yes; fi
done
if [ "$lane" = yes ]; then
  filtered=no
  for word in "$@"; do if [ "$word" = -E ]; then filtered=yes; fi; done
  if [ "$filtered" = yes ]; then echo lane-filtered >>"$AE_SPEC_REC"; else echo lane-whole >>"$AE_SPEC_REC"; fi
  printf '%s\n' "$@" >>"$HOME/lane-argv"
  case "${AE_P2_LANE_ACTION:-}" in
  dirty) echo dirt >.affected-dirt ;;
  head) "$(cat "$HOME/real-git")" commit --allow-empty -qm moved ;;
  tree) echo '// changed' >>src/lib.rs; "$(cat "$HOME/real-git")" add src/lib.rs; "$(cat "$HOME/real-git")" commit -qm moved ;;
  esac
  exit "${AE_P2_LANE_EXIT:-0}"
fi
for word in "$@"; do
  if [ "$word" = check ]; then
    echo shell-check >>"$AE_SPEC_REC"
    if [ "${AE_P2_CHECK_ACTION:-}" = dirty ]; then echo dirt >.affected-dirt; fi
    exit "${AE_P2_CHECK_EXIT:-0}"
  fi
done
exec "$(cat "$HOME/real-just")" "$@"
"#,
        );
        spec_install(
            &old.bin().join("git"),
            r#"#!/bin/sh
for word in "$@"; do
  if [ "${AE_P2_IDENTITY_FAIL:-}" = yes ] && [ "$word" = rev-parse ]; then exit 31; fi
  if [ "${AE_P2_PRODUCER_FAIL:-}" = yes ]; then
    case "$word" in
    show|cat-file|ls-files|diff) printf '%s\n' 'pub fn valid_prefix() {}'; exit 29 ;;
    esac
  fi
done
exec "$(cat "$HOME/real-git")" "$@"
"#,
        );
        let fixture = Self { old };
        fixture.protocol(false, false, false);
        fixture.old.clear_record();
        fixture
    }

    fn protocol(&self, whole: bool, doctests: bool, check: bool) {
        let (doc, shell) = (
            if doctests { "yes" } else { "no" },
            if check { "yes" } else { "no" },
        );
        let text = if whole {
            format!(
                "version 1\nverdict whole\nreason justfile justfile\ndoctests {doc}\ncheck {shell}\nend\n"
            )
        } else {
            format!(
                "version 1\nverdict subset\nowner doors\nowner floor\nowner gate\ndoctests {doc}\ncheck {shell}\ncounter 4 7\nfilterset {FILTER}\nend\n"
            )
        };
        spec_put(&self.old.base.join("selector-output"), &text);
    }

    fn run(&self, preview: bool, env: &[(&str, &str)]) -> (i32, String, String) {
        let mut args = vec!["base=affected-base"];
        if preview {
            args.push("print=yes");
        }
        self.old
            .just_in(&self.old.tree, "test-affected", &args, env)
    }

    fn count(&self, label: &str) -> usize {
        self.old
            .recorded()
            .iter()
            .filter(|s| s.as_str() == label)
            .count()
    }

    fn refused(&self, env: &[(&str, &str)]) {
        let (code, out, err) = self.run(false, env);
        assert_ne!(
            code, 0,
            "a moving or unreadable identity passed: {out}{err}"
        );
        assert!(
            format!("{out}{err}").contains("test-affected: refused:"),
            "wrong refusal: {out}{err}"
        );
        assert_eq!(
            self.count("lane-filtered") + self.count("lane-whole"),
            0,
            "tests ran after a pre-test refusal"
        );
    }

    fn no_receipt(&self) {
        let (_, tree) = self.old.tree_head();
        assert!(
            self.old.receipt(&tree).is_none(),
            "an affected gate is never a whole-suite receipt"
        );
    }

    fn producer_matches_the_fixture(&self, preview: bool) {
        use super::super::affected::{Mode, Status, decode};

        let bytes = std::fs::read(self.old.base.join("selector-input")).expect("producer stream");
        let input = decode(&bytes).unwrap_or_else(|r| {
            panic!(
                "the real producer wrote an invalid stream: {} {}",
                r.id, r.detail
            )
        });
        assert!(matches!(
            (&input.mode, preview),
            (Mode::Gate, false) | (Mode::Preview, true)
        ));
        let (head, _) = self.old.tree_head();
        assert_eq!(input.head, head);
        let (code, merge_base, err) = self.old.git(&["rev-parse", "affected-base"]);
        assert_eq!(code, 0, "base identity: {err}");
        assert_eq!(input.merge_base, merge_base.trim());
        assert_eq!(input.base, "affected-base");
        assert_eq!(input.changes.len(), if preview { 2 } else { 1 });
        assert!(
            input
                .changes
                .iter()
                .any(|c| c.path == "src/lib.rs" && matches!(c.status, Status::Modified))
        );
        assert_eq!(input.new["src/lib.rs"], "pub fn value() -> u8 { 1 }\n");
        assert_eq!(
            input.old["src/lib.rs"], "",
            "old content comes from the base, never from the new tree"
        );
        for path in [
            "src/lib.rs",
            "tests/it/main.rs",
            "tests/it/doors.rs",
            "tests/it/floor.rs",
            "tests/it/gate.rs",
        ] {
            assert!(input.new.contains_key(path), "the producer omitted {path}");
            assert!(
                input.tracked.contains(path),
                "the producer omitted tracked {path}"
            );
        }
        if preview {
            assert!(
                input
                    .changes
                    .iter()
                    .any(|c| c.path == "untracked-file" && matches!(c.status, Status::Untracked))
            );
            assert!(input.tracked.contains("untracked-file"));
        }
    }
}

#[test]
fn preview_allows_dirt_but_runs_only_the_selector() {
    let f = AffectedFixture::new("preview");
    spec_put(&f.old.tree.join("untracked-file"), "preview dirt\n");
    let (code, out, err) = f.run(true, &[]);
    assert_eq!(code, 0, "preview: {out}{err}");
    assert_eq!(f.count("selector"), 1);
    for label in [
        "fmt",
        "clippy",
        "check",
        "lane-filtered",
        "lane-whole",
        "doctest",
        "shell-check",
    ] {
        assert_eq!(f.count(label), 0, "preview ran {label}: {out}{err}");
    }
    assert!(
        out.contains(&format!("filterset {FILTER}")),
        "preview did not print its selection: {out}{err}"
    );
    f.producer_matches_the_fixture(true);
    f.no_receipt();
}

#[test]
fn gate_refuses_tracked_or_untracked_dirt_before_any_check() {
    for tracked in [false, true] {
        let f = AffectedFixture::new(if tracked {
            "tracked-dirt"
        } else {
            "untracked-dirt"
        });
        let path = if tracked { "src/lib.rs" } else { "new-file" };
        spec_put(&f.old.tree.join(path), "dirty\n");
        f.refused(&[]);
        assert!(
            f.old.recorded().is_empty(),
            "dirty startup ran checks or selection"
        );
    }
}

#[test]
fn unreadable_identity_refuses_before_checks_instead_of_falling_back_to_whole() {
    let f = AffectedFixture::new("identity-error");
    f.refused(&[("AE_P2_IDENTITY_FAIL", "yes")]);
    assert!(f.old.recorded().is_empty());
}

#[test]
fn clean_subset_runs_all_three_checks_and_forwards_one_exact_filter() {
    let f = AffectedFixture::new("subset");
    let (code, out, err) = f.run(false, &[]);
    assert_eq!(code, 0, "subset: {out}{err}");
    let recorded = f.old.recorded();
    for label in ["fmt", "clippy", "check", "selector", "lane-filtered"] {
        assert_eq!(
            f.count(label),
            1,
            "missing or duplicate {label}: {recorded:?}; {out}{err}"
        );
    }
    for label in ["lane-whole", "doctest", "shell-check"] {
        assert_eq!(f.count(label), 0);
    }
    let selected = recorded
        .iter()
        .position(|x| x == "selector")
        .expect("selector");
    for label in ["fmt", "clippy", "check"] {
        assert!(recorded.iter().position(|x| x == label).expect("check") < selected);
    }
    let argv = std::fs::read_to_string(f.old.base.join("lane-argv")).expect("lane argv");
    let words: Vec<_> = argv.lines().collect();
    assert!(
        words.windows(2).any(|a| a == ["-E", FILTER]),
        "filter was split or rewritten: {words:?}"
    );
    assert!(
        format!("{out}{err}")
            .contains("test-affected: not selected: 4 of 7 integration tests (M1 gap)")
    );
    f.producer_matches_the_fixture(false);
    f.no_receipt();
}

#[test]
fn source_subset_runs_doctests_once_after_the_filtered_lane() {
    let f = AffectedFixture::new("doc");
    f.protocol(false, true, false);
    let (code, out, err) = f.run(false, &[]);
    assert_eq!(code, 0, "doctests: {out}{err}");
    assert_eq!(f.count("doctest"), 1);
    let labels = f.old.recorded();
    assert!(
        labels
            .iter()
            .position(|x| x == "lane-filtered")
            .expect("lane")
            < labels.iter().position(|x| x == "doctest").expect("doctest")
    );
    assert_eq!(f.count("lane-whole"), 0);
    f.no_receipt();
}

#[test]
fn a_whole_verdict_runs_the_bare_lane_and_does_not_duplicate_its_doctests() {
    let f = AffectedFixture::new("whole");
    f.protocol(true, true, false);
    let (code, out, err) = f.run(false, &[]);
    assert_eq!(code, 0, "whole: {out}{err}");
    assert_eq!(f.count("lane-whole"), 1);
    assert_eq!(f.count("lane-filtered"), 0);
    assert_eq!(f.count("doctest"), 0, "the bare lane already owns doctests");
    let argv = std::fs::read_to_string(f.old.base.join("lane-argv")).expect("bare lane argv");
    assert!(!argv.lines().any(|arg| arg == "-E"));
    assert!(format!("{out}{err}").contains("test-affected: whole: justfile:"));
    f.no_receipt();
}

#[test]
fn the_check_flag_runs_the_install_checks() {
    for whole in [false, true] {
        let f = AffectedFixture::new("shell-check");
        f.protocol(whole, false, true);
        let (code, out, err) = f.run(false, &[]);
        assert_eq!(code, 0, "shell checks: {out}{err}");
        assert_eq!(f.count("shell-check"), 1);
        assert_eq!(
            f.count(if whole { "lane-whole" } else { "lane-filtered" }),
            1
        );
        f.no_receipt();
    }
}

#[test]
fn head_tree_or_dirt_movement_during_selection_refuses_before_tests() {
    for action in ["head", "tree", "dirty"] {
        let f = AffectedFixture::new(&format!("selection-{action}"));
        f.refused(&[("AE_P2_SELECTION_ACTION", action)]);
        assert_eq!(f.count("selector"), 1);
    }
}

#[test]
fn head_tree_or_dirt_movement_during_a_green_lane_is_still_a_red_gate() {
    for action in ["head", "tree", "dirty"] {
        let f = AffectedFixture::new("post-lane");
        let (code, out, err) = f.run(false, &[("AE_P2_LANE_ACTION", action)]);
        assert_ne!(code, 0, "post-lane {action} was accepted: {out}{err}");
        assert!(format!("{out}{err}").contains("test-affected: refused:"));
        assert_eq!(f.count("lane-filtered"), 1);
        f.no_receipt();
    }
}

#[test]
fn identity_is_checked_after_the_last_doctest_and_shell_check_step() {
    let f = AffectedFixture::new("last-step");
    f.protocol(false, true, true);
    let (code, out, err) = f.run(false, &[("AE_P2_CHECK_ACTION", "dirty")]);
    assert_ne!(code, 0, "last-step dirt was accepted: {out}{err}");
    assert!(format!("{out}{err}").contains("test-affected: refused:"));
    for label in ["lane-filtered", "doctest", "shell-check"] {
        assert_eq!(f.count(label), 1);
    }
    f.no_receipt();
}

#[test]
fn producer_error_after_partial_output_skips_selection_and_runs_whole() {
    let f = AffectedFixture::new("producer");
    let (code, out, err) = f.run(false, &[("AE_P2_PRODUCER_FAIL", "yes")]);
    assert_eq!(code, 0, "whole fallback: {out}{err}");
    assert_eq!(
        f.count("selector"),
        0,
        "a failed producer reached selection"
    );
    assert_eq!(f.count("lane-whole"), 1);
    assert_eq!(f.count("lane-filtered"), 0);
    assert!(format!("{out}{err}").contains("test-affected: whole: producer:"));
    f.no_receipt();
}

#[test]
fn selector_failure_runs_whole_even_when_its_stdout_claims_subset() {
    let f = AffectedFixture::new("selector-exit");
    let (code, out, err) = f.run(false, &[("AE_P2_SELECTOR_EXIT", "37")]);
    assert_eq!(code, 0, "whole fallback: {out}{err}");
    assert_eq!(f.count("selector"), 1);
    assert_eq!(f.count("lane-whole"), 1);
    assert_eq!(f.count("lane-filtered"), 0);
    assert!(format!("{out}{err}").contains("test-affected: whole: selector-exit:"));
}

#[test]
fn malformed_or_incomplete_selector_output_runs_the_whole_lane() {
    for output in [
        "version 1\nverdict subset\nend\n",
        "version 1\nverdict subset\ndoctests no\ncheck no\ncounter 4 7\nfilterset kind(lib)\n",
        "version 1\nverdict whole\ndoctests no\ncheck no\nend\n",
    ] {
        let f = AffectedFixture::new("selector-output");
        spec_put(&f.old.base.join("selector-output"), output);
        let (code, out, err) = f.run(false, &[]);
        assert_eq!(code, 0, "whole fallback: {out}{err}");
        assert_eq!(f.count("lane-whole"), 1);
        assert_eq!(f.count("lane-filtered"), 0);
        assert!(format!("{out}{err}").contains("test-affected: whole: selector-output:"));
    }
}

#[test]
fn a_failed_test_lane_remains_red() {
    for whole in [false, true] {
        let f = AffectedFixture::new("lane-red");
        f.protocol(whole, false, false);
        let (code, out, err) = f.run(false, &[("AE_P2_LANE_EXIT", "19")]);
        assert_ne!(code, 0, "a failed lane became green: {out}{err}");
        assert_eq!(
            f.count(if whole { "lane-whole" } else { "lane-filtered" }),
            1
        );
        f.no_receipt();
    }
}

#[test]
fn failed_doctests_remain_red() {
    let f = AffectedFixture::new("doctest-red");
    f.protocol(false, true, false);
    let (code, out, err) = f.run(false, &[("AE_P2_DOCTEST_EXIT", "17")]);
    assert_ne!(code, 0, "failed doctests became green: {out}{err}");
    assert_eq!(f.count("lane-filtered"), 1);
    assert_eq!(f.count("doctest"), 1);
    f.no_receipt();
}

#[test]
fn failed_shell_checks_remain_red_under_both_verdicts() {
    for whole in [false, true] {
        let f = AffectedFixture::new("check-red");
        f.protocol(whole, false, true);
        let (code, out, err) = f.run(false, &[("AE_P2_CHECK_EXIT", "13")]);
        assert_ne!(code, 0, "a failed shell check became green: {out}{err}");
        assert_eq!(f.count("shell-check"), 1);
        f.no_receipt();
    }
}

#[test]
fn failed_prerequisite_checks_stop_before_selection_or_tests() {
    for step in ["fmt", "clippy", "check"] {
        let f = AffectedFixture::new("prerequisite-red");
        let (code, out, err) = f.run(false, &[("AE_P2_PREREQ_FAIL", step)]);
        assert_ne!(code, 0, "failed {step} became green: {out}{err}");
        assert_eq!(f.count(step), 1);
        for label in ["selector", "lane-filtered", "lane-whole"] {
            assert_eq!(f.count(label), 0);
        }
        f.no_receipt();
    }
}

#[test]
fn incomplete_filtersets_fall_back_to_the_whole_lane() {
    let positive = AffectedFixture::new("review-valid-filter");
    let (code, out, err) = positive.run(false, &[]);
    assert_eq!(code, 0, "positive: {out}{err}");
    assert_eq!(positive.count("lane-filtered"), 1);
    for filter in [
        "test(/^consumer::/)",
        "kind(lib) | kind(bin) | (binary_id(ae::it) & test(/^(doors|floor)::/))",
        "(binary_id(ae::it) & test(/^(doors|floor|gate)::/))",
        "kind(bin) | (binary_id(ae::it) & test(/^(doors|floor|gate)::/))",
        "kind(lib) | (binary_id(ae::it) & test(/^(doors|floor|gate)::/))",
    ] {
        let f = AffectedFixture::new("review-incomplete-filter");
        let old = std::fs::read_to_string(f.old.base.join("selector-output")).expect("protocol");
        spec_put(
            &f.old.base.join("selector-output"),
            &old.replace(FILTER, filter),
        );
        let (code, out, err) = f.run(false, &[]);
        assert_eq!(code, 0, "whole fallback: {out}{err}");
        assert_eq!(
            f.count("lane-whole"),
            1,
            "incomplete filter must not gate: {out}{err}"
        );
        assert_eq!(f.count("lane-filtered"), 0);
        assert!(format!("{out}{err}").contains("test-affected: whole: selector-output:"));
        f.no_receipt();
    }
}

#[test]
fn an_install_edit_keeps_shell_checks_when_selection_fails() {
    let positive = AffectedFixture::new("review-check-positive");
    positive.protocol(true, false, true);
    let (code, out, err) = positive.run(false, &[]);
    assert_eq!(code, 0, "positive: {out}{err}");
    assert_eq!(positive.count("shell-check"), 1);
    for failure in ["AE_P2_PRODUCER_FAIL", "AE_P2_SELECTOR_EXIT"] {
        let f = AffectedFixture::new("review-check-fallback");
        spec_put(
            &f.old.tree.join("install"),
            "#!/bin/sh\nprintf '%s\\n' changed\n",
        );
        for words in [
            vec!["add", "install"],
            vec!["commit", "-qm", "install change"],
        ] {
            let (code, out, err) = f.old.git(&words);
            assert_eq!(code, 0, "fixture git: {out}{err}");
        }
        f.protocol(true, false, false);
        let value = if failure == "AE_P2_PRODUCER_FAIL" {
            "yes"
        } else {
            "37"
        };
        let (code, out, err) = f.run(false, &[(failure, value)]);
        assert_eq!(code, 0, "whole fallback: {out}{err}");
        assert_eq!(f.count("lane-whole"), 1);
        assert_eq!(
            f.count("shell-check"),
            1,
            "a changed install still needs just check: {out}{err}"
        );
        f.no_receipt();
    }
}
