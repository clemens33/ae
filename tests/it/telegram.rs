//! Structural guards for the outbound Telegram bridge.
//!
//! The BEHAVIOURAL gate — the fake Telegram server, the cursor's crash window,
//! the hostile-response cases — lives in `src/telegram.rs`'s own test module,
//! because the test-only egress seam it drives is `cfg(test)`-gated and does not
//! exist outside the library. What lives HERE is the class of assertion a
//! behavioural test cannot make: that there is exactly one place the locked
//! agent can be built, that the module's error type is incapable of carrying the
//! bot token, and that nothing in this crate installs a `log` implementation
//! that would give ureq's internal `debug!` lines somewhere to go.
//!
//! These are SOURCE SCANS, and a source scan closes the spellings it enumerates,
//! not the capability. They are early warnings about the invariants a reviewer
//! would otherwise have to re-derive from the whole file — they are not proof.

#![allow(
    clippy::disallowed_methods,
    reason = "the guards read this crate's own source; the boundary is about what PRODUCT \
              code may reach"
)]

use std::path::Path;

/// The product half of a module: comment lines dropped, and everything from the
/// `#[cfg(test)]` marker onwards cut off.
fn product(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{name} must be readable"));
    let code: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    // Cut at the test MODULE, not at the first `#[cfg(test)]`: this module gates
    // individual items on it (the loopback egress seam), and cutting there would
    // hand every guard below an eighty-line file and a green result.
    code.split_once("#[cfg(test)]\nmod tests {")
        .map_or(code.clone(), |(module, _)| module.to_owned())
}

/// The product half of a module WITH its comments — the only way to assert
/// something about documentation, which `product` strips by design.
fn product_with_comments(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{name} must be readable"));
    text.split_once("#[cfg(test)]\nmod tests {")
        .map_or(text.clone(), |(module, _)| module.to_owned())
}

/// Every `src/**.rs`, whole — comments and test modules included, because a
/// crate-wide absence claim that stops at the first `#[cfg(test)]` is not one.
fn all_sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                out.push((path.display().to_string(), text));
            }
        }
    }
    let mut out = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    assert!(
        !out.is_empty(),
        "the source walk found nothing; it did not run"
    );
    out
}

#[test]
fn the_locked_agent_has_exactly_one_construction_site() {
    // R2's guard, in the shape of the watchdog delivery-site guard. A second
    // `Agent` built anywhere is a second set of defaults — and ureq's defaults
    // are proxy-from-environment, cleartext-allowed, ten-redirects, which is
    // three ways to send a URL containing the bot token somewhere else.
    let sites: Vec<String> = all_sources()
        .iter()
        .filter(|(_, text)| text.contains("Agent::new_with_config"))
        .map(|(path, _)| path.rsplit('/').next().unwrap_or(path).to_owned())
        .collect();
    assert_eq!(
        sites,
        vec!["telegram.rs"],
        "the locked ureq Agent is built somewhere other than src/telegram.rs"
    );

    let telegram = product("telegram.rs");
    assert_eq!(
        telegram.matches("Agent::new_with_config").count(),
        1,
        "src/telegram.rs builds more than one Agent"
    );
    // The same for the other two ways to get one, both of which skip the
    // config entirely and would inherit every default.
    for shortcut in [
        "Agent::new_with_defaults",
        "Agent::with_parts",
        "new_agent()",
    ] {
        assert!(
            !telegram.contains(shortcut),
            "an unlocked agent constructor is reachable: {shortcut}"
        );
    }
}

#[test]
fn the_one_construction_site_sets_every_lock() {
    // The behavioural test asserts the resolved config; this asserts that the
    // settings are written where the reviewer will look, so a future edit that
    // drops one is a deleted line rather than a silent default.
    let telegram = product("telegram.rs");
    for setting in [
        ".https_only(",
        ".proxy(None)",
        ".max_redirects(0)",
        ".timeout_connect(",
        ".timeout_recv_response(",
        ".timeout_global(",
        "rustls::crypto::ring::default_provider()",
    ] {
        assert!(
            telegram.contains(setting),
            "the locked agent no longer sets {setting}"
        );
    }
}

#[test]
fn the_failure_type_cannot_hold_a_url_and_therefore_cannot_leak_the_token() {
    // The strongest guarantee in this module, and the reason it is structural
    // rather than careful: the request URL contains `/bot<TOKEN>/`, and
    // `ureq::Error` has variants that quote the URI back. If `SendFailure`
    // cannot HOLD a string, no caller — not one written later, not one written
    // carelessly — can format a token out of it.
    //
    // THE COMPILER OWNS MOST OF THIS CLAIM, and this scan owns the remainder.
    // `SendFailure` is `Copy` (asserted at the type, in src/telegram.rs), so an
    // owned `String`/`PathBuf`/boxed-error payload does not compile — measured
    // as a control, not assumed. What `Copy` still admits is a `&'static str`,
    // and that is what the scan below is actually for.
    let telegram = product("telegram.rs");
    assert!(
        telegram.contains("holds_no_owned_text::<SendFailure>()"),
        "the compile-time proof that SendFailure owns no text is gone"
    );
    let enum_body = telegram
        .split_once("pub enum SendFailure {")
        .and_then(|(_, rest)| rest.split_once("\n}"))
        .map(|(body, _)| body.to_owned())
        .expect("SendFailure must be declared");
    for forbidden in ["String", "&str", "Uri", "url", "Url", "PathBuf", "ureq::"] {
        assert!(
            !enum_body.contains(forbidden),
            "SendFailure gained a variant that can carry text ({forbidden}):\n{enum_body}"
        );
    }
    // And the mapper must not format the upstream error on its way past.
    let classify = telegram
        .split_once("fn classify(")
        .and_then(|(_, rest)| rest.split_once("\n}"))
        .map(|(body, _)| body.to_owned())
        .expect("classify must exist");
    for forbidden in ["to_string", "format!", "{error", "{err"] {
        assert!(
            !classify.contains(forbidden),
            "classify renders the ureq error ({forbidden}) — its Display can quote the URI"
        );
    }
}

#[test]
fn the_bridge_writes_to_no_log_of_its_own() {
    // ureq's internal `debug!` lines carry the request URI (redacted to
    // `/******` unless TRACE is enabled — which is a property of ureq's
    // formatter, not a guarantee we control). ae installs no `log`
    // implementation, so those macros compile to nothing at run time. The day
    // one is installed, this test is the reminder that the bridge's URLs
    // acquire somewhere to go.
    for (path, text) in all_sources() {
        for installer in ["set_logger", "set_boxed_logger", "log::set"] {
            assert!(
                !text.contains(installer),
                "{path} installs a log implementation ({installer}); ureq's internal \
                 request logging is no longer inert"
            );
        }
    }
    let telegram = product("telegram.rs");
    for printer in ["println!", "eprintln!", "dbg!", "print!", "eprint!"] {
        assert!(
            !telegram.contains(printer),
            "src/telegram.rs prints ({printer}); this module handles the bot token"
        );
    }
}

#[test]
fn the_destination_is_a_constant_and_the_test_seam_is_compiled_out() {
    let telegram = product("telegram.rs");
    assert!(
        telegram.contains(r#"const TELEGRAM_API: &str = "https://api.telegram.org";"#),
        "the production API root is no longer a constant"
    );
    // Exactly one literal: a second one is a second destination.
    assert_eq!(
        telegram.matches("https://api.telegram.org").count(),
        1,
        "more than one production URL literal"
    );
    // The credentials reader knows two keys. A third would be an operator-
    // settable place to send the token.
    let keys: Vec<&str> = telegram
        .lines()
        .filter(|line| line.contains("=> token_file = ") || line.contains("=> chat_id = "))
        .collect();
    assert_eq!(
        keys.len(),
        2,
        "the config keys the bridge reads changed: {keys:?}"
    );
    for destination_key in ["base_url", "api_url", "api_root", "endpoint", "host ="] {
        assert!(
            !telegram.contains(destination_key),
            "a config key that could name a destination appeared: {destination_key}"
        );
    }
    // And the loopback seam is gated, so a release build has no other egress at
    // all — not merely no configured one.
    assert!(
        telegram.contains("#[cfg(test)]\n    Loopback(String),"),
        "the loopback egress is no longer cfg(test)-gated"
    );
}

#[test]
fn the_one_durable_write_syncs_both_the_file_and_its_directory() {
    // An fsync is not observable from inside the process that issued it, so
    // this is a source-level guard and claims nothing more. What it protects is
    // the difference between atomic and durable: a rename is atomic for a
    // reader while the directory entry is still unwritten, and a checkpoint that
    // rolls back on power loss turns this bridge's honest one-item crash window
    // into an unbounded one.
    //
    // It moved from `store_cursor` to `durable_write` when the INBOUND offset
    // acquired the same need. That is a strengthening, not a relocation: one
    // implementation now carries the property for both checkpoints, and the
    // two assertions below are what stop either of them from growing a second
    // spelling of it.
    let telegram = product("telegram.rs");
    let write = telegram
        .split_once("pub(crate) fn durable_write(")
        .and_then(|(_, rest)| rest.split_once("\n}\n"))
        .map(|(body, _)| body.to_owned())
        .expect("durable_write must exist");
    assert_eq!(
        write.matches("sync_all()").count(),
        2,
        "the durable write no longer syncs BOTH the temp file and the parent directory:\n{write}"
    );
    assert!(
        write.contains("fs::rename("),
        "the durable write no longer renames into place"
    );
    let sync_at = write.find("sync_all()").expect("a sync");
    let rename_at = write.find("fs::rename(").expect("the rename");
    assert!(
        sync_at < rename_at,
        "the temp file is renamed before it is synced — the rename can publish an \
         empty file"
    );

    // BOTH checkpoints must go through it. A second durable write would be a
    // second thing to get right, and the one that got it wrong would be the one
    // nobody re-read.
    let store_cursor = telegram
        .split_once("pub fn store_cursor(")
        .and_then(|(_, rest)| rest.split_once("\n}"))
        .map(|(body, _)| body.to_owned())
        .expect("store_cursor must exist");
    assert!(
        store_cursor.contains("durable_write(path,"),
        "the outbound cursor no longer checkpoints through the one durable write"
    );
    let inbound = product("telegram/inbound.rs");
    assert!(
        inbound.contains("durable_write(path,"),
        "the inbound offset no longer checkpoints through the one durable write"
    );
    for module in ["telegram.rs", "telegram/inbound.rs", "telegram/bridge.rs"] {
        let source = product(module);
        assert!(
            !source.contains("fs::write("),
            "{module} writes a file without the durable path"
        );
    }
}

#[test]
fn the_response_body_is_bounded_without_consulting_content_length() {
    let telegram = product("telegram.rs");
    let reader = telegram
        .split_once("fn read_bounded(")
        .and_then(|(_, rest)| rest.split_once("\n}"))
        .map(|(body, _)| body.to_owned())
        .expect("read_bounded must exist");
    assert!(
        reader.contains(".take("),
        "the body reader no longer caps what it streams"
    );
    for trusting in ["content_length", "Content-Length", "content-length"] {
        assert!(
            !reader.contains(trusting),
            "the body cap consults the declared length ({trusting}); a declared length is \
             the remote's claim about a body it has not sent"
        );
    }
}

/// The one live test, and it is gated twice.
///
/// `#[ignore]` keeps it out of every ordinary run, and the environment check
/// keeps it a no-op even when someone runs `--ignored` on a machine with no
/// bridge configured. It NEVER requires credentials: absent config, absent
/// token file and an unparseable config all end the test quietly. A suite that
/// cannot pass without a secret is a suite nobody else can run.
///
/// `AE_TELEGRAM_LIVE_SMOKE=1 cargo test --test it -- --ignored live_smoke`
#[test]
#[ignore = "live: posts a real Telegram message; needs AE_TELEGRAM_LIVE_SMOKE=1 and existing credentials"]
fn live_smoke_posts_one_message_to_the_configured_chat() {
    if std::env::var_os("AE_TELEGRAM_LIVE_SMOKE").is_none() {
        eprintln!("live smoke: AE_TELEGRAM_LIVE_SMOKE is not set — nothing attempted");
        return;
    }
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        eprintln!("live smoke: no HOME — nothing attempted");
        return;
    };
    let ae_home =
        std::env::var_os("AE_HOME").map_or_else(|| home.join(".ae"), std::path::PathBuf::from);
    let credentials = match ae::telegram::load_credentials(&ae_home.join("config"), &home) {
        Ok(credentials) => credentials,
        Err(why) => {
            eprintln!("live smoke: no usable credentials ({why}) — nothing attempted");
            return;
        }
    };
    ae::telegram::Api::production(credentials)
        .send_message("ae: rust outbound bridge live smoke")
        .expect("the live send failed");
}

#[test]
fn the_event_log_is_read_through_one_descriptor_and_never_looked_up_twice() {
    // FIX 2, and it is a STRUCTURAL assertion on purpose. A TOCTOU that has been
    // closed by construction cannot be exercised behaviourally: there is no
    // longer a moment between the two lookups to rotate the file in, and a test
    // that manufactured one would be testing a seam rather than the product.
    // What CAN be asserted is that the second lookup does not exist — the shape
    // the bug needed — and the control for it is reverting to the two-step,
    // which turns this red.
    let telegram = product("telegram.rs");
    assert!(
        !telegram.contains("fs::metadata(&self.log)"),
        "the log's identity is sampled by PATH again; that is the TOCTOU's first half"
    );
    // `read_window` must take the descriptor, not a path to re-open.
    let window = telegram
        .split_once("fn read_window(")
        .and_then(|(_, rest)| rest.split_once("\n}"))
        .map(|(body, _)| body.to_owned())
        .expect("read_window must exist");
    assert!(
        window.starts_with("file: &mut fs::File"),
        "read_window still takes a path: {}",
        window.lines().next().unwrap_or_default()
    );
    assert!(
        !window.contains("File::open"),
        "read_window opens the log for itself — that is the TOCTOU's second half"
    );
}

#[test]
fn the_append_only_invariant_is_recorded_where_it_is_spent() {
    // FIX 3 is resolved by PROOF rather than by a guard, so the artifact this
    // test protects is the proof itself: an unreachable case whose reasoning is
    // not written down becomes a mystery the next reader either re-derives or
    // "fixes" with speculative machinery.
    let telegram = product_with_comments("telegram.rs");
    let start = telegram
        .split_once("fn start(")
        .map(|(before, _)| before.to_owned())
        .expect("start must exist");
    let doc = start
        .rsplit_once("/// # THE INVARIANT THIS RESTS ON")
        .map(|(_, rest)| rest.to_owned())
        .expect("the append-only invariant must be documented at start()");
    for citation in [
        "crate::state::emit",
        "append_locked",
        "OpenOptions::append(true)",
        "compact",
        "NEW INODE",
    ] {
        assert!(
            doc.contains(citation),
            "the append-only proof no longer cites {citation}"
        );
    }
    // The trigger matters more than the proof: a future in-place ledger rewrite
    // must land on this sentence.
    assert!(
        doc.contains("INVALIDATES this and must revisit"),
        "the proof no longer records what would invalidate it"
    );
}

#[test]
fn every_read_goes_through_one_open_that_classifies_first() {
    // FIX 4 and FIX 6 together, and the strengthening is the point: it is no
    // longer "the credential reader classifies first", it is "there is ONE open
    // in this module and it classifies first". Four readers — config, token,
    // cursor, event log — and one hardened pattern, so a fifth cannot acquire
    // its own spelling by accident.
    //
    // The ORDER is the property: `open(2)` on a FIFO blocks until a writer
    // appears, so a check performed after the open is not a check. Both the
    // cursor and the log live in a meta directory other processes write to.
    let telegram = product("telegram.rs");
    assert_eq!(
        telegram.matches("fs::File::open").count(),
        1,
        "there is more than one open in this module; they will drift"
    );
    let reader = telegram
        .split_once("fn open_regular(")
        .and_then(|(_, rest)| rest.split_once("\n}"))
        .map(|(body, _)| body.to_owned())
        .expect("open_regular must exist");
    assert!(
        reader.contains("fs::File::open"),
        "the one open is not inside open_regular"
    );
    let classify = reader.find("is_file()").expect("a classification");
    let open = reader.find("File::open").expect("the open");
    assert!(
        classify < open,
        "the path is opened before it is classified — a FIFO would block here"
    );
    // And the descriptor is re-checked, because the name could have moved.
    assert!(
        reader.matches("is_file()").count() >= 2,
        "the opened descriptor is not re-checked; only the name was"
    );
    // The custody check must be on the mode of what was opened.
    assert!(
        telegram.contains("mode & 0o077 != 0"),
        "the token's custody is no longer checked"
    );
    // The two sibling readers must route through it rather than around it.
    for (reader_fn, marker) in [
        ("pub fn load_cursor(", "read_regular_file("),
        ("fn pass(&mut self, api: &Api)", "open_regular(&self.log)"),
    ] {
        let Some(body) = telegram
            .split_once(reader_fn)
            .and_then(|(_, rest)| rest.split_once("\n    }"))
            .map(|(body, _)| body.to_owned())
        else {
            panic!("{reader_fn} must exist")
        };
        assert!(
            body.contains(marker),
            "{reader_fn} no longer reads through the hardened open ({marker})"
        );
    }
}

#[test]
fn the_durable_write_temp_cannot_follow_a_planted_symlink() {
    // FIX 5. `create_new(true)` is `O_EXCL`; `create(true).truncate(true)` is
    // the shape that follows a symlink and truncates its target. The temp name
    // is predictable by necessity, so this is the only thing standing between a
    // planted link and whatever it points at.
    let telegram = product("telegram.rs");
    let write = telegram
        .split_once("pub(crate) fn durable_write(")
        .and_then(|(_, rest)| rest.split_once("\n}\n"))
        .map(|(body, _)| body.to_owned())
        .expect("durable_write must exist");
    assert!(
        write.contains("create_new(true)"),
        "the temp is not created with O_EXCL"
    );
    assert!(
        !write.contains("truncate(true)"),
        "the temp still truncates whatever is at its path"
    );
    assert!(
        write.contains("mode(0o600)"),
        "the temp is not created owner-only"
    );
    // Durability is unchanged — FIX 5 must not have cost the fsyncs.
    assert_eq!(write.matches("sync_all()").count(), 2);
}

#[test]
fn the_daemon_drops_the_word_channel_before_it_joins_the_poller() {
    // A SHUTDOWN DEADLOCK, guarded structurally because it cannot be reached
    // behaviourally: `run` builds the production client, so no test in this
    // suite executes the ordering below.
    //
    // The inbound thread may be blocked awaiting confirmation of a give-up
    // notice, and that wait is UNBOUNDED by design (a bounded one abandons the
    // word in the channel and lets the next retry queue another, which is how
    // one give-up became many notices). It ends when the answer channel
    // disconnects — and the answer's sender lives inside the queued word, which
    // the word RECEIVER owns. So while `inbox_words` is alive the word is
    // alive, nothing wakes the thread, and `join` never returns.
    //
    // Dropping before the join ends that wait with a rejection, which is also
    // the correct answer: a bridge on its way out must not advance its offset
    // past a notice it never sent. This asserts the ORDER, which is the whole
    // property — the drop existing further down the scope would not do.
    let bridge = product("telegram/bridge.rs");
    let dropped = bridge
        .find("drop(inbox_words)")
        .expect("run() must drop the word channel explicitly, not let it fall out of scope");
    let joined = bridge
        .find("poller.join()")
        .expect("run() must join the inbound thread");
    assert!(
        dropped < joined,
        "the word channel is dropped AFTER the poller is joined; an inbound thread blocked \
         on a give-up confirmation can never be woken, and the daemon hangs on shutdown"
    );
}

// ─── the give-up's hard/transient split, against a real tmux server ──────
//
// The only branch of the inbound bridge's refusal classifier that a unit test
// cannot reach: "hard" requires an enumeration that RAN and ANSWERED. The unit
// side proves the fail-safe direction (every failure is transient); this proves
// the arm that makes that direction mean something — without it, `Refusal::Hard`
// is produced by nothing and the short bound is dead policy.

/// A short scratch dir — `/tmp` directly, for `sun_path`'s 104 bytes on macOS.
fn tg_scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(format!("/tmp/ae-tg-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(std::fs::create_dir_all(&dir).is_ok(), "a short scratch dir");
    dir
}

/// Plant a session dir whose meta points at `socket`, and NO `send` helper — so
/// delivery through it always refuses and the classifier is what is measured.
fn plant_session(root: &std::path::Path, socket: &std::path::Path) -> std::path::PathBuf {
    let dir = root.join("sessions").join("work");
    let planted = std::fs::create_dir_all(&dir).and_then(|()| {
        std::fs::write(
            dir.join("meta"),
            format!(
                "mode=local\nagent.main=cl:lead\ntmux_server_kind=socket\ntmux_server={}\n",
                socket.display()
            ),
        )
    });
    assert!(planted.is_ok(), "a planted session");
    dir
}

#[test]
fn a_pane_a_real_server_does_not_list_is_the_only_thing_that_counts_as_hard() {
    use ae::telegram::bridge::Helper;
    use ae::telegram::inbound::{Deliver as _, Delivered, Refusal};
    use ae::telegram::routing::Verb;
    use ae::telegram::routing::World as _;

    let scratch = tg_scratch("hard");
    assert!(
        super::phase2::tmux_present(&scratch),
        "tmux is not runnable here, so the hard/transient split cannot be proven; \
         install tmux or run this suite where one exists"
    );
    let socket = scratch.join("s.sock");
    let dir = plant_session(&scratch, &socket);

    // ARM 1 — no server at all. The enumeration cannot run, and a probe that
    // could not run is not evidence of death.
    let refusal = Helper.deliver(Verb::Send, "work", &dir, "cl:lead", "hello", "42");
    assert!(
        matches!(refusal, Delivered::No(Refusal::Transient)),
        "an unreachable server must not shorten the bound: {refusal:?}"
    );

    // ARM 2 — a real server, really answering, with a pane that is NOT the
    // target. This is the one shape that earns the short bound.
    let server = ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(socket.clone()));
    let mut create = ae::tmux::server_args(&server);
    create.extend(["new-session", "-d", "-s", "work"].map(ToOwned::to_owned));
    let (created, _) = super::phase2::run_tmux(&create, &scratch);
    assert!(created, "the fixture server must start");

    let refusal = Helper.deliver(Verb::Send, "work", &dir, "cl:lead", "hello", "42");
    assert!(
        matches!(refusal, Delivered::No(Refusal::Hard)),
        "a server that answered and does not hold the target is the hard case: {refusal:?}"
    );

    // ARM 3 — the same server, now holding a pane stamped as the target. A pane
    // that is THERE is never hard, however the delivery failed.
    let mut stamp = ae::tmux::server_args(&server);
    stamp.extend(["set-option", "-p", "-t", "work", "@ae_agent", "cl:lead"].map(ToOwned::to_owned));
    let (stamped, _) = super::phase2::run_tmux(&stamp, &scratch);
    assert!(stamped, "stamping the pane must succeed");

    let refusal = Helper.deliver(Verb::Send, "work", &dir, "cl:lead", "hello", "42");
    assert!(
        matches!(refusal, Delivered::No(Refusal::Transient)),
        "a pane that is present cannot be dead, whatever the send helper did: {refusal:?}"
    );

    // ARM 4 — the same real server, seen through the World the router resolves
    // against. `Machine::running` is what turns a chat's "work" into a session
    // dir, and it is gated on the record's OWN server answering (SC-947); a
    // liveness check that answered from an ambient server would be an answer
    // about a different machine's-worth of state. Nothing but a real server can
    // tell that apart from an empty world.
    let machine = ae::telegram::bridge::Machine::under(ae::inventory::Roots::under(&scratch));
    let running = machine.running();
    assert_eq!(
        running
            .iter()
            .map(|session| session.name.as_str())
            .collect::<Vec<_>>(),
        vec!["work"],
        "the planted session is live on its own server and must be routable"
    );
    assert_eq!(running[0].dir, dir);
    assert_eq!(running[0].main.as_deref(), Some("cl:lead"));

    let mut kill = ae::tmux::server_args(&server);
    kill.push("kill-server".to_owned());
    let _ = super::phase2::run_tmux(&kill, &scratch);

    // And with the server gone, the same record is NOT running: the scan reads
    // liveness, not the presence of a directory.
    assert!(
        machine.running().is_empty(),
        "a session whose server no longer answers must drop out of the world"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}
