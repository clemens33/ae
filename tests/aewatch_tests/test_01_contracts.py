"""Slice 2 contract: the CONTRACTS.md fixture matrix + its loader/validator.

`contrib/aewatch/CONTRACTS.md` is human-readable prose, but the machine source of
truth is ONE JSON object between the literal markers below. This slice proves:
  - the marked JSON block extracts and parses,
  - the versioned schema holds (ids, required keys, expect.effects, families),
  - the loader is not vacuous (missing markers / invalid JSON are rejected),
  - `aewatch contracts validate` agrees on the committed contracts (exit 0).

Loader + validator live in the `aewatch` sidecar (single source of truth); this
test imports them from the extensionless script and also drives the CLI. Pure
stdlib; no network, no real ~/.ae.
"""

import importlib.machinery
import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
AEWATCH = REPO_ROOT / "contrib" / "aewatch" / "aewatch"
CONTRACTS = REPO_ROOT / "contrib" / "aewatch" / "CONTRACTS.md"

# Literal marker contract (kept in sync with the sidecar + CONTRACTS.md).
START = "<!-- AEWATCH_CONTRACTS_JSON_START -->"
END = "<!-- AEWATCH_CONTRACTS_JSON_END -->"

# Every behavior family has at least one REPRESENTATIVE fixture (s20 filled the phase-1
# placeholders); the validator + these guards keep later work from silently shrinking it.
REQUIRED_FAMILIES = (
    "session.discovery",
    "watchdog.status",
    "watchdog.nudge",
    "watchdog.alert",
    "watchdog.meta",
    "watchdog.telegram-supervise",
    "telegram.outbound",
    "telegram.inbound",
    "telegram.security",
    "telegram.command-menu",
    "daemon.runtime",
)


def load_aewatch():
    """Import the extensionless sidecar as a module (top-level main() is guarded)."""
    loader = importlib.machinery.SourceFileLoader("aewatch_sidecar", str(AEWATCH))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    mod = importlib.util.module_from_spec(spec)
    # Register before exec so @dataclass can resolve annotations under
    # `from __future__ import annotations` (sys.modules[cls.__module__]).
    sys.modules[loader.name] = mod
    loader.exec_module(mod)
    return mod


def marked(json_text: str) -> str:
    return f"prose before\n{START}\n```json\n{json_text}\n```\n{END}\nprose after\n"


class ContractsLoaderTest(unittest.TestCase):
    def setUp(self):
        self.aw = load_aewatch()

    def test_contracts_file_exists(self):
        self.assertTrue(CONTRACTS.is_file(), f"missing contracts matrix: {CONTRACTS}")

    def test_marked_block_extracts_and_parses(self):
        obj = self.aw.extract_contracts_json(CONTRACTS.read_text(encoding="utf-8"))
        self.assertIsInstance(obj, dict)
        self.assertEqual(obj.get("schema_version"), 1)
        self.assertIsInstance(obj.get("fixtures"), list)
        self.assertGreater(len(obj["fixtures"]), 0)

    def test_committed_contracts_validate_clean(self):
        obj = self.aw.extract_contracts_json(CONTRACTS.read_text(encoding="utf-8"))
        errors = self.aw.validate_contracts(obj)
        self.assertEqual(errors, [], f"committed contracts have errors: {errors}")

    def test_required_families_match_the_validator(self):
        # The test-side copy must not drift from the sidecar's source of truth.
        self.assertEqual(tuple(self.aw.REQUIRED_FIXTURE_FAMILIES), REQUIRED_FAMILIES)

    def test_all_required_families_have_a_fixture(self):
        obj = self.aw.extract_contracts_json(CONTRACTS.read_text(encoding="utf-8"))
        ids = [f["id"] for f in obj["fixtures"]]
        for fam in REQUIRED_FAMILIES:
            self.assertTrue(
                any(i == fam or i.startswith(fam + ".") for i in ids),
                f"no fixture for required family {fam!r}",
            )

    def test_validator_owns_family_coverage(self):
        # codex IMPORTANT: the CLI validator itself — not only the test above —
        # must reject a shrunk contract surface. Cover every family but one on an
        # otherwise-valid object and assert the missing family is NAMED.
        fixtures = [_min_fixture(fam + ".a") for fam in REQUIRED_FAMILIES if fam != "daemon.runtime"]
        errs = self.aw.validate_contracts({"schema_version": 1, "fixtures": fixtures})
        self.assertTrue(
            any("daemon.runtime" in e for e in errs),
            f"validator must name the missing required family: {errs}",
        )

    # ── non-vacuity: the validator must REJECT bad input ────────────────
    def test_missing_markers_rejected(self):
        with self.assertRaises(ValueError):
            self.aw.extract_contracts_json('{"schema_version": 1, "fixtures": []}')

    def test_invalid_json_rejected(self):
        with self.assertRaises(ValueError):
            self.aw.extract_contracts_json(marked("{ not valid json "))

    def test_wrong_schema_version_flagged(self):
        obj = {"schema_version": 2, "fixtures": []}
        self.assertTrue(self.aw.validate_contracts(obj), "schema_version!=1 must error")

    def test_duplicate_ids_flagged(self):
        obj = {
            "schema_version": 1,
            "fixtures": [_min_fixture("session.discovery.a"), _min_fixture("session.discovery.a")],
        }
        errs = self.aw.validate_contracts(obj)
        self.assertTrue(any("duplicate" in e.lower() for e in errs), errs)

    def test_bad_id_token_flagged(self):
        obj = {"schema_version": 1, "fixtures": [_min_fixture("Session.Discovery.UPPER")]}
        self.assertTrue(self.aw.validate_contracts(obj), "non-kebab/dotted id must error")

    def test_missing_expect_effects_flagged(self):
        fx = _min_fixture("session.discovery.a")
        del fx["expect"]["effects"]
        obj = {"schema_version": 1, "fixtures": [fx]}
        errs = self.aw.validate_contracts(obj)
        self.assertTrue(any("effects" in e.lower() for e in errs), errs)

    def test_non_object_expect_flagged(self):
        # codex IMPORTANT: a non-object `expect` must not satisfy the effects
        # contract by default. Require expect to be a dict before probing effects.
        fx = _min_fixture("session.discovery.a")
        fx["expect"] = "bad"
        errs = self.aw.validate_contracts({"schema_version": 1, "fixtures": [fx]})
        self.assertTrue(any("expect" in e for e in errs), errs)

    def test_non_list_sessions_flagged(self):
        # codex extension: the schema prose says sessions is a list — enforce it.
        fx = _min_fixture("session.discovery.a")
        fx["sessions"] = {}
        errs = self.aw.validate_contracts({"schema_version": 1, "fixtures": [fx]})
        self.assertTrue(any("sessions" in e for e in errs), errs)

    def test_non_object_time_config_telegram_flagged(self):
        # codex extension: time/config/telegram must be objects per the prose.
        for key in ("time", "config", "telegram"):
            fx = _min_fixture("session.discovery.a")
            fx[key] = "bad"
            errs = self.aw.validate_contracts({"schema_version": 1, "fixtures": [fx]})
            self.assertTrue(any(key in e for e in errs), f"{key}: {errs}")

    def test_multiple_fenced_blocks_rejected(self):
        # NIT hardening: exactly one json block between the markers; a stale or
        # duplicate matrix must fail fast rather than silently using the first.
        two = f"{START}\n```json\n{{}}\n```\n```json\n{{}}\n```\n{END}\n"
        with self.assertRaises(ValueError):
            self.aw.extract_contracts_json(two)

    # ── CLI parity: `aewatch contracts validate` agrees on committed file ─
    def test_cli_contracts_validate_exit_zero(self):
        proc = subprocess.run(
            [sys.executable, str(AEWATCH), "contracts", "validate"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(proc.returncode, 0, f"validate exited {proc.returncode}: {proc.stderr}")


def _min_fixture(fixture_id: str) -> dict:
    """A schema-complete fixture with only the keys the validator requires."""
    return {
        "id": fixture_id,
        "description": "x",
        "tags": [],
        "time": {"now": "2026-07-05T07:00:00Z", "epoch": 1783234800},
        "config": {"ae_home": "$TMP/ae", "ini": ""},
        "sessions": [],
        "telegram": {"enabled": False, "offset": 0, "state_tsv": ""},
        "expect": {"effects": [], "files": {}, "tmux_options": {}, "exit_code": 0, "log_contains": []},
    }


if __name__ == "__main__":
    unittest.main()
