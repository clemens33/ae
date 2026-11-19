//! `requests --verify <id>`: whether request `id`, as this session holds it,
//! carries the human's own words, read-only and from stored records alone.
//!
//! Two shapes read as `owner ingress`. An ask this session's own chat opened
//! (`ae chat` or Telegram: actor `console:local`, no relay keys, addressed to
//! this session's `session_id`). And a bound relay to this session's
//! `session_id` whose cited ask still sits in the dispatcher's ledger: the
//! dispatcher is the one session holding the `relay_origin` id now, whatever
//! its name; its journal holds exactly one claim of `id` that cites the same
//! ask, and exactly one opening of that ask, human-opened and addressed to the
//! dispatcher; and the words quoted in this session's stored relay equal the
//! words stored there, byte for byte, with the same time. Everything else reads
//! `not owner ingress` with its reason: an attested quote, an interim ruling, a
//! relay of an older ae, an unknown mode, an imported ask, a changed copy, an
//! ask for another session id.
//!
//! Nothing here writes: the journals and bodies are read, never locked.

use std::io::{self, Write};
use std::path::Path;

use super::{ATTEMPT, TASK_HEAD, claimed, logical_lines, opening, source_ask};
use crate::console::{self, lane::Body};
use crate::events::Event;
use crate::state::EXIT_FAILED;
use crate::tracked::{self, Kind};
use crate::{archive, lifecycle, meta, store};

/// What a proven owner ingress says of itself.
struct Proof {
    /// The stored sender of the human's ask.
    origin: String,
    /// The session whose ledger holds the human's ask.
    holding: String,
    /// When the human said it.
    time: String,
    /// The seat the request is addressed to here.
    target: String,
    /// The words, whole.
    text: String,
}

/// Print the verdict on request `id` held by the session at `dir`: exit 0 for
/// `owner ingress`, [`EXIT_FAILED`] for `not owner ingress`.
///
/// # Errors
///
/// Only a failure to write `out`.
pub fn run(dir: &Path, id: &str, out: &mut impl Write) -> io::Result<u8> {
    match prove(dir, id) {
        Ok(proof) => {
            write!(
                out,
                "owner ingress\norigin: {}\nholding session: {}\ntime: {}\ntarget: {}\ntext:\n{}\n",
                proof.origin, proof.holding, proof.time, proof.target, proof.text
            )?;
            Ok(0)
        }
        Err(why) => {
            write!(out, "not owner ingress\nreason: {why}\n")?;
            Ok(EXIT_FAILED)
        }
    }
}

/// The journal of the session at `dir`, read strictly and without a lock.
fn journal(dir: &Path) -> Result<Vec<Event>, String> {
    console::snapshot(store::read_source(&store::open(dir).events_path()))
}

/// The one ask opening of `id` among `events`.
fn the_opening<'a>(events: &'a [Event], id: &str) -> Result<&'a Event, String> {
    let mut openings = events.iter().filter(|event| {
        event.action == Kind::Ask.action() && event.reference.as_deref() == Some(id)
    });
    match (openings.next(), openings.next()) {
        (Some(ask), None) => Ok(ask),
        (None, _) => Err(format!("{id} has no ask opening here")),
        (Some(_), Some(_)) => Err(format!("{id} has more than one ask opening here")),
    }
}

fn prove(dir: &Path, id: &str) -> Result<Proof, String> {
    let own = meta::session_id_in(dir);
    if own.is_empty() {
        return Err("this session records no usable session id".to_owned());
    }
    let session = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    let events = journal(dir)?;
    let ask = the_opening(&events, id)?;
    if ask.actor != tracked::CONSOLE_SINK {
        return Err(format!("{id} was not opened by the console"));
    }
    if ask.target_session_id.value() != Some(own.as_str()) {
        return Err(format!("{id} is not addressed to this session id"));
    }
    let target = ask.target.clone().unwrap_or_default();
    let Some(origin) = ask.relay_origin.as_deref() else {
        let owner = source_ask(dir, &events, id, &own)?;
        return Ok(Proof {
            origin: ask.actor.clone(),
            holding: session,
            time: owner.said.to_string(),
            target,
            text: owner.text,
        });
    };
    match ask.relay_mode.as_deref() {
        Some("bound") => {}
        Some(mode) => return Err(format!("{id} is a {mode} relay: its words are not proven")),
        None => {
            return Err(format!(
                "{id} is a relay of an older ae: no words are proven"
            ));
        }
    }
    let Some(cited) = ask.owner_input.as_deref().filter(|cited| !cited.is_empty()) else {
        return Err(format!("{id} is a bound relay that cites no ask"));
    };
    let (copy_id, copy_time, copy_words) = match console::relay_body(dir, ask) {
        Body::Whole(sections) => owner_section(&sections)?,
        gap => {
            return Err(format!(
                "the relay cannot be read whole ({})",
                console::lane::body_gap(&gap).unwrap_or_default()
            ));
        }
    };
    if copy_id != cited {
        return Err(format!("the relay's words cite {copy_id}, not {cited}"));
    }
    let (holder, holder_dir) = dispatcher(dir, origin)?;
    let held = journal(&holder_dir).map_err(|why| format!("the dispatcher's journal: {why}"))?;
    let claim = claimed(&held, id, origin)?;
    // `claimed` proved exactly one attempt of `id`: it must be a bound one
    // citing this ask, not a quote or an interim ruling that reuses the id.
    let bound_claim = held
        .iter()
        .filter(|event| event.action == ATTEMPT && event.reference.as_deref() == Some(id))
        .any(|event| {
            event.relay_mode.as_deref() == Some("bound")
                && event.owner_input.as_deref() == Some(cited)
        });
    if !bound_claim {
        return Err(format!(
            "the dispatcher's claim of {id} is not a bound relay citing {cited}"
        ));
    }
    if claim.target_uuid != own {
        return Err(format!(
            "the dispatcher claimed {id} for another session id"
        ));
    }
    if opening(&events, id, origin, &claim)?.is_none() {
        return Err(format!("{id} is not the claimed relay's opening"));
    }
    let source = source_ask(&holder_dir, &held, cited, origin)
        .map_err(|why| format!("the dispatcher's ledger: {why}"))?;
    let (words, time) = (source.text, source.said.to_string());
    if words != copy_words {
        return Err("the relay's quoted words differ from the stored ask".to_owned());
    }
    if time != copy_time {
        return Err("the relay's time differs from the stored ask".to_owned());
    }
    Ok(Proof {
        origin: tracked::CONSOLE_SINK.to_owned(),
        holding: holder,
        time,
        target,
        text: words,
    })
}

/// The one session whose meta records `origin` now, by name and directory.
fn dispatcher(dir: &Path, origin: &str) -> Result<(String, std::path::PathBuf), String> {
    let origin = archive::canonical_uuid(origin);
    if origin.is_empty() {
        return Err("the relay origin is not a session id".to_owned());
    }
    let root = dir
        .parent()
        .and_then(Path::parent)
        .ok_or("the session root could not be resolved")?;
    let names = lifecycle::sessions_with_id(root, &origin)
        .map_err(|why| format!("cannot enumerate sessions to find the dispatcher ({why})"))?;
    let [name] = names.as_slice() else {
        return Err(format!(
            "{} sessions hold the dispatcher's session id, not one",
            names.len()
        ));
    };
    Ok((name.clone(), lifecycle::sessions_dir(root).join(name)))
}

/// The owner section of a relay's `sections`: the ask id it cites, the time it
/// names, and the quoted words with their separators restored. The section is
/// exactly what ae renders for a bound relay; anything else is refused.
fn owner_section(sections: &str) -> Result<(String, String, String), String> {
    const HEAD: &str = "OWNER'S WORDS (owner input ";
    const MID: &str = ", stored by ae; said ";
    let refuse = |why: &str| Err(format!("the relay's owner section is not ae's own: {why}"));
    let mut lines = logical_lines(sections).into_iter();
    let (first, separator) = lines.next().unwrap_or(("", ""));
    let Some(inner) = first
        .strip_prefix(HEAD)
        .and_then(|rest| rest.strip_suffix("):"))
    else {
        return refuse("no bound header");
    };
    let Some((cited, said)) = inner.split_once(MID) else {
        return refuse("no bound header");
    };
    if separator != "\n" {
        return refuse("the header ends oddly");
    }
    let mut quoted: Vec<(&str, &str)> = Vec::new();
    let mut task = None;
    for (line, separator) in lines {
        if let Some(words) = line.strip_prefix("> ") {
            quoted.push((words, separator));
        } else {
            task = Some((line, separator));
            break;
        }
    }
    if quoted.is_empty() || task.map(|(line, _)| line) != Some(TASK_HEAD) {
        return refuse("no quoted words before TASK");
    }
    // The last quoted line's separator is the LF ae writes before TASK, not
    // part of the words.
    let mut words = String::new();
    let last = quoted.len() - 1;
    for (at, (line, separator)) in quoted.into_iter().enumerate() {
        words.push_str(line);
        if at < last {
            words.push_str(separator);
        } else if separator != "\n" {
            return refuse("the quote does not end at TASK");
        }
    }
    Ok((cited.to_owned(), said.to_owned(), words))
}

#[cfg(test)]
mod tests {
    use super::owner_section;
    use crate::relay::{OwnerAsk, parse, sections};
    use crate::time::Timestamp;

    /// The bound owner section ae renders for `text`, as one relay carries it.
    fn rendered(text: &str, said: &str) -> String {
        let args: Vec<String> = [
            "t",
            "--owner-input",
            "ae-20261119T120000Z-0000abcd",
            "--task",
            "go",
        ]
        .map(str::to_owned)
        .into();
        let parsed = parse(&args).expect("bound argv");
        let ask = OwnerAsk {
            said: Timestamp::parse(said).expect("time"),
            text: text.to_owned(),
        };
        sections(&parsed, Some(&ask))
    }

    #[test]
    fn the_owner_section_gives_back_the_stored_words_whatever_their_line_ends() {
        let said = "2026-11-19T10:15:23Z";
        for text in [
            "",
            "a",
            "a\nb",
            "a\rb",
            "a\r\nb",
            "a\n",
            "a\r",
            "a\r\n",
            "\n\n",
            "a\n\r\nb\r",
            "> quoted",
            "TASK (orchestrator's statement; no human authority):",
        ] {
            let (id, time, words) = owner_section(&rendered(text, said))
                .unwrap_or_else(|why| panic!("{text:?} read back as {why}"));
            assert_eq!(id, "ae-20261119T120000Z-0000abcd");
            assert_eq!(time, said);
            assert_eq!(words, text, "the quoted words are the stored words");
        }
    }

    #[test]
    fn an_owner_section_ae_did_not_render_reads_as_not_proven() {
        let ok = rendered("a", "2026-11-19T10:15:23Z");
        for changed in [
            ok.replacen(
                "OWNER'S WORDS (owner input",
                "OWNER'S WORDS (attested by",
                1,
            ),
            ok.replacen("\n> a", "\na", 1),
            ok.replacen("\nTASK", "\ntask", 1),
            ok.replacen("):\n", "):\r\n", 1),
            "go".to_owned(),
        ] {
            assert!(owner_section(&changed).is_err(), "{changed:?}");
        }
    }
}
