"""Phase-3 Slice 13 contract: outbound state.tsv + at-least-once retry.

Durable, byte-accurate tailing of each session's events.jsonl so outbound forwarding
neither replays history nor drops events across restart/failure (ae:3826-3934).

  OutboundState (ae:3826-3856): per-session (session_id -> inode, byte_offset) persisted
    to state.tsv; malformed rows skipped.
  process_session_events (ae:3875-3934): an UNSEEN session OR an INODE CHANGE jumps to
    EOF (never replay history / handle transfer+rotation); size<=offset is a no-op; else
    tail complete newline-terminated lines from the saved byte offset, forward each
    allowed event (s12), and advance the offset BYTE-accurately. A partial trailing line
    is ignored (retried when complete). AT-LEAST-ONCE: on a send failure the offset does
    NOT advance past the failed line (it + everything after retries next cycle); a
    filtered / empty / unparseable record carries no delivery obligation and advances.

Pure stdlib.
"""

import json
import tempfile
import unittest
from pathlib import Path

from harness import AW

_CFG = AW.TelegramConfig(enabled=True, chat_id="42", include="send,ask,alert")


def _line(**ev):
    return json.dumps(ev, separators=(",", ":")) + "\n"


class OutboundStateTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.path = Path(self._tmp.name) / "state.tsv"

    def test_load_empty(self):
        self.assertEqual(AW.OutboundState(self.path).load(), {})

    def test_save_load_roundtrip(self):
        s = AW.OutboundState(self.path)
        s.save({"sid-a": (111, 2048), "sid-b": (222, 0)})
        self.assertEqual(AW.OutboundState(self.path).load(), {"sid-a": (111, 2048), "sid-b": (222, 0)})

    def test_malformed_and_comment_rows_skipped(self):
        self.path.write_text("# session_id\tinode\tbyte_offset\tlast_ts\n"
                             "good\t5\t10\t\n"
                             "bad-not-enough-cols\n"
                             "nonnum\tx\ty\t\n")
        self.assertEqual(AW.OutboundState(self.path).load(), {"good": (5, 10)})


class ProcessSessionTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.events = Path(self._tmp.name) / "events.jsonl"
        self.events.write_text("")
        self.sent = []
        self.send_results = None  # None -> always ok

    def _send(self, chat_id, text):
        self.sent.append((chat_id, text))
        if self.send_results:
            return self.send_results.pop(0)
        return {"ok": True}

    def _inode(self):
        return self.events.stat().st_ino

    def _process(self, prev, recorder=None):
        return AW.process_session_events(self.events, prev, "work", _CFG, send=self._send, recorder=recorder)

    def test_unseen_session_jumps_to_eof_no_replay(self):
        self.events.write_text(_line(action="send", actor="a:b", summary="old1") + _line(action="send", actor="a:b", summary="old2"))
        new = self._process(None)  # unseen
        self.assertEqual(new, (self._inode(), self.events.stat().st_size), "unseen -> EOF")
        self.assertEqual(self.sent, [], "history is NOT replayed")

    def test_inode_change_jumps_to_eof(self):
        self.events.write_text(_line(action="send", actor="a:b", summary="x"))
        # a stale inode (transfer/rotation) -> jump to EOF, forward nothing this pass
        new = self._process((999999, 0))
        self.assertEqual(new, (self._inode(), self.events.stat().st_size))
        self.assertEqual(self.sent, [])

    def test_size_leq_offset_is_noop(self):
        self.events.write_text(_line(action="send", actor="a:b", summary="x"))
        size = self.events.stat().st_size
        self.assertEqual(self._process((self._inode(), size)), (self._inode(), size))
        self.assertEqual(self.sent, [])

    def test_same_inode_truncation_resets_to_new_eof(self):
        # If events.jsonl is truncated in place (same inode, size < stored offset), the
        # offset must reset to the new EOF — else it stalls beyond EOF forever (codex).
        self.events.write_text(_line(action="send", actor="a:b", summary="fresh"))
        small_size = self.events.stat().st_size
        new = self._process((self._inode(), 100_000))  # stored offset way past a smaller file
        self.assertEqual(new, (self._inode(), small_size), "reset to the new (smaller) EOF")
        self.assertEqual(self.sent, [], "no forward on a truncation reset")
        # and future growth from that reset offset IS picked up (no permanent stall).
        with self.events.open("a") as fh:
            fh.write(_line(action="send", actor="a:b", summary="after"))
        grown = self._process(new)
        self.assertEqual(len(self.sent), 1)

    def test_forwards_new_events_and_advances_byte_accurately(self):
        # start at EOF-of-empty, then append two events; both forward, offset -> new EOF.
        base = (self._inode(), 0)
        self.events.write_text(_line(action="send", actor="a:b", summary="one") + _line(action="ask", actor="c:d", summary="two"))
        new = self._process(base)
        self.assertEqual(len(self.sent), 2)
        self.assertEqual(new[1], self.events.stat().st_size, "offset advances to EOF")

    def test_partial_trailing_line_is_ignored(self):
        # a complete event + an unterminated partial write -> forward the complete one,
        # offset stops BEFORE the partial (retried when the newline arrives).
        complete = _line(action="send", actor="a:b", summary="ok")
        self.events.write_bytes(complete.encode() + b'{"action":"send","actor":"a:b","summ')
        new = self._process((self._inode(), 0))
        self.assertEqual(len(self.sent), 1)
        self.assertEqual(new[1], len(complete.encode()), "offset stops before the partial line")

    def test_send_failure_keeps_offset_for_retry(self):
        self.send_results = [{"ok": True}, {"ok": False, "error_code": 429}]  # 2nd send fails
        first = _line(action="send", actor="a:b", summary="one")
        second = _line(action="send", actor="a:b", summary="two")
        third = _line(action="send", actor="a:b", summary="three")
        self.events.write_text(first + second + third)
        new = self._process((self._inode(), 0))
        # first delivered (offset past it); second failed -> stop; third NOT attempted.
        self.assertEqual([t[1].splitlines()[-1] for t in self.sent], ["one", "two"])
        self.assertEqual(new[1], len(first.encode()), "offset advances only past the delivered event")

    def test_filtered_and_unparseable_advance_freely(self):
        # a filtered action, a non-JSON line, and a blank line carry no delivery
        # obligation -> no send, but the offset advances past all of them.
        content = (_line(action="recover", actor="h", summary="filtered")
                   + "not json at all\n"
                   + "\n"
                   + _line(action="send", actor="a:b", summary="deliver me"))
        self.events.write_text(content)
        new = self._process((self._inode(), 0))
        self.assertEqual(len(self.sent), 1, "only the allowed, parseable event forwards")
        self.assertEqual(new[1], self.events.stat().st_size, "everything advances to EOF")

    def test_utf8_multibyte_offset_is_byte_accurate(self):
        ev = _line(action="send", actor="a:b", summary="emoji 🙂🙂🙂")
        self.events.write_text(ev)
        new = self._process((self._inode(), 0))
        self.assertEqual(new[1], len(ev.encode("utf-8")), "offset counts BYTES, not codepoints")


if __name__ == "__main__":
    unittest.main()
