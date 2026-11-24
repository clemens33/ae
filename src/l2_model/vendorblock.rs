//! A client scope its vendor refuses to serve: the record one first sighting
//! writes, the words every surface says about it, and nothing else.
//!
//! A block is MEASURED, never guessed: the watchdog books one only from an
//! exact vendor line a tool draws (`watchdog::blocked`), so a damaged or
//! unknown error stays unclassified. The record is one small file per
//! `[clients]` label under `<state root>/vendorblock/`, the scope the quota
//! table already names. It is hostile persisted state: a human, a crashed write
//! or another tool may have put anything there, so [`parse`] is strict, bounded
//! and total, and a record it refuses is no block at all.
//!
//! The scope is the `[clients]` label, except for an `opencode` client, which
//! serves several upstream providers at once (Copilot, `OpenCode` Go, the free Zen
//! models): a profile whose `--model` / `-m` id is `provider/model` is in the
//! scope of that provider, so a Copilot block leaves the other providers of the
//! same client usable. A model id with no `/` has no provider to tell apart and
//! falls back to the client's own scope. The record's file is `<scope>.rec`.
//!
//! Only `ae quota clear <scope>` removes a record. No pane fact, no completed
//! turn and no auto reseat clears it: nothing measured proves the account works
//! again, and a false clear sends the next spawn onto a broken account.

use crate::time::Timestamp;

/// The directory under the state root that holds one `<client>.rec` per block.
pub const DIR: &str = "vendorblock";

/// The most bytes a record may be; a larger file is damage.
pub const CAP: u64 = 4096;

/// The one record version this build writes and reads.
const VERSION: &str = "1";

/// The longest reason a record keeps, in chars.
pub const REASON_MAX: usize = 200;

/// The longest seat name a record keeps, matching the agent-name grammar.
const SEAT_MAX: usize = 64;

/// The longest tool word a record keeps.
const TOOL_MAX: usize = 32;

/// The longest upstream provider word a record keeps.
const PROVIDER_MAX: usize = 48;

/// The rows a record holds, in the order [`render`] writes them: the first
/// seven are required, `provider` is optional and written last.
const KEYS: [&str; 8] = [
    "version", "tool", "client", "since", "kind", "seat", "reason", "provider",
];

/// What the vendor refused: the account's payment, or its subscription quota.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A payment or billing verification failure (`needs you (billing)`).
    Billing,
    /// A quota-exhausted subscription (`needs you (account)`).
    Account,
}

impl Kind {
    /// The one spelling a record, a cell, a summary and a refusal share.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Billing => "billing",
            Self::Account => "account",
        }
    }

    /// The kind a spelling names, or `None`.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "billing" => Some(Self::Billing),
            "account" => Some(Self::Account),
            _ => None,
        }
    }
}

/// One block: which scope, since when, which seat saw it first, and the
/// vendor's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The harness binary the client runs (`muse`, `opencode`).
    pub tool: String,
    /// The `[clients]` label whose seat saw the line.
    pub client: String,
    /// When the first sighting happened, in epoch seconds.
    pub since: i64,
    /// What the vendor refused.
    pub kind: Kind,
    /// The seat whose pane showed the line first.
    pub seat: String,
    /// The vendor's reason, one clean line.
    pub reason: String,
    /// The upstream provider the seat's model id named (`github-copilot` from
    /// `github-copilot/gpt-6.1-sol`), when the block is that provider's scope.
    /// `None` is the client's own scope.
    pub provider: Option<String>,
}

/// Whether `word` can be an upstream provider in a record: what an `OpenCode`
/// model id carries before its first `/`.
#[must_use]
pub fn is_provider(word: &str) -> bool {
    let mut bytes = word.bytes();
    matches!(bytes.next(), Some(first) if first.is_ascii_alphanumeric())
        && word.len() <= PROVIDER_MAX
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// The upstream provider a model id names: the part before its first `/` when
/// a model part follows and the provider spells as one, else `None`.
#[must_use]
pub fn provider_of(model: &str) -> Option<String> {
    let (provider, rest) = model.split_once('/')?;
    (!rest.is_empty() && is_provider(provider)).then(|| provider.to_owned())
}

/// The provider a profile's model id names, read from the profile's own launch
/// command in `cfg`, or `None` for an unknown profile, no `--model`, or a model
/// id with no provider part.
#[must_use]
pub fn profile_provider(cfg: &crate::config::IdentityConfig, profile: &str) -> Option<String> {
    provider_of(&crate::launch_cmd::model_word(cfg.profile(profile)?)?)
}

/// The file name a scope's record has, or `None` for a word that could not name
/// one file in [`DIR`]. A scope is a `[clients]` label or an upstream provider;
/// the provider grammar is the wider of the two.
#[must_use]
pub fn file_name(scope: &str) -> Option<String> {
    is_provider(scope).then(|| format!("{scope}.rec"))
}

/// One clean line of `text` for a record: control characters become blanks,
/// runs of blanks become one, the ends are trimmed and the line is cut at
/// [`REASON_MAX`] chars. Empty when `text` held nothing printable.
#[must_use]
pub fn clean(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    let mut out = String::new();
    for word in flat.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    let cut: String = out.chars().take(REASON_MAX).collect();
    cut.trim_end().to_owned()
}

/// The record a body holds, or why it is damage.
///
/// # Errors
///
/// A one-phrase reason: the body is over [`CAP`], is not UTF-8, is not the
/// seven required `key=value` rows once each plus at most one `provider` row (an
/// empty, repeated, unknown or malformed row), or any value fails its grammar. A damaged record is never
/// repaired and never read partway.
pub fn parse(bytes: &[u8]) -> Result<Record, &'static str> {
    if bytes.len() as u64 > CAP {
        return Err("over the record cap");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "not UTF-8")?;
    let body = text.strip_suffix('\n').unwrap_or(text);
    let mut values: [Option<&str>; KEYS.len()] = [None; KEYS.len()];
    for row in body.split('\n') {
        let (key, value) = row.split_once('=').ok_or("a row without `=`")?;
        let slot = KEYS
            .iter()
            .position(|known| *known == key)
            .ok_or("an unknown key")?;
        if values[slot].replace(value).is_some() {
            return Err("a repeated key");
        }
        if value.chars().any(char::is_control) {
            return Err("a control character");
        }
    }
    let [
        Some(version),
        Some(tool),
        Some(client),
        Some(since),
        Some(kind),
        Some(seat),
        Some(reason),
        provider,
    ] = values
    else {
        return Err("a missing key");
    };
    if version != VERSION {
        return Err("an unknown version");
    }
    if tool.is_empty()
        || tool.len() > TOOL_MAX
        || !tool
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("a bad tool");
    }
    if !crate::config::is_client_label(client) {
        return Err("a bad client label");
    }
    if since.is_empty()
        || since.len() > 12
        || !since.bytes().all(|byte| byte.is_ascii_digit())
        || (since.len() > 1 && since.starts_with('0'))
    {
        return Err("a bad since");
    }
    let since: i64 = since.parse().map_err(|_| "a bad since")?;
    if since <= 0 {
        return Err("a bad since");
    }
    let kind = Kind::from_word(kind).ok_or("an unknown kind")?;
    if !crate::config::is_agent_name(seat) || seat.len() > SEAT_MAX {
        return Err("a bad seat");
    }
    if reason.is_empty()
        || reason.chars().count() > REASON_MAX
        || reason != reason.trim()
        || reason.contains("  ")
    {
        return Err("a bad reason");
    }
    if provider.is_some_and(|provider| !is_provider(provider)) {
        return Err("a bad provider");
    }
    Ok(Record {
        tool: tool.to_owned(),
        client: client.to_owned(),
        since,
        kind,
        seat: seat.to_owned(),
        reason: reason.to_owned(),
        provider: provider.map(str::to_owned),
    })
}

/// The canonical body of `record`: [`parse`]'s exact inverse for any record
/// whose fields passed their grammar.
#[must_use]
pub fn render(record: &Record) -> String {
    let mut body = format!(
        "version={VERSION}\ntool={}\nclient={}\nsince={}\nkind={}\nseat={}\nreason={}\n",
        record.tool,
        record.client,
        record.since,
        record.kind.word(),
        record.seat,
        record.reason
    );
    if let Some(provider) = &record.provider {
        body.push_str("provider=");
        body.push_str(provider);
        body.push('\n');
    }
    body
}

impl Record {
    /// Whether a reader at `now` may honour this record: a stamp from the
    /// future is damage, never a block.
    #[must_use]
    pub const fn stands_at(&self, now: i64) -> bool {
        self.since <= now
    }

    /// The scope the record blocks, and its file stem: the provider when the
    /// block is an upstream provider's, else the client label.
    #[must_use]
    pub fn scope(&self) -> &str {
        self.provider.as_deref().unwrap_or(&self.client)
    }

    /// The command that clears this scope, as every surface spells it.
    #[must_use]
    pub fn clear_command(&self) -> String {
        format!("ae quota clear {}", self.scope())
    }

    /// `blocked (billing since 2026-09-08T09:10:00Z)` — the quota cell.
    #[must_use]
    pub fn cell(&self) -> String {
        format!(
            "blocked ({} since {})",
            self.kind.word(),
            Timestamp::from_epoch(self.since)
        )
    }

    /// Who saw the line, and on which client when the scope is a provider.
    fn witness(&self) -> String {
        match &self.provider {
            Some(_) => format!("seat {}, client {}", self.seat, self.client),
            None => format!("seat {}", self.seat),
        }
    }

    /// The line `ae quota` prints for this scope.
    #[must_use]
    pub fn quota_line(&self) -> String {
        format!(
            "  {} {} — {} ({}) — `{}` once the vendor has fixed it",
            self.scope(),
            self.cell(),
            self.reason,
            self.witness(),
            self.clear_command()
        )
    }

    /// The refusal a spawn or a launch prints for this scope.
    #[must_use]
    pub fn refusal(&self) -> String {
        format!(
            "scope {} blocked ({}: {}) since {} ({}); run `{}` once the vendor has fixed it",
            self.scope(),
            self.kind.word(),
            self.reason,
            Timestamp::from_epoch(self.since),
            self.witness(),
            self.clear_command()
        )
    }
}

/// One scope a seat can be blocked in: the `[clients]` label that runs it and,
/// for an `OpenCode` profile whose model id is `provider/model`, that provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// The file stem and the word `ae quota clear` takes: the provider, else
    /// the client label.
    pub key: String,
    /// The `[clients]` label.
    pub client: String,
    /// The harness binary the client runs.
    pub tool: String,
    /// The upstream provider, for a provider scope.
    pub provider: Option<String>,
}

impl Scope {
    /// The scope of a seat on `client` running `tool`, whose profile's model id
    /// names `provider`. Only `OpenCode` serves several providers through one
    /// client, so any other tool is always its client's scope. A provider that
    /// spells the client's own label (the `opencode/` Zen models on an `opencode`
    /// client) has the one file of that label, so it is the client's scope.
    #[must_use]
    pub fn new(client: &str, tool: crate::tool::ToolKind, provider: Option<String>) -> Self {
        let provider =
            provider.filter(|word| tool == crate::tool::ToolKind::OpenCode && word != client);
        Self {
            key: provider.clone().unwrap_or_else(|| client.to_owned()),
            client: client.to_owned(),
            tool: tool.as_str().to_owned(),
            provider,
        }
    }

    /// The scope a launch of `profile` is in: on the client the `profile@client`
    /// suffix names, else the one its launch word names. `None` for a profile
    /// that names no configured client.
    #[must_use]
    pub fn of_profile(
        cfg: &crate::config::IdentityConfig,
        profile: &str,
        client: Option<&str>,
    ) -> Option<Self> {
        let label = match client {
            Some(label) => label.to_owned(),
            None => cfg.profile_client_label(profile)?,
        };
        let tool = cfg.client(&label)?.tool;
        Some(Self::new(&label, tool, profile_provider(cfg, profile)))
    }

    /// Every scope the config can name, in config order: each client's own, then
    /// the provider scopes of the `OpenCode` profiles bound to it.
    #[must_use]
    pub fn configured(cfg: &crate::config::IdentityConfig) -> Vec<Self> {
        let mut scopes: Vec<Self> = Vec::new();
        for (label, client) in &cfg.clients {
            scopes.push(Self::new(label, client.tool, None));
            for (profile, _) in &cfg.profiles {
                if cfg.profile_client_label(profile).as_deref() != Some(label.as_str()) {
                    continue;
                }
                let scope = Self::new(label, client.tool, profile_provider(cfg, profile));
                if scope.provider.is_some() && !scopes.contains(&scope) {
                    scopes.push(scope);
                }
            }
        }
        scopes
    }
}

/// How one scope's record stands on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// Nothing is there.
    Absent,
    /// A record that parsed. The caller still judges its scope, client, tool and
    /// stamp against what it knows ([`block`]).
    Held(Record),
    /// Something is there that is no record: the reason.
    Damaged(String),
}

/// The record file of `scope` (a file stem) under `state_root`, read through the
/// capped store door: never followed through a link, never read past [`CAP`].
#[must_use]
pub fn standing(state_root: &std::path::Path, scope: &str) -> Standing {
    let Some(name) = file_name(scope) else {
        return Standing::Absent;
    };
    match crate::store::read_capped_in(&state_root.join(DIR), &name, CAP) {
        Ok(crate::store::SourceRead::Absent) => Standing::Absent,
        Ok(crate::store::SourceRead::Ready(bytes)) => match parse(&bytes) {
            Ok(record) => Standing::Held(record),
            Err(why) => Standing::Damaged(why.to_owned()),
        },
        Ok(crate::store::SourceRead::Invalid(why) | crate::store::SourceRead::Unreadable(why)) => {
            Standing::Damaged(why)
        }
        Err(crate::store::Oversized) => Standing::Damaged("over the record cap".to_owned()),
    }
}

/// The block that stands in `scope` at `now`: a record that parsed, sits at its
/// own scope's file, names this client, tool and provider, and was not stamped in
/// the future. Anything else (a misfiled, mismatched or future record) is no
/// block.
#[must_use]
pub fn block(state_root: &std::path::Path, scope: &Scope, now: i64) -> Option<Record> {
    match standing(state_root, &scope.key) {
        Standing::Held(record)
            if record.scope() == scope.key
                && record.client == scope.client
                && record.tool == scope.tool
                && record.provider == scope.provider
                && record.stands_at(now) =>
        {
            Some(record)
        }
        _ => None,
    }
}

/// Write `record` unless a record already stands at its scope: the FIRST
/// sighting wins and later ones change nothing. A damaged, misfiled or
/// future-stamped file is replaced. `Ok(true)` when this call wrote.
///
/// # Errors
///
/// A scope that names no file, a state root whose `vendorblock` is not a real
/// directory, or the publish itself.
pub fn record_first(state_root: &std::path::Path, record: &Record) -> std::io::Result<bool> {
    let Some(name) = file_name(record.scope()) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "a scope names no record file",
        ));
    };
    if let Standing::Held(held) = standing(state_root, record.scope())
        && held.scope() == record.scope()
        && held.stands_at(record.since)
    {
        return Ok(false);
    }
    let dir = state_root.join(DIR);
    {
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Err(why) if why.kind() != std::io::ErrorKind::AlreadyExists => return Err(why),
            _ => {}
        }
    }
    crate::store::publish_private_in(&dir, &name, render(record).as_bytes())?;
    Ok(true)
}

/// The refusal a seat launched from `profile` (or, for `profile@client`, from
/// the client `client` names) owes, or `None` when its scope is not blocked.
///
/// The scope is the `[clients]` label, except that an `OpenCode` profile whose
/// model id is `provider/model` is in that provider's scope: a Copilot block
/// refuses the Copilot profiles and no other provider of the same client. A
/// model id with no `/` falls back to the client's scope. A profile that names
/// no client, an unreadable config or an unknown label owes nothing here: those
/// are the launch's own refusals, said where they already are. Read from
/// `state_root`; never clears or writes.
#[must_use]
pub fn scope_refusal(
    state_root: &std::path::Path,
    global: &std::path::Path,
    local: Option<&std::path::Path>,
    profile: &str,
    client: Option<&str>,
    now: i64,
) -> Option<String> {
    let cfg = crate::config::read_identity(Some(global), local).ok()?;
    scope_refusal_in(&cfg, state_root, profile, client, now)
}

/// [`scope_refusal`] over a config the caller already read: the launch judges
/// its seat overrides against the very snapshot it resolves them through.
#[must_use]
pub fn scope_refusal_in(
    cfg: &crate::config::IdentityConfig,
    state_root: &std::path::Path,
    profile: &str,
    client: Option<&str>,
    now: i64,
) -> Option<String> {
    let scope = Scope::of_profile(cfg, profile, client)?;
    block(state_root, &scope, now).map(|record| record.refusal())
}

/// What clearing one scope's record did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cleared {
    /// A record or a damaged file was there and is gone.
    Removed,
    /// Nothing was there.
    Absent,
    /// The node could not be removed: the reason.
    Refused(String),
}

/// Remove `scope`'s record, whatever it holds: a damaged file must never
/// strand a human who types the command the refusal named.
#[must_use]
pub fn clear(state_root: &std::path::Path, scope: &str) -> Cleared {
    let Some(name) = file_name(scope) else {
        return Cleared::Absent;
    };
    let dir = state_root.join(DIR);
    if let Err(why) = crate::store::prove_real_directory(&dir) {
        return match crate::store::source_presence(&dir) {
            crate::store::SourcePresence::Absent => Cleared::Absent,
            _ => Cleared::Refused(why.to_string()),
        };
    }
    let path = dir.join(name);
    match crate::store::source_presence(&path) {
        crate::store::SourcePresence::Absent => Cleared::Absent,
        crate::store::SourcePresence::Regular | crate::store::SourcePresence::Irregular(_) => {
            match std::fs::remove_file(&path) {
                Ok(()) => Cleared::Removed,
                Err(why) if why.kind() == std::io::ErrorKind::NotFound => Cleared::Absent,
                Err(why) => Cleared::Refused(why.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CAP, Cleared, Kind, REASON_MAX, Record, Scope, Standing, block, clean, clear, file_name,
        parse, provider_of, record_first, render, scope_refusal_in, standing,
    };

    fn sample() -> Record {
        Record {
            tool: "muse".to_owned(),
            client: "muse".to_owned(),
            since: 1_788_858_600,
            kind: Kind::Billing,
            seat: "testpins-rev".to_owned(),
            reason: "Billing verification failed. Billing Issue".to_owned(),
            provider: None,
        }
    }

    fn copilot() -> Record {
        Record {
            tool: "opencode".to_owned(),
            client: "opencode".to_owned(),
            since: 1_788_858_600,
            kind: Kind::Account,
            seat: "trustfix-nav".to_owned(),
            reason: "You have exceeded your monthly quota".to_owned(),
            provider: Some("github-copilot".to_owned()),
        }
    }

    #[test]
    fn a_rendered_record_parses_back_to_itself() {
        let record = sample();
        assert_eq!(parse(render(&record).as_bytes()), Ok(record.clone()));
        assert_eq!(parse(render(&record).trim_end().as_bytes()), Ok(record));
    }

    #[test]
    fn every_damage_shape_is_no_record() {
        let good = render(&sample());
        let cases: Vec<(String, &str)> = vec![
            (good.replace("version=1", "version=2"), "version"),
            (good.replace("since=1788858600", "since=0"), "zero since"),
            (good.replace("since=1788858600", "since=-5"), "negative"),
            (good.replace("since=1788858600", "since=+5"), "signed"),
            (
                good.replace("since=1788858600", "since=017"),
                "leading zero",
            ),
            (
                good.replace("since=1788858600", "since=99999999999999"),
                "wide since",
            ),
            (good.replace("kind=billing", "kind=other"), "kind"),
            (good.replace("client=muse", "client=a b"), "client"),
            (good.replace("client=muse", "client=../x"), "client path"),
            (good.replace("seat=testpins-rev", "seat="), "empty seat"),
            (good.replace("seat=testpins-rev", "seat=a b"), "seat blank"),
            (good.replace("tool=muse", "tool=mu se"), "tool"),
            (
                good.replace("reason=Billing", "reason= Billing"),
                "lead blank",
            ),
            (
                good.replace("Billing Issue", "Billing  Issue"),
                "double blank",
            ),
            (good.replace("Billing Issue", "Billing\u{1b}[2J"), "escape"),
            (good.replace("reason=", "reason=\u{0}"), "nul"),
            (format!("{good}reason=again\n"), "repeat"),
            (format!("{good}extra=1\n"), "unknown key"),
            (good.replacen("tool=muse\n", "", 1), "missing key"),
            (good.replace('\n', "\r\n"), "crlf"),
            (format!("\n{good}"), "leading blank row"),
            (format!("{good}\n"), "trailing blank row"),
            (String::new(), "empty"),
            ("not a record".to_owned(), "no equals"),
            (
                format!("{good}{}", "x".repeat(usize::try_from(CAP).unwrap_or(0))),
                "oversize",
            ),
        ];
        for (body, name) in cases {
            assert!(parse(body.as_bytes()).is_err(), "{name}: {body:?}");
        }
        assert!(parse(&[0xff, 0xfe]).is_err(), "not UTF-8");
    }

    #[test]
    fn a_reason_is_one_clean_bounded_line() {
        assert_eq!(clean("  a\u{1b}[0m \tb\nc  "), "a [0m b c");
        assert_eq!(clean("\u{7}\u{7}"), "");
        assert_eq!(
            clean(&"x".repeat(REASON_MAX + 50)).chars().count(),
            REASON_MAX
        );
        let cut = clean(&"é ".repeat(REASON_MAX));
        assert!(cut.chars().count() <= REASON_MAX);
        assert_eq!(cut, cut.trim());
    }

    #[test]
    fn only_a_label_names_a_file() {
        assert_eq!(file_name("muse").as_deref(), Some("muse.rec"));
        assert_eq!(file_name("cc-mic").as_deref(), Some("cc-mic.rec"));
        for bad in ["", "../x", "a/b", ".hidden", "a b", "-lead"] {
            assert_eq!(file_name(bad), None, "{bad:?}");
        }
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let root = std::env::temp_dir().join(format!(
            "ae-vendorblock-unit-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch root");
        root
    }

    #[test]
    fn the_first_sighting_wins_and_damage_is_replaced() {
        let root = scratch("first");
        let first = sample();
        assert_eq!(record_first(&root, &first).ok(), Some(true));
        let mut later = sample();
        later.since += 60;
        later.seat = "other".to_owned();
        assert_eq!(record_first(&root, &later).ok(), Some(false), "kept");
        assert_eq!(standing(&root, "muse"), Standing::Held(first.clone()));
        std::fs::write(root.join("vendorblock/muse.rec"), "damage").expect("damage");
        assert!(matches!(standing(&root, "muse"), Standing::Damaged(_)));
        assert_eq!(record_first(&root, &later).ok(), Some(true), "replaced");
        assert_eq!(standing(&root, "muse"), Standing::Held(later));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn muse_scope() -> Scope {
        Scope::new("muse", crate::tool::ToolKind::Muse, None)
    }

    #[test]
    fn a_block_stands_only_for_its_scope_its_tool_and_a_past_stamp() {
        let root = scratch("stands");
        let record = sample();
        assert_eq!(record_first(&root, &record).ok(), Some(true));
        let at = record.since;
        assert_eq!(block(&root, &muse_scope(), at), Some(record.clone()));
        let other_tool = Scope {
            tool: "opencode".to_owned(),
            ..muse_scope()
        };
        assert_eq!(block(&root, &other_tool, at), None, "tool");
        assert_eq!(block(&root, &muse_scope(), at - 1), None, "future");
        let other_client = Scope {
            client: "other".to_owned(),
            ..muse_scope()
        };
        assert_eq!(block(&root, &other_client, at), None, "client");
        // A record filed under another scope's name is no block of that scope.
        std::fs::write(root.join("vendorblock/free.rec"), render(&record)).expect("misfiled");
        let free = Scope {
            key: "free".to_owned(),
            ..muse_scope()
        };
        assert_eq!(block(&root, &free, at), None, "misfiled");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_provider_block_is_not_the_client_block_and_the_other_way_round() {
        let root = scratch("kinds");
        let record = copilot();
        assert_eq!(record_first(&root, &record).ok(), Some(true));
        assert!(matches!(
            standing(&root, "github-copilot"),
            Standing::Held(_)
        ));
        let at = record.since;
        let tool = crate::tool::ToolKind::OpenCode;
        let provider = |word: &str| Scope::new("opencode", tool, Some(word.to_owned()));
        assert_eq!(block(&root, &provider("github-copilot"), at), Some(record));
        assert_eq!(block(&root, &provider("opencode-go"), at), None);
        assert_eq!(
            block(&root, &Scope::new("opencode", tool, None), at),
            None,
            "a Copilot record is not the client's"
        );
        let client = Record {
            provider: None,
            seat: "bare-seat".to_owned(),
            ..copilot()
        };
        assert_eq!(record_first(&root, &client).ok(), Some(true));
        assert!(matches!(standing(&root, "opencode"), Standing::Held(_)));
        assert_eq!(
            block(&root, &Scope::new("opencode", tool, None), at),
            Some(client),
            "the client record blocks a model with no provider"
        );
        assert_eq!(
            block(&root, &provider("opencode-go"), at),
            None,
            "the client record does not block another provider"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn clear_removes_one_record_and_says_when_none_is_there() {
        let root = scratch("clear");
        assert_eq!(clear(&root, "muse"), Cleared::Absent, "no directory");
        assert_eq!(record_first(&root, &sample()).ok(), Some(true));
        assert_eq!(clear(&root, "muse"), Cleared::Removed);
        assert_eq!(clear(&root, "muse"), Cleared::Absent);
        assert_eq!(
            clear(&root, "../muse"),
            Cleared::Absent,
            "no label, no file"
        );
        std::fs::write(root.join("vendorblock/muse.rec"), "damage").expect("damage");
        assert_eq!(clear(&root, "muse"), Cleared::Removed, "damage clears too");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_provider_is_optional_last_and_strict() {
        let record = copilot();
        let body = render(&record);
        assert!(body.ends_with("provider=github-copilot\n"), "{body}");
        assert_eq!(parse(body.as_bytes()), Ok(record.clone()));
        // A record without the row is the whole client's, byte for byte today's.
        let whole = render(&Record {
            provider: None,
            ..record
        });
        assert!(!whole.contains("provider"), "{whole}");
        assert_eq!(parse(whole.as_bytes()).map(|held| held.provider), Ok(None));
        for bad in [
            "provider=",
            "provider=a b",
            "provider=../x",
            "provider=-lead",
            "provider=a/b",
        ] {
            let damaged = body.replace("provider=github-copilot", bad);
            assert!(parse(damaged.as_bytes()).is_err(), "{bad}");
        }
        let long = body.replace("github-copilot", &"p".repeat(49));
        assert!(parse(long.as_bytes()).is_err(), "over the provider cap");
        let twice = format!("{body}provider=github-copilot\n");
        assert!(parse(twice.as_bytes()).is_err(), "a repeated provider");
    }

    #[test]
    fn a_model_id_names_its_provider_only_before_a_slash_and_a_model() {
        assert_eq!(
            provider_of("github-copilot/gpt-6.1-sol").as_deref(),
            Some("github-copilot")
        );
        assert_eq!(
            provider_of("opencode-go/deepseek-v4#high").as_deref(),
            Some("opencode-go")
        );
        assert_eq!(
            provider_of("a/b/c").as_deref(),
            Some("a"),
            "the first slash"
        );
        for none in [
            "deepseek",
            "",
            "/model",
            "provider/",
            "bad provider/m",
            "-x/m",
        ] {
            assert_eq!(provider_of(none), None, "{none:?}");
        }
    }

    #[test]
    fn only_opencode_has_provider_scopes() {
        let provider = Some("github-copilot".to_owned());
        let scope = Scope::new(
            "opencode",
            crate::tool::ToolKind::OpenCode,
            provider.clone(),
        );
        assert_eq!(scope.key, "github-copilot");
        assert_eq!(scope.client, "opencode");
        assert_eq!(scope.tool, "opencode");
        assert_eq!(scope.provider, provider.clone());
        let muse = Scope::new("muse", crate::tool::ToolKind::Muse, provider);
        assert_eq!((muse.key.as_str(), muse.provider), ("muse", None));
        let zen = Scope::new(
            "opencode",
            crate::tool::ToolKind::OpenCode,
            Some("opencode".to_owned()),
        );
        assert_eq!((zen.key.as_str(), zen.provider), ("opencode", None));
    }

    const OPENCODE_CONFIG: &str = "[clients]\nopencode = opencode\nmuse = muse\n[profiles]\n\
        cp = opencode --model github-copilot/gpt-6.1-sol\n\
        short = opencode -m github-copilot/claude-x\n\
        go = opencode --model opencode-go/deepseek-v4\n\
        zen = opencode --model opencode/big-pickle\n\
        bare = opencode --model deepseek\n\
        nomodel = opencode\n\
        emberm = muse --model muse-ember-1.0\n";

    #[test]
    fn the_scope_refusal_keys_an_opencode_block_by_the_model_provider() {
        let cfg = crate::config::parse_identity(OPENCODE_CONFIG).expect("config");
        let root = scratch("scope");
        let record = copilot();
        assert_eq!(record_first(&root, &record).ok(), Some(true));
        let at = record.since;
        let refuse = |profile, client| scope_refusal_in(&cfg, &root, profile, client, at);
        for blocked in ["cp", "short"] {
            let why = refuse(blocked, None).expect(blocked);
            assert!(
                why.starts_with("scope github-copilot blocked (account: "),
                "{why}"
            );
            assert!(why.contains("ae quota clear github-copilot"), "{why}");
        }
        for open in ["go", "zen", "bare", "nomodel", "emberm"] {
            assert_eq!(refuse(open, None), None, "{open} is not the Copilot scope");
        }
        assert_eq!(
            refuse("unknown", None),
            None,
            "an unknown profile owes nothing"
        );
        assert_eq!(
            refuse("go", Some("opencode")),
            None,
            "the suffix keeps the profile's own provider"
        );
        assert!(refuse("cp", Some("opencode")).is_some());
        assert_eq!(
            scope_refusal_in(&cfg, &root, "cp", None, at - 1),
            None,
            "a future stamp"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_model_with_no_provider_falls_back_to_the_client_scope_and_names_it() {
        let cfg = crate::config::parse_identity(OPENCODE_CONFIG).expect("config");
        let root = scratch("fallback");
        let record = Record {
            provider: None,
            seat: "bare-seat".to_owned(),
            ..copilot()
        };
        assert_eq!(record_first(&root, &record).ok(), Some(true));
        let at = record.since;
        for bare in ["bare", "nomodel"] {
            let why = scope_refusal_in(&cfg, &root, bare, None, at).expect(bare);
            assert!(
                why.starts_with("scope opencode blocked (account: "),
                "{why}"
            );
            assert!(why.contains("ae quota clear opencode"), "{why}");
        }
        for provider in ["cp", "go"] {
            assert_eq!(
                scope_refusal_in(&cfg, &root, provider, None, at),
                None,
                "{provider} names a provider: not the client fallback"
            );
        }
        assert!(
            scope_refusal_in(&cfg, &root, "zen", None, at).is_some(),
            "the `opencode/` Zen provider spells the client label: the client's scope"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_configured_scopes_are_each_client_and_each_provider_a_profile_names() {
        let cfg = crate::config::parse_identity(OPENCODE_CONFIG).expect("config");
        let keys: Vec<String> = Scope::configured(&cfg)
            .into_iter()
            .map(|scope| scope.key)
            .collect();
        assert_eq!(
            keys,
            ["opencode", "github-copilot", "opencode-go", "muse"]
                .map(str::to_owned)
                .to_vec(),
            "client first, then its profiles' providers; zen shares the client scope"
        );
    }

    #[test]
    fn the_words_every_surface_says_agree() {
        let record = sample();
        assert_eq!(
            record.cell(),
            "blocked (billing since 2026-09-08T09:10:00Z)"
        );
        assert_eq!(record.clear_command(), "ae quota clear muse");
        assert!(record.refusal().contains(
            "blocked (billing: Billing verification failed. Billing Issue) since 2026-09-08T09:10:00Z"
        ));
        assert!(record.refusal().contains("ae quota clear muse"));
        assert!(record.quota_line().contains(&record.cell()));
        assert!(record.stands_at(record.since));
        assert!(!record.stands_at(record.since - 1));
    }
}
