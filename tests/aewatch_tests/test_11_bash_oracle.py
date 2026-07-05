"""Phase-2 Slice 3 contract: the bash oracle skeleton.

run_bash_watchdog_fixture() drives the REAL generated watchdog helper (from the
current ae, via `ae doctor --refresh`) under fake tmux/date/sleep/ae shims, and
returns the ordered EFFECT_KINDS record stream for one fixture — byte-identical in
field names + normalization to what FakeTmux emits. This is the load-bearing seam
the Python-vs-bash diff (slice 4+) compares against.

This slice proves the skeleton for a STATUS-ONLY fixture (non-git work_dir, no
agent panes): the watchdog emits only tmux.set_option/tmux.unset_option status
effects — no nudge/alert/paste/event. No real tmux, no real ~/.ae, no ae edits.

Skips cleanly if `ae doctor --refresh` cannot generate helpers (requires a bash/
tmux-shaped environment). Pure stdlib.
"""

import shutil
import tempfile
import unittest
from pathlib import Path

from harness import run_bash_watchdog_fixture


def _status_only_fixture(work_dir, tmux_server=""):
    """One session, an ISOLATED NON-git work_dir, and NO @ae_agent panes — so the
    watchdog cycle does status bookkeeping only (branch unset, 0 agents). work_dir
    is a caller-owned temp path so the branch path is deterministic (codex: a fixed
    /tmp path could accidentally be a git repo).

    tmux_server selects the fake server: "" is the default server (plain `tmux`);
    a non-empty value makes the watchdog route every call through `command tmux
    -L <server>`, which the fake must strip back to the default (codex regression)."""
    return {
        "id": "watchdog.status.branch-side-channel",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        "sessions": [
            {
                "name": "work",
                "tmux_server": tmux_server,
                "meta": {"session": "work", "session_id": "s1", "tmux_server": tmux_server, "work_dir": work_dir},
                "events": [],
                "panes": [],
            }
        ],
        "ticks": [{"epoch": 1783234800, "now": "2026-07-05T07:00:00Z", "captures": {}}],
        "expect": {"effects": [], "final_events": {}, "final_tmux_options": {}, "final_files": {}},
    }


@unittest.skipUnless(shutil.which("tmux") or True, "bash oracle needs a bash-shaped env")
class BashOracleSkeletonTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.nogit = self.root / "nogit"  # isolated, deterministically NON-git
        self.nogit.mkdir()

    def test_status_only_fixture_emits_ordered_status_effects(self):
        effects = run_bash_watchdog_fixture(_status_only_fixture(str(self.nogit)), self.root)
        self.assertIsInstance(effects, list)
        self.assertTrue(effects, "the bash oracle must emit at least the bootstrap status effects")

        kinds = {e["kind"] for e in effects}
        # Status-only: ONLY tmux option writes — no nudge/alert/event in this fixture.
        self.assertEqual(kinds - {"tmux.set_option", "tmux.unset_option"}, set(),
                         f"status-only fixture emitted unexpected kinds: {kinds}")
        for forbidden in ("event.append", "tmux.paste", "tmux.display_message", "telegram.send"):
            self.assertNotIn(forbidden, kinds)

        # Partial order (codex): the FIRST @ae_watchdog_status write is the bootstrap
        # 'starting'; a LATER one carries the end-of-cycle '0/0' health.
        wd_status = [e["value"] for e in effects if e["kind"] == "tmux.set_option" and e["option"] == "@ae_watchdog_status"]
        self.assertTrue(wd_status, "no @ae_watchdog_status writes")
        self.assertIn("starting", wd_status[0])
        self.assertTrue(any("0/0" in v for v in wd_status[1:]), f"no later [watch ... 0/0] after bootstrap: {wd_status}")

        # Non-git work_dir -> @ae_branch_name is UNSET, and @ae_branch_status is set
        # to "" (the display channel cleared) — the real branch-segment behavior.
        self.assertTrue(
            any(e["kind"] == "tmux.unset_option" and e["option"] == "@ae_branch_name" for e in effects),
            "non-git work_dir must unset @ae_branch_name",
        )
        self.assertTrue(
            any(e["kind"] == "tmux.set_option" and e["option"] == "@ae_branch_status" and e["value"] == "" for e in effects),
            "non-git work_dir must set @ae_branch_status to empty",
        )

        # Every effect targets the session and is JSON-normalized like FakeTmux.
        for e in effects:
            self.assertEqual(e["target"], "work")
            self.assertIn("option", e)

    def test_non_default_tmux_server_is_stripped(self):
        """With a non-empty tmux_server the watchdog prefixes every tmux call with
        `-L <server>`. The fake must strip that global option and serve the same
        default-server state — otherwise `-L` reads as the subcommand, the call
        fails loud, and _run aborts (which the rc check now raises on). The effect
        stream must be identical to the default-server run (server is normalized
        away, exactly like FakeTmux)."""
        default = run_bash_watchdog_fixture(_status_only_fixture(str(self.nogit)), self.root)
        alt_root = self.root / "alt"
        alt_root.mkdir()
        alt = run_bash_watchdog_fixture(
            _status_only_fixture(str(self.nogit), tmux_server="ae-alt"), alt_root
        )
        self.assertEqual(alt, default,
                         "non-default tmux_server must produce the same normalized effect stream")


if __name__ == "__main__":
    unittest.main()
