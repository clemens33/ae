"""s18a regression: the watchdog must carry the per-session tmux `-L` server to
EVERY tmux call, not just `list_panes`.

B1 (codex, s18 green review): `run_watchdog_cycle` threaded `tmux_server` only
into `list_panes`; `capture_pane` / `paste` / `display_message` and the
`_push_status` / `_write_branch_segment` / `watchdog_bootstrap` status writes all
defaulted to the AMBIENT server. Under a named `-L` server every
read / nudge / alert / status / bootstrap went to the WRONG server — discovered
right, then acted on nothing.

The dual-run oracle is BLIND to this whole class of bug because `FakeTmux`
ignored `server`. This slice makes `FakeTmux` server-aware (harness: a
non-effect `server_calls` log, so the effect stream the oracle diffs is
unperturbed) and closes the class with a propagation assertion over a single
cycle-set that exercises EVERY downstream tmux method.

Drives the production `WatchdogRunner.run_cycle` (bootstrap + cycle), so the test
is identical before and after the fix — only the production threading moves it
from red to green.

Pure stdlib.
"""

import datetime
import tempfile
import unittest
from pathlib import Path

from harness import AW, MultiTickEnv

_SERVER = "srv-alt"          # a NAMED server -> `-L srv-alt`; must reach every call
_E = 1783234800             # fixed epoch (no wall clock)


def _iso(epoch: int) -> str:
    return datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _fixture(work_dir: str) -> dict:
    # pane %1 (node, stays alive): capture_pane every tick + a nudge (paste) on
    #   tick 2 (1000s > STALE 900s, static frame, no matching events).
    # pane %2 (bash) with has_descendant->False: reads as DEAD -> display_message
    #   alert (once, tick 1), then latched.
    # every cycle + the one-time bootstrap also push status
    #   (set_option x2 + unset_option, non-git work_dir -> @ae_branch_name unset).
    return {
        "id": "watchdog.server.threading",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        "sessions": [
            {
                "name": "work",
                "tmux_server": _SERVER,
                "meta": {
                    "session": "work", "session_id": "s1", "tmux_server": _SERVER,
                    "work_dir": work_dir, "goal": "ship it", "agent.main": "codex:w:s1",
                },
                "events": [],
                "panes": [
                    {"pane_id": "%1", "agent": "codex:w", "current_command": "node", "capture": ""},
                    {"pane_id": "%2", "agent": "claude:x", "current_command": "bash", "capture": ""},
                ],
            }
        ],
        "ticks": [
            {"epoch": _E, "now": _iso(_E), "captures": {"%1": "idle-frame"}},
            {"epoch": _E + 1000, "now": _iso(_E + 1000), "captures": {"%1": "idle-frame"}},
        ],
    }


class ServerThreadingTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.nogit = self.root / "nogit"   # isolated, deterministically NON-git
        self.nogit.mkdir()
        self.envdir = self.root / "env"
        self.envdir.mkdir()

    def _drive(self):
        """Run bootstrap + 2 cycles through the production WatchdogRunner against a
        NAMED server; return the FakeTmux so the caller can read server_calls."""
        env = MultiTickEnv(_fixture(str(self.nogit)), str(self.envdir))
        config = AW.WatchdogConfig(interval=60, stale_min=15, max_nudges=2)
        runner = AW.WatchdogRunner(
            config, env.tmux, sessions_dir=env.home.sessions,
            emit_event=env.append_event,
            has_descendant=lambda *a, **k: False,   # the bash pane reads as dead
            recover_pending=lambda s: [],
            supervise_bridge=lambda server: None,
        )
        session = AW.DiscoveredSession(
            name="work", session_id="s1", work_dir=str(self.nogit),
            tmux_server=_SERVER, running=True, agents=[],
        )
        for i, _tick in enumerate(env.ticks):
            env.start_tick(i)
            runner.run_cycle(session, env.ticks[i]["epoch"])
        return env.tmux

    def test_named_server_propagates_to_every_tmux_call(self):
        tmux = self._drive()
        # anti-vacuous: the cycle-set really exercised each downstream method, so a
        # green result means propagation, not "these branches never ran".
        methods = {m for m, _ in tmux.server_calls}
        for required in ("list_panes", "capture_pane", "paste", "display_message",
                         "set_option", "unset_option"):
            self.assertIn(required, methods,
                          f"fixture did not exercise {required} — propagation test is vacuous")
        # the class-closer: NOT ONE tmux call may fall back to the ambient server.
        bad = [(m, s) for m, s in tmux.server_calls if s != _SERVER]
        self.assertEqual(
            bad, [],
            f"these watchdog tmux calls lost the -L {_SERVER!r} server (went to the "
            f"ambient server instead): {bad}",
        )

    def test_downstream_calls_exist_so_the_guard_can_bite(self):
        # Guard the guard: list_panes was ALREADY server-correct pre-fix, so a check
        # that only inspected list_panes would pass even with B1 live. Prove there are
        # non-list_panes tmux calls for the propagation assertion to catch.
        tmux = self._drive()
        downstream = [(m, s) for m, s in tmux.server_calls if m != "list_panes"]
        self.assertTrue(downstream, "no downstream tmux calls recorded — cannot detect B1")


if __name__ == "__main__":
    unittest.main()
