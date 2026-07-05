"""Phase-2 Slice 12 contract: telegram supervise boundary (scheduler-only).

The watchdog best-effort revives the machine-global telegram bridge if it died
while this session is alive, THROTTLED to TG_SUPERVISE_SECS (ae:7977-7997). It is
purely a SCHEDULER + a tmux_server propagation seam: `AE_TMUX_SERVER=<server> ae
telegram _supervise`. Phase 3 owns the real bridge/network behavior — here there is
no offset, no poll, no send.

Observability: the supervise ATTEMPT is modeled as a `telegram.supervise` effect
carrying the propagated tmux_server (NOT a telegram.send). The bash fake ae records
it when `telegram _supervise` is invoked; the Python cycle records it via an
injected BridgeSupervisor boundary when the schedule is due. So the dual run
compares WHICH ticks supervised + with which server.

last_tg_supervise starts at 0, so cycle 1 is always due; then it is quiet for
TG_SUPERVISE_SECS (120). NO-OP when ae_path is absent/non-executable. Non-git
work_dir. No ae edits.
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


def _fixture(work_dir, ticks, tmux_server=""):
    return {
        "id": "watchdog.telegram",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        # Opt into the bridge-supervise scheduler (disabled by default in the oracle).
        "tg_supervise_secs": 120,
        "sessions": [{"name": "work", "tmux_server": tmux_server,
            "meta": {"session": "work", "session_id": "s1", "tmux_server": tmux_server,
                     "work_dir": work_dir, "agent.main": f"{_AGENT}:s1"},
            "events": [], "panes": [{"pane_id": _PANE, "agent": _AGENT, "current_command": "node",
                                     "pane_pid": 999999, "capture": ""}]}],
        "ticks": ticks,
    }


def _due_not_due_fixture(work_dir):
    """3 ticks: t1 due (last=0), t2 within 120s -> not due, t3 past 120s -> due."""
    return _fixture(work_dir, [
        {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "working"}},
        {"epoch": _E + 60, "now": _iso(_E + 60), "captures": {_PANE: "working"}},
        {"epoch": _E + 180, "now": _iso(_E + 180), "captures": {_PANE: "working"}},
    ])


class TelegramParityTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.nogit = self.root / "nogit"
        self.nogit.mkdir()

    def _parity(self, fixture, label):
        bash = run_bash_watchdog_fixture(fixture, self.root / f"bash-{label}")
        python = run_python_watchdog_fixture(fixture)
        self.assertEqual(python, bash, f"[{label}] python telegram stream diverged from bash")
        self.assertTrue(bash, f"[{label}] bash produced no effects")
        return python

    def _supervises(self, effects):
        return [e for e in effects if e["kind"] == "telegram.supervise"]

    def test_supervise_schedule(self):
        python = self._parity(_due_not_due_fixture(str(self.nogit)), "schedule")
        # Due on t1 + t3, quiet on t2 (within TG_SUPERVISE_SECS).
        self.assertEqual(len(self._supervises(python)), 2, "supervise fires only on the due cycles")

    def test_tmux_server_propagation(self):
        python = self._parity(_due_not_due_fixture(str(self.nogit)), "srv-default")
        for e in self._supervises(python):
            self.assertEqual(e.get("tmux_server"), "", "default server propagates empty")

        alt = self._fixture_alt()
        python_alt = self._parity(alt, "srv-alt")
        sup = self._supervises(python_alt)
        self.assertTrue(sup, "supervise fired")
        for e in sup:
            self.assertEqual(e.get("tmux_server"), "ae-alt", "non-default tmux_server propagates")

    def _fixture_alt(self):
        return _fixture(str(self.nogit), [
            {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "working"}},
        ], tmux_server="ae-alt")

    def test_telegram_supervise_is_a_known_effect_kind(self):
        from harness import AW
        self.assertIn("telegram.supervise", AW.EFFECT_KINDS)
        self.assertEqual(AW.make_effect("telegram.supervise", tmux_server="x")["kind"], "telegram.supervise")
        with self.assertRaises(ValueError):
            AW.make_effect("telegram.bogus")

    def test_recover_then_supervise_then_status_order(self):
        # ae step 9 (recover) -> step 10 (telegram supervise) -> end-of-cycle status.
        fx = _fixture(str(self.nogit), [{"epoch": _E, "now": _iso(_E), "captures": {_PANE: "working"},
            "recover": [{"kind": "ok", "slot": "main", "agent": _AGENT, "tool": "codex", "captured": "abcd1234"}]}])
        python = self._parity(fx, "order")
        rec = next(i for i, e in enumerate(python)
                   if e["kind"] == "event.append" and e["event"].get("action") == "recover")
        sup = next(i for i, e in enumerate(python) if e["kind"] == "telegram.supervise")
        status = [i for i, e in enumerate(python)
                  if e["kind"] == "tmux.set_option" and e["option"] == "@ae_watchdog_status"][-1]
        self.assertLess(rec, sup, "recover before supervise")
        self.assertLess(sup, status, "supervise before the cycle-end status")

    def test_no_supervise_when_ae_path_absent(self):
        from harness import AW, FakeTmux
        with tempfile.TemporaryDirectory() as tmp:
            sd = Path(tmp) / "work"
            sd.mkdir()
            (sd / "meta").write_text(f"session=work\nwork_dir={self.nogit}\n", encoding="utf-8")  # NO ae_path
            (sd / "events.jsonl").write_text("", encoding="utf-8")
            calls = []
            AW.run_watchdog_cycle(
                AW.WatchdogConfig(), AW.WatchdogState(), FakeTmux(AW.EffectRecorder()), "work",
                work_dir=str(self.nogit), session_dir=sd, now=_E,
                git=(lambda work_dir, args, **kw: (1, "")),
                supervise_bridge=(lambda server: calls.append(server)),
            )
            self.assertEqual(calls, [], "no supervise when ae_path is absent")


if __name__ == "__main__":
    unittest.main()
