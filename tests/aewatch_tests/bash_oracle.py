"""Bash side of the phase-2 dual-run oracle.

run_bash_watchdog_fixture() drives the REAL generated watchdog helper (from the
current `ae`, via `ae doctor --refresh`) under the fakebin tmux/date/sleep/ae
shims, and returns the ordered EFFECT_KINDS record stream for one fixture. No real
tmux, no real ~/.ae, no `ae` edits.

Design (see .local/plan-aewatch-p2-impl.md, codex-reviewed):
- Helpers are generated under fake tmux with recording OFF, so `doctor --refresh`
  setup writes (status-left, the watchdog-cycle gate) are not counted; the fake
  tmux lists the session but NO _watchdog pane, so the refresh restart gate skips.
- `_lib` is wrapped so ae_emit_event also records an event.append effect from the
  JSON line it actually appends.
- `ae_path` is overridden to the fake ae so telegram/recover boundaries never hit
  real tmux or the network.
- The loop is stopped WITHOUT editing `ae`: fake sleep advances a tick only on the
  sentinel interval and flips `control` after the last tick, so the watchdog's
  `has-session` loop breaks. Effects before the final boundary are the cycle
  effects; the EXIT-trap unsets after it are harness shutdown and are dropped.
"""

import json
import os
import re
import subprocess
from pathlib import Path

_REPO_ROOT = Path(__file__).resolve().parents[2]
_AE = _REPO_ROOT / "ae"
_FAKEBIN = Path(__file__).resolve().parent / "fakebin"
_SLEEP_SENTINEL = "424242"  # forced AE_WATCHDOG_INTERVAL_SEC; only this sleep advances a tick

# The watchdog tunables this oracle forces on the bash side (run_env below). The
# Python side (run_python_watchdog_fixture) must build WatchdogConfig from the SAME
# values so config-derived text — e.g. the throttle alert's 'throttled for
# {THROTTLE_ALERT_CYCLES * INTERVAL}s' — stays byte-exact WITHOUT normalization
# (codex: normalizing would hide a hardcoded/wrong figure).
_ORACLE_INTERVAL = int(_SLEEP_SENTINEL)
_ORACLE_STALE_MIN = 15
_ORACLE_MAX_NUDGES = 2


def _write_meta(meta_dir, meta):
    (meta_dir / "meta").write_text("".join(f"{k}={v}\n" for k, v in meta.items()), encoding="utf-8")


def _write_heartbeat(session_dir, tick):
    """Ensure session_dir/meta-agent-state.json exists with mtime = INTEGER
    epoch - heartbeat_age, so the watchdog's `stat -c %Y` reads the intended age."""
    hb = Path(session_dir) / "meta-agent-state.json"
    hb.write_text("{}", encoding="utf-8")
    mtime = int(tick["epoch"]) - int(tick["heartbeat_age"])
    os.utime(hb, (mtime, mtime))


def _install_lib_wrapper(meta_dir):
    lib = meta_dir / "_lib"
    lib.rename(meta_dir / "_lib.real")
    lib.write_text(
        "# aewatch bash-oracle _lib wrapper: source the real lib, then wrap\n"
        "# ae_emit_event to ALSO record an event.append effect from the line it\n"
        "# actually appends (so actor/ts/summary/escaping match bash exactly).\n"
        'source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_lib.real"\n'
        "eval \"$(declare -f ae_emit_event | sed '1s/^ae_emit_event /ae_emit_event_real /')\"\n"
        "ae_emit_event() {\n"
        '    ae_emit_event_real "$@"\n'
        '    [[ "${AEWATCH_ORACLE_RECORD:-0}" == "1" ]] || return 0\n'
        "    local _ev\n"
        '    _ev="$(tail -n 1 "${META_DIR}/events.jsonl" 2>/dev/null)"\n'
        '    [[ -n "$_ev" ]] || return 0\n'
        "    printf '{\"kind\":\"event.append\",\"session\":\"%s\",\"event\":%s}\\n' "
        '"$_AE_SESSION" "$_ev" >>"${AEWATCH_ORACLE_DIR}/effects.jsonl"\n'
        "}\n",
        encoding="utf-8",
    )


def _set_meta_value(meta_dir, key, value):
    path = meta_dir / "meta"
    lines = [ln for ln in path.read_text(encoding="utf-8").splitlines() if not ln.startswith(f"{key}=")]
    lines.append(f"{key}={value}")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def _effects_before_final_boundary(effects_path):
    records = [json.loads(ln) for ln in effects_path.read_text(encoding="utf-8").splitlines() if ln.strip()]
    boundary_idxs = [i for i, r in enumerate(records) if "boundary" in r]
    cutoff = boundary_idxs[-1] if boundary_idxs else len(records)
    return [r for r in records[:cutoff] if "boundary" not in r]


def run_bash_watchdog_fixture(fixture, root):
    root = Path(root)
    ae_home = root / "ae"
    sessions = ae_home / "sessions"
    oracle = root / "oracle"
    (root / "home").mkdir(parents=True, exist_ok=True)
    ae_home.mkdir(parents=True, exist_ok=True)
    oracle.mkdir(parents=True, exist_ok=True)

    (ae_home / "config").write_text(fixture.get("config", {}).get("ini", ""), encoding="utf-8")

    session_names, panes_map = [], {}
    for s in fixture.get("sessions", []):
        name = s["name"]
        session_names.append(name)
        sdir = sessions / name
        sdir.mkdir(parents=True, exist_ok=True)
        meta = dict(s.get("meta", {}))
        meta.setdefault("session", name)
        _write_meta(sdir, meta)
        # COMPACT JSON — no spaces — matching ae's ae_emit_event. The bash
        # watchdog's _last_event_age greps for `"actor":"..."` (compact), so a
        # spaced json.dumps would make it miss the event and go falsely stale.
        (sdir / "events.jsonl").write_text(
            "".join(json.dumps(e, separators=(",", ":")) + "\n" for e in s.get("events", [])),
            encoding="utf-8",
        )
        panes_map[name] = s.get("panes", [])
    if not session_names:
        raise ValueError("bash oracle fixture needs at least one session")

    # Key presence, NOT truthiness: an explicit `ticks: []` is a malformed fixture,
    # not a request for a single time tick (the slice-2 bug). Omit the key entirely
    # to derive one tick from `time`.
    if "ticks" in fixture:
        ticks = fixture["ticks"]
    else:
        t = fixture.get("time", {})
        ticks = [{"epoch": t.get("epoch"), "now": t.get("now")}]
    if not ticks:
        raise ValueError("fixture 'ticks' must not be empty (omit the key for a single time tick)")
    (oracle / "ticks.json").write_text(json.dumps(ticks))
    (oracle / "tick.idx").write_text("0")
    (oracle / "n_ticks").write_text(str(len(ticks)))
    (oracle / "options.json").write_text("{}")
    (oracle / "panes.json").write_text(json.dumps(panes_map))
    (oracle / "sessions.json").write_text(json.dumps(session_names))
    (oracle / "effects.jsonl").write_text("")

    # Per-tick INPUT events: tick 0's are seeded now (before cycle 0, matching the
    # Python start_tick(0)); ticks 1..N are appended by the fake sleep shim at each
    # tick boundary. Route to the PRIMARY session (single-session default). Compact,
    # matching ae's on-disk event form. events_target.txt tells the sleep shim where.
    primary_events = sessions / session_names[0] / "events.jsonl"
    tick0_events = ticks[0].get("events") if ticks else None
    if tick0_events:
        with primary_events.open("a", encoding="utf-8") as fh:
            for ev in tick0_events:
                fh.write(json.dumps(ev, separators=(",", ":")) + "\n")
    (oracle / "events_target.txt").write_text(str(primary_events), encoding="utf-8")
    # Steward heartbeat for tick 0 (before cycle 0); ticks 1+ are set by the sleep
    # shim. Key presence; mtime = INTEGER epoch - age (matches ae `stat -c %Y`).
    if ticks and "heartbeat_age" in ticks[0]:
        _write_heartbeat(primary_events.parent, ticks[0])

    env = dict(os.environ)
    env["PATH"] = f"{_FAKEBIN}{os.pathsep}{env['PATH']}"
    env["HOME"] = str(root / "home")
    env["AE_HOME"] = str(ae_home)
    env["AEWATCH_ORACLE_DIR"] = str(oracle)
    env["AEWATCH_ORACLE_SLEEP_SENTINEL"] = _SLEEP_SENTINEL

    session = session_names[0]
    meta_dir = sessions / session

    # 1. Generate helpers from the current ae (recording OFF).
    refresh_env = dict(env, AEWATCH_ORACLE_RECORD="0")
    refresh = subprocess.run(
        [str(_AE), "doctor", "--refresh", session],
        env=refresh_env, cwd=str(ae_home), capture_output=True, text=True,
    )
    # `doctor` returns nonzero for UNRELATED config FAILs (no [agents]/workspace.main
    # in a bare oracle config), so the overall rc is not a success signal. Gate on
    # the session-specific `OK ... refresh:<session>` line instead, so a genuinely
    # failed/partial refresh (which prints FAIL, not OK, for that line) raises.
    if not re.search(rf"(?m)^OK\s+refresh:{re.escape(session)}\b", refresh.stdout):
        raise RuntimeError(
            f"session refresh did not report success:\nSTDOUT:{refresh.stdout}\nSTDERR:{refresh.stderr}"
        )
    if not (meta_dir / "watchdog").exists():
        raise RuntimeError(f"helper generation produced no watchdog helper:\nSTDOUT:{refresh.stdout}\nSTDERR:{refresh.stderr}")

    # 2. Wrap _lib; 3. override ae_path to the fake ae (no real tmux/network).
    _install_lib_wrapper(meta_dir)
    _set_meta_value(meta_dir, "ae_path", str(_FAKEBIN / "ae"))

    # 4. Run the real watchdog loop (recording ON) with the sentinel interval.
    run_env = dict(
        env,
        AEWATCH_ORACLE_RECORD="1",
        AE_WATCHDOG_INTERVAL_SEC=str(_ORACLE_INTERVAL),
        AE_WATCHDOG_STALE_MIN=str(_ORACLE_STALE_MIN),
        AE_WATCHDOG_MAX_NUDGES=str(_ORACLE_MAX_NUDGES),
    )
    run = subprocess.run(
        [str(meta_dir / "watchdog"), "_run"],
        env=run_env, cwd=str(ae_home), capture_output=True, text=True, timeout=30,
    )
    # The loop exits 0 when the fake `has-session` reports the session gone. A
    # nonzero exit means the watchdog aborted mid-cycle (e.g. a fake command failed
    # loud, an unmodeled tmux call), so the effect stream is partial — raise rather
    # than silently return a truncated oracle. The 30s timeout is only the backstop.
    if run.returncode != 0:
        effects_dump = (oracle / "effects.jsonl").read_text(encoding="utf-8")
        raise RuntimeError(
            f"watchdog _run exited {run.returncode} (partial effect stream):\n"
            f"STDOUT:{run.stdout}\nSTDERR:{run.stderr}\nEFFECTS:\n{effects_dump}"
        )

    # 5. Ordered effects before the final cycle boundary (trap unsets after are shutdown).
    return _effects_before_final_boundary(oracle / "effects.jsonl")
