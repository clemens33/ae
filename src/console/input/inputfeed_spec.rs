//! Frozen characterisation of `Keys::feed` (slice inputfeed, #300).
//!
//! Navigator inputfeed-nav (Meta Muse Spark), frozen against base 83b518a2 and
//! driver inputfeed's plan r2 (`.local/inputfeed/plan.md`); lead spec-gates.
//! Driver installs byte-identically as `src/console/input/inputfeed_spec.rs`,
//! wired by `#[cfg(test)] mod inputfeed_spec;` under the `imgtoken_spec` mod.
//! Behaviour unchanged is the invariant: same `(Key, Instant)` sequence for
//! every byte stream, every chunk split and every stamp pattern.
//!
//! Oracle rule, per row: documented terminal protocol where it speaks (xterm
//! ctlseqs, ECMA-48 CSI finals `0x40..=0x7e` and SS3, DECSET 2004 bracketed
//! paste, SGR 1006 and X10 mouse), the `Keys`/`Key` doc comments where they
//! speak (stamps, app-only keys, paste literalness), else the base output,
//! labelled `behaviour pin`. Bug-shaped behaviour is pinned, never fixed.
//!
//! Mutant notes for the later rounds: the paste-open `lf_skip` reset
//! (input.rs `:235`) is load-bearing (see `lf_skip_reset_between_pastes`);
//! the paste-close reset (`:185`) is equivalent-given-open — every `literal`
//! read sits under a paste opened at `:235` — and the driver drops it in 2c
//! with that re-proof. The `held.len() > 1` guard (`:188`) is dead per the
//! plan's I5 and goes in 2c. This file holds no tmux-format literal, no
//! child process, no world read and no lint relaxation; it holds no
//! test-module block, so the floor/doors scans read it whole as
//! product-shaped text.
//!
//! What this file pins that the older tests do not: the begun stamp carried
//! across chunks, the app `moving` table at every split, the double-ESC
//! chat/app split, the 16-held/17-dropped and resync-stamp bounds, x10
//! ESC-swallow against mouse ESC-resync, `begun()` in every state, the idle
//! bound from both sides, the chat/app control and report tables, and the
//! every-split property over adversarial streams.

use super::{ESC_IDLE, Effect, Input, Key, Keys, Mouse, MouseKind, Reading};
use crate::console::submit::Draft;
use std::time::{Duration, Instant};

/// `ms` after `base`.
fn at(base: Instant, ms: u64) -> Instant {
    base + Duration::from_millis(ms)
}

/// A zero-based SGR click, the report `ESC [ < 0 ; 1 ; 1 M` decodes to.
fn click() -> Key {
    Key::Mouse(Mouse {
        kind: MouseKind::Click,
        column: 0,
        row: 0,
    })
}

/// `out` with stamps dropped and adjacent runs of the same literal kind
/// joined, so a chunk split that only breaks runs compares equal.
fn norm(out: &[(Key, Instant)]) -> Vec<Key> {
    let mut normed: Vec<Key> = Vec::new();
    for (key, _) in out {
        match (normed.last_mut(), key) {
            (Some(Key::Text(run)), Key::Text(more))
            | (Some(Key::Pasted(run)), Key::Pasted(more)) => run.extend(more.iter()),
            _ => normed.push(key.clone()),
        }
    }
    normed
}

/// A fresh decoder, chat or app.
fn decoder(app: bool) -> Keys {
    if app { Keys::app() } else { Keys::default() }
}

#[test]
fn escape_begun_stamp_carried_across_chunks() {
    // Oracle: `Keys` docs — a key carries the stamp of the read holding its
    // first byte.
    for app in [false, true] {
        let base = Instant::now();
        let mut keys = decoder(app);
        assert!(keys.feed(b"\x1b", base).is_empty());
        assert_eq!(keys.begun(), Some(base));
        assert_eq!(keys.feed(b"[D", at(base, 7)), [(Key::Left, base)]);
        assert_eq!(keys.begun(), None, "app={app}");
    }
}

#[test]
fn paste_content_carries_open_stamp_across_chunks() {
    // Oracle: `Keys` docs — paste text carries the read holding the paste's
    // opening ESC.
    for app in [false, true] {
        let base = Instant::now();
        let mut keys = decoder(app);
        let literal = |bytes: &[u8]| {
            if app {
                Key::Pasted(bytes.to_vec())
            } else {
                Key::Text(bytes.to_vec())
            }
        };
        assert_eq!(keys.feed(b"\x1b[200~ab", base), [(literal(b"ab"), base)]);
        assert_eq!(
            keys.feed(b"cd\x1b[201~", at(base, 3)),
            [(literal(b"cd"), base)],
            "app={app}"
        );
        assert_eq!(
            keys.feed(b"e", at(base, 5)),
            [(Key::Text(b"e".to_vec()), at(base, 5))],
            "app={app}: typed text after a paste keeps its own stamp"
        );
        // The opener itself split across reads: content carries the read
        // holding the opening ESC, not the one holding the opener end.
        let mut keys = decoder(app);
        assert!(keys.feed(b"\x1b[20", base).is_empty(), "app={app}");
        assert_eq!(
            keys.feed(b"0~ab", at(base, 2)),
            [(literal(b"ab"), base)],
            "app={app}"
        );
    }
}

#[test]
fn runs_split_by_origin_within_one_read() {
    // Oracle: behaviour pin — the merge compares origins, so paste text an
    // earlier read opened never joins text this read holds, even in one
    // `feed` call. Chat joins Text to Text, the app Pasted to Pasted.
    let base = Instant::now();
    let mut chat = Keys::default();
    assert!(chat.feed(b"\x1b[200~", base).is_empty());
    assert_eq!(
        chat.feed(b"c\x1b[201~d", at(base, 1)),
        [
            (Key::Text(b"c".to_vec()), base),
            (Key::Text(b"d".to_vec()), at(base, 1)),
        ]
    );
    let mut app = Keys::app();
    assert_eq!(
        app.feed(b"\x1b[200~a", base),
        [(Key::Pasted(b"a".to_vec()), base)]
    );
    assert_eq!(
        app.feed(b"b\x1b[201~\x1b[200~c\x1b[201~", at(base, 1)),
        [
            (Key::Pasted(b"b".to_vec()), base),
            (Key::Pasted(b"c".to_vec()), at(base, 1)),
        ]
    );
}

#[test]
fn text_splits_into_per_chunk_runs() {
    // Oracle: `Keys` docs — runs merge by stamp within one `feed` call only.
    for app in [false, true] {
        let base = Instant::now();
        let mut keys = decoder(app);
        assert_eq!(keys.feed(b"ab", base), [(Key::Text(b"ab".to_vec()), base)]);
        assert_eq!(
            keys.feed(b"cd", at(base, 1)),
            [(Key::Text(b"cd".to_vec()), at(base, 1))],
            "app={app}"
        );
    }
}

#[test]
fn mouse_report_carries_begun_stamp() {
    // Oracle: behaviour pin — the SGR report's key carries the read holding
    // its opening ESC.
    let base = Instant::now();
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b[<0;1", base).is_empty());
    assert_eq!(app.feed(b";1M", at(base, 4)), [(click(), base)]);
}

#[test]
fn x10_tail_bytes_then_text_stamp() {
    // Oracle: behaviour pin — the legacy report eats three payload bytes and
    // the byte after it keeps its own read's stamp.
    let base = Instant::now();
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b[Mab", base).is_empty());
    assert_eq!(
        app.feed(b"cq", at(base, 2)),
        [(Key::Text(b"q".to_vec()), at(base, 2))]
    );
    assert_eq!(app.begun(), None);
}

#[test]
fn app_moving_spellings_every_split() {
    // Oracle: protocol (xterm cursor keys in normal and application mode,
    // PageUp/PageDown; SS3 one byte) for the spellings, `Keys` docs for
    // app-only.
    let rows: [(&[u8], Key); 9] = [
        (b"\x1b[A", Key::Up),
        (b"\x1bOA", Key::Up),
        (b"\x1b[B", Key::Down),
        (b"\x1bOB", Key::Down),
        (b"\x1b[5~", Key::PageUp),
        (b"\x1b[6~", Key::PageDown),
        (b"\x1b[D", Key::Left),
        (b"\x1bOD", Key::Left),
        (b"\x1b[3~", Key::Delete),
    ];
    for (seq, key) in rows {
        let stream = [b"a".as_slice(), seq, b"b"].concat();
        let want = [Key::Text(b"a".to_vec()), key, Key::Text(b"b".to_vec())];
        for cut in 0..=stream.len() {
            let stamp = Instant::now();
            let mut keys = Keys::app();
            let mut out = keys.feed(&stream[..cut], stamp);
            out.extend(keys.feed(&stream[cut..], stamp));
            assert_eq!(norm(&out), want, "{seq:?} cut at {cut}");
        }
    }
}

#[test]
fn chat_double_esc_drops_and_types_on() {
    // Oracle: behaviour pin — `editing` docs say any other sequence is no
    // key, so `ESC ESC` vanishes and later bytes type as usual.
    let base = Instant::now();
    let mut chat = Keys::default();
    assert!(chat.feed(b"\x1b\x1b", base).is_empty());
    assert_eq!(chat.begun(), None);
    assert_eq!(
        chat.feed(b"q", at(base, 1)),
        [(Key::Text(b"q".to_vec()), at(base, 1))]
    );
    let mut chat = Keys::default();
    assert_eq!(
        chat.feed(b"\x1b\x1b[A", base),
        [(Key::Text(b"[A".to_vec()), base)]
    );
    let mut chat = Keys::default();
    assert!(chat.feed(b"\x1b", base).is_empty());
    assert_eq!(chat.begun(), Some(base));
    assert!(chat.feed(b"\x1b", at(base, 2)).is_empty());
    assert_eq!(chat.begun(), None);
}

#[test]
fn app_double_esc_emits_escape_and_reholds() {
    // Oracle: behaviour pin — nothing continues an escape with a second one,
    // so the first was a lone Esc and the second opens a new sequence.
    let base = Instant::now();
    let mut app = Keys::app();
    assert_eq!(app.feed(b"\x1b\x1b", base), [(Key::Escape, base)]);
    assert_eq!(app.begun(), Some(base));
    assert_eq!(app.feed(b"[A", at(base, 1)), [(Key::Up, base)]);
    let mut app = Keys::app();
    assert_eq!(
        app.feed(b"\x1b\x1b\x1b", base),
        [(Key::Escape, base), (Key::Escape, base)]
    );
    assert_eq!(app.begun(), Some(base));
    assert_eq!(
        app.idle(at(base, 50)),
        [(Key::Escape, base)],
        "the third ESC expires on idle"
    );
    assert_eq!(app.begun(), None);
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b", base).is_empty());
    assert_eq!(
        app.feed(b"\x1b[A", at(base, 3)),
        [(Key::Escape, base), (Key::Up, at(base, 3))]
    );
}

#[test]
fn sixteen_open_bytes_held_seventeenth_drops() {
    // Oracle: behaviour pin — `PENDING_MAX` docs: a sequence still open past
    // 16 bytes is dropped whole.
    for app in [false, true] {
        let open16 = [b"\x1b[".as_slice(), &[b'1'; 14]].concat();
        assert_eq!(open16.len(), 16);
        let base = Instant::now();
        let mut keys = decoder(app);
        assert!(keys.feed(&open16, base).is_empty(), "app={app}");
        assert_eq!(keys.begun(), Some(base), "app={app}: 16 held");
        assert!(keys.feed(b"m", at(base, 1)).is_empty(), "app={app}");
        assert_eq!(keys.begun(), None, "app={app}: a final ends it");
        let mut keys = decoder(app);
        assert!(keys.feed(&open16, base).is_empty());
        assert!(keys.feed(b"1", at(base, 1)).is_empty(), "app={app}");
        assert_eq!(keys.begun(), None, "app={app}: the 17th drops it whole");
        assert_eq!(
            keys.feed(b"z", at(base, 2)),
            [(Key::Text(b"z".to_vec()), at(base, 2))],
            "app={app}"
        );
        let open17 = [b"\x1b[".as_slice(), &[b'1'; 15]].concat();
        let mut keys = decoder(app);
        assert!(keys.feed(&open17, base).is_empty(), "app={app}");
        assert_eq!(keys.begun(), None, "app={app}");
    }
}

#[test]
fn mouse_resync_restamps_begun() {
    // Oracle: behaviour pin — a fresh escape resynchronizes a damaged report
    // and the new sequence carries the current read's stamp, even out of a
    // discarded report.
    let base = Instant::now();
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b[<0;1", base).is_empty());
    assert_eq!(app.begun(), Some(base));
    assert_eq!(app.feed(b"\x1b[A", at(base, 6)), [(Key::Up, at(base, 6))]);
    assert_eq!(app.begun(), None);
    let overlong = [b"\x1b[<".as_slice(), &[b'9'; 37]].concat();
    let mut app = Keys::app();
    assert!(app.feed(&overlong, base).is_empty());
    assert_eq!(app.begun(), None, "discarded reports read no begun");
    assert_eq!(app.feed(b"\x1b[A", at(base, 6)), [(Key::Up, at(base, 6))]);
}

#[test]
fn x10_swallows_esc_while_mouse_resyncs() {
    // Oracle: behaviour pin — legacy payload bytes are consumed even when
    // they are ESC; an SGR report instead resynchronizes on a fresh escape.
    let base = Instant::now();
    let mut app = Keys::app();
    assert_eq!(
        app.feed(b"\x1b[M\x1b[Dq", base),
        [(Key::Text(b"q".to_vec()), base)]
    );
    assert_eq!(app.begun(), None);
    let mut app = Keys::app();
    assert_eq!(app.feed(b"\x1b[<0;\x1b[D", base), [(Key::Left, base)]);
}

#[test]
fn x10_end_releases_next_sequence() {
    // Oracle: behaviour pin — once the third payload byte is eaten the next
    // escape opens fresh with its own begun stamp.
    let base = Instant::now();
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b[Mab", base).is_empty());
    assert_eq!(app.begun(), Some(base));
    assert_eq!(
        app.feed(b"c\x1b[D", at(base, 2)),
        [(Key::Left, at(base, 2))]
    );
    assert_eq!(app.begun(), None);
}

#[test]
fn app_paste_holds_escape_bytes_literal() {
    // Oracle: `Keys` docs — a paste is literal until its own close; only the
    // close itself is held back.
    let base = Instant::now();
    let mut app = Keys::app();
    assert_eq!(
        app.feed(b"\x1b[200~\x1b[D\x1b[201~", base),
        [(Key::Pasted(b"\x1b[D".to_vec()), base)]
    );
}

#[test]
fn begun_tracks_open_sequence() {
    // Oracle: `begun` docs — the stamp of the read that began the waiting
    // sequence, none for a discarded report; plus behaviour pins for x10,
    // SGR and paste states.
    let base = Instant::now();
    let mut app = Keys::app();
    assert_eq!(app.begun(), None);
    assert!(app.feed(b"", base).is_empty());
    assert!(app.feed(b"\x1b", base).is_empty());
    assert_eq!(app.begun(), Some(base));
    assert_eq!(app.feed(b"[D", at(base, 1)), [(Key::Left, base)]);
    assert_eq!(app.begun(), None);
    assert!(app.feed(b"\x1b[Ma", at(base, 2)).is_empty());
    assert_eq!(app.begun(), Some(at(base, 2)));
    assert!(app.feed(b"bc", at(base, 3)).is_empty());
    assert_eq!(app.begun(), None);
    assert!(app.feed(b"\x1b[<0;1", at(base, 4)).is_empty());
    assert_eq!(app.begun(), Some(at(base, 4)));
    assert_eq!(app.feed(b";1M", at(base, 5)), [(click(), at(base, 4))]);
    assert_eq!(app.begun(), None);
    let overlong = [b"\x1b[<".as_slice(), &[b'9'; 37]].concat();
    assert!(app.feed(&overlong, at(base, 6)).is_empty());
    assert_eq!(app.begun(), None);
    assert!(app.feed(b"M", at(base, 7)).is_empty());
    assert_eq!(app.begun(), None);
    assert_eq!(
        app.feed(b"\x1b[<0;1;1M", at(base, 8)),
        [(click(), at(base, 8))]
    );
    assert_eq!(
        app.feed(b"\x1b[200~ab", at(base, 9)),
        [(Key::Pasted(b"ab".to_vec()), at(base, 9))]
    );
    assert_eq!(app.begun(), None, "a paste takes begun as its origin");
    assert!(app.feed(b"\x1b[201~", at(base, 10)).is_empty());
    let mut chat = Keys::default();
    assert_eq!(
        chat.feed(b"\x1b[200~ab", base),
        [(Key::Text(b"ab".to_vec()), base)]
    );
    assert_eq!(chat.begun(), None);
}

#[test]
fn idle_before_bound_leaves_sequence_untouched() {
    // Oracle: `ESC_IDLE` docs — a lone ESC reads as Escape only once the
    // bound passed with nothing after it.
    let base = Instant::now();
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b", base).is_empty());
    assert!(app.idle(at(base, 49)).is_empty());
    assert_eq!(app.begun(), Some(base));
    assert_eq!(app.feed(b"[A", at(base, 60)), [(Key::Up, base)]);
    assert_eq!(ESC_IDLE.as_millis(), 50, "the bound these pins read");
}

#[test]
fn idle_at_bound_expires_lone_esc() {
    // Oracle: behaviour pin — the bound is reached at exactly 50 ms (the
    // check reads `< ESC_IDLE`, so equality expires).
    for ms in [50, 51] {
        let base = Instant::now();
        let mut app = Keys::app();
        assert!(app.feed(b"\x1b", base).is_empty());
        assert_eq!(app.idle(at(base, ms)), [(Key::Escape, base)]);
        assert_eq!(app.begun(), None);
    }
}

#[test]
fn idle_drops_non_esc_pending() {
    // Oracle: `Keys::idle` docs — any other stalled sequence is dropped.
    let base = Instant::now();
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b[", base).is_empty());
    assert!(app.idle(at(base, 50)).is_empty());
    assert_eq!(app.begun(), None);
    assert_eq!(
        app.feed(b"q", at(base, 51)),
        [(Key::Text(b"q".to_vec()), at(base, 51))]
    );
}

#[test]
fn chat_idle_never_expires() {
    // Oracle: `Keys::idle` docs — expiry reads only in the app's decoder.
    let base = Instant::now();
    let mut chat = Keys::default();
    assert!(chat.feed(b"\x1b", base).is_empty());
    assert!(chat.idle(at(base, 100)).is_empty());
    assert_eq!(chat.begun(), Some(base));
    assert_eq!(chat.feed(b"[D", at(base, 101)), [(Key::Left, base)]);
}

#[test]
fn idle_inside_paste_expires_nothing() {
    // Oracle: `Keys::idle` docs — nothing inside a paste ever expires.
    for app in [false, true] {
        let base = Instant::now();
        let mut keys = decoder(app);
        let literal = |bytes: &[u8]| {
            if app {
                Key::Pasted(bytes.to_vec())
            } else {
                Key::Text(bytes.to_vec())
            }
        };
        assert_eq!(keys.feed(b"\x1b[200~ab", base), [(literal(b"ab"), base)]);
        assert!(keys.idle(at(base, 3_600_000)).is_empty(), "app={app}");
        assert!(keys.feed(b"\x1b[201~", at(base, 3_600_001)).is_empty());
        assert_eq!(
            keys.feed(b"e", at(base, 3_600_002)),
            [(Key::Text(b"e".to_vec()), at(base, 3_600_002))],
            "app={app}"
        );
    }
}

#[test]
fn offered_chat_expires_lone_esc_to_discard() {
    // Oracle: `Input::idle` docs — only a pending offer needs the app's
    // lone-Esc timeout in the chat, and there Esc discards the offer.
    let base = Instant::now();
    let mut input = Input::new(vec!["lead".to_owned()]);
    assert_eq!(input.tick(Reading::Owner, base), [Effect::Paste(true)]);
    let _ = input.restore(Draft::Offer(b"kept".to_vec()));
    assert!(input.chunk(b"\x1b", at(base, 10)).is_empty());
    assert_eq!(input.idle(at(base, 60)), [Effect::DiscardDraft]);
    assert!(input.offering(), "storage acknowledgement still owed");
    let mut plain = Input::new(vec!["lead".to_owned()]);
    let _ = plain.tick(Reading::Owner, base);
    assert!(plain.chunk(b"\x1b", at(base, 10)).is_empty());
    assert!(plain.idle(at(base, 60)).is_empty());
}

#[test]
fn chat_drops_sgr_shaped_bytes_as_plain_csi() {
    // Oracle: behaviour pin — the chat has no report arm, so an SGR-shaped
    // run is a CSI ending at its final `M` and `editing` says no key.
    let base = Instant::now();
    let mut chat = Keys::default();
    assert!(chat.feed(b"\x1b[<0;1;1M", base).is_empty());
    assert_eq!(chat.begun(), None);
    assert_eq!(
        chat.feed(b"q", at(base, 1)),
        [(Key::Text(b"q".to_vec()), at(base, 1))]
    );
}

#[test]
fn chat_spells_no_x10_report() {
    // Oracle: behaviour pin — the chat has no legacy arm either: `ESC [ M`
    // completes as a CSI with final `M`, says no key, and the payload that
    // follows is typed text.
    let base = Instant::now();
    let mut chat = Keys::default();
    assert_eq!(
        chat.feed(b"\x1b[Mabc", base),
        [(Key::Text(b"abc".to_vec()), base)]
    );
    assert_eq!(chat.begun(), None);
}

#[test]
fn app_consumes_x10_payload() {
    // Oracle: protocol (X10 mouse: three payload bytes after `ESC [ M`) for
    // the count, behaviour pin for the eaten bytes reading as nothing.
    let base = Instant::now();
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b[Mabc", base).is_empty());
    assert_eq!(app.begun(), None);
    assert_eq!(
        app.feed(b"q", at(base, 1)),
        [(Key::Text(b"q".to_vec()), at(base, 1))]
    );
}

#[test]
fn control_byte_table_both_decoders() {
    // Oracle, grouped: protocol for CR/LF; the `Key` docs (readline bindings
    // the code documents) for Home End ClearLine KillBigWord KillToEnd Yank
    // Backspace; behaviour pin for the app's drop set, the chat's keep set,
    // Tab/Interrupt app-only, and ImagePaste in both decoders.
    let text = |byte: u8| Key::Text(vec![byte]);
    // (byte, chat key, app key or None when the app drops it)
    let rows: [(u8, Key, Option<Key>); 32] = [
        (0x00, text(0x00), None),
        (0x01, Key::Home, Some(Key::Home)),
        (0x02, text(0x02), None),
        (0x03, text(0x03), Some(Key::Interrupt)),
        (0x04, text(0x04), None),
        (0x05, Key::End, Some(Key::End)),
        (0x06, text(0x06), None),
        (0x07, text(0x07), None),
        (0x08, Key::Backspace, Some(Key::Backspace)),
        (0x09, text(0x09), Some(Key::Tab)),
        (0x0a, Key::Newline, Some(Key::Newline)),
        (0x0b, Key::KillToEnd, Some(Key::KillToEnd)),
        (0x0c, text(0x0c), None),
        (0x0d, Key::Enter, Some(Key::Enter)),
        (0x0e, text(0x0e), None),
        (0x0f, text(0x0f), None),
        (0x10, text(0x10), None),
        (0x11, text(0x11), None),
        (0x12, text(0x12), None),
        (0x13, text(0x13), None),
        (0x14, text(0x14), None),
        (0x15, Key::ClearLine, Some(Key::ClearLine)),
        (0x16, Key::ImagePaste, Some(Key::ImagePaste)),
        (0x17, Key::KillBigWord, Some(Key::KillBigWord)),
        (0x18, text(0x18), None),
        (0x19, Key::Yank, Some(Key::Yank)),
        (0x1a, text(0x1a), None),
        (0x1c, text(0x1c), None),
        (0x1d, text(0x1d), None),
        (0x1e, text(0x1e), None),
        (0x1f, text(0x1f), None),
        (0x7f, Key::Backspace, Some(Key::Backspace)),
    ];
    for (byte, chat_key, app_key) in rows {
        let stamp = Instant::now();
        assert_eq!(
            Keys::default().feed(&[byte], stamp),
            [(chat_key, stamp)],
            "chat {byte:#04x}"
        );
        let want: Vec<(Key, Instant)> = app_key.into_iter().map(|key| (key, stamp)).collect();
        assert_eq!(Keys::app().feed(&[byte], stamp), want, "app {byte:#04x}");
    }
    // ESC alone opens a sequence in both decoders: no key, begun held.
    for app in [false, true] {
        let stamp = Instant::now();
        let mut keys = decoder(app);
        assert!(keys.feed(b"\x1b", stamp).is_empty(), "app={app}");
        assert_eq!(keys.begun(), Some(stamp), "app={app}");
    }
}

#[test]
fn high_bytes_are_plain_text_split_by_chunk() {
    // Oracle: behaviour pin — the decoder is byte-oriented; no UTF-8
    // assembly happens here (the composer walks units later).
    for app in [false, true] {
        let base = Instant::now();
        let mut keys = decoder(app);
        assert_eq!(
            keys.feed(b"\xc3\xa9", base),
            [(Key::Text(b"\xc3\xa9".to_vec()), base)],
            "app={app}"
        );
        let mut keys = decoder(app);
        assert_eq!(
            keys.feed(b"\xc3", base),
            [(Key::Text(b"\xc3".to_vec()), base)],
            "app={app}"
        );
        assert_eq!(
            keys.feed(b"\xa9", at(base, 1)),
            [(Key::Text(b"\xa9".to_vec()), at(base, 1))],
            "app={app}"
        );
        assert_eq!(
            keys.feed(b"\xff", at(base, 2)),
            [(Key::Text(b"\xff".to_vec()), at(base, 2))],
            "app={app}"
        );
    }
}

#[test]
fn unterminated_paste_emits_and_carries_origin() {
    // Oracle: `Keys` docs — a paste is literal until its own close or the
    // end of input; what arrived already reads as keys with the open stamp.
    for app in [false, true] {
        let base = Instant::now();
        let mut keys = decoder(app);
        let literal = |bytes: &[u8]| {
            if app {
                Key::Pasted(bytes.to_vec())
            } else {
                Key::Text(bytes.to_vec())
            }
        };
        assert_eq!(
            keys.feed(b"\x1b[200~abc", base),
            [(literal(b"abc"), base)],
            "app={app}"
        );
        assert_eq!(keys.begun(), None, "app={app}");
        assert_eq!(
            keys.feed(b"de", at(base, 1)),
            [(literal(b"de"), base)],
            "app={app}"
        );
        assert!(keys.feed(b"\x1b[201~", at(base, 2)).is_empty());
        assert_eq!(
            keys.feed(b"f", at(base, 3)),
            [(Key::Text(b"f".to_vec()), at(base, 3))],
            "app={app}"
        );
    }
}

#[test]
fn nested_paste_start_is_literal() {
    // Oracle: behaviour pin — inside a paste only the close is held back; a
    // nested open mismatches at its fifth byte and flushes as literal.
    for app in [false, true] {
        let base = Instant::now();
        let mut keys = decoder(app);
        let stream = b"\x1b[200~a\x1b[200~b\x1b[201~";
        let literal = |bytes: &[u8]| {
            if app {
                Key::Pasted(bytes.to_vec())
            } else {
                Key::Text(bytes.to_vec())
            }
        };
        assert_eq!(
            keys.feed(stream, base),
            [(literal(b"a\x1b[200~b"), base)],
            "app={app}"
        );
    }
}

#[test]
fn paste_end_mismatch_flushes_at_each_depth() {
    // Oracle: behaviour pin — a held prefix that breaks flushes whole,
    // whatever depth it reached, and the paste reads on.
    let frags: [&[u8]; 4] = [b"\x1b[Y", b"\x1b[2X", b"\x1b[20Z", b"\x1b[201W"];
    for app in [false, true] {
        for frag in frags {
            let base = Instant::now();
            let mut keys = decoder(app);
            let stream = [b"\x1b[200~Q".as_slice(), frag, b"q\x1b[201~"].concat();
            let mut body = b"Q".to_vec();
            body.extend(frag.iter());
            body.extend(b"q");
            let literal = if app {
                Key::Pasted(body)
            } else {
                Key::Text(body)
            };
            assert_eq!(keys.feed(&stream, base), [(literal, base)], "app={app}");
        }
    }
}

#[test]
fn lf_skip_lost_across_held_flush() {
    // Oracle: behaviour pin — the fold state is per literal call: bytes a
    // mismatch flushes pass through the folder, so a CR left waiting by an
    // earlier byte no longer eats the LF after the flush.
    for app in [false, true] {
        let base = Instant::now();
        let mut keys = decoder(app);
        let stream = b"\x1b[200~a\r\x1bX\nQ\x1b[201~";
        let literal = |bytes: &[u8]| {
            if app {
                Key::Pasted(bytes.to_vec())
            } else {
                Key::Text(bytes.to_vec())
            }
        };
        assert_eq!(
            keys.feed(stream, base),
            [(literal(b"a\n\x1bX\nQ"), base)],
            "app={app}"
        );
    }
}

#[test]
fn lf_skip_reset_between_pastes_keeps_next_lf() {
    // Oracle: behaviour pin — a paste ending in CR must not eat the LF
    // opening the next paste. This pins the paste-open reset (`:235`); the
    // paste-close reset (`:185`) is equivalent-given-open and goes in 2c.
    for app in [false, true] {
        let base = Instant::now();
        let mut keys = decoder(app);
        let literal = |bytes: &[u8]| {
            if app {
                Key::Pasted(bytes.to_vec())
            } else {
                Key::Text(bytes.to_vec())
            }
        };
        assert_eq!(
            keys.feed(b"\x1b[200~a\r\x1b[201~", base),
            [(literal(b"a\n"), base)],
            "app={app}"
        );
        assert_eq!(
            keys.feed(b"\x1b[200~\nX\x1b[201~", at(base, 1)),
            [(literal(b"\nX"), at(base, 1))],
            "app={app}"
        );
    }
}

#[test]
fn app_adjacent_pastes_merge_within_one_chunk_only() {
    // Oracle: behaviour pin — runs merge by origin stamp, so two pastes
    // share one key only when one read opened both.
    for app in [false, true] {
        let literal = |bytes: &[u8]| {
            if app {
                Key::Pasted(bytes.to_vec())
            } else {
                Key::Text(bytes.to_vec())
            }
        };
        let base = Instant::now();
        let mut keys = decoder(app);
        assert_eq!(
            keys.feed(b"\x1b[200~a\x1b[201~\x1b[200~b\x1b[201~", base),
            [(literal(b"ab"), base)],
            "app={app}"
        );
        let mut keys = decoder(app);
        assert_eq!(
            keys.feed(b"\x1b[200~a\x1b[201~", base),
            [(literal(b"a"), base)],
            "app={app}"
        );
        assert_eq!(
            keys.feed(b"\x1b[200~b\x1b[201~", at(base, 1)),
            [(literal(b"b"), at(base, 1))],
            "app={app}"
        );
    }
}

#[test]
fn sgr_right_press_spells_menu_and_its_release_drops() {
    // Oracle: `mouse` docs — the app admits an unmodified right press as
    // the session menu and drops its release; SGR 1006 for button code 2.
    let press = b"\x1b[<2;5;5M";
    let menu = Key::Mouse(Mouse {
        kind: MouseKind::Menu,
        column: 4,
        row: 4,
    });
    for cut in 0..=press.len() {
        let stamp = Instant::now();
        let mut app = Keys::app();
        let mut out = app.feed(&press[..cut], stamp);
        out.extend(app.feed(&press[cut..], stamp));
        assert_eq!(out, [(menu.clone(), stamp)], "press cut at {cut}");
    }
    let stamp = Instant::now();
    let mut app = Keys::app();
    assert!(app.feed(b"\x1b[<2;5;5m", stamp).is_empty());
    let after = Key::Mouse(Mouse {
        kind: MouseKind::Click,
        column: 8,
        row: 8,
    });
    assert_eq!(app.feed(b"\x1b[<0;9;9M", stamp), [(after, stamp)]);
}

#[test]
fn every_split_keeps_keys_stamps_and_waiting() {
    // Oracle: self-consistency — the slice invariant. However the reads
    // split a stream, the keys read the same (runs joined), every stamp is
    // one of the reads' own, and the waiting state agrees with one chunk.
    let overlong = [b"\x1b[".as_slice(), &[b'1'; 20]].concat();
    let discarded = [b"\x1b[<".as_slice(), &[b'9'; 40], b"M\x1b[<0;1;1M"].concat();
    let rows: [&[u8]; 9] = [
        b"",
        b"x\x1b[200~a\r\nb\x1b[D\x1b[201~y\x03",
        b"\x1b[<0;1;1M\x1b[Mabc\x1b[Aq",
        b"\x1b\x1b\x1b[5~",
        b"\x1b[1;5D\x7f\x01\x05\x15\x17\x0b\x19\x16\r\n\x09",
        &overlong,
        b"\x1b[200~\x1bXa\r\x1b[20Y\n\x1b[201~",
        &discarded,
        b"\xff\xc3\xa9\x00\x1f\x7f",
    ];
    for app in [false, true] {
        for (row, stream) in rows.iter().enumerate() {
            let here = format!("app={app} row={row}");
            let base = Instant::now();
            let mut whole = decoder(app);
            let one = whole.feed(stream, base);
            let (want_keys, want_waiting) = (norm(&one), whole.begun().is_some());
            assert!(
                one.iter().all(|(_, stamp)| *stamp == base),
                "{here}: one chunk, one stamp"
            );
            for cut in 0..=stream.len() {
                for second in [base, at(base, 1)] {
                    let mut keys = decoder(app);
                    let mut out = keys.feed(&stream[..cut], base);
                    out.extend(keys.feed(&stream[cut..], second));
                    assert_eq!(norm(&out), want_keys, "{here} cut={cut}");
                    assert!(
                        out.iter()
                            .all(|(_, stamp)| *stamp == base || *stamp == second),
                        "{here} cut={cut}: stamps are the reads' own"
                    );
                    assert_eq!(keys.begun().is_some(), want_waiting, "{here} cut={cut}");
                }
            }
            for first in 0..=stream.len() {
                for second in first..=stream.len() {
                    let mut keys = decoder(app);
                    let mut out = keys.feed(&stream[..first], base);
                    out.extend(keys.feed(&stream[first..second], base));
                    out.extend(keys.feed(&stream[second..], base));
                    assert_eq!(norm(&out), want_keys, "{here} {first}/{second}");
                    assert_eq!(
                        keys.begun().is_some(),
                        want_waiting,
                        "{here} {first}/{second}"
                    );
                }
            }
        }
    }
}
