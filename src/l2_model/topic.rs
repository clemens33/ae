//! Topic checkpoints: the memo grammar of `topic-<slug>` records, the `#slug`
//! tags in free text, and the cards a set of memo sources folds into.
//!
//! Pure: no I/O. A memo row filed under `topic-<slug>` carries `name: value`
//! parts separated by unescaped `|`. A card takes each field from the
//! highest-ranked row that carries it, where rank is the source's incarnation
//! and then the row's place in its file; timestamps are shown, never ordered.

use std::cmp::Ordering;
use std::collections::BTreeMap;

/// The longest slug, in bytes (all ASCII).
pub const SLUG_MAX: usize = 48;

/// The memo topic prefix of a topic checkpoint.
pub const KEY_PREFIX: &str = "topic-";

/// The characters that may end a tag right after its slug, besides
/// whitespace and the end of the text.
const CLOSERS: [char; 11] = ['.', ',', ';', ':', '!', '?', ')', ']', '}', '"', '\''];

/// Whether `text` is a whole slug: `[a-z0-9][a-z0-9-]{0,47}`, ASCII only.
#[must_use]
pub fn is_slug(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() <= SLUG_MAX
        && matches!(bytes.first(), Some(b'a'..=b'z' | b'0'..=b'9'))
        && bytes.iter().all(|byte| slug_byte(*byte))
}

const fn slug_byte(byte: u8) -> bool {
    matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-')
}

/// The slug of memo topic `topic`: `Some` only for `topic-<whole slug>`.
#[must_use]
pub fn slug_of(topic: &str) -> Option<&str> {
    topic.strip_prefix(KEY_PREFIX).filter(|slug| is_slug(slug))
}

/// The fields one checkpoint record carries, unescaped.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Fields {
    pub title: Option<String>,
    pub state: Option<String>,
    pub what: Option<String>,
    pub last: Option<String>,
    pub next: Option<String>,
    pub src: Option<String>,
    pub close: Option<String>,
}

/// One field name of a checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Name {
    Title,
    State,
    What,
    Last,
    Next,
    Src,
    Close,
}

impl Name {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "title" => Self::Title,
            "state" => Self::State,
            "what" => Self::What,
            "last" => Self::Last,
            "next" => Self::Next,
            "src" => Self::Src,
            "close" => Self::Close,
            _ => return None,
        })
    }
}

impl Fields {
    const fn slot(&mut self, name: Name) -> &mut Option<String> {
        match name {
            Name::Title => &mut self.title,
            Name::State => &mut self.state,
            Name::What => &mut self.what,
            Name::Last => &mut self.last,
            Name::Next => &mut self.next,
            Name::Src => &mut self.src,
            Name::Close => &mut self.close,
        }
    }

    const fn get(&self, name: Name) -> Option<&String> {
        match name {
            Name::Title => self.title.as_ref(),
            Name::State => self.state.as_ref(),
            Name::What => self.what.as_ref(),
            Name::Last => self.last.as_ref(),
            Name::Next => self.next.as_ref(),
            Name::Src => self.src.as_ref(),
            Name::Close => self.close.as_ref(),
        }
    }
}

/// The parts of `text` split at each unescaped `|`, with `\|` read as `|`
/// and `\\` as `\`; any other backslash stays as it is.
fn parts(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '|' => parts.push(std::mem::take(&mut part)),
            '\\' => match chars.clone().next() {
                Some(next @ ('|' | '\\')) => {
                    part.push(next);
                    chars.next();
                }
                _ => part.push('\\'),
            },
            _ => part.push(ch),
        }
    }
    parts.push(part);
    parts
}

fn trim(text: &str) -> &str {
    text.trim_matches(|ch: char| ch.is_ascii_whitespace())
}

/// The fields of one checkpoint record. Each part `name: value` splits at its
/// first `:`; an unknown name, a part without `:` and an empty value carry
/// nothing; a name given twice keeps the later value.
#[must_use]
pub fn parse_record(text: &str) -> Fields {
    let mut fields = Fields::default();
    for part in parts(text) {
        let Some((name, value)) = part.split_once(':') else {
            continue;
        };
        let (Some(name), value) = (Name::parse(trim(name)), trim(value)) else {
            continue;
        };
        if !value.is_empty() {
            *fields.slot(name) = Some(value.to_owned());
        }
    }
    fields
}

/// A topic's state word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Needs,
    Moving,
    Quiet,
    Parked,
    Closed,
}

impl State {
    /// The state named by exactly `word`, lowercase.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "needs" => Self::Needs,
            "moving" => Self::Moving,
            "quiet" => Self::Quiet,
            "parked" => Self::Parked,
            "closed" => Self::Closed,
            _ => return None,
        })
    }

    /// The state's word.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Needs => "needs",
            Self::Moving => "moving",
            Self::Quiet => "quiet",
            Self::Parked => "parked",
            Self::Closed => "closed",
        }
    }
}

/// The `#slug` tags of `text`, in text order, each once. A tag starts at the
/// text start or after a whitespace character, takes the maximal run of
/// `[a-z0-9-]`, which must be a whole slug, and ends at the text end, a
/// whitespace character or one closing punctuation mark. An overlong run is
/// no tag; it is never cut to a shorter one.
#[must_use]
pub fn tags(text: &str) -> Vec<&str> {
    let mut found: Vec<&str> = Vec::new();
    let mut previous: Option<char> = None;
    for (at, ch) in text.char_indices() {
        let opens = previous.is_none_or(char::is_whitespace);
        previous = Some(ch);
        if ch != '#' || !opens {
            continue;
        }
        let rest = text.get(at + 1..).unwrap_or_default();
        let run = rest.bytes().take_while(|byte| slug_byte(*byte)).count();
        let (slug, after) = rest.split_at_checked(run).unwrap_or_default();
        let closes = after
            .chars()
            .next()
            .is_none_or(|next| next.is_whitespace() || CLOSERS.contains(&next));
        if closes && is_slug(slug) && !found.contains(&slug) {
            found.push(slug);
        }
    }
    found
}

/// Where a checkpoint record came from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Origin {
    pub session: String,
    pub uuid: String,
    /// The record's timestamp text, as written.
    pub at: String,
}

/// One checkpoint record of a slug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub origin: Origin,
    /// (incarnation, place in its file): the higher ranks higher.
    pub rank: (usize, usize),
    /// The record's raw text.
    pub text: String,
}

/// One memo source: a seat incarnation's `memo.tsv` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub session: String,
    pub uuid: String,
    /// The higher ranks higher.
    pub incarnation: usize,
    pub memo: Vec<u8>,
}

/// A shown field and the record it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub value: String,
    pub origin: Origin,
}

/// One topic as the desk shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub slug: String,
    pub title: Option<Field>,
    pub state: Option<Field>,
    pub what: Option<Field>,
    pub last: Option<Field>,
    pub next: Option<Field>,
    pub src: Option<Field>,
    /// Only when the shown state is `closed`.
    pub close: Option<Field>,
    /// The highest-ranked record's raw text, when no record carries a state
    /// or the shown one is no state word.
    pub unreadable: Option<String>,
    /// The timestamp of the highest-ranked record of the slug, any record.
    pub reviewed: String,
    /// The records that show no field, highest rank first.
    pub history: Vec<Entry>,
}

impl Card {
    /// A card with no record: a tag that names a slug no memo files.
    #[must_use]
    pub fn bare(slug: &str) -> Self {
        Self {
            slug: slug.to_owned(),
            title: None,
            state: None,
            what: None,
            last: None,
            next: None,
            src: None,
            close: None,
            unreadable: None,
            reviewed: String::new(),
            history: Vec::new(),
        }
    }
}

/// Every checkpoint record of `sources`, by slug, highest rank first.
#[must_use]
pub fn entries(sources: &[Source]) -> BTreeMap<String, Vec<Entry>> {
    let mut found: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    for source in sources {
        for (place, record) in crate::memo::records(&source.memo).enumerate() {
            let Some(slug) = std::str::from_utf8(record.topic).ok().and_then(slug_of) else {
                continue;
            };
            found.entry(slug.to_owned()).or_default().push(Entry {
                origin: Origin {
                    session: source.session.clone(),
                    uuid: source.uuid.clone(),
                    at: String::from_utf8_lossy(record.ts).into_owned(),
                },
                rank: (source.incarnation, place),
                text: String::from_utf8_lossy(record.text).into_owned(),
            });
        }
    }
    for list in found.values_mut() {
        list.sort_by_key(|entry| std::cmp::Reverse(entry.rank));
    }
    found
}

/// The cards of `sources`, sorted by slug.
#[must_use]
pub fn cards(sources: &[Source]) -> Vec<Card> {
    entries(sources)
        .into_iter()
        .map(|(slug, list)| fold(slug, list))
        .collect()
}

/// One slug's card from its records, highest rank first.
fn fold(slug: String, records: Vec<Entry>) -> Card {
    let parsed: Vec<Fields> = records
        .iter()
        .map(|entry| parse_record(&entry.text))
        .collect();
    let mut shown = vec![false; records.len()];
    let mut pick = |name: Name| {
        let index = parsed
            .iter()
            .position(|fields| fields.get(name).is_some())?;
        let value = parsed.get(index)?.get(name)?.clone();
        let origin = records.get(index)?.origin.clone();
        if let Some(flag) = shown.get_mut(index) {
            *flag = true;
        }
        Some(Field { value, origin })
    };
    let state = pick(Name::State);
    let parsed_state = state.as_ref().and_then(|field| State::parse(&field.value));
    let close = if parsed_state == Some(State::Closed) {
        pick(Name::Close)
    } else {
        None
    };
    let title = pick(Name::Title);
    let what = pick(Name::What);
    let last = pick(Name::Last);
    let next = pick(Name::Next);
    let src = pick(Name::Src);
    let unreadable = match parsed_state {
        Some(_) => None,
        None => records.first().map(|entry| entry.text.clone()),
    };
    let reviewed = records
        .first()
        .map(|entry| entry.origin.at.clone())
        .unwrap_or_default();
    let history = records
        .into_iter()
        .zip(shown)
        .filter_map(|(entry, shown)| (!shown).then_some(entry))
        .collect();
    Card {
        slug,
        title,
        state,
        what,
        last,
        next,
        src,
        close,
        unreadable,
        reviewed,
        history,
    }
}

/// A desk group, in the desk's fixed order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    NeedsYou,
    Moving,
    Quiet,
    Parked,
    Closed,
    Unreadable,
}

impl Group {
    /// Every group, in the desk's order.
    pub const ALL: [Self; 6] = [
        Self::NeedsYou,
        Self::Moving,
        Self::Quiet,
        Self::Parked,
        Self::Closed,
        Self::Unreadable,
    ];

    /// The group's word on the wire.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::NeedsYou => "needs_you",
            Self::Moving => "moving",
            Self::Quiet => "quiet",
            Self::Parked => "parked",
            Self::Closed => "closed",
            Self::Unreadable => "unreadable",
        }
    }
}

/// The group of `card`: a waiting owner action or the `needs` state puts it
/// in Needs You, any other state in its own group, and no readable state in
/// Unreadable.
#[must_use]
pub fn group(card: &Card, waits: bool) -> Group {
    if waits {
        return Group::NeedsYou;
    }
    match card
        .state
        .as_ref()
        .and_then(|field| State::parse(&field.value))
    {
        Some(State::Needs) => Group::NeedsYou,
        Some(State::Moving) => Group::Moving,
        Some(State::Quiet) => Group::Quiet,
        Some(State::Parked) => Group::Parked,
        Some(State::Closed) => Group::Closed,
        None => Group::Unreadable,
    }
}

/// The longest closing reason a page may send, in Unicode scalar values. The
/// page owns the reason's rules; the server only names the number.
pub const REASON_MAX: usize = 200;

/// What the owner decides about a topic in the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Keep,
    Ask,
    Close,
}

impl Verdict {
    /// The verdict's word in a chat line.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Ask => "ask",
            Self::Close => "close",
        }
    }
}

/// The chat line of a verdict on `slug`: `#<slug> keep` and `#<slug> ask`
/// whole, and for [`Verdict::Close`] only its start, `#<slug> close ` with
/// its trailing space, which the page completes with the owner's reason.
/// `None` for a text that is no slug.
#[must_use]
pub fn verdict_line(slug: &str, verdict: Verdict) -> Option<String> {
    if !is_slug(slug) {
        return None;
    }
    let tail = if verdict == Verdict::Close { " " } else { "" };
    Some(format!("#{slug} {}{tail}", verdict.word()))
}

/// The three verdict lines of one topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdicts {
    pub keep: String,
    pub ask: String,
    /// The start of the closing line, ending in one space.
    pub close: String,
}

/// The verdict lines of a card in `group`, or `None` when the card may not
/// take a verdict: only Quiet, Moving and Parked cards can be kept, asked
/// about or closed. A card with a wait is in Needs you, so a wait takes it out.
#[must_use]
pub fn verdicts(slug: &str, group: Group) -> Option<Verdicts> {
    if !matches!(group, Group::Quiet | Group::Moving | Group::Parked) {
        return None;
    }
    Some(Verdicts {
        keep: verdict_line(slug, Verdict::Keep)?,
        ask: verdict_line(slug, Verdict::Ask)?,
        close: verdict_line(slug, Verdict::Close)?,
    })
}

/// The order of quiet cards: the oldest review first. A review time that is
/// no `YYYY-MM-DDTHH:MM:SSZ` (empty, or hand-edited) counts as the oldest,
/// and equal times fall back to the slug.
#[must_use]
pub fn review_order(left: &Card, right: &Card) -> Ordering {
    let time = |card: &Card| crate::time::Timestamp::parse(&card.reviewed);
    time(left)
        .cmp(&time(right))
        .then_with(|| left.slug.cmp(&right.slug))
}

/// One quiet topic as the Revisit line shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revisit {
    pub slug: String,
    pub title: Option<String>,
    /// The first line of the topic's next step, else its last state, else
    /// what it is.
    pub line: Option<String>,
    pub reviewed: String,
    /// The slug of the following entry; the last entry points to the first,
    /// and a lone entry to itself.
    pub next: String,
}

/// The Revisit entries of `quiet`, in the order given, closed into a ring by
/// each entry's `next`.
#[must_use]
pub fn revisit(quiet: &[&Card]) -> Vec<Revisit> {
    quiet
        .iter()
        .enumerate()
        .map(|(at, card)| {
            let line = [&card.next, &card.last, &card.what]
                .into_iter()
                .flatten()
                .find_map(|field| field.value.lines().next().map(str::to_owned));
            let following = quiet.get(at + 1).or(quiet.first()).unwrap_or(card);
            Revisit {
                slug: card.slug.clone(),
                title: card.title.as_ref().map(|field| field.value.clone()),
                line,
                reviewed: card.reviewed.clone(),
                next: following.slug.clone(),
            }
        })
        .collect()
}
