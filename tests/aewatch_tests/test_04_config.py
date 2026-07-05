"""Slice 5 contract: an ae-compatible INI parser (parity port).

Mirrors ae's own `parse_config` regex semantics exactly, so aewatch reads the
SAME config ae does:
  - sections [a-zA-Z_-]+; key [a-zA-Z_][a-zA-Z0-9_-]*; spaces allowed around '='.
  - double-quoted values keep their content verbatim (NO comment stripping, so a
    '#' inside quotes survives).
  - unquoted values strip an inline '#...' comment, then right-trim.
  - duplicate keys: last one wins (ae's effective get_config behavior).
  - full-line comments and any non-matching line are ignored.
It is deliberately NOT a general TOML/YAML parser: every value is a string.

Pure stdlib; isolated (load_config reads only the given AE_HOME).
"""

import tempfile
import unittest
from pathlib import Path

from harness import AW, FakeAeHome


class ConfigParserTest(unittest.TestCase):
    def parse(self, text):
        return AW.parse_ae_ini(text)

    def test_sections_and_simple_keys(self):
        cfg = self.parse("[telegram]\nenabled = true\n[workspace]\nmain = dummy\n")
        self.assertEqual(cfg["telegram"]["enabled"], "true")
        self.assertEqual(cfg["workspace"]["main"], "dummy")

    def test_quoted_value_has_quotes_stripped(self):
        cfg = self.parse('[agents]\ndummy = "bash"\n')
        self.assertEqual(cfg["agents"]["dummy"], "bash")

    def test_hash_inside_quotes_is_preserved(self):
        # Done criterion: a '#' inside a quoted value must NOT be treated as a comment.
        cfg = self.parse('[prompt]\ninstructions = "use # sparingly, and pass --flag"\n')
        self.assertEqual(cfg["prompt"]["instructions"], "use # sparingly, and pass --flag")

    def test_inline_comment_stripped_on_unquoted_value(self):
        cfg = self.parse("[workspace]\nmain = dummy   # the lead agent\n")
        self.assertEqual(cfg["workspace"]["main"], "dummy")

    def test_full_line_comment_and_blank_lines_ignored(self):
        cfg = self.parse("# a comment\n\n[workspace]\n\nmain = dummy\n# trailing\n")
        self.assertEqual(cfg, {"workspace": {"main": "dummy"}})

    def test_whitespace_around_equals_and_line_trimmed(self):
        cfg = self.parse("[workspace]\n   layout    =    vertical   \n")
        self.assertEqual(cfg["workspace"]["layout"], "vertical")

    def test_key_without_spaces_around_equals(self):
        cfg = self.parse("[telegram]\nenabled=true\n")
        self.assertEqual(cfg["telegram"]["enabled"], "true")

    def test_duplicate_key_last_wins(self):
        cfg = self.parse("[workspace]\nmain = first\nmain = second\n")
        self.assertEqual(cfg["workspace"]["main"], "second")

    def test_quoted_value_with_spaces_and_metachars(self):
        cfg = self.parse('[agents]\nclaude = "claude --model opus --flag"\n')
        self.assertEqual(cfg["agents"]["claude"], "claude --model opus --flag")

    def test_not_a_toml_parser_values_stay_strings(self):
        cfg = self.parse("[x]\nnum = 123\narr = [1, 2, 3]\nbool = true\n")
        self.assertEqual(cfg["x"]["num"], "123")      # not an int
        self.assertEqual(cfg["x"]["arr"], "[1, 2, 3]")  # not a list
        self.assertEqual(cfg["x"]["bool"], "true")     # not a bool

    def test_non_matching_lines_ignored(self):
        cfg = self.parse("[workspace]\nnot a valid line\n: also not\nmain = dummy\n")
        self.assertEqual(cfg, {"workspace": {"main": "dummy"}})

    def test_pre_section_keys_land_in_empty_bucket(self):
        # codex parity: ae emits ".key=value" (section="") for keys before any
        # section header — a plausible Python INI parser would reject or throw.
        cfg = self.parse("foo = bar\n[x]\na = b\n")
        self.assertEqual(cfg, {"": {"foo": "bar"}, "x": {"a": "b"}})

    def test_section_name_hyphen_underscore_ok_digits_rejected(self):
        # codex parity: ae's section regex is exactly [a-zA-Z_-]+ (NO digits).
        self.assertIn("work-space", self.parse("[work-space]\na = 1\n"))
        self.assertIn("work_space", self.parse("[work_space]\na = 1\n"))
        # a digit-bearing header is not a valid section -> ignored, key falls to "".
        cfg = self.parse("[work2]\na = 1\n")
        self.assertNotIn("work2", cfg)
        self.assertEqual(cfg.get(""), {"a": "1"})

    def test_tab_around_equals_is_not_accepted(self):
        # codex parity NIT: ae allows only literal spaces around '=' (' *'), never
        # tabs — a \s*-based parser would silently diverge and accept this.
        self.assertEqual(self.parse("[workspace]\nmain = dummy\n"), {"workspace": {"main": "dummy"}})
        self.assertEqual(self.parse("[workspace]\nmain\t= dummy\n"), {})

    def test_final_line_without_trailing_newline_is_dropped(self):
        # codex parity: ae reads with `while IFS= read -r line`, which does NOT
        # process a final line that has no trailing newline. Mirror it so the
        # phase-2 bash-vs-python diff has no hidden exception.
        self.assertEqual(self.parse("[x]\nk = v\n"), {"x": {"k": "v"}})  # terminated -> parsed
        self.assertEqual(self.parse("[x]\nk = v"), {})                   # unterminated -> dropped
        self.assertEqual(self.parse("[x]\nk = v\nj = w"), {"x": {"k": "v"}})  # only the last dropped

    def test_load_config_reads_ae_home_config(self):
        with tempfile.TemporaryDirectory() as d:
            home = FakeAeHome(Path(d))
            home.write_config("[telegram]\nenabled = true\n")
            cfg = AW.load_config(home.home)
            self.assertEqual(cfg["telegram"]["enabled"], "true")

    def test_load_config_missing_file_is_empty(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(AW.load_config(Path(d)), {})


if __name__ == "__main__":
    unittest.main()
