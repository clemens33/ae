"""Phase-2 Slice 11 contract: pending session-id recovery.

Codex/gemini/opencode can't set their session id at launch, so ae captures it
post-launch. The watchdog retries that capture each cycle via
`ae _recover-pending <session>` (ae:7966-7975) and, for each 'ok' row it wins,
emits a `recover` event (action=recover, target=<agent>, ref=<captured_id>,
summary "captured <tool> session id (<first 8 of id>)", a DIRECT emit -> actor
'human'). Non-ok rows (already/miss/skip) are ignored. The block is a NO-OP when
ae_path is absent or non-executable.

The recovery result is an INPUT boundary: `ae _recover-pending` output on the bash
side (fake ae, per-tick recover rows) and an injected recover_pending() on the
Python side. Non-git work_dir. No ae edits.
"""

import datetime
import shutil
import tempfile
import unittest
from pathlib import Path

from harness import run_bash_watchdog_fixture, run_python_watchdog_fixture

_E = 1783234800
_AGENT = "optimal:cw"
_PANE = "%1"


def _iso(epoch: int) -> str:
    return datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _fixture(work_dir, recover_rows):
    """One tick; the tick carries the `recover` rows that `ae _recover-pending`
    returns. A recent pane so the agent is active (no nudge/stale noise)."""
    return {
        "id": "watchdog.recover",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        "sessions": [{"name": "work", "tmux_server": "",
            "meta": {"session": "work", "session_id": "s1", "tmux_server": "", "work_dir": work_dir,
                     "agent.main": f"{_AGENT}:s1"},
            "events": [], "panes": [{"pane_id": _PANE, "agent": _AGENT, "current_command": "node",
                                     "pane_pid": 999999, "capture": ""}]}],
        "ticks": [{"epoch": _E, "now": _iso(_E), "captures": {_PANE: "working"}, "recover": {"work": recover_rows}}],
    }


def _ok_row():
    return {"kind": "ok", "slot": "main", "agent": _AGENT, "tool": "codex", "captured": "abcd1234efgh5678"}


class RecoverParityTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.nogit = self.root / "nogit"
        self.nogit.mkdir()

    def _parity(self, fixture, label):
        bash = run_bash_watchdog_fixture(fixture, self.root / f"bash-{label}")
        python = run_python_watchdog_fixture(fixture)
        self.assertEqual(python, bash, f"[{label}] python recover stream diverged from bash")
        self.assertTrue(bash, f"[{label}] bash produced no effects")
        return python

    def _recovers(self, effects):
        return [e for e in effects if e["kind"] == "event.append" and e["event"].get("action") == "recover"]

    def test_recover_ok_row_emits_event(self):
        python = self._parity(_fixture(str(self.nogit), [_ok_row()]), "ok")
        recs = self._recovers(python)
        self.assertEqual(len(recs), 1, "one recover event for the ok row")
        ev = recs[0]["event"]
        self.assertEqual(ev.get("actor"), "human")
        self.assertEqual(ev.get("target"), _AGENT)
        self.assertEqual(ev.get("ref"), "abcd1234efgh5678")
        self.assertEqual(ev.get("summary"), "captured codex session id (abcd1234)")
        # Order (codex): the recover event precedes the end-of-cycle status push.
        rec_idx = python.index(recs[0])
        status_idxs = [i for i, e in enumerate(python)
                       if e["kind"] == "tmux.set_option" and e["option"] == "@ae_watchdog_status"]
        self.assertLess(rec_idx, status_idxs[-1], "recover event must precede the cycle-end status")

    def test_no_recover_when_ae_path_absent(self):
        # ae_path NO-OP: the bash oracle always sets ae_path, so exercise it Python-
        # side — a cycle whose meta lacks ae_path must skip recovery entirely.
        from harness import AW, FakeTmux
        with tempfile.TemporaryDirectory() as tmp:
            sd = Path(tmp) / "work"
            sd.mkdir()
            (sd / "meta").write_text(f"session=work\nwork_dir={self.nogit}\n", encoding="utf-8")  # NO ae_path
            (sd / "events.jsonl").write_text("", encoding="utf-8")
            emitted = []
            AW.run_watchdog_cycle(
                AW.WatchdogConfig(), AW.WatchdogState(), FakeTmux(AW.EffectRecorder()), "work",
                work_dir=str(self.nogit), session_dir=sd, now=_E,
                git=(lambda work_dir, args, **kw: (1, "")),
                emit_event=(lambda s, e: emitted.append(e)),
                recover_pending=(lambda s: [_ok_row()]),
            )
            self.assertEqual([e for e in emitted if e.get("action") == "recover"], [],
                             "no recover event when ae_path is absent")

    def test_non_ok_rows_are_ignored(self):
        rows = [
            {"kind": "already", "slot": "main", "agent": _AGENT, "tool": "codex", "captured": ""},
            {"kind": "miss", "slot": "w0", "agent": "worker:w0", "tool": "gemini", "captured": ""},
            {"kind": "skip", "slot": "w1", "agent": "worker:w1", "tool": "", "captured": ""},
        ]
        python = self._parity(_fixture(str(self.nogit), rows), "non-ok")
        self.assertEqual(len(self._recovers(python)), 0, "non-ok rows emit no recover event")


if __name__ == "__main__":
    unittest.main()
