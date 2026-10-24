//! Frozen tgquota acceptance: an authorized `/quota` gets ONE plain message of HELD quota
//! evidence, refreshing nothing and cut visibly at Telegram's bound. Oracle: owner request
//! 2026-10-24, plan-r2 reqs 1-9 minus exhaustion (lead R1), lead F6, and the AGENTS.md quota
//! invariant (100 raw + 1 reset = 50, fresh unlimited = 0, spend cap = 100, smallest count).
#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real scratch files; the boundary is about PRODUCT code"
    )]

    use super::super::tests::{Recorder, message, policy, temp};
    use super::super::{Handled, Inbox, Knobs, Update, routing};
    use crate::quota::view::{Level, Scope, Why, Window};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    const NOW: i64 = 1_788_858_600; // 2026-09-08T09:10:00Z, the quota fixtures' own day
    const ID_A: &str = "01a08046-1974-7352-ade3-81a786200795";
    const ID_B: &str = "01a08046-2000-7abc-8abc-aaaaaaaaaaaa";
    const HEADER: &str = "Quota · cached only";
    const CONFIG: &str = "[clients]\nclaude = claude\ncodex = codex\n\
        mic = claude config_home=$HOME/.claude-mic manual_resets=2\n\
        mic2 = claude config_home=$HOME/.claude-mic manual_resets=1\n\
        capped = codex config_home=$HOME/.codex-capped manual_resets=3\n\
        solo = claude config_home=$HOME/.claude-solo\ngone = claude config_home=$HOME/.claude-gone\nbad = claude config_home=$HOME/.claude-bad\n\
        [profiles]\nfablex = claude --model fable\nopusx = claude --model opus\n\
        micx = mic --model fable\nmicy = mic2 --model opus\nastrax = codex --model astra\n\
        capx = capped --model sol\nspark = muse\ngem = agy\ngemini = gemini\n\
        [workspace]\nmuse_quota = on\n";
    const META: &str = "schema=2\nmode=local\nseat.main=lead\nprofile.main=astrax\n\
        harness_session.main=01a08046-1974-7352-ade3-81a786200795\nagent_bin.main=codex\n\
        seat.worker.0=rev\nprofile.worker.0=capx\nagent_bin.worker.0=codex\n\
        harness_session.worker.0=01a08046-2000-7abc-8abc-aaaaaaaaaaaa\n";
    const FREE: &str = r#""credits":{"has_credits":true,"unlimited":true,"balance":"0"},"#;
    const CAP: &str = r#""credits":{"has_credits":false,"unlimited":false,"balance":"0"},"#;
    const PLAN: &str = r#""plan_type":"pro""#;
    /// A held Muse usage limit for `HOME/.config/muse`, asked 600 s before NOW.
    const MUSE: &str = r#"{"v":1,"home":"HOME/.config/muse","attempt_at":1788858000,"outcome":"limit","resets_at":1788865800}"#;
    const DECOY: &str = "[clients]\ndecoy = claude config_home=$HOME/.decoy\n";
    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    /// `home`, `ae` (state root) and `cfg/config` apart: a derived config reads the decoys.
    struct Rig(PathBuf);
    fn rig(tag: &str) -> Rig {
        let dir = temp(&format!("tgq-{tag}"));
        fs::create_dir_all(dir.join("telegram")).unwrap();
        let rig = Rig(fs::canonicalize(&dir).unwrap());
        let (home, ae) = (rig.at("home"), rig.at("ae"));
        put(&rig.at("cfg/config"), CONFIG);
        put(&ae.join("config"), DECOY);
        put(&home.join(".ae/config"), DECOY);
        put(&ae.join("sessions/work/meta"), META);
        let claude = include_str!("../fixtures/quota/claude-cache.json");
        put(&home.join(".claude.json"), claude);
        // Every window 100 raw, observed 8600 s before NOW: stale, 2h23m old.
        let mic = claude.replace("\"percent\": 66", "\"percent\": 100");
        let mic = mic.replace("\"percent\": 9,", "\"percent\": 100,");
        let mic = mic.replace("1788858413691", "1788850000000");
        put(&home.join(".claude-mic/.claude.json"), &mic);
        let solo = claude.replace("2026-09-08T11:49:59", "2026-09-08T08:00:00");
        put(&home.join(".claude-solo/.claude.json"), &solo);
        let bad = "{\"cachedUsageUtilization\": ["; // corrupt
        put(&home.join(".claude-bad/.claude.json"), bad);
        fs::create_dir_all(home.join(".claude-gone")).unwrap();
        let rollout = include_str!("../fixtures/quota/codex-rollout.jsonl");
        let free = format!("{FREE}\"spend_control_reached\":false,{PLAN}");
        let free = rollout.replace(PLAN, &free).replace(":7.0", ":40.0");
        let cap = format!("{CAP}\"spend_control_reached\":true,{PLAN}");
        let cap = rollout.replace(PLAN, &cap).replace(":7.0", ":100.0");
        let day = "sessions/2026/09/08/rollout-2026-09-08T09-00-00";
        put(&home.join(format!(".codex/{day}-{ID_A}.jsonl")), &free);
        let capped = format!(".codex-capped/{day}-{ID_B}.jsonl");
        put(&home.join(capped), &cap);
        // NOT due (600 s < TTL): even a wrong ask through the rationed refresh never pays.
        fs::create_dir_all(home.join(".config/muse")).unwrap();
        let held = MUSE.replace("HOME", &home.display().to_string());
        put(&ae.join("quota/muse.json"), &held);
        rig
    }
    impl Rig {
        fn at(&self, leaf: &str) -> PathBuf {
            self.0.join(leaf)
        }
        fn inbox(&self, knobs: Knobs) -> Inbox {
            let (home, config, ae) = (self.at("home"), self.at("cfg/config"), self.at("ae"));
            Inbox::new(self.at("telegram"), knobs).with_quota(home, config, ae)
        }
        /// Every file outside the telegram state: its bytes and mtime.
        fn files(&self) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
            let (mut all, mut todo) = (BTreeMap::new(), vec![self.0.clone()]);
            while let Some(dir) = todo.pop() {
                for entry in fs::read_dir(&dir).unwrap() {
                    let path = entry.unwrap().path();
                    let meta = fs::symlink_metadata(&path).unwrap();
                    if !meta.is_dir() {
                        let seen = (fs::read(&path).unwrap(), meta.modified().unwrap());
                        all.insert(path, seen);
                    } else if path != self.at("telegram") {
                        todo.push(path);
                    }
                }
            }
            all
        }
    }
    fn ask(inbox: &Inbox, text: &str, from: &str, reply: Option<&str>) -> (Handled, Vec<String>) {
        let mut m = message(text);
        (m.from_id, m.reply_to, m.date) = (from.to_owned(), reply.map(str::to_owned), NOW);
        let (message, seen) = (Some(m), Recorder::default());
        let update = Update { id: 7, message };
        let none: Vec<routing::RunningSession> = Vec::new();
        let handled = inbox.handle(&update, &policy(), &none, &seen, &seen, NOW);
        let asked_none = seen.delivered.lock().unwrap().is_empty();
        assert!(asked_none, "/quota asks no session");
        (handled, seen.said.lock().unwrap().clone())
    }
    /// The one blank-line separated block that starts with `title`.
    fn block(text: &str, title: &str) -> String {
        let mut found = text.split("\n\n").filter(|b| b.starts_with(title));
        let one = found.next().expect(title).to_owned();
        assert!(found.next().is_none(), "one {title:?} block: {text}");
        one
    }
    fn want(text: &str, yes: &[&str], no: &[&str]) {
        for word in yes {
            assert!(text.contains(word), "missing {word:?}: {text}");
        }
        for word in no {
            assert!(!text.contains(word), "unwanted {word:?}: {text}");
        }
    }
    #[test]
    fn quota_is_a_command_in_every_spelling_and_state_and_only_for_its_owner() {
        let rig = rig("route");
        let target = rig.at("telegram").join(routing::TARGET_FILE);
        for sticky in [None, Some("work\t*\n"), Some("work\tlead\n")] {
            let _ = fs::remove_file(&target);
            if let Some(bytes) = sticky {
                put(&target, bytes);
            }
            for text in "/quota|  /quota \n|/quota@ae_bot|/quota please now".split('|') {
                for reply in [None, Some("[work] lead: forwarded")] {
                    let (handled, said) = ask(&rig.inbox(Knobs::default()), text, "42", reply);
                    assert!(matches!(handled, Handled::Routed), "{text:?}");
                    assert_eq!(said.len(), 1, "ONE message: {said:?}");
                    assert_eq!(said[0].lines().next(), Some(HEADER), "{text:?}: {said:?}");
                    let kept = fs::read_to_string(&target).ok();
                    assert_eq!(kept.as_deref(), sticky, "the /use selection is untouched");
                }
            }
        }
        for near in ["/quotas", "/quotax", "quota"] {
            let said = ask(&rig.inbox(Knobs::default()), near, "42", None).1;
            let not = said.iter().all(|s| !s.starts_with(HEADER));
            assert!(not, "{near:?} is not /quota");
        }
        let help = ask(&rig.inbox(Knobs::default()), "/help", "42", None).1;
        want(&help[0], &["/quota"], &[]);
        let unbound = Inbox::new(rig.at("telegram"), Knobs::default());
        let said = ask(&unbound, "/quota", "42", None).1;
        assert_eq!(said, ["quota: unavailable"], "unbound: no HOME guess");
        // A rejected sender or a stale message starts no read and says nothing.
        let before = rig.files();
        let (handled, said) = ask(&rig.inbox(Knobs::default()), "/quota", "43", None);
        assert!(matches!(handled, Handled::Dropped) && said.is_empty());
        let late = Knobs {
            stale_before: NOW + 1,
            ..Knobs::default()
        };
        let (handled, said) = ask(&rig.inbox(late), "/quota", "42", None);
        assert!(matches!(handled, Handled::Stale) && said.is_empty());
        assert_eq!(rig.files(), before, "nothing read wrote anything");
        let _ = fs::remove_dir_all(&rig.0);
    }
    #[test]
    fn the_answer_is_the_held_judged_evidence_of_every_configured_scope_and_writes_nothing() {
        let rig = rig("held");
        let before = rig.files();
        let (_, said) = ask(&rig.inbox(Knobs::default()), "/quota", "42", None);
        assert_eq!(rig.files(), before, "no cache, history or record moved");
        let text = &said[0];
        want(text, &[], &["decoy", "not shown in full"]);
        plain_and_bounded(text);
        // Each row: block title | words it must say | words it must not say (`^` apart).
        let rows = [
            // Two profiles on the default login: ONE account, 66 raw stays 66, own reset + age.
            "claude · ~/.claude\n|66%^reset in 2h39m · seen 3m|stale",
            // Two labels on .claude-mic: one block, the SMALLEST count (1): 100 raw is 50.
            "claude · mic|50%^1 reset in hand · stale^2h23m|100%^33",
            // Fresh unlimited credits judge 0 whatever the vendor's 40.
            "codex · ~/.codex\n|0%^credits unlimited|40%",
            // A reached spend cap judges 100 despite three resets (100 raw / 4 would be 25).
            "codex · capped|100%^spend cap reached|25%",
            "muse · |limit-derived, not a measured %^critical|", // held limit keeps its words
            "agy · |not checked|asking agy /quota^agy not found on PATH", // never asked here
            "gemini · |\n|",                                     // unsupported still says why
            // [clients]-only rows: a passed reset drops its window (no fresh 0%); a missing
            // or corrupt cache says so and claims no percentage.
            "claude · solo|Weekly 7d: 9%|Session^: 0%",
            "claude · gone|no reading ae can trust|%",
            "claude · bad|quota file unreadable|%",
        ];
        for row in rows {
            let part: Vec<&str> = row.split('|').collect();
            let words =
                |f: &'static str| f.split('^').filter(|w| !w.is_empty()).collect::<Vec<_>>();
            want(&block(text, part[0]), &words(part[1]), &words(part[2]));
        }
        let _ = fs::remove_dir_all(&rig.0);
    }
    fn window(name: &str, raw: Option<&str>, counts: Option<&str>, level: Level) -> Window {
        Window {
            name: name.to_owned(),
            raw: raw.map(str::to_owned),
            counts: counts.map(str::to_owned),
            fill: None,
            why: None,
            level,
            stale: false,
            resets: Some("1h05m".to_owned()),
            seen: Some("4m".to_owned()),
            burn: None,
        }
    }
    fn scope(title: &str, only: Option<String>, windows: Vec<Window>) -> Scope {
        Scope {
            title: title.to_owned(),
            profiles: "p1, p2".to_owned(),
            account: None,
            only,
            windows,
        }
    }
    fn plain_and_bounded(text: &str) {
        assert!(text.len() <= 3900 && text.starts_with(HEADER), "{text}");
        assert!(text.encode_utf16().count() < 4096 && text.chars().count() < 4096);
        let plain = text.bytes().all(|b| b == b'\n' || (b >= 0x20 && b != 0x7f));
        assert!(plain, "control output: {text:?}");
    }
    fn marker(n: usize) -> String {
        let noun = if n == 1 { "scope" } else { "scopes" };
        format!("…({n} {noun} not shown in full; run ae quota)")
    }
    #[test]
    fn the_reply_is_judged_bounded_plain_and_cut_visibly_with_the_count_not_shown() {
        let critical = window("Weekly 7d", Some("97%"), None, Level::Critical);
        let mut reset = window("Session 5h", Some("100%"), Some("50%"), Level::Low);
        (reset.why, reset.resets) = (Some(Why::Resets(1)), Some("2d03h".to_owned()));
        let mut capped = window("Spend 30d", Some("12%"), Some("100%"), Level::Critical);
        (capped.why, capped.stale) = (Some(Why::SpendCap), true);
        capped.seen = Some("6h40m".to_owned());
        let mut unknown = window("Hourly", None, None, Level::Headroom);
        (unknown.resets, unknown.seen) = (None, None);
        let windows = vec![critical.clone(), reset.clone(), capped.clone(), unknown];
        let text = routing::render_quota(&[scope("acct · ~/.x", None, windows)]);
        plain_and_bounded(&text);
        // Plan: two lines per window, the judged % + note, then its own reset and age.
        let rows = |name: &str| {
            let at = text.lines().skip_while(|l| !l.contains(name));
            at.take(2).collect::<Vec<_>>().join("\n")
        };
        for (w, pct) in [(&critical, "97%"), (&reset, "50%"), (&capped, "100%")] {
            let (reset, ago) = (w.resets.as_deref().unwrap(), w.seen.as_deref().unwrap());
            let note = crate::quota::view::note(w, true);
            want(&rows(&w.name), &[pct, &note, reset, ago], &[]);
        }
        want(&rows("Session 5h"), &["low"], &["critical"]); // Level::Low says its word
        let hourly = rows("Hourly");
        let unknown = !hourly.contains('%') && hourly.matches('?').count() >= 2;
        assert!(unknown, "unknown: no %, `?` cells: {hourly}");
        want(&text, &[], &["12%", "p1, p2"]);
        plain_and_bounded(&routing::render_quota(&[]));
        let calm = |i| window(&format!("w{i:03}"), Some("5%"), None, Level::Headroom);
        let calm_scope = |i| scope(&format!("s{i:03}"), None, vec![calm(i)]);
        let many: Vec<Scope> = (0..200).map(calm_scope).collect();
        let text = routing::render_quota(&many);
        plain_and_bounded(&text);
        // A scope is shown in full when its whole block, as drawn alone, is in the reply.
        let alone = |s: &Scope| routing::render_quota(std::slice::from_ref(s));
        let shown = |s: &&Scope| text.contains(&alone(s)[HEADER.len()..]);
        let full = many.iter().take_while(shown).count();
        assert!((1..200).contains(&full), "{text}");
        assert!(many[full + 1..].iter().all(|s| !text.contains(&s.title)));
        assert_eq!(text.lines().last(), Some(marker(200 - full).as_str()));
        // Filled to the bound: shown whole, the next block and its marker would pass 3900.
        let drawn = |s: &Scope| alone(s).len() - HEADER.len();
        let kept: usize = HEADER.len() + many[..full].iter().map(drawn).sum::<usize>();
        let whole = kept + drawn(&many[full]) + 1 + marker(199 - full).len();
        assert!(whole > 3900, "cut early at {full}: {text}");
        for unit in ["a", "é", "🦀", "\u{1b}[31m\u{7}\t\r\u{7f}"] {
            for len in [10, 3700, 3850, 3880, 3890, 3899, 3900, 4000, 9000] {
                let only = unit.repeat(len / unit.len() + 1);
                let one = [scope(&format!("t{unit}"), Some(only.clone()), Vec::new())];
                let text = routing::render_quota(&one);
                plain_and_bounded(&text);
                let clean = crate::board::terminal_text(&only).replace('\t', " ");
                let cut = text.lines().last() == Some(marker(1).as_str());
                assert_eq!(cut, text.contains("not shown in full"), "{len}");
                assert!(cut || text.contains(&clean), "omitted unmarked: {len}");
                if unit.len() < 5 && (len == 3700 || len >= 4000) {
                    assert_eq!(cut, len >= 4000, "fits iff uncut: {unit} {len}");
                }
                // An oversize field keeps its title and a long safe prefix, filled to the bound.
                if cut && unit.len() < 5 {
                    let prefix = |l: &str| l.len() > 100 && clean.starts_with(l);
                    let kept = text.contains(&one[0].title) && text.lines().any(prefix);
                    assert!(kept, "title and a long safe prefix: {unit} {len}");
                    let filled = text.len() + unit.len() + marker(1).len() > 3900;
                    assert!(filled, "safe prefix fills to the bound: {unit} {len}");
                }
            }
        }
    }
    /// Req 9: the tests above hold only NOT-DUE states; a due branch would spend (a Muse
    /// model call, an agy run, a network fetch), so its call edges are pinned in source.
    #[test]
    fn the_telegram_read_cannot_reach_a_refresh_leg() {
        let src = |file: &str| fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(file));
        let body = |file: &str, start: &str, end: &[&str]| {
            let text = src(file).unwrap();
            let rest = &text[text.find(start).expect(start) + start.len()..];
            let cut = end.iter().filter_map(|end| rest.find(end)).min();
            rest[..cut.unwrap_or(rest.len())].to_owned()
        };
        let stored = body("src/quota.rs", "pub(crate) fn view_stored(", &["\n}\n"]);
        let ends = ["Route::", "\n    }\n"]; // the next arm, or the end of `execute`
        let arm = body("src/telegram/inbound.rs", "Route::Quota =>", &ends);
        let banned = "AgyCache view_cached view_of( refresh( refresh_ run_with( run_as( \
            quota::run( ask( transport:: env:: var_os muse_ask";
        let banned: Vec<&str> = banned.split_whitespace().collect();
        want(&stored, &["compose("], &banned);
        want(&arm, &["view_stored("], &banned);
        let routing = src("src/telegram/routing.rs").unwrap();
        let pure = "pub fn decide(\n    message: Inbound<'_>,\n    sessions: &[RunningSession],\n";
        want(&routing, &[pure], &["view_stored", "Inputs {"]);
        let bridge = src("src/telegram/bridge.rs").unwrap();
        want(&bridge, &[".with_quota(", "(\"quota\","], &[]);
    }
}
