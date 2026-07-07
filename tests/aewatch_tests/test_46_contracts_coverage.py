"""s20 (phase closer): the CONTRACTS.md coverage guards are MUTATION-PROVEN.

The phase gate is only worth running if a guard that cannot fail is not shipped as
decoration. For every guard added in s20 — family coverage, EFFECT_KIND coverage,
non-effect assertion coverage (command-menu setMyCommands, the s19 bridge handoff
order/killed-sessions), source anchors, and effect shape/kind — deleting the thing it
guards must make `validate_contracts` FAIL and NAME the gap. The committed matrix itself
validates clean.

Pure stdlib.
"""

import copy
import pathlib
import unittest

from harness import AW

_CONTRACTS = pathlib.Path(__file__).resolve().parents[2] / "contrib" / "aewatch" / "CONTRACTS.md"


def _committed():
    return AW.extract_contracts_json(_CONTRACTS.read_text(encoding="utf-8"))


class ContractsCoverageGuardsTest(unittest.TestCase):
    def setUp(self):
        self.obj = _committed()
        # The committed matrix must itself be clean — a guard is only meaningful against a
        # valid baseline.
        self.assertEqual(AW.validate_contracts(self.obj), [],
                         "the committed CONTRACTS.md must validate clean")

    def test_every_required_family_is_guarded(self):
        # Delete each required family's fixture(s) -> validate NAMES the missing family.
        for fam in AW.REQUIRED_FIXTURE_FAMILIES:
            o = copy.deepcopy(self.obj)
            o["fixtures"] = [f for f in o["fixtures"]
                             if not (f.get("id") == fam or str(f.get("id", "")).startswith(fam + "."))]
            errs = AW.validate_contracts(o)
            self.assertTrue(any(fam in e and "no fixture" in e for e in errs),
                            f"deleting family {fam!r} must fail contracts validate; got {errs}")

    def test_every_effect_kind_is_guarded(self):
        # Strip each EFFECT_KIND from every fixture -> validate NAMES the unrepresented kind.
        for kind in AW.EFFECT_KINDS:
            o = copy.deepcopy(self.obj)
            for f in o["fixtures"]:
                exp = f.get("expect")
                if isinstance(exp, dict) and isinstance(exp.get("effects"), list):
                    exp["effects"] = [e for e in exp["effects"] if e.get("kind") != kind]
            errs = AW.validate_contracts(o)
            self.assertTrue(any(kind in e and "no representative" in e for e in errs),
                            f"removing all {kind!r} effects must fail; got {errs}")

    def test_command_menu_non_effect_assertion_is_guarded(self):
        o = copy.deepcopy(self.obj)
        for f in o["fixtures"]:
            if str(f.get("id", "")).startswith("telegram.command-menu"):
                f["expect"].pop("telegram_commands", None)
        errs = AW.validate_contracts(o)
        self.assertTrue(any("telegram_commands" in e for e in errs),
                        f"dropping command-menu's telegram_commands must fail; got {errs}")

    def test_bridge_handoff_non_effect_assertions_are_guarded(self):
        for key in ("handoff_order", "killed_sessions"):
            o = copy.deepcopy(self.obj)
            for f in o["fixtures"]:
                if str(f.get("id", "")).startswith("daemon.runtime.bridge-handoff"):
                    f["expect"].pop(key, None)
            errs = AW.validate_contracts(o)
            self.assertTrue(any(key in e for e in errs),
                            f"dropping bridge-handoff's {key} must fail; got {errs}")

    def test_source_anchor_is_guarded(self):
        o = copy.deepcopy(self.obj)
        o["fixtures"][1].pop("source", None)
        errs = AW.validate_contracts(o)
        self.assertTrue(any("source" in e for e in errs), f"dropping a fixture source must fail; got {errs}")

    def test_bad_source_anchor_is_rejected(self):
        o = copy.deepcopy(self.obj)
        o["fixtures"][1]["source"] = ["not-an-anchor"]
        errs = AW.validate_contracts(o)
        self.assertTrue(any("source anchor" in e for e in errs), f"a malformed anchor must fail; got {errs}")

    def test_unknown_effect_kind_is_rejected(self):
        o = copy.deepcopy(self.obj)
        o["fixtures"][1]["expect"]["effects"].append({"kind": "tmux.teleport"})
        errs = AW.validate_contracts(o)
        self.assertTrue(any("unknown effect kind" in e for e in errs), f"an unknown kind must fail; got {errs}")

    def test_gate_no_longer_blind_to_a_missing_bridge_family(self):
        # The s20 RED: pre-guard, a matrix missing the command-menu family validated CLEAN
        # (the gate was blind). Post-guard, it MUST be named.
        o = copy.deepcopy(self.obj)
        o["fixtures"] = [f for f in o["fixtures"]
                         if not str(f.get("id", "")).startswith("telegram.command-menu")]
        errs = AW.validate_contracts(o)
        self.assertTrue(errs, "a missing bridge family must not validate clean — the gate is no longer blind")

    def test_aewatch_source_anchor_is_accepted_for_python_only_behavior(self):
        # The s19 handoff has no bash twin; its aewatch:NNNN implementation anchor must validate.
        o = copy.deepcopy(self.obj)
        for f in o["fixtures"]:
            if str(f.get("id", "")).startswith("daemon.runtime.bridge-handoff"):
                self.assertTrue(any(a.startswith("aewatch:") for a in f["source"]),
                                "the handoff must carry an aewatch: implementation anchor")
        self.assertEqual(AW.validate_contracts(o), [])


    def test_command_menu_malformed_telegram_commands_rejected(self):
        # B1: truthiness is not enough — a non-list / list-of-non-objects must FAIL.
        for bad in ("not-a-list", [], ["list"], [{"description": "no command key"}], [{"command": "x"}], [{"command": "x", "description": ""}]):
            o = copy.deepcopy(self.obj)
            for f in o["fixtures"]:
                if str(f.get("id", "")).startswith("telegram.command-menu"):
                    f["expect"]["telegram_commands"] = bad
            errs = AW.validate_contracts(o)
            self.assertTrue(any("telegram_commands" in e for e in errs),
                            f"telegram_commands={bad!r} must fail; got {errs}")

    def test_handoff_wrong_order_rejected(self):
        # B1: handoff_order must be EXACTLY the canonical no-double-send sequence.
        for bad in (["send"], ["stop-bash", "bridge-owner-marker", "send"], "not-a-list", []):
            o = copy.deepcopy(self.obj)
            for f in o["fixtures"]:
                if str(f.get("id", "")).startswith("daemon.runtime.bridge-handoff"):
                    f["expect"]["handoff_order"] = bad
            errs = AW.validate_contracts(o)
            self.assertTrue(any("handoff_order" in e for e in errs),
                            f"handoff_order={bad!r} must fail; got {errs}")

    def test_killed_sessions_must_contain_ae_telegram(self):
        # B1: killed_sessions must be a list of strings containing 'ae-telegram'.
        for bad in ("not-a-list", [], ["other-session"], [123]):
            o = copy.deepcopy(self.obj)
            for f in o["fixtures"]:
                if str(f.get("id", "")).startswith("daemon.runtime.bridge-handoff"):
                    f["expect"]["killed_sessions"] = bad
            errs = AW.validate_contracts(o)
            self.assertTrue(any("killed_sessions" in e for e in errs),
                            f"killed_sessions={bad!r} must fail; got {errs}")


if __name__ == "__main__":
    unittest.main()
