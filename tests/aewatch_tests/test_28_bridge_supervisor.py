"""Phase-3 Slice 6 contract: real BridgeSupervisor boundary.

Phase 2's supervise seam was a fixture lambda that only recorded telegram.supervise.
The live daemon (slice 16) binds a real BridgeSupervisor.ensure_started as the
supervise_bridge seam. It records the SAME telegram.supervise observation, then
best-effort revives the machine-global bridge by shelling the legacy
`ae telegram _supervise` — propagating this session's tmux server as AE_TMUX_SERVER
(ae:7986-7995) so the bridge is checked on the SAME server the session uses.

Behaviors pinned: tmux-server propagation, best-effort failure logging (a nonzero
exit / spawn failure warns through a redacted log.write and never raises), and a full
no-op (no record, no shell) when the bridge is disabled or there is no executable
ae to revive it. Proven against a temp ae helper that records its argv + AE_TMUX_SERVER.
Pure stdlib.
"""

import json
import stat
import tempfile
import textwrap
import unittest
import unittest.mock
from pathlib import Path

from harness import AW

# A temp `ae` that records its argv + the propagated AE_TMUX_SERVER, then exits with a
# baked code (to exercise best-effort failure logging).
_FAKE_AE = textwrap.dedent('''\
    #!/usr/bin/env python3
    import json, os, sys
    from pathlib import Path
    Path("__RECORD__").write_text(json.dumps({
        "argv": sys.argv[1:],
        "tmux_server": os.environ.get("AE_TMUX_SERVER", "<unset>"),
    }))
    sys.exit(__EXIT__)
''')


class BridgeSupervisorTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.record = self.root / "ae-call.json"

    def _fake_ae(self, exit_code=0):
        path = self.root / "fake-ae"
        path.write_text(_FAKE_AE.replace("__RECORD__", str(self.record)).replace("__EXIT__", str(exit_code)))
        path.chmod(path.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
        return path

    def _call(self):
        return json.loads(self.record.read_text()) if self.record.exists() else None

    def test_records_supervise_and_shells_with_server(self):
        rec = AW.EffectRecorder()
        sup = AW.BridgeSupervisor(rec, ae_path=str(self._fake_ae()))
        sup.ensure_started("ae-srv")
        self.assertEqual(rec.as_list()[0],
                         {"kind": "telegram.supervise", "tmux_server": "ae-srv"})
        call = self._call()
        self.assertEqual(call["argv"], ["telegram", "_supervise"])
        self.assertEqual(call["tmux_server"], "ae-srv", "the session's server propagates as AE_TMUX_SERVER")

    def test_empty_server_omits_env(self):
        rec = AW.EffectRecorder()
        sup = AW.BridgeSupervisor(rec, ae_path=str(self._fake_ae()))
        sup.ensure_started("")
        self.assertEqual(rec.as_list()[0], {"kind": "telegram.supervise", "tmux_server": ""})
        self.assertEqual(self._call()["tmux_server"], "<unset>",
                         "no server -> AE_TMUX_SERVER stays unset (ae default server)")

    def test_empty_server_does_not_leak_inherited_parent_env(self):
        # The MAIN footgun (codex): even if the daemon's ambient env carries a stale
        # AE_TMUX_SERVER, an empty-server revive must hit the DEFAULT server — the
        # supervisor pops it from the child env, not inherits it.
        import os
        rec = AW.EffectRecorder()
        sup = AW.BridgeSupervisor(rec, ae_path=str(self._fake_ae()))
        with unittest.mock.patch.dict(os.environ, {"AE_TMUX_SERVER": "leaked-srv"}):
            sup.ensure_started("")
        self.assertEqual(self._call()["tmux_server"], "<unset>",
                         "an inherited AE_TMUX_SERVER must NOT leak into the default-server revive")

    def test_nonzero_exit_logs_warning_and_does_not_raise(self):
        rec = AW.EffectRecorder()
        sup = AW.BridgeSupervisor(rec, ae_path=str(self._fake_ae(exit_code=1)))
        sup.ensure_started("ae-srv")  # must not raise
        kinds = [e["kind"] for e in rec.as_list()]
        self.assertEqual(kinds, ["telegram.supervise", "log.write"],
                         "the attempt is recorded; the failure is logged best-effort")
        self.assertEqual(rec.as_list()[1]["level"], "WARNING")

    def test_noop_when_disabled(self):
        rec = AW.EffectRecorder()
        sup = AW.BridgeSupervisor(rec, ae_path=str(self._fake_ae()), enabled=False)
        sup.ensure_started("ae-srv")
        self.assertEqual(rec.as_list(), [], "disabled -> no supervise record")
        self.assertIsNone(self._call(), "disabled -> no shell")

    def test_noop_when_ae_path_absent(self):
        rec = AW.EffectRecorder()
        sup = AW.BridgeSupervisor(rec, ae_path=None)
        sup.ensure_started("ae-srv")
        self.assertEqual(rec.as_list(), [], "no ae to revive the bridge -> no record")

    def test_noop_when_ae_path_not_executable(self):
        plain = self.root / "plain"
        plain.write_text("#!/bin/sh\n")  # exists, NOT chmod +x
        rec = AW.EffectRecorder()
        sup = AW.BridgeSupervisor(rec, ae_path=str(plain))
        sup.ensure_started("ae-srv")
        self.assertEqual(rec.as_list(), [], "non-executable ae -> no record, no shell")
        self.assertIsNone(self._call())

    def test_ensure_started_is_the_supervise_bridge_seam(self):
        # The daemon binds supervise_bridge = BridgeSupervisor.ensure_started; the cycle
        # calls supervise_bridge(tmux_server). Prove the bound method has that shape.
        rec = AW.EffectRecorder()
        supervise_bridge = AW.BridgeSupervisor(rec, ae_path=str(self._fake_ae())).ensure_started
        supervise_bridge("ae-srv")
        self.assertEqual(rec.as_list()[0]["kind"], "telegram.supervise")


if __name__ == "__main__":
    unittest.main()
