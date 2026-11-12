//! Independent release-proof acceptance, lead ruling F2/F3-C/F4/F5.
//! SSH and cargo are external dependencies. The real justfile makes every decision.
use super::*;

struct ProofFixture {
    old: ReceiptFixture,
    env: Vec<(String, String)>,
    original_reply: String,
}

impl ProofFixture {
    fn new(tag: &str) -> Self {
        let old = ReceiptFixture::new(&format!("proof-{tag}"), false);
        let pin = read(&old.tree.join("justfile"))
            .lines()
            .find_map(|line| {
                line.strip_prefix("NEXTEST_VERSION := \"")
                    .and_then(|s| s.strip_suffix('"'))
            })
            .expect("the nextest exact pin")
            .to_owned();
        let real_git = std::fs::read_link(old.bin().join("git")).expect("real git link");
        let timeout_path = std::env::var_os("PATH").unwrap_or_default();
        let real_timeout = ["timeout", "gtimeout"]
            .into_iter()
            .find_map(|name| {
                std::env::split_paths(&timeout_path)
                    .map(|dir| dir.join(name))
                    .find(|path| path.is_file())
            })
            .expect("the real timeout used by the release");
        spec_install(
            &old.bin().join("timeout"),
            r#"#!/bin/sh
printf 'CALL' >>"$AE_PROOF_TIMEOUT_ARGS"
printf '\t%s' "$@" >>"$AE_PROOF_TIMEOUT_ARGS"
printf '\n' >>"$AE_PROOF_TIMEOUT_ARGS"
exec "$AE_PROOF_REAL_TIMEOUT" "$@"
"#,
        );
        std::fs::rename(old.bin().join("cargo"), old.bin().join("cargo-original"))
            .expect("keep the old cargo fixture");
        spec_install(
            &old.bin().join("cargo"),
            r#"#!/bin/sh
case "$1:$2" in
nextest:--version) printf 'cargo-nextest %s\n' "$AE_PROOF_NEXTEST"; exit 0 ;;
nextest:run)
    printf '%s\n' "$@" >>"$AE_PROOF_NEXT_ARGS"
    "$AE_PROOF_MUTATOR" mac || exit 71
    ;;
deny:*)
    "$AE_PROOF_MUTATOR" deny || exit 72
    if [ "$AE_PROOF_STOP_DENY" = 1 ]; then echo proof-deny-stop >&2; exit 73; fi
    ;;
esac
exec "$AE_PROOF_CARGO" "$@"
"#,
        );
        spec_install(
            &old.bin().join("git"),
            r#"#!/bin/sh
for arg in "$@"; do
    case "$arg" in
    status) if [ "$AE_PROOF_GIT_FAIL" = status ] || [ -f "$AE_PROOF_READ_ERROR" ]; then exit 74; fi ;;
    rev-parse) if [ "$AE_PROOF_GIT_FAIL" = head ] || [ -f "$AE_PROOF_READ_ERROR" ]; then exit 75; fi ;;
    esac
done
exec "$AE_PROOF_REAL_GIT" "$@"
"#,
        );
        spec_install(
            &old.bin().join("ssh"),
            r#"#!/bin/sh
printf '%s\n' "$@" >>"$AE_PROOF_SSH_ARGS"
"$AE_PROOF_MUTATOR" ssh || exit 76
if [ "$AE_PROOF_SSH_FAIL" = 1 ]; then echo unavailable >&2; exit 255; fi
cat "$AE_PROOF_REPLY"
"#,
        );
        let mutator = old.base.join("mutator");
        spec_install(
            &mutator,
            r#"#!/bin/sh
[ "$AE_PROOF_MUTATE_AT" = "$1" ] || exit 0
case "$AE_PROOF_MUTATE" in
dirty) printf '\n// changed during proof\n' >>"$AE_PROOF_TREE/src/lib.rs" ;;
untracked) echo dirty >"$AE_PROOF_TREE/untracked-proof-file" ;;
head) "$AE_PROOF_REAL_GIT" -C "$AE_PROOF_TREE" commit --allow-empty -qm 'concurrent head' ;;
tree)
    echo '// committed during proof' >>"$AE_PROOF_TREE/src/lib.rs"
    "$AE_PROOF_REAL_GIT" -C "$AE_PROOF_TREE" add src/lib.rs &&
    "$AE_PROOF_REAL_GIT" -C "$AE_PROOF_TREE" commit -qm 'concurrent tree'
    ;;
readerror) echo fail >"$AE_PROOF_READ_ERROR" ;;
*) exit 77 ;;
esac
"#,
        );
        // A minimal source tree and manifest make the real subset recipe runnable.
        spec_put(
            &old.tree.join("tests/it/main.rs"),
            "mod boundary; mod plain;\n",
        );
        spec_put(
            &old.tree.join("tests/it/boundary.rs"),
            "#[test] fn mac() { let _ = cfg!(target_os = \"macos\"); }\n",
        );
        spec_put(
            &old.tree.join("tests/it/plain.rs"),
            "#[test] fn ordinary() {}\n",
        );
        spec_put(
            &old.tree.join(".config/macos-subset"),
            "unit\nmodule boundary\n",
        );
        for args in [
            vec!["add", "-A"],
            vec!["commit", "-qm", "proof fixture"],
            vec!["push", "-q", "origin", "main"],
        ] {
            let (code, out, err) = old.git(&args);
            assert_eq!(code, 0, "{args:?}: {out}{err}");
        }
        let mut env = vec![
            ("AE_RELEASE_HP".into(), "spec@hp.invalid".into()),
            ("AE_RELEASE_HP_ROOT".into(), "/tmp/ae-proof".into()),
            ("AE_PROOF_NEXTEST".into(), pin),
            (
                "AE_PROOF_REAL_TIMEOUT".into(),
                real_timeout.to_string_lossy().into_owned(),
            ),
            (
                "AE_PROOF_REAL_GIT".into(),
                real_git.to_string_lossy().into_owned(),
            ),
        ];
        for (key, path) in [
            ("AE_PROOF_CARGO", old.bin().join("cargo-original")),
            ("AE_PROOF_MUTATOR", mutator),
            ("AE_PROOF_TREE", old.tree.clone()),
            ("AE_PROOF_REPLY", old.base.join("reply")),
            ("AE_PROOF_SSH_ARGS", old.base.join("ssh-args")),
            ("AE_PROOF_NEXT_ARGS", old.base.join("next-args")),
            ("AE_PROOF_READ_ERROR", old.base.join("read-error")),
            ("AE_PROOF_TIMEOUT_ARGS", old.base.join("timeout-args")),
        ] {
            env.push((key.into(), path.to_string_lossy().into_owned()));
        }
        let mut fixture = Self {
            old,
            env,
            original_reply: String::new(),
        };
        let refs = fixture.env_refs(&[]);
        green_gate(&fixture.old, &refs);
        let (_, tree) = fixture.old.tree_head();
        let receipt = fixture
            .old
            .receipt(&tree)
            .expect("a real writer produced a green receipt");
        spec_parse(receipt.as_bytes()).expect("the fixture writer emits the unchanged v1 grammar");
        fixture.original_reply = format!(
            "hp-spec\n{}",
            receipt.replace("host=spec-host", "host=hp-spec")
        );
        fixture.reply(&fixture.original_reply);
        fixture.clear_trace();
        fixture
    }

    fn env_refs<'a>(&'a self, extra: &'a [(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
        self.env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .chain(extra.iter().copied())
            .collect()
    }

    fn run(&self, recipe: &str, args: &[&str], extra: &[(&str, &str)]) -> (i32, String, String) {
        self.old
            .just_in(&self.old.tree, recipe, args, &self.env_refs(extra))
    }

    fn release(&self, extra: &[(&str, &str)]) -> (i32, String, String) {
        self.old.release(&self.env_refs(extra))
    }

    fn reply(&self, text: &str) {
        spec_put(&self.old.base.join("reply"), text);
    }

    fn trace(&self, name: &str) -> String {
        std::fs::read_to_string(self.old.base.join(name)).unwrap_or_default()
    }

    fn clear_trace(&self) {
        self.old.clear_record();
        for name in ["ssh-args", "next-args", "timeout-args"] {
            let _ = std::fs::remove_file(self.old.base.join(name));
        }
    }

    fn refuse(&self, extra: &[(&str, &str)], before_mac: bool) {
        self.clear_trace();
        let (code, out, err) = self.run("release-proof", &[], extra);
        assert_ne!(code, 0, "proof must refuse: {out}{err}");
        assert!(
            format!("{out}{err}").contains("release-proof: refused:"),
            "{out}{err}"
        );
        if before_mac {
            assert!(
                !self.old.recorded().contains(&"nextest".into()),
                "no Mac run: {:?}",
                self.old.recorded()
            );
        }
    }
}

fn replace_field(reply: &str, key: &str, value: &str) -> String {
    reply
        .lines()
        .map(|line| {
            if line.starts_with(&format!("{key}=")) {
                format!("{key}={value}\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect()
}

#[test]
fn a_fresh_hp_receipt_runs_a_filtered_mac_subset() {
    let fixture = ProofFixture::new("green");
    let (head, tree) = fixture.old.tree_head();
    let expect = format!("expect={head}:{tree}");
    let (code, out, err) = fixture.run("release-proof", &[&expect], &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        format!("{out}{err}").contains("release-proof:"),
        "{out}{err}"
    );
    let args = fixture.trace("next-args");
    assert!(
        args.lines().any(|line| line == "-E"),
        "the Mac run is filtered: {args}"
    );
    assert_eq!(
        fixture
            .old
            .recorded()
            .iter()
            .filter(|s| *s == "nextest")
            .count(),
        1
    );
    let ssh = fixture.trace("ssh-args");
    let ssh_args: Vec<_> = ssh.lines().collect();
    let destination = ssh_args
        .iter()
        .position(|arg| *arg == "spec@hp.invalid")
        .expect("destination argument");
    assert_eq!(
        ssh_args.get(destination.wrapping_sub(1)),
        Some(&"--"),
        "{ssh}"
    );
    assert!(
        fixture
            .trace("timeout-args")
            .lines()
            .any(|line| line.starts_with("CALL\t30\tssh\t")),
        "SSH must have the 30-second bound"
    );
    for required in ["BatchMode=yes", "ConnectTimeout=10", "spec@hp.invalid"] {
        assert!(ssh.contains(required), "{ssh}");
    }
    assert!(
        !ssh.contains("StrictHostKeyChecking=no") && !ssh.contains("UserKnownHostsFile=/dev/null"),
        "{ssh}"
    );
    let hp_root = "/tmp/ae-proof";
    assert!(
        ssh.contains(&format!("{hp_root}/.local/gate-receipts/{tree}")),
        "{ssh}"
    );
}

#[test]
fn every_cross_host_deciding_fact_is_checked_before_mac_execution() {
    let fixture = ProofFixture::new("facts");
    for (key, value) in [
        ("version", "2"),
        ("tree", &"a".repeat(40)),
        ("head", &"b".repeat(40)),
        ("lock_hash", &"c".repeat(64)),
        ("toolchain_pin", &"d".repeat(64)),
        ("rustc", "rustc 0.0.0 (wrong)"),
        ("nextest", "cargo-nextest 0.0.0"),
        ("host", "another-hp"),
        ("inventory_count", "not-a-number"),
    ] {
        fixture.reply(&replace_field(&fixture.original_reply, key, value));
        fixture.refuse(&[], true);
    }
    fixture.reply(
        &fixture
            .original_reply
            .replacen("hp-spec\n", "spec-host\n", 1)
            .replace("host=hp-spec", "host=spec-host"),
    );
    fixture.refuse(&[], true);
}

#[test]
fn cross_host_recorded_only_facts_do_not_invent_identity_requirements() {
    let fixture = ProofFixture::new("recorded");
    let mut reply = fixture.original_reply.clone();
    for (key, value) in [
        ("tmux", "tmux 3.4"),
        (
            "harness",
            "claude=absent;codex=absent;gemini=absent;agy=absent;grok=absent;muse=absent;opencode=absent",
        ),
        ("inventory", &"e".repeat(64)),
        ("inventory_count", "999"),
        ("scratch", "tmpfs"),
    ] {
        reply = replace_field(&reply, key, value);
    }
    fixture.reply(&reply);
    let (code, out, err) = fixture.run("release-proof", &[], &[]);
    assert_eq!(code, 0, "{out}{err}");
}

#[test]
fn age_future_skew_and_hostile_reply_bytes_fail_closed() {
    let fixture = ProofFixture::new("bytes");
    let stamp = fixture
        .original_reply
        .lines()
        .find_map(|l| l.strip_prefix("time="))
        .expect("stamp");
    let now = stamp_secs(stamp).expect("real timestamp");
    for reply in [
        String::new(),
        "hp-spec\n".into(),
        fixture
            .original_reply
            .replace("version=1\n", "version=1\nversion=1\n"),
        format!("{}unknown=1\n", fixture.original_reply),
        fixture.original_reply.replace("tree=", "bogus="),
        fixture.original_reply.replace('\n', "\r\n"),
        replace_field(
            &fixture.original_reply,
            "time",
            &secs_to_stamp(now - 90_000),
        ),
        replace_field(&fixture.original_reply, "time", &secs_to_stamp(now + 600)),
        format!("{}{}", fixture.original_reply, "x".repeat(8193)),
    ] {
        fixture.reply(&reply);
        fixture.refuse(&[], true);
    }
    // A prefix that looks good is still invalid when later bytes exist.
    let mut bytes = fixture.original_reply.as_bytes().to_vec();
    bytes.extend_from_slice(&[0xff, 0x00]);
    std::fs::write(fixture.old.base.join("reply"), bytes).expect("hostile bytes");
    fixture.refuse(&[], true);
    fixture.reply(&replace_field(
        &fixture.original_reply,
        "time",
        &secs_to_stamp(now + 120),
    ));
    let (code, out, err) = fixture.run("release-proof", &[], &[]);
    assert_eq!(code, 0, "allowed small skew: {out}{err}");
}

#[test]
fn missing_or_unsafe_hp_configuration_never_uses_a_local_receipt() {
    let fixture = ProofFixture::new("config");
    for extra in [
        vec![("AE_RELEASE_HP", "")],
        vec![("AE_RELEASE_HP", "-oProxyCommand=evil")],
        vec![("AE_RELEASE_HP", "user@hp;touch /tmp/no")],
        vec![("AE_RELEASE_HP_ROOT", "")],
        vec![("AE_RELEASE_HP_ROOT", "relative")],
        vec![("AE_RELEASE_HP_ROOT", "/tmp/../other")],
        vec![("AE_RELEASE_HP_ROOT", "/tmp/x; touch /tmp/no")],
    ] {
        fixture.refuse(&extra, true);
        assert!(
            fixture.trace("ssh-args").is_empty(),
            "unsafe config must refuse before ssh"
        );
    }
    fixture.refuse(&[("AE_PROOF_SSH_FAIL", "1")], true);
    // Even with a fresh local writer receipt, normal release refuses without hp.
    fixture.clear_trace();
    let (code, out, err) = fixture.release(&[("AE_RELEASE_HP", "")]);
    assert_ne!(code, 0, "{out}{err}");
    assert!(fixture.trace("next-args").is_empty(), "{out}{err}");
    assert!(read(&fixture.old.tree.join("Cargo.toml")).contains("version = \"0.0.0\""));
}

#[test]
fn an_unset_hp_destination_refuses_without_a_baked_default() {
    let fixture = ProofFixture::new("unset");
    let env: Vec<_> = fixture
        .env
        .iter()
        .filter(|(key, _)| key != "AE_RELEASE_HP")
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let (code, out, err) = fixture
        .old
        .just_in(&fixture.old.tree, "release-proof", &[], &env);
    assert_ne!(code, 0, "{out}{err}");
    assert!(
        format!("{out}{err}").contains("release-proof: refused:"),
        "{out}{err}"
    );
    assert!(
        fixture.trace("ssh-args").is_empty(),
        "no baked default destination"
    );
    assert!(fixture.trace("next-args").is_empty());
}

#[test]
fn a_dirty_or_unreadable_start_and_a_wrong_expected_identity_refuse() {
    let fixture = ProofFixture::new("start");
    fixture.refuse(&[("AE_PROOF_GIT_FAIL", "status")], true);
    fixture.refuse(&[("AE_PROOF_GIT_FAIL", "head")], true);
    let wrong = format!("expect={}:{}", "0".repeat(40), "1".repeat(40));
    let (code, out, err) = fixture.run("release-proof", &[&wrong], &[]);
    assert_ne!(code, 0, "{out}{err}");
    assert!(
        format!("{out}{err}").contains("release-proof: refused:"),
        "{out}{err}"
    );
    for staged in [false, true] {
        spec_put(&fixture.old.tree.join("src/lib.rs"), "// dirty\n");
        if staged {
            assert_eq!(fixture.old.git(&["add", "src/lib.rs"]).0, 0);
        }
        fixture.refuse(&[], true);
        // Restore only fixture bytes that this test owns.
        spec_put(&fixture.old.tree.join("src/lib.rs"), "");
        assert_eq!(fixture.old.git(&["add", "src/lib.rs"]).0, 0);
    }
    spec_put(&fixture.old.tree.join("untracked"), "dirty");
    fixture.refuse(&[], true);
}

fn proof_window(phase: &str, mutation: &str) {
    let fixture = ProofFixture::new(&format!("window-{phase}-{mutation}"));
    let before = fixture.old.tree_head();
    fixture.refuse(
        &[("AE_PROOF_MUTATE_AT", phase), ("AE_PROOF_MUTATE", mutation)],
        phase == "ssh",
    );
    match mutation {
        "head" => {
            let after = fixture.old.tree_head();
            assert_ne!(before.0, after.0, "the empty commit control must run");
            assert_eq!(
                before.1, after.1,
                "tree alone cannot detect an empty commit"
            );
        }
        "tree" => assert_ne!(before, fixture.old.tree_head()),
        "readerror" => assert!(fixture.old.base.join("read-error").is_file()),
        _ => assert!(
            !fixture
                .old
                .git(&["status", "--porcelain", "--untracked-files=all"])
                .1
                .is_empty()
        ),
    }
}

#[test]
fn ssh_window_refuses_dirty() {
    proof_window("ssh", "dirty");
}

#[test]
fn ssh_window_refuses_untracked() {
    proof_window("ssh", "untracked");
}

#[test]
fn ssh_window_refuses_head() {
    proof_window("ssh", "head");
}

#[test]
fn ssh_window_refuses_tree() {
    proof_window("ssh", "tree");
}

#[test]
fn ssh_window_refuses_readerror() {
    proof_window("ssh", "readerror");
}

#[test]
fn mac_window_refuses_dirty() {
    proof_window("mac", "dirty");
}

#[test]
fn mac_window_refuses_untracked() {
    proof_window("mac", "untracked");
}

#[test]
fn mac_window_refuses_head() {
    proof_window("mac", "head");
}

#[test]
fn mac_window_refuses_tree() {
    proof_window("mac", "tree");
}

#[test]
fn mac_window_refuses_readerror() {
    proof_window("mac", "readerror");
}

#[test]
fn a_red_mac_subset_refuses_and_green_then_red_hp_preserves_the_prior_proof() {
    let fixture = ProofFixture::new("red");
    fixture.refuse(&[("AE_SPEC_NEXT_EXIT", "19")], false);
    let (_, tree) = fixture.old.tree_head();
    let before = fixture.old.receipt(&tree).expect("prior fresh green");
    let (code, out, err) = fixture.run("test", &[], &[("AE_SPEC_NEXT_EXIT", "19")]);
    assert_ne!(code, 0, "the whole gate red control ran: {out}{err}");
    assert_eq!(
        fixture.old.receipt(&tree).as_deref(),
        Some(before.as_str()),
        "existing writer semantics preserve the previous green for this same tree"
    );
    let kept = fixture
        .old
        .receipt(&tree)
        .expect("the receipt preserved after red");
    fixture.reply(&format!(
        "hp-spec\n{}",
        kept.replace("host=spec-host", "host=hp-spec")
    ));
    fixture.clear_trace();
    let (code, out, err) = fixture.run("release-proof", &[], &[]);
    assert_eq!(
        code, 0,
        "a later red does not invalidate that green: {out}{err}"
    );
}

fn deny_window(mutation: &str) {
    let fixture = ProofFixture::new(&format!("deny-{mutation}"));
    let (code, out, err) = fixture.release(&[
        ("AE_PROOF_MUTATE_AT", "deny"),
        ("AE_PROOF_MUTATE", mutation),
    ]);
    assert_ne!(code, 0, "{mutation}: {out}{err}");
    assert!(
        fixture.old.recorded().contains(&"deny".into()),
        "the final window was exercised"
    );
    let reason = format!("{out}{err}");
    assert!(
        reason.contains("release-proof: refused:") && reason.contains("before bump"),
        "the identity recheck, before invoking bump, must refuse {mutation}: {reason}"
    );
    assert!(
        read(&fixture.old.tree.join("Cargo.toml")).contains("version = \"0.0.0\""),
        "no bump after {mutation}: {out}{err}"
    );
    assert!(
        fixture
            .old
            .git(&["tag", "--list", "v*"])
            .1
            .trim()
            .is_empty()
    );
}

#[test]
fn after_deny_refuses_dirty_before_bump() {
    deny_window("dirty");
}

#[test]
fn after_deny_refuses_untracked_before_bump() {
    deny_window("untracked");
}

#[test]
fn after_deny_refuses_head_before_bump() {
    deny_window("head");
}

#[test]
fn after_deny_refuses_tree_before_bump() {
    deny_window("tree");
}

#[test]
fn after_deny_refuses_readerror_before_bump() {
    deny_window("readerror");
}

#[test]
fn subset_print_mode_is_pure_and_uses_the_same_filter_as_the_guard() {
    let fixture = ProofFixture::new("print");
    let (code, out, err) = fixture.run("test-macos", &["print=yes"], &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        fixture.trace("next-args").is_empty(),
        "print mode must not run tests"
    );
    let expected = super::super::macos_subset::filterset(
        &super::super::macos_subset::parse_manifest("unit\nmodule boundary\n").expect("valid"),
    );
    assert!(
        out.contains(&expected),
        "recipe and pure renderer differ: {out} versus {expected}"
    );
    let every_kind = "# a comment\n\nunit # all unit tests\nmodule boundary # marker\n\
        test plain::ordinary # exact\n\
        helper tests/it/helper.rs::launch = plain::ordinary # representative\n\
        history boundary # 330\n";
    spec_put(
        &fixture.old.tree.join("tests/it/main.rs"),
        "mod boundary; mod plain; mod helper;\n",
    );
    spec_put(
        &fixture.old.tree.join("tests/it/helper.rs"),
        "pub fn launch() { Command::new(\"ae\"); }\n",
    );
    spec_put(
        &fixture.old.tree.join("tests/it/plain.rs"),
        "#[test] fn ordinary() { super::helper::launch(); }\n",
    );
    spec_put(&fixture.old.tree.join(".config/macos-subset"), every_kind);
    let (code, out, err) = fixture.run("test-macos", &["print=yes"], &[]);
    assert_eq!(code, 0, "every manifest kind: {out}{err}");
    let expected = super::super::macos_subset::filterset(
        &super::super::macos_subset::parse_manifest(every_kind).expect("all kinds plus comments"),
    );
    assert!(
        out.contains(&expected),
        "every-kind renderer mismatch: {out} versus {expected}"
    );
    assert!(
        fixture.trace("next-args").is_empty(),
        "print mode never runs tests"
    );
    for manifest in [
        "",
        "module boundary\n",
        "unit\nunit\n",
        "unit\nmodule x); all()\n",
    ] {
        spec_put(&fixture.old.tree.join(".config/macos-subset"), manifest);
        let (code, out, err) = fixture.run("test-macos", &[], &[]);
        assert_ne!(
            code, 0,
            "malformed subset refuses: {manifest:?}: {out}{err}"
        );
        assert!(fixture.trace("next-args").is_empty());
    }
}

pub(super) fn normal_release_uses_hp_then_mac() {
    let fixture = ProofFixture::new("release-normal");
    let (code, out, err) = fixture.release(&[("AE_PROOF_STOP_DENY", "1")]);
    assert_ne!(
        code, 0,
        "the fixture deliberately stops at deny: {out}{err}"
    );
    assert!(
        err.contains("proof-deny-stop"),
        "the release reached deny after the proof: {out}{err}"
    );
    assert!(!fixture.trace("ssh-args").is_empty());
    assert!(fixture.trace("next-args").lines().any(|line| line == "-E"));
    assert!(read(&fixture.old.tree.join("Cargo.toml")).contains("version = \"0.0.0\""));
}

pub(super) fn missing_hp_is_a_refusal() {
    let fixture = ProofFixture::new("release-nohp");
    let (code, out, err) = fixture.release(&[("AE_RELEASE_HP", "")]);
    assert_ne!(code, 0, "{out}{err}");
    assert!(
        format!("{out}{err}").contains("release-proof: refused:"),
        "{out}{err}"
    );
    assert!(
        fixture.trace("next-args").is_empty(),
        "no silent whole-Mac fallback"
    );
    assert!(read(&fixture.old.tree.join("Cargo.toml")).contains("version = \"0.0.0\""));
}

pub(super) fn non_version_bump_is_rejected() {
    if !(cfg!(target_os = "macos") && cfg!(target_arch = "aarch64")) {
        return;
    }
    let fixture = ProofFixture::new("bump");
    let head = fixture.old.tree_head().0;
    let (code, out, err) = fixture.release(&[("AE_SPEC_TOUCH", "1")]);
    assert_ne!(code, 0, "{out}{err}");
    assert!(
        err.contains("src/lib.rs"),
        "the offending path is named: {out}{err}"
    );
    assert!(
        fixture
            .old
            .git(&["tag", "--list", "v*"])
            .1
            .trim()
            .is_empty()
    );
    assert_eq!(fixture.old.tree_head().0, head, "no bump commit landed");
}

#[test]
fn a_grammar_valid_oversized_receipt_still_refuses() {
    let fixture = ProofFixture::new("valid-oversized");
    let reply = replace_field(
        &fixture.original_reply,
        "tmux",
        &format!("tmux {}", "x".repeat(9000)),
    );
    let (_, receipt) = reply.split_once('\n').expect("SSH hostname line");
    spec_parse(receipt.as_bytes()).expect("the size control must otherwise satisfy the v1 grammar");
    fixture.reply(&reply);
    fixture.refuse(&[], true);
}

#[test]
fn the_local_override_is_explicit_and_never_runs_a_duplicate_subset() {
    // A controlled fixture models a human's override. It cannot publish outside
    // its local fake remote. Agents do not set this override for real releases.
    let fixture = ProofFixture::new("local-override");
    let (code, out, err) = fixture.run("release-proof", &[], &[("AE_RELEASE_HP", "local")]);
    assert_eq!(code, 0, "fresh local receipt: {out}{err}");
    assert!(format!("{out}{err}").contains("DEVIATION"), "{out}{err}");
    assert!(fixture.trace("ssh-args").is_empty());
    assert!(
        fixture.trace("next-args").is_empty(),
        "fresh local receipt reuses nextest"
    );

    let (_, tree) = fixture.old.tree_head();
    let local = fixture.old.receipt(&tree).expect("fresh local receipt");
    let at = local
        .lines()
        .find_map(|line| line.strip_prefix("time="))
        .expect("time");
    let stale = replace_field(
        &local,
        "time",
        &secs_to_stamp(stamp_secs(at).expect("real time") - 90_000),
    );
    spec_put(&fixture.old.receipts_dir().join(&tree), &stale);
    fixture.clear_trace();
    let (code, out, err) = fixture.run("release-proof", &[], &[("AE_RELEASE_HP", "local")]);
    assert_eq!(
        code, 0,
        "stale local receipt runs the whole gate: {out}{err}"
    );
    assert!(format!("{out}{err}").contains("DEVIATION"), "{out}{err}");
    assert!(fixture.trace("ssh-args").is_empty());
    let args = fixture.trace("next-args");
    assert!(
        args.lines().any(|line| line == "run"),
        "the whole Mac run happened: {args}"
    );
    assert!(
        !args.lines().any(|line| line == "-E"),
        "a whole run has no subset filter: {args}"
    );
    assert_eq!(
        fixture
            .old
            .recorded()
            .iter()
            .filter(|step| *step == "nextest")
            .count(),
        1,
        "the whole run already covers the subset"
    );
}
