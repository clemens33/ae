#!/usr/bin/env python3
"""Emit the step-0 section of batch-c-artifacts/MANIFEST.md from run artifacts.
Byte facts only — no verdicts, no expected-vs-actual statements."""
import json, os, sys

DEST = "/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts/twd-precursor"
ARMS = [
    ("a1", "kill only the fake-agent child of the pane (pane shell returns to foreground)"),
    ("a2", "none after launch — fake agent alive, pane left static; shortened stale threshold and nudge cap"),
    ("a3", "two-phase pane content in ONE sandbox / ONE running watchdog: phase A prints a documented generic phrase; phase B prints nonmatching lines that displace it"),
]

def kv(path):
    d = {}
    order = []
    for line in open(path, encoding="utf-8", errors="replace"):
        line = line.rstrip("\n")
        if "=" in line:
            k, v = line.split("=", 1)
            if k in d:
                k2 = k
                i = 2
                while f"{k}#{i}" in d: i += 1
                k2 = f"{k}#{i}"
                d[k2] = v; order.append(k2)
            else:
                d[k] = v; order.append(k)
    return d, order

out = []
out.append("## Step 0 — T-WD producer precursor (fixture harvest for G2)\n")
out.append("Design executed: `docs/migration/evidence/twd-precursor.md` v2 (approved 4af7be0).")
out.append("Producer: the REAL generated watchdog of a real `ae` launch at the frozen commit")
out.append("(`<meta>/watchdog _run`). No hook patch, no clock shim; pacing rides the documented")
out.append("`AE_WATCHDOG_*` knobs, recorded per arm. Arms are named by MANIPULATION.\n")

for arm, manip in ARMS:
    rm = os.path.join(DEST, arm, "run-manifest.txt")
    if not os.path.exists(rm):
        out.append(f"### Arm `{arm}` — ARTIFACTS MISSING\n"); continue
    d, order = kv(rm)
    sm = os.path.join(DEST, "specimens", f"summary.{arm}.json")
    summ = json.load(open(sm)) if os.path.exists(sm) else {}
    out.append(f"### Arm `{arm}` — manipulation: {manip}\n")
    out.append("| field | value |")
    out.append("|---|---|")
    for k in ("frozen_sha", "frozen_ae_sha256", "producer", "interpreter_version",
              "interpreter_sha256", "tmux_version", "fake_agent_bin", "fake_agent_sha256",
              "fake_agent_src_sha256", "uname", "clock_shim", "hook_patch",
              "knob.AE_WATCHDOG_INTERVAL_SEC", "knob.AE_WATCHDOG_STALE_MIN",
              "knob.AE_WATCHDOG_MAX_NUDGES", "knob.AE_WATCHDOG_THROTTLE_ALERT_CYCLES",
              "knob.AE_WATCHDOG_TG_SUPERVISE_SEC", "knob.AE_SEND_DEFER_SEC",
              "env.TZ", "env.LANG", "env.SHELL",
              "launch_rc", "wd_pane", "agent_pane",
              "instrument_selfcheck_positive_rc", "instrument_selfcheck_negative_rc",
              "start_utc", "end_utc"):
        if k in d:
            out.append(f"| `{k}` | `{d[k]}` |")
    for k in order:
        if k.startswith(("phaseA_", "phaseB_", "pre_manipulation", "post_manipulation",
                         "manipulation_utc", "observation_window_cycles",
                         "producer_pane_view_cmd", "agent_stdin_log_", "final_aefake_pids")):
            out.append(f"| `{k}` | `{d[k]}` |")
    inc = [f"{k}={d[k]}" for k in order if "INCONCLUSIVE" in d[k] or k == "OUTCOME"]
    out.append(f"| `barriers_crossed` | `{sum(1 for k in order if k == 'barrier' or k.startswith('barrier#'))}` |")
    out.append(f"| `inconclusive_barriers` | `{len(inc)}` |")
    out.append("")
    if inc:
        out.append("INCONCLUSIVE records (bounded-window expiry, recorded not interpreted):\n")
        for i in inc:
            out.append(f"- `{i}`")
        out.append("")
    out.append("Artifact paths (all under `docs/migration/evidence/batch-c-artifacts/twd-precursor/`):\n")
    out.append(f"- `{arm}/run-manifest.txt` — knobs, hashes, barrier ledger")
    out.append(f"- `{arm}/events/events.<label>.jsonl` — events.jsonl bytes copied at each barrier")
    out.append(f"- `{arm}/watchdog/watchdog.<label>.log` — the producer's own log lines (which code path ran)")
    out.append(f"- `{arm}/panes/panes.<label>.txt` — pane snapshots, producer's own capture form")
    out.append(f"- `{arm}/tmux/tmux.<label>.txt` — server/session/window/pane/client snapshots")
    out.append(f"- `{arm}/fs-manifests/manifest.<label>.txt` — recursive AE_HOME manifest (type/mode/hash/symlink/path)")
    out.append(f"- `{arm}/stamps/stamp.<label>.txt` — barrier stamp (epoch, utc, pgrep, byte counts)")
    out.append(f"- `{arm}/meta.at-launch.txt`, `{arm}/meta.final.txt` — session meta bytes")
    out.append(f"- `{arm}/ae-launch.out`, `{arm}/ae-launch.err` — launch stdout/stderr")
    if os.path.exists(os.path.join(DEST, arm, "agent-stdin.log")):
        out.append(f"- `{arm}/agent-stdin.log` — the bytes the pane RECEIVED (fake agent's stdin, no echo)")
    for f in sorted(os.listdir(os.path.join(DEST, arm))):
        if f.startswith("producer-view."):
            out.append(f"- `{arm}/{f}` — positive capture of the producer's pane view at that point")
    out.append(f"- `{arm}/SHA256SUMS.txt` — hash of every file above")
    out.append("")
    if summ:
        out.append("Harvested event bytes:\n")
        out.append("| field | value |")
        out.append("|---|---|")
        for k in ("source_events_file", "source_events_sha256", "source_events_bytes",
                  "total_specimens", "alert_family_specimens"):
            out.append(f"| `{k}` | `{summ.get(k)}` |")
        out.append(f"| `all_actions` | `{summ.get('all_actions')}` |")
        out.append(f"| `alert_family_actions` | `{summ.get('alert_family_actions')}` |")
        out.append("")

out.append("### Per-specimen enumeration (every harvested event line, individually hashed)\n")
out.append("Machine-readable: `twd-precursor/specimens/specimens.<arm>.jsonl` (ALL lines) and")
out.append("`twd-precursor/specimens/alert-specimens.<arm>.jsonl` (the alert-family subset:")
out.append("`alert`, `alert-cleared`, `throttled`, `throttle-cleared`). Each record carries")
out.append("`arm`, `line_no`, `byte_offset`, `byte_len_with_nl`, `byte_len_no_nl`,")
out.append("`sha256_line_with_nl`, `sha256_line_no_nl`, the captured `action`/`actor`/`ref`/")
out.append("`summary`/`ts` byte values, `first_seen_capture_label`, and `raw_line_no_nl`.")
out.append("A field absent from the producer's bytes is recorded as the sentinel `\\u0000ABSENT`,")
out.append("distinct from an emitted empty string.\n")
out.append("Alert-family specimens, by hash:\n")
out.append("| arm | line | byte offset | len (+nl) | sha256 (line, no trailing newline) | action | actor | target/ref | first seen at |")
out.append("|---|---|---|---|---|---|---|---|---|")
total = 0
for arm, _ in ARMS:
    p = os.path.join(DEST, "specimens", f"alert-specimens.{arm}.jsonl")
    if not os.path.exists(p): continue
    for line in open(p, encoding="utf-8"):
        s = json.loads(line)
        total += 1
        ref = s.get("ref")
        out.append("| `{arm}` | {ln} | {off} | {l} | `{h}` | `{a}` | `{ac}` | `{r}` | `{f}` |".format(
            arm=s["arm"], ln=s["line_no"], off=s["byte_offset"], l=s["byte_len_with_nl"],
            h=s["sha256_line_no_nl"], a=s["action"], ac=s["actor"],
            r=(ref if ref not in (None, "\x00ABSENT") else "ABSENT"),
            f=s["first_seen_capture_label"]))
out.append("")
out.append(f"Alert-family specimen count across all arms: **{total}**.")
out.append("Set equality against this enumeration is provable from")
out.append("`twd-precursor/specimens/alert-specimens.*.jsonl` + `specimens/SHA256SUMS.txt`.\n")

sys.stdout.write("\n".join(out) + "\n")
