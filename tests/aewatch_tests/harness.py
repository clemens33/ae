"""Fake-fs / fake-tmux / effect harness for the aewatch test suite.

PHASE-2 PARITY ORACLE INPUT. In phase 2 this same harness drives the bash-vs-
aewatch dual run: a `fake_tmux` executable shim will feed the bash watchdog and
emit the SAME normalized Effect records this FakeTmux produces, and both runs are
diffed on the recorder's ordered effect list. So keep the fake side faithful to
the real contracts:

  - FakeAeHome builds realistic ae paths under a temp root and NEVER touches the
    real ~/.ae (all reads/writes are scoped to the given root).
  - FakeTmux implements aewatch.TmuxClient: READ methods return data and record
    nothing; MUTATION methods each append exactly one normalized effect via the
    aewatch EffectRecorder (the canonical schema lives in the sidecar).
  - load_fixture()/build_fixture_env() seed the env from the committed
    CONTRACTS.md fixtures, so tests and the future oracle share one input source.

Pure stdlib. The sidecar is imported from its extensionless path (its top-level
main() is guarded, so import has no side effects).
"""

import copy
import importlib.machinery
import importlib.util
import json
import os
import sys
from pathlib import Path

_REPO_ROOT = Path(__file__).resolve().parents[2]
_AEWATCH_PATH = _REPO_ROOT / "contrib" / "aewatch" / "aewatch"
_CONTRACTS_PATH = _REPO_ROOT / "contrib" / "aewatch" / "CONTRACTS.md"


def _load_aewatch():
    loader = importlib.machinery.SourceFileLoader("aewatch_sidecar", str(_AEWATCH_PATH))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    mod = importlib.util.module_from_spec(spec)
    # Register before exec: @dataclass + `from __future__ import annotations`
    # resolves field annotations via sys.modules[cls.__module__] at class-creation.
    sys.modules[loader.name] = mod
    loader.exec_module(mod)
    return mod


AW = _load_aewatch()


def _set_meta_value(meta_path, key: str, value) -> None:
    """Set key=value in a meta file with REMOVE + APPEND semantics — drop any
    existing `key=` line(s), then append. Mirrors bash_oracle._set_meta_value so the
    boundary metas stay byte-faithful even if a fixture already carried the key
    (_meta_value reads the FIRST match, so a plain append could diverge)."""
    lines = [ln for ln in meta_path.read_text(encoding="utf-8").splitlines() if not ln.startswith(f"{key}=")]
    lines.append(f"{key}={value}")
    meta_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def _dump_event(event: dict) -> str:
    """Serialize an ae event COMPACTLY — `{"k":"v"}`, no spaces — matching what
    ae's ae_emit_event writes. events.jsonl is an ae file contract, and ae's
    bash-side parity reads (e.g. _last_event_age's `grep '"actor":"..."'`) depend
    on the compact form; every harness path that writes an event uses this."""
    return json.dumps(event, separators=(",", ":"))


class FakeAeHome:
    """A temp $AE_HOME with realistic ae paths; isolated from the real ~/.ae."""

    def __init__(self, root):
        self.home = Path(root)
        self.sessions = self.home / "sessions"
        self.aewatch = self.home / "aewatch"
        self.config = self.home / "config"
        self.sessions.mkdir(parents=True, exist_ok=True)
        self.aewatch.mkdir(parents=True, exist_ok=True)

    def write_config(self, text: str) -> Path:
        self.config.write_text(text, encoding="utf-8")
        return self.config

    def session(self, name: str, *, meta: dict, events: list | None = None) -> Path:
        session_dir = self.sessions / name
        session_dir.mkdir(parents=True, exist_ok=True)
        (session_dir / "meta").write_text(
            "".join(f"{k}={v}\n" for k, v in meta.items()), encoding="utf-8"
        )
        (session_dir / "events.jsonl").write_text(
            "".join(_dump_event(e) + "\n" for e in (events or [])), encoding="utf-8"
        )
        return session_dir

    def read_meta(self, name: str) -> dict:
        out: dict[str, str] = {}
        text = (self.sessions / name / "meta").read_text(encoding="utf-8")
        for line in text.splitlines():
            if "=" in line:
                key, value = line.split("=", 1)
                out[key] = value
        return out

    def read_jsonl(self, name: str) -> list:
        path = self.sessions / name / "events.jsonl"
        if not path.is_file():
            return []
        return [
            json.loads(line)
            for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]

    def runtime_file(self, name: str) -> Path:
        return self.aewatch / name


# Single source of truth: the production pane type. Aliased (not re-declared) so the
# fake and the real client can never drift on the fields the cycle reads.
Pane = AW.Pane


class FakeTmux(AW.TmuxClient):
    """In-memory TmuxClient. Reads return seeded data; mutations record effects."""

    def __init__(self, recorder, spec: dict | None = None):
        spec = spec or {}
        self._rec = recorder
        self._sessions_by_server = {k: list(v) for k, v in spec.get("sessions_by_server", {}).items()}
        self._panes = {k: list(v) for k, v in spec.get("panes", {}).items()}
        self._options = {t: dict(o) for t, o in spec.get("options", {}).items()}
        self._captures = dict(spec.get("captures", {}))

    # ── reads: return data, record nothing ──────────────────────────────
    # `None` (the real client's ambient/default server) and `""` (a fixture's
    # default `tmux_server` from meta) resolve to the SAME default server.
    def list_sessions(self, server=None):
        return list(self._sessions_by_server.get(server or "", []))

    def list_panes(self, session, server=None):
        server = server or ""  # normalized for when list_panes becomes server-aware
        return list(self._panes.get(session, []))

    def capture_pane(self, pane_id, server=None):
        return self._captures.get(pane_id, "")

    def display_option(self, target, option, server=None):
        return self._options.get(target, {}).get(option)

    # ── mutations: each records exactly one normalized effect ───────────
    def set_option(self, target, option, value):
        self._options.setdefault(target, {})[option] = value
        self._rec.record("tmux.set_option", target=target, option=option, value=value)

    def unset_option(self, target, option):
        self._options.get(target, {}).pop(option, None)
        self._rec.record("tmux.unset_option", target=target, option=option)

    def paste(self, target, text, submit):
        self._rec.record("tmux.paste", target=target, text=text, submit=submit)

    def display_message(self, text, *, duration_ms=None):
        # Mirrors the bash watchdog's `tmux display-message -d <ms> "<text>"` alert
        # (no -t target). A user-visible mutation, so it records an effect.
        self._rec.record("tmux.display_message", text=text, duration_ms=duration_ms)


def load_fixture(fixture_id: str) -> dict:
    """Return the committed CONTRACTS.md fixture with the given id (via the sidecar loader)."""
    obj = AW.extract_contracts_json(_CONTRACTS_PATH.read_text(encoding="utf-8"))
    for fixture in obj["fixtures"]:
        if fixture["id"] == fixture_id:
            return fixture
    raise KeyError(f"no fixture {fixture_id!r} in {_CONTRACTS_PATH}")


def build_fixture_env(fixture: dict, root, recorder):
    """Seed a FakeAeHome + FakeTmux from a fixture. Returns (FakeAeHome, FakeTmux).

    Setup writes (config, session meta/events) are test scaffolding, NOT effects —
    only FakeTmux mutations and explicit recorder calls produce effect records.
    """
    home = FakeAeHome(root)
    home.write_config(fixture.get("config", {}).get("ini", ""))

    sessions_by_server: dict = {}
    panes: dict = {}
    options: dict = {}
    captures: dict = {}
    for session in fixture.get("sessions", []):
        name = session["name"]
        server = session.get("tmux_server", "")
        # A session dir always exists (meta written); it is only "running" (in tmux)
        # when running != False — so a stopped session is discoverable but running=False.
        if session.get("running", True):
            sessions_by_server.setdefault(server, []).append(name)
        home.session(name, meta=session.get("meta", {}), events=session.get("events", []))
        pane_objs = []
        for pane in session.get("panes", []):
            obj = Pane(
                pane_id=pane["pane_id"],
                agent=pane.get("agent", ""),
                current_command=pane.get("current_command", ""),
                capture=pane.get("capture", ""),
                pane_pid=str(pane.get("pane_pid", "")),
            )
            pane_objs.append(obj)
            captures[obj.pane_id] = obj.capture
        panes[name] = pane_objs
        for target, opts in session.get("tmux_options", {}).items():
            options.setdefault(target, {}).update(opts)

    tmux = FakeTmux(
        recorder,
        {"sessions_by_server": sessions_by_server, "panes": panes, "options": options, "captures": captures},
    )
    return home, tmux


def canonical(effects: list) -> list:
    """Deterministic presentation ordering (delegates to the sidecar helper)."""
    return AW.canonical_effects(effects)


# ── Multi-tick fixture harness (phase-2 dual-run oracle input) ─────────────
# The watchdog is a per-cycle state machine, so a fixture models N cycles and
# effects recorded in one tick MUST mutate the fake state so the next tick reads
# them (the plan's Effect Application rule). Tick boundaries stay explicit —
# start_tick(i) — so the oracle can diff per cycle; effect-recording helpers
# couple record + mutate to avoid any double-apply.


def load_ticks(fixture: dict) -> list:
    """The fixture's `ticks`, or a single tick derived from `time` when the key is
    ABSENT. Key presence (not truthiness) — an explicit `ticks: []` is honored as
    empty, never masked by a synthetic time tick (the validator rejects empty)."""
    if "ticks" in fixture:
        return fixture["ticks"]
    t = fixture.get("time", {})
    return [{"epoch": t.get("epoch"), "now": t.get("now")}]


class TickClock:
    """Fake clock stepping through a fixture's ticks (no real time)."""

    def __init__(self, ticks: list):
        self._ticks = ticks
        self._i = 0

    def start(self, i: int) -> None:
        self._i = i

    def epoch(self):
        return self._ticks[self._i]["epoch"]

    def now(self):
        return self._ticks[self._i].get("now")


class MultiTickEnv:
    """Multi-cycle fake environment for a phase-2 watchdog fixture."""

    def __init__(self, fixture: dict, root):
        self.fixture = fixture
        self.recorder = AW.EffectRecorder()
        self.home, self.tmux = build_fixture_env(fixture, root, self.recorder)
        self.ticks = load_ticks(fixture)
        self.clock = TickClock(self.ticks)

    def start_tick(self, i: int) -> None:
        """Advance the clock to tick i, apply that tick's pane captures, and inject
        its per-session INPUT events + heartbeats.

        Phase-3: tick inputs are SESSION-KEYED (no first-session shortcut). events is
        [{session, event}] — each event.INPUT (agent declaration / inbound message)
        is appended to THAT session's events.jsonl, NEVER recorded as an event.append
        effect. heartbeats is {session: {age}} — each sets that session's
        meta-agent-state.json mtime to INTEGER epoch - age (identical on both oracle
        sides). recover ({session:[rows]}) is served by the injected recover_pending."""
        self.clock.start(i)
        tick = self.ticks[i]
        for pane_id, capture in tick.get("captures", {}).items():
            self.tmux._captures[pane_id] = capture
        for entry in tick.get("events", []):
            path = self.home.sessions / entry["session"] / "events.jsonl"
            with path.open("a", encoding="utf-8") as handle:
                handle.write(_dump_event(entry["event"]) + "\n")
        for session, spec in tick.get("heartbeats", {}).items():
            hb = self.home.sessions / session / "meta-agent-state.json"
            hb.write_text("{}", encoding="utf-8")
            mtime = int(tick["epoch"]) - int(spec["age"])
            os.utime(hb, (mtime, mtime))

    def append_event(self, session: str, event: dict) -> dict:
        """Record an event.append effect AND append it to events.jsonl so later
        ticks see it. The event is snapshotted so a later caller mutation of the
        original dict cannot retroactively change the effect or the file."""
        snapshot = copy.deepcopy(event)
        self.recorder.record("event.append", session=session, event=snapshot)
        path = self.home.sessions / session / "events.jsonl"
        with path.open("a", encoding="utf-8") as handle:
            handle.write(_dump_event(snapshot) + "\n")
        return snapshot

    def read_events(self, session: str) -> list:
        return self.home.read_jsonl(session)


def run_python_watchdog_fixture(fixture: dict, *, git=None) -> list:
    """Python side of the phase-2 dual-run oracle: mirrors run_bash_watchdog_fixture
    but drives the in-process aewatch cycle instead of the generated bash watchdog.

    Reuses MultiTickEnv as the SINGLE per-tick capture/pane mutation path (codex —
    no second driver), then drives bootstrap once (ae `_run` startup before the
    loop) + one run_watchdog_cycle per tick (ae's one cycle per sentinel sleep).
    The cycle reads the session's ACTUAL events.jsonl (via session_dir), so an
    event.append effect a tick records is visible to a later tick's recency scan.
    Returns the ordered effect stream (recorder.as_list()) for byte-identical
    diffing against the bash oracle. `git` injects a fake git runner (branch
    fixtures); default is real git, so a non-git work_dir resolves identically
    on both sides."""
    import tempfile

    session = fixture["sessions"][0]
    name = session["name"]
    meta = session.get("meta", {})
    work_dir = meta.get("work_dir", "")
    server = meta.get("tmux_server", "")
    git_runner = git or AW.default_git_runner
    # Mirror the tunables run_bash_watchdog_fixture forces on the bash side, so
    # config-derived text (e.g. the throttle alert's THROTTLE_ALERT_CYCLES *
    # INTERVAL figure) stays byte-exact across the dual run — no normalization.
    config = AW.WatchdogConfig(
        interval=bash_oracle._ORACLE_INTERVAL,
        stale_min=bash_oracle._ORACLE_STALE_MIN,
        max_nudges=bash_oracle._ORACLE_MAX_NUDGES,
        # DISABLED by default (mirror bash_oracle): the bridge-supervise scheduler
        # opts in per fixture so it never pollutes unrelated slices' streams.
        tg_supervise_secs=int(fixture.get("tg_supervise_secs", 0)),
    )
    state = AW.WatchdogState()
    with tempfile.TemporaryDirectory() as tmp:
        env = MultiTickEnv(fixture, tmp)
        session_dir = env.home.sessions / name
        # Mirror the bash oracle: point meta ae_path at the (executable) fake ae so
        # the recover / telegram-supervise ae_path gate passes IDENTICALLY on both
        # sides. REPLACE semantics (matching bash_oracle._set_meta_value) — a plain
        # append could diverge if a fixture already set ae_path. Python doesn't shell
        # it (it uses the injected boundaries below).
        _set_meta_value(session_dir / "meta", "ae_path", bash_oracle._FAKEBIN / "ae")
        AW.watchdog_bootstrap(env.tmux, name, work_dir=work_dir, git=git_runner)
        for i, tick in enumerate(env.ticks):
            env.start_tick(i)  # sole tick-mutation path: applies this tick's captures
            AW.run_watchdog_cycle(
                config, state, env.tmux, name,
                work_dir=work_dir, session_dir=session_dir, tmux_server=server,
                now=tick.get("epoch"), git=git_runner,
                # sole event-emit path: records event.append AND appends compactly
                # to events.jsonl (feed-forward), so a later tick's recency scan sees it.
                emit_event=env.append_event,
                # injected recover boundary: this tick's fixture recover rows for the
                # named session (recover is {session:[rows]}; the production impl
                # shells `ae _recover-pending <session>` + parses the TSV).
                recover_pending=(lambda s, _rec=tick.get("recover", {}): _rec.get(s, [])),
                # injected bridge supervisor: records a telegram.supervise effect
                # (NOT on FakeTmux — it is not a tmux op). Production reboots the
                # bridge with AE_TMUX_SERVER=<server>.
                supervise_bridge=(lambda server: env.recorder.record("telegram.supervise", tmux_server=server)),
            )
        return env.recorder.as_list()


def run_daemon_tick_fixture(fixture: dict, *, git=None) -> list:
    """Drive a multi-session fixture through the COMPOSED daemon tick (run_daemon_tick
    with an injected WatchdogRunner) and return the watchdog effects only (the daemon's
    own file.write/log.write are dropped). One run_daemon_tick call per tick; the
    runner carries per-session state + bootstrap-once across ticks.

    Slice-13 fixtures are captures-only (MultiTickEnv routes per-tick events/heartbeat
    to the first session), and carry no recover/telegram opt-in, so those boundaries
    are inert here (recover_pending returns [] — a per-tick multi-session recover
    binding waits for the {session,event} extension)."""
    import tempfile

    git_runner = git or AW.default_git_runner
    config = AW.WatchdogConfig(
        interval=bash_oracle._ORACLE_INTERVAL,
        stale_min=bash_oracle._ORACLE_STALE_MIN,
        max_nudges=bash_oracle._ORACLE_MAX_NUDGES,
        tg_supervise_secs=int(fixture.get("tg_supervise_secs", 0)),
    )
    with tempfile.TemporaryDirectory() as tmp:
        env = MultiTickEnv(fixture, tmp)
        for session in fixture.get("sessions", []):
            _set_meta_value(env.home.sessions / session["name"] / "meta", "ae_path", bash_oracle._FAKEBIN / "ae")
        runner = AW.WatchdogRunner(
            config, env.tmux, sessions_dir=env.home.sessions, git=git_runner,
            emit_event=env.append_event,
            recover_pending=(lambda s: []),
            supervise_bridge=(lambda server: env.recorder.record("telegram.supervise", tmux_server=server)),
        )
        runtime = AW.AewatchRuntime(env.home.home)
        for i, tick in enumerate(env.ticks):
            env.start_tick(i)
            AW.run_daemon_tick(runtime, env.tmux, env.recorder, now=tick.get("epoch"), watchdog_runner=runner)
        return [e for e in env.recorder.as_list() if e["kind"] not in ("file.write", "log.write")]


def run_daemon_tick_smoke(fixture: dict) -> list:
    """The DEFAULT-CLI path: run_daemon_tick with NullTmuxClient and NO runner. Returns
    every recorded effect — must be only the daemon's own file.write/log.write, never a
    watchdog mutation (proves the CLI is smoke-safe)."""
    import tempfile

    recorder = AW.EffectRecorder()
    with tempfile.TemporaryDirectory() as tmp:
        env = MultiTickEnv(fixture, tmp)
        runtime = AW.AewatchRuntime(env.home.home)
        for i, tick in enumerate(env.ticks):
            env.start_tick(i)
            AW.run_daemon_tick(runtime, AW.NullTmuxClient(), recorder, now=tick.get("epoch"))
        return recorder.as_list()


# Bash side of the phase-2 dual-run oracle (re-exported for the test suite).
import bash_oracle  # noqa: E402
from bash_oracle import run_bash_watchdog_fixture  # noqa: E402,F401
