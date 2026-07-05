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

import importlib.machinery
import importlib.util
import json
from dataclasses import dataclass
from pathlib import Path

_REPO_ROOT = Path(__file__).resolve().parents[2]
_AEWATCH_PATH = _REPO_ROOT / "contrib" / "aewatch" / "aewatch"
_CONTRACTS_PATH = _REPO_ROOT / "contrib" / "aewatch" / "CONTRACTS.md"


def _load_aewatch():
    loader = importlib.machinery.SourceFileLoader("aewatch_sidecar", str(_AEWATCH_PATH))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


AW = _load_aewatch()


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
            "".join(json.dumps(e) + "\n" for e in (events or [])), encoding="utf-8"
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


@dataclass
class Pane:
    pane_id: str
    agent: str = ""
    current_command: str = ""
    capture: str = ""


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

    def capture_pane(self, pane_id):
        return self._captures.get(pane_id, "")

    def display_option(self, target, option):
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
        sessions_by_server.setdefault(server, []).append(name)
        home.session(name, meta=session.get("meta", {}), events=session.get("events", []))
        pane_objs = []
        for pane in session.get("panes", []):
            obj = Pane(
                pane_id=pane["pane_id"],
                agent=pane.get("agent", ""),
                current_command=pane.get("current_command", ""),
                capture=pane.get("capture", ""),
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
