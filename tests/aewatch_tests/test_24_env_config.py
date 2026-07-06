"""Phase-3 Slice 2 contract: watchdog env config (`_env_int` / `WatchdogConfig.from_env`).

`from_env` is the REAL config source the phase-3 daemon loop (slice 16) reads at
startup — but nothing pins it yet. This slice characterizes the malformed / legacy /
empty / numeric matrix so a regression in the env parse is caught before the live
daemon depends on it.

Ported semantics (ae:7595-7607 — `${VAR:-${LEGACY:-N}}` for all six knobs, plus the
SWEEP numeric guard at ae:7608):
  - the first PRESENT (set and non-empty) `AE_WATCHDOG_*` / `AE_LOOP_*` name wins;
  - an unset OR empty name is SKIPPED (falls through to the next name, then default);
  - a set-but-non-numeric value normalizes to the DEFAULT (not the legacy name, not a
    crash). Bash instead raw-propagates the bad string (only SWEEP is guarded), and
    that divergence is a DELIBERATE, deferred normalization — pinned here so a future
    reader does not "fix" `_env_int` into bash's raw-propagation. `isdigit()` is the
    numeric gate, so negative / whitespaced / leading-`+` forms are non-numeric ->
    default, and a leading-zero decimal stays decimal (no octal reinterpretation).

Pure stdlib, no bash oracle — runs in the fast lane too.
"""

import unittest

from harness import AW

# The six bash defaults (ae:7595-7607). A drift here without a matching ae edit is a
# real divergence, so they are pinned as literals, not read back from the Python code.
_DEFAULTS = {
    "interval": 60,
    "stale_min": 15,
    "max_nudges": 2,
    "throttle_alert_cycles": 5,
    "tg_supervise_secs": 120,
    "sweep_secs": 300,
}

# knob field -> (AE_WATCHDOG_* primary name, AE_LOOP_* legacy name), from from_env.
_KNOBS = {
    "interval": ("AE_WATCHDOG_INTERVAL_SEC", "AE_LOOP_INTERVAL_SEC"),
    "stale_min": ("AE_WATCHDOG_STALE_MIN", "AE_LOOP_STALE_MIN"),
    "max_nudges": ("AE_WATCHDOG_MAX_NUDGES", "AE_LOOP_MAX_NUDGES"),
    "throttle_alert_cycles": ("AE_WATCHDOG_THROTTLE_ALERT_CYCLES", "AE_LOOP_THROTTLE_ALERT_CYCLES"),
    "tg_supervise_secs": ("AE_WATCHDOG_TG_SUPERVISE_SEC", "AE_LOOP_TG_SUPERVISE_SEC"),
    "sweep_secs": ("AE_WATCHDOG_SWEEP_SEC", "AE_LOOP_SWEEP_SEC"),
}

_PRIMARY = "AE_WATCHDOG_INTERVAL_SEC"
_LEGACY = "AE_LOOP_INTERVAL_SEC"


class EnvIntTest(unittest.TestCase):
    def _val(self, env, default=60):
        return AW._env_int(env, _PRIMARY, _LEGACY, default=default)

    def test_unset_uses_default(self):
        self.assertEqual(self._val({}), 60)

    def test_empty_primary_uses_default_when_no_legacy(self):
        # Empty is SKIPPED like unset (not treated as 0), so with no legacy -> default.
        self.assertEqual(self._val({_PRIMARY: ""}), 60)

    def test_numeric_primary_applied(self):
        self.assertEqual(self._val({_PRIMARY: "30"}), 30)

    def test_zero_is_a_real_value_not_default(self):
        # "0" is numeric and meaningful (e.g. SWEEP=0 disables sweeps) — never default.
        self.assertEqual(self._val({_PRIMARY: "0"}), 0)

    def test_legacy_used_when_primary_unset(self):
        self.assertEqual(self._val({_LEGACY: "45"}), 45)

    def test_legacy_used_when_primary_empty(self):
        # Empty primary falls THROUGH to the legacy name (mirrors bash `:-`).
        self.assertEqual(self._val({_PRIMARY: "", _LEGACY: "45"}), 45)

    def test_primary_wins_over_legacy(self):
        self.assertEqual(self._val({_PRIMARY: "30", _LEGACY: "45"}), 30)

    def test_nonnumeric_primary_normalizes_to_default(self):
        # DELIBERATE divergence from bash (which keeps "abc"): a set-but-non-numeric
        # value -> default, and it does NOT fall through to a valid legacy name.
        self.assertEqual(self._val({_PRIMARY: "abc"}), 60)
        self.assertEqual(self._val({_PRIMARY: "abc", _LEGACY: "45"}), 60)

    def test_negative_is_nonnumeric_so_default(self):
        self.assertEqual(self._val({_PRIMARY: "-5"}), 60)

    def test_whitespace_is_nonnumeric_so_default(self):
        self.assertEqual(self._val({_PRIMARY: " 30 "}), 60)

    def test_leading_plus_is_nonnumeric_so_default(self):
        # "+30".isdigit() is False (same class as negative / whitespace) -> default.
        self.assertEqual(self._val({_PRIMARY: "+30"}), 60)

    def test_leading_zero_stays_decimal(self):
        # int("0030") == 30 — NOT octal (bash `$((0030))` would be 24). Pinned so the
        # decimal normalization is intentional, not accidental.
        self.assertEqual(self._val({_PRIMARY: "0030"}), 30)


class FromEnvTest(unittest.TestCase):
    def test_empty_env_gives_bash_defaults(self):
        cfg = AW.WatchdogConfig.from_env({})
        for field, expected in _DEFAULTS.items():
            self.assertEqual(getattr(cfg, field), expected, f"default {field}")

    def test_every_knob_reads_its_watchdog_primary(self):
        # Distinct values per knob so a crossed-wire (reading the wrong name) is caught.
        env, expected = {}, {}
        for i, (field, (primary, _legacy)) in enumerate(_KNOBS.items()):
            val = 7 + i  # 7,8,9,... — each distinct and != every default
            env[primary] = str(val)
            expected[field] = val
        cfg = AW.WatchdogConfig.from_env(env)
        for field, val in expected.items():
            self.assertEqual(getattr(cfg, field), val, f"{field} from its AE_WATCHDOG_* name")

    def test_every_knob_falls_back_to_its_loop_legacy(self):
        # Legacy AE_LOOP_* wiring must exist for ALL six knobs, not just interval.
        env, expected = {}, {}
        for i, (field, (_primary, legacy)) in enumerate(_KNOBS.items()):
            val = 21 + i
            env[legacy] = str(val)
            expected[field] = val
        cfg = AW.WatchdogConfig.from_env(env)
        for field, val in expected.items():
            self.assertEqual(getattr(cfg, field), val, f"{field} from its AE_LOOP_* legacy name")

    def test_nonnumeric_overrides_fall_back_to_defaults(self):
        env = {primary: "garbage" for primary, _legacy in _KNOBS.values()}
        cfg = AW.WatchdogConfig.from_env(env)
        for field, expected in _DEFAULTS.items():
            self.assertEqual(getattr(cfg, field), expected, f"non-numeric {field} -> default")

    def test_sweep_guard_matches_bash(self):
        # SWEEP is the one knob bash explicitly guards (ae:7608). `_env_int`'s uniform
        # default-on-nonnumeric reproduces it: garbage -> 300, "0" -> 0 (disabled).
        self.assertEqual(AW.WatchdogConfig.from_env({"AE_WATCHDOG_SWEEP_SEC": "abc"}).sweep_secs, 300)
        self.assertEqual(AW.WatchdogConfig.from_env({"AE_WATCHDOG_SWEEP_SEC": "-5"}).sweep_secs, 300)
        self.assertEqual(AW.WatchdogConfig.from_env({"AE_WATCHDOG_SWEEP_SEC": "0"}).sweep_secs, 0)
        self.assertEqual(AW.WatchdogConfig.from_env({"AE_WATCHDOG_SWEEP_SEC": "600"}).sweep_secs, 600)

    def test_stale_secs_derives_from_stale_min(self):
        self.assertEqual(AW.WatchdogConfig.from_env({"AE_WATCHDOG_STALE_MIN": "3"}).stale_secs, 180)


if __name__ == "__main__":
    unittest.main()
