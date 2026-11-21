//! The seat's own `orchestrator.config` across a publish, and the running lead
//! seats whose relay context stamp this core refuses.
//!
//! **The ledger.** ae seeds `<state root>/orchestrator.config` from
//! `contrib/aeorchestrator/orchestrator.config` once, and a later release that
//! changed the template left the seeded copy behind. [`LEDGER`] holds one row
//! per template ever released, the SHA-256 of the whole file and of its
//! `[prompt]` stanza, with the current template last: a publish recognises what
//! an earlier ae wrote without git, a network or the old files. A change to the
//! template appends its row in the same commit, and a guard test fails without
//! it.
//!
//! **The verdict.** [`classify`] reads one file's text and answers, in order: the
//! current template (nothing to do), a whole earlier template (replace the file:
//! a release that changed only the `[workspace]` shares its prompt stanza with
//! the current one, so the whole file is asked first), the current stanza
//! (nothing to do), an earlier stanza (replace only the prompt row), a prompt
//! that already teaches the bound owner words and the current charter (nothing
//! to do, owner edits kept), one that teaches the owner words but an older
//! charter (leave the file, name the repair) or none of these (leave the file,
//! name the repair).
//!
//! **The reconcile.** [`reconcile`] runs once per root after its sweep, with or
//! without an orchestrator session. A replacement keeps the old file at the
//! first free `<file>.pre-v<version>[-N]` and says so in one line. No step here
//! stops or starts a seat: the seat reads its prompt at launch, and a running
//! seat whose charter the replacement raised gets the new prompt as one
//! CHARTER UPDATE turn from its watchdog after it next declares `done` or
//! `waiting-user` at an idle input box.

use super::{SEAT_CONFIG_CAP, publish_reseed};
use crate::config::{
    SEAT_CHARTER_3_MARKER, SEAT_CHARTER_MARKER, SEAT_CHARTER_VERSION, SEAT_TEMPLATE,
    seat_backup_path, seat_config_directory_repair, seat_config_repair, seat_prompt_charter,
    seat_prompt_is_current, seat_prompt_replaced, seat_prompt_stanza,
};
use crate::install::sha256_hex;
use crate::store::{SourcePresence, SourceRead};
use std::io;
use std::path::Path;

/// One seat template ae released: the release that first carried it, the
/// SHA-256 of the whole file and of the `[prompt]` stanza
/// ([`seat_prompt_stanza`]), which a template without a `[prompt]` section has
/// none of.
struct Released {
    tag: &'static str,
    file: &'static str,
    stanza: Option<&'static str>,
}

/// Every released seat template, oldest first; the last row is the template
/// this core seeds.
const LEDGER: &[Released] = &[
    Released {
        tag: "v2026.8.2",
        file: "0a8307fbb68d1787b3bc487125f733b2134488736ccaaab8b014407c6f708a91",
        stanza: Some("64edb507ff916271708cecc6455b045e2623d93f11b5b1f8696b57064bf6d911"),
    },
    Released {
        tag: "v2026.9.1",
        file: "ea45bcc3fceac9fff3a334d2b8cd5f890866f0117f393ab1712103c9bc31dabb",
        stanza: Some("64edb507ff916271708cecc6455b045e2623d93f11b5b1f8696b57064bf6d911"),
    },
    Released {
        tag: "v2026.9.2",
        file: "8274b3dbf7614abece6883103298d589d85e0c0af4d7ebcdf4643f34339dc80c",
        stanza: Some("64edb507ff916271708cecc6455b045e2623d93f11b5b1f8696b57064bf6d911"),
    },
    Released {
        tag: "v2026.9.6",
        file: "4033660a0601003f8c4e5ec965c03f24cb8401ee7e5dabd7af10cc68a7117664",
        stanza: Some("a57e785f81611939b39166d0cbb36ccf119d97473cc34b52b800f14e5d8de4b1"),
    },
    Released {
        tag: "v2026.9.9",
        file: "83a89f9e0924843ec7eb00246d614432ed9a46fbf93bb1638320d12249ad362d",
        stanza: Some("a57e785f81611939b39166d0cbb36ccf119d97473cc34b52b800f14e5d8de4b1"),
    },
    Released {
        tag: "v2026.9.12",
        file: "ea4dabb17a3e04d2128f0bf1eacd3fc7f313433dc6393584220b874ecc36e9d1",
        stanza: Some("a57e785f81611939b39166d0cbb36ccf119d97473cc34b52b800f14e5d8de4b1"),
    },
    Released {
        tag: "v2026.9.15",
        file: "7a8dcf4f1aa2264660dcbdb6e7e452960ec1aed8e5d74e23465c2ef23369e430",
        stanza: Some("eb52ad68a3ba75a420b20e24efebdca53a98d8f88dee483c8ee8e86c5727cded"),
    },
    Released {
        tag: "v2026.9.18",
        file: "2ffca2bfb61ddd6a9355cbc69585fa2dfddad611e56befd1fab40de45e68fb18",
        stanza: Some("18c1497b0d32dcfd3485a3cf197863ca79811a0005c8f80390d1827cc37ce601"),
    },
    Released {
        tag: "v2026.9.19",
        file: "55e16f35e96ca984b4c87d102fd3f2431c1e17c3fea72938cdaa8e237e5cf296",
        stanza: Some("ae6c692826bd83f58c7767485a0e4f92d09eeed13ebeb2605dd186151f9a7d8a"),
    },
    Released {
        tag: "v2026.9.22",
        file: "a40a512693f7af45e3c10e0179b65c82adceddd6881464e9d2e9e91245f39870",
        stanza: Some("839f70c45e485d05bd646bdefc6c6d93f6b2dcc0a7ef98bf27518d6b1e6dde50"),
    },
    Released {
        tag: "v2026.9.23",
        file: "4cd66084e07fbf44406597a17954065a8f9055a51d8697e7a7a0f08c13160597",
        stanza: Some("592aaab8d188b09333cb78a37f7ec0b3f00285890d4907346244bb20779c0aab"),
    },
    Released {
        tag: "v2026.9.25",
        file: "2fb7396ff18df495f2cd3d1b21020f7bc5dd4bb1b04611a5aa1d05cd829fa30d",
        stanza: Some("dea1bbcfd830c46309d333837fa87fcc2825fbf14e893150a94fe070945aec1d"),
    },
    Released {
        tag: "v2026.9.27",
        file: "2717280abb4d6ee0d7d051f83d3df8dc7dc0e3280c1544f4945a1e4f265d88ef",
        stanza: Some("c213c5dd3b822de0e90d270083b4ba6d58a2820d1e12350a338c58a995951dfb"),
    },
    Released {
        tag: "v2026.9.61",
        file: "6df47c08ed4ed3ae5cde6106d4608e8dbe065a9052a8b3f7a6629e6d9f0d5ead",
        stanza: Some("6e9972f229da5be4c10b7ec266c89bdc4b1bec06d280046cb7cf04e195dc72e5"),
    },
    Released {
        tag: "v2026.10.133",
        file: "55da19d7f3e3d28701b1ab8607222b517ba8d2d38cb5587dc8696e2b6863b05a",
        stanza: Some("5caed9916831b0d975d6cb8335aa3b3ce66b5e267a5ff338bfdca64118d87d2b"),
    },
    Released {
        tag: "v2026.10.142",
        file: "cbb1ca0df09fbe49d4d977bee18c3223a14e2281972c14421983acb3ded6811f",
        stanza: Some("ec7da9db5ad2f40050d18ac99adeedb748173ccde6faf88e97a83cf9ee371f27"),
    },
    Released {
        tag: "v2026.10.144",
        file: "5a667f94bcd4b9f6da1211e77a5d1663b2cf082a76ed809a23c5e4f734d9119b",
        stanza: Some("ffd5d20aefc44f0a47edc1117a2fc87f3ed0e78238ebd4e88c031a92a3fb6c4f"),
    },
    Released {
        tag: "v2026.11.32",
        file: "d94f74dca9e9da4debe36f17e0e379f66fd11de0d7580ba05310bca314d6a808",
        stanza: Some("01e290210f2b3142746bd33e3383b2d453201d54368819d5428b752bc878b1ab"),
    },
    Released {
        tag: "v2026.11.94",
        file: "a5b22eefb74cfcca57e5b521ec0be9ef16820c5e74efe12a056ebd8e956a8540",
        stanza: Some("c516017d3caef922239317b06ee5b72933ba4f78983c57e3317693a850817911"),
    },
    Released {
        tag: "v2026.11.100",
        file: "ba40b53d3e210fb82cd5f8ad28b6ce2c13cd53d81ed531cacfe218df9a91f808",
        stanza: Some("493c14e1281a99e67352d58b6ff2584e8ddb287849cfa479e9d0bce44d9e2e58"),
    },
    Released {
        tag: "v2026.11.101",
        file: "bd9362fc5be863a7c93fbac6c73ea0b18a710200ede0efdb4779157e1164334f",
        stanza: Some("da7ed4af79a961e45a241d329339c871f4eb463a5d5f90ed488e5d60afefb95b"),
    },
    Released {
        tag: "v2026.11.102",
        file: "7521c7b364d38d1a993a64b8fc0559c6c69a02a72b76211f9943af347fac0e6a",
        stanza: Some("1e20c028c1964bf25f7929e23f543b2c15fd8e6945e4bfe6f157d410822a10eb"),
    },
    Released {
        tag: "v2026.11.103",
        file: "44f69983ac246b53cff9fc5e662d03fa4946081a02de020653b94bf45057fa74",
        stanza: Some("c2c1aa8290d28afdc62abcbca387b17610e7eecbf0cc97bd0f9c6282c73f910f"),
    },
];

/// What a publish does with one seat config.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// The current template, or a prompt that already teaches the bound owner
    /// words and the current charter: nothing to do and nothing to say.
    Compatible,
    /// A prompt that teaches the bound owner words but an older charter, which
    /// no released template wrote: the file stays as it is, the relay keeps
    /// accepting it, and the repair is named.
    Charterless,
    /// Byte for byte an earlier template: the file becomes the current template.
    Whole(&'static str),
    /// An earlier template's prompt stanza: only the prompt row is replaced, and
    /// the text is the file with it.
    Stanza(&'static str, String),
    /// A prompt without the bound owner words that no released template wrote.
    Stale,
}

/// [`classify_with`] over the [`LEDGER`].
fn classify(text: &str) -> Verdict {
    classify_with(text, LEDGER)
}

/// The verdict on the seat config `text` against `ledger`, whose last row is
/// the current template.
fn classify_with(text: &str, ledger: &[Released]) -> Verdict {
    let Some((current, earlier)) = ledger.split_last() else {
        return Verdict::Stale;
    };
    let file = sha256_hex(text.as_bytes());
    if file == current.file {
        return Verdict::Compatible;
    }
    // BEFORE the stanza: a released template that differs from the current one
    // in its [workspace] alone carries the current prompt stanza, and the
    // stanza check would take it for current.
    if let Some(row) = earlier.iter().find(|row| row.file == file) {
        return Verdict::Whole(row.tag);
    }
    let stanza = seat_prompt_stanza(text).map(|stanza| sha256_hex(stanza.as_bytes()));
    if stanza.is_some() && stanza.as_deref() == current.stanza {
        return Verdict::Compatible;
    }
    if let Some(stanza) = stanza.as_deref()
        && let Some(row) = earlier.iter().find(|row| row.stanza == Some(stanza))
        && let Some(new) = seat_prompt_replaced(text)
    {
        return Verdict::Stanza(row.tag, new);
    }
    if !seat_prompt_is_current(text) {
        Verdict::Stale
    } else if seat_prompt_charter(text) < SEAT_CHARTER_VERSION {
        Verdict::Charterless
    } else {
        Verdict::Compatible
    }
}

/// Bring the seat config of `root` to the current template as far as a publish
/// of `version` may, and return the one line that says what happened, or `None`
/// when there is nothing to say: no file, a current template, or a prompt that
/// already teaches the bound owner words and the current charter. A prompt with
/// the owner words and an older charter is left as it is and named with its
/// repair. A file that is not a regular file, is
/// over the cap or is not UTF-8 is left as it is and named with its repair, like
/// a prompt no released template wrote.
pub(super) fn reconcile(root: &Path, version: &str) -> Option<String> {
    let path = root.join(crate::meta::SEAT_CONFIG_FILE);
    let instance = crate::instance::instance_of(root);
    let unchanged = |why: &str| {
        let backup = seat_backup_path(&path, version);
        Some(format!(
            "left {} unchanged: {why}; {}",
            path.display(),
            seat_config_repair(&path, &backup, instance)
        ))
    };
    let bytes = match crate::store::read_capped(&path, SEAT_CONFIG_CAP) {
        Ok(SourceRead::Absent) => return None,
        Ok(SourceRead::Ready(bytes)) => bytes,
        Ok(SourceRead::Invalid(why) | SourceRead::Unreadable(why)) => {
            // A directory cannot be hard-linked aside, so it gets its own move.
            if matches!(
                crate::store::source_presence(&path),
                SourcePresence::Irregular(ref what) if what == "a directory"
            ) {
                let backup = seat_backup_path(&path, version);
                return Some(format!(
                    "left {} unchanged: it is a directory, not a file; {}",
                    path.display(),
                    seat_config_directory_repair(&path, &backup, instance)
                ));
            }
            return unchanged(&format!(
                "it is not a regular file this upgrade can read ({why})"
            ));
        }
        Err(_) => return unchanged("it is larger than this upgrade reads"),
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return unchanged("it is not UTF-8 text this upgrade can classify");
    };
    let (new, what) = match classify(&text) {
        Verdict::Compatible => return None,
        Verdict::Charterless => {
            // The sentence of the first charter version the prompt lacks.
            let missing = if seat_prompt_charter(&text) < 2 {
                SEAT_CHARTER_MARKER
            } else {
                SEAT_CHARTER_3_MARKER
            };
            return unchanged(&format!(
                "its seat prompt keeps the bound owner words but lacks the charter sentence \
                 \"{missing}\", so the seat is not taught the current charter and its \
                 watchdog sends it no CHARTER UPDATE turn"
            ));
        }
        Verdict::Stale => {
            return unchanged(
                "its seat prompt predates the bound owner words (or an owner edit removed \
                 them) and the file matches no released template",
            );
        }
        Verdict::Whole(tag) => (
            SEAT_TEMPLATE.to_owned(),
            format!(
                "replaced {} with the current seat template (it matched the {tag} template \
                 byte for byte)",
                path.display()
            ),
        ),
        Verdict::Stanza(tag, new) => (
            new,
            format!(
                "replaced the [prompt] of {} with the current one (it matched the {tag} \
                 prompt byte for byte, and your other edits stay)",
                path.display()
            ),
        ),
    };
    let backup = seat_backup_path(&path, version);
    if let Err(why) = replace(&path, &text, &new, &backup) {
        return unchanged(&format!("it could not be replaced ({why})"));
    }
    let raised = seat_prompt_charter(&new) > seat_prompt_charter(&text);
    Some(format!(
        "{what}; the previous file is kept at {}; {}",
        backup.display(),
        restart(instance.is_some(), raised)
    ))
}

/// How a replacement line tells the owner the seat loads the new file. When the
/// replacement raises the charter, a running seat gets the new prompt as one
/// CHARTER UPDATE turn from its watchdog after it next declares `done` or
/// `waiting-user` at an idle input box, and only a restart loads the rest of
/// the file. A named instance's line names no ae command, because the one on
/// the path starts the default root.
fn restart(named_instance: bool, raised: bool) -> &'static str {
    match (named_instance, raised) {
        (false, true) => {
            "a running orchestrator seat gets the new prompt as one CHARTER UPDATE turn after it \
             next declares done or waiting-user at an idle input box, and a restart loads the \
             whole file: ae stop orchestrator, then ae orchestrator"
        }
        (true, true) => {
            "a running orchestrator seat of that instance gets the new prompt as one CHARTER \
             UPDATE turn after it next declares done or waiting-user at an idle input box, and a \
             restart with that instance's own ae command loads the whole file"
        }
        (false, false) => {
            "restart the orchestrator seat to load it: ae stop orchestrator, then ae orchestrator"
        }
        (true, false) => {
            "restart that instance's orchestrator seat with its own ae command to load it"
        }
    }
}

/// Keep `seen` at `backup` and publish `new` over `path`. The backup is made
/// exclusively and read back equal before the file is touched, so a name that
/// is taken in any form stops the replacement and nothing is overwritten.
fn replace(path: &Path, seen: &str, new: &str, backup: &Path) -> io::Result<()> {
    write_new(backup, seen.as_bytes())?;
    publish_reseed(path, seen.as_bytes(), new)
}

/// Write `bytes` to a file that did not exist at `path` and prove they read back.
fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    match crate::store::read_capped(path, SEAT_CONFIG_CAP) {
        Ok(SourceRead::Ready(back)) if back == bytes => Ok(()),
        _ => Err(io::Error::other("the backup did not read back equal")),
    }
}

/// The lead seats of one running session whose relay context stamp is not
/// current, split by what makes it current again.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct StaleSeats {
    /// The named instance root the session lives in, `None` for the default root.
    instance: Option<String>,
    session: String,
    /// A stop and a resume hands the seat's conversation the relay context.
    resume: Vec<String>,
    /// The tool's resume carries no context turn: only a fresh conversation does.
    fresh: Vec<String>,
}

/// The lead seats of session `name` (state at `dir`) that are running and carry
/// no `context.<slot>` row at or above this core's [`CONTEXT_VERSION`], or
/// `None`. A seat counts as running when its session exists on its recorded
/// server and a pane stamped with its slot is alive: not dead and not at a
/// shell. A tool without a context channel is left out, because no restart
/// records a stamp for it.
///
/// [`CONTEXT_VERSION`]: crate::render::CONTEXT_VERSION
pub(super) fn stale_context(dir: &Path, name: &str, instance: Option<&str>) -> Option<StaleSeats> {
    use crate::meta::{ContextStamp, Meta};
    use crate::run::ContextRoute;

    let server = crate::session_launch::recorded_server_resolved(dir)?;
    if !crate::transport::session_exists(&server, name) {
        return None;
    }
    let bytes = crate::meta::read_bytes(dir).ok()?;
    let parsed = Meta::parse(&String::from_utf8_lossy(&bytes));
    let slots: Vec<String> = ["main", "worker.0"]
        .into_iter()
        .filter(|slot| crate::render::seat_role(&bytes, slot) == crate::render::SeatRole::Lead)
        .filter(|slot| {
            !matches!(
                parsed.context_stamp(slot),
                ContextStamp::Valid(version) if version >= crate::render::CONTEXT_VERSION
            )
        })
        .map(str::to_owned)
        .collect();
    if slots.is_empty() {
        return None;
    }
    let panes = crate::transport::observe_panes(&server, name)?;
    let mut stale = StaleSeats {
        instance: instance.map(str::to_owned),
        session: name.to_owned(),
        ..StaleSeats::default()
    };
    for seat in crate::liveness::agent_runtimes(&panes, &slots) {
        if seat.alive != Some(true) {
            continue;
        }
        let tool = crate::tool::ToolKind::from_binary_name(
            &crate::meta::first_value(&bytes, &format!("agent_bin.{}", seat.slot))
                .map(|value| String::from_utf8_lossy(value).into_owned())
                .unwrap_or_default(),
        );
        match crate::run::context_route(tool) {
            ContextRoute::Resume => stale.resume.push(seat.slot),
            ContextRoute::FreshOnly => stale.fresh.push(seat.slot),
            ContextRoute::NoChannel => {}
        }
    }
    (!stale.resume.is_empty() || !stale.fresh.is_empty()).then_some(stale)
}

/// A session name as one shell word for `ae stop <name>; ae <name>`, or `None`
/// when it cannot make the round trip through both commands: `all` makes
/// `ae stop` stop the whole fleet, a name that starts with `-` reads as an
/// option (neither command takes an option terminator), and one with a control
/// character would break the line. A name outside the ordinary grammar, which an
/// existing session may still carry, is single-quoted. A name that equals a
/// subcommand is not excluded: the verb list lives in the shell layer, which
/// this layer may not read.
fn command_word(name: &str) -> Option<String> {
    if name == "all"
        || name.is_empty()
        || name.starts_with('-')
        || name.chars().any(char::is_control)
    {
        return None;
    }
    if crate::lifecycle::name_is_valid(name) {
        return Some(name.to_owned());
    }
    Some(crate::words::shell_quote(name))
}

/// The session as the notice names it, qualified by its instance: the name as
/// a shell word when a command can take it, else its escaped form.
fn shown(seats: &StaleSeats) -> String {
    let name = command_word(&seats.session).unwrap_or_else(|| format!("{:?}", seats.session));
    match &seats.instance {
        None => name,
        Some(instance) => format!("instance {instance}/{name}"),
    }
}

/// What the owner types to stop and resume the session of `seats`.
fn resume_how(seats: &StaleSeats) -> String {
    match (&seats.instance, command_word(&seats.session)) {
        (None, Some(word)) => format!("ae stop {word}; ae {word} --resume"),
        (Some(_), Some(_)) => "with that instance's own ae command".to_owned(),
        (_, None) => "by hand: the session name cannot be given to ae stop or ae as an \
                      argument"
            .to_owned(),
    }
}

/// The one line that names every seat in `stale`, across every root the publish
/// swept, with what makes each seat relayable again, or `None` for no seat. One
/// entry per session, with a remedy for each affected slot, since the two lead
/// seats of a pair can run different tools. A named instance's entries name no
/// ae command, because the one this core could print starts the default root.
pub(super) fn context_notice(stale: &[StaleSeats]) -> Option<String> {
    let count: usize = stale
        .iter()
        .map(|seats| seats.resume.len() + seats.fresh.len())
        .sum();
    if count == 0 {
        return None;
    }
    let (plural, verb) = if count == 1 {
        ("", "lacks")
    } else {
        ("s", "lack")
    };
    let entries: Vec<String> = stale
        .iter()
        .filter(|seats| !seats.resume.is_empty() || !seats.fresh.is_empty())
        .map(|seats| {
            let mut remedies: Vec<String> = Vec::new();
            if !seats.resume.is_empty() {
                let need = if seats.resume.len() == 1 {
                    "needs"
                } else {
                    "need"
                };
                remedies.push(format!(
                    "{} {need} a stop and resume ({})",
                    seats.resume.join(","),
                    resume_how(seats)
                ));
            }
            if !seats.fresh.is_empty() {
                let need = if seats.fresh.len() == 1 {
                    "needs"
                } else {
                    "need"
                };
                remedies.push(format!(
                    "{} {need} a fresh conversation (a resume records no stamp for its tool)",
                    seats.fresh.join(",")
                ));
            }
            format!("{}: {}", shown(seats), remedies.join(", "))
        })
        .collect();
    Some(format!(
        "{count} running lead seat{plural} {verb} a current relay context stamp (the row is \
         absent, malformed or older than version {}), so orchestrator relays to them refuse: {}",
        crate::render::CONTEXT_VERSION,
        entries.join(" | ")
    ))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real directories; the boundary is about what \
                  PRODUCT code may reach"
    )]

    use super::*;
    use std::path::PathBuf;

    /// The seat templates the acceptance fixtures hold, by file stem.
    fn fixtures() -> Vec<(String, String)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/orchestrator");
        let mut found: Vec<(String, String)> = std::fs::read_dir(dir)
            .expect("fixture directory")
            .map(|entry| entry.expect("entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "config"))
            .map(|path| {
                let stem = path.file_stem().expect("stem").to_string_lossy();
                let tag = match stem.as_ref() {
                    "s1a-previous" => "v2026.11.32".to_owned(),
                    // The executor acceptance's released overlay: v2026.11.98
                    // still seeded the template v2026.11.94 first carried.
                    "orchexec-v2026.11.98" => "v2026.11.94".to_owned(),
                    _ => stem.into_owned(),
                };
                (tag, std::fs::read_to_string(&path).expect("fixture text"))
            })
            .collect();
        found.sort();
        found
    }

    fn digest(text: &str) -> String {
        sha256_hex(text.as_bytes())
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ae-seatcfg-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch root");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_last_ledger_row_is_the_template_this_core_seeds() {
        let last = LEDGER.last().expect("a ledger");
        assert_eq!(
            digest(SEAT_TEMPLATE),
            last.file,
            "contrib/aeorchestrator/orchestrator.config changed: append this row to LEDGER in \
             src/l4_orchestration/migrate/seat_config.rs, with the release tag that first \
             carries it, and add the previous template as tests/fixtures/orchestrator/<tag>.config:\n\
             Released {{ tag: \"<release tag>\", file: {:?}, stanza: {:?} }}",
            digest(SEAT_TEMPLATE),
            seat_prompt_stanza(SEAT_TEMPLATE).map(digest)
        );
        assert_eq!(
            seat_prompt_stanza(SEAT_TEMPLATE).map(digest).as_deref(),
            last.stanza
        );
    }

    #[test]
    fn the_ledger_rows_are_digests_each_template_once() {
        let digests = |row: &Released| {
            [Some(row.file), row.stanza]
                .into_iter()
                .flatten()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let mut files = Vec::new();
        for row in LEDGER {
            for hash in digests(row) {
                assert!(
                    hash.len() == 64
                        && hash
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                    "{}: {hash}",
                    row.tag
                );
            }
            assert!(
                !files.contains(&row.file),
                "{}: a template is one row",
                row.tag
            );
            files.push(row.file);
        }
        let (current, earlier) = LEDGER.split_last().expect("a ledger");
        assert!(earlier.iter().all(|row| row.file != current.file));
    }

    #[test]
    fn every_released_template_the_fixtures_hold_matches_its_own_ledger_row() {
        let held = fixtures();
        let mut report = String::new();
        for (tag, text) in &held {
            let stanza = seat_prompt_stanza(text).map(digest);
            let row = LEDGER.iter().find(|row| row.tag == tag);
            let expected = format!(
                "    Released {{\n        tag: {tag:?},\n        file: {:?},\n        stanza: {:?},\n    }},\n",
                digest(text),
                stanza
            );
            match row {
                Some(row) if row.file == digest(text) && row.stanza == stanza.as_deref() => {}
                _ => report.push_str(&expected),
            }
        }
        assert!(report.is_empty(), "ledger rows to write:\n{report}");
        // Every earlier row has its template here, so no row stands without a file to prove it.
        let tags: Vec<&str> = held.iter().map(|(tag, _)| tag.as_str()).collect();
        let (_, earlier) = LEDGER.split_last().expect("a ledger");
        for row in earlier {
            assert!(tags.contains(&row.tag), "no fixture for {}", row.tag);
        }
        assert_eq!(held.len(), earlier.len());
    }

    #[test]
    fn a_released_template_is_replaced_whole_and_the_current_one_is_not() {
        for (tag, text) in fixtures() {
            let verdict = classify(&text);
            assert_eq!(
                verdict,
                Verdict::Whole(LEDGER.iter().find(|row| row.tag == tag).expect("row").tag),
                "{tag}"
            );
        }
        assert_eq!(classify(SEAT_TEMPLATE), Verdict::Compatible);
    }

    #[test]
    fn an_old_stanza_in_an_edited_file_gets_only_its_prompt_row_replaced() {
        let septenber = fixtures()
            .into_iter()
            .find(|(tag, _)| tag == "v2026.9.61")
            .expect("fixture")
            .1;
        let edited = format!("# mine\n{septenber}");
        let Verdict::Stanza(tag, new) = classify(&edited) else {
            panic!("an edit outside the stanza is kept");
        };
        assert_eq!(tag, "v2026.9.61");
        assert!(new.starts_with("# mine\n") && seat_prompt_is_current(&new));
        // An edit inside the stanza matches no stanza, and the file no template.
        let inside = septenber.replace("You are the ae orchestrator:", "Owner's prompt:");
        assert_ne!(inside, septenber);
        assert_eq!(classify(&inside), Verdict::Stale);
    }

    #[test]
    fn a_prompt_that_teaches_the_owner_words_stays_whatever_else_the_owner_edited() {
        let (body, tail) = SEAT_TEMPLATE
            .rsplit_once("\n\"\"\"")
            .expect("the template's prompt block");
        let edited = format!("{body}\nAn owner note.\n\"\"\"{tail}");
        assert_ne!(edited, SEAT_TEMPLATE);
        assert_eq!(classify(&edited), Verdict::Compatible);
        // Only a prompt row carries it: a comment does not.
        let comment = format!(
            "# {}\n{}",
            crate::config::SEAT_PROMPT_MARKER,
            "[prompt]\ninstructions = old\n"
        );
        assert_eq!(classify(&comment), Verdict::Stale);
    }

    #[test]
    fn the_ledger_is_asked_before_the_marker_so_an_old_template_carrying_it_still_migrates() {
        let old = "[workspace]\nmain = a\n\n[prompt]\ninstructions = old words\n";
        let marked = format!(
            "[workspace]\nmain = a\n\n[prompt]\ninstructions = {}\n",
            crate::config::SEAT_PROMPT_MARKER
        );
        let ledger = [
            Released {
                tag: "v-old",
                file: Box::leak(digest(&marked).into_boxed_str()),
                stanza: None,
            },
            Released {
                tag: "v-current",
                file: Box::leak(digest(old).into_boxed_str()),
                stanza: None,
            },
        ];
        assert_eq!(classify_with(&marked, &ledger), Verdict::Whole("v-old"));
        assert_eq!(classify_with(old, &ledger), Verdict::Compatible);
        assert_eq!(classify_with("", &[]), Verdict::Stale);
    }

    #[test]
    fn a_released_template_sharing_the_current_stanza_still_migrates_whole() {
        let leak = |text: String| -> &'static str { Box::leak(text.into_boxed_str()) };
        let current = "[workspace]\nmain = b\n\n[prompt]\ninstructions = new words\n";
        // A release that changed only the [workspace]: the current prompt stanza, another file.
        let earlier = "[workspace]\nmain = a\n\n[prompt]\ninstructions = new words\n";
        let owner = "[workspace]\nmain = mine\n\n[prompt]\ninstructions = new words\n";
        let stanza = seat_prompt_stanza(current).map(|stanza| leak(digest(stanza)));
        assert_eq!(
            stanza.map(str::to_owned),
            seat_prompt_stanza(earlier).map(digest)
        );
        let ledger = [
            Released {
                tag: "v-old",
                file: leak(digest(earlier)),
                stanza,
            },
            Released {
                tag: "v-current",
                file: leak(digest(current)),
                stanza,
            },
        ];
        assert_eq!(classify_with(earlier, &ledger), Verdict::Whole("v-old"));
        assert_eq!(classify_with(current, &ledger), Verdict::Compatible);
        // An owner's own workspace edit under the current stanza matches no file.
        assert_eq!(classify_with(owner, &ledger), Verdict::Compatible);
    }

    #[test]
    fn hostile_text_is_classified_and_never_panics() {
        let big = format!("[prompt]\ninstructions = {}\n", "x".repeat(2 * 1024 * 1024));
        for text in [
            "",
            "[prompt]",
            "[prompt]\r\ninstructions = \0",
            "[prompt]\ninstructions = \"\"\"\nopen",
            "\u{a0}[prompt]\ninstructions = \u{ffff}\n",
            "[[[[\n]]]]\n=\n= =\n",
            big.as_str(),
        ] {
            let _ = classify(text);
        }
    }

    fn root_with(tag: &str, text: Option<&[u8]>) -> Scratch {
        let scratch = Scratch::new(tag);
        if let Some(text) = text {
            std::fs::write(scratch.0.join("orchestrator.config"), text).expect("seat config");
        }
        scratch
    }

    #[test]
    fn reconcile_replaces_a_released_template_and_keeps_it_at_the_first_free_backup() {
        let old = fixtures()
            .into_iter()
            .find(|(tag, _)| tag == "v2026.9.61")
            .expect("fixture")
            .1;
        let scratch = root_with("whole", Some(old.as_bytes()));
        let path = scratch.0.join("orchestrator.config");
        let taken = scratch.0.join("orchestrator.config.pre-v9.9");
        std::fs::write(&taken, "mine\n").expect("occupied");
        let line = reconcile(&scratch.0, "9.9").expect("a line");
        assert_eq!(std::fs::read_to_string(&path).expect("file"), SEAT_TEMPLATE);
        assert_eq!(std::fs::read_to_string(&taken).expect("kept"), "mine\n");
        let saved = scratch.0.join("orchestrator.config.pre-v9.9-2");
        assert_eq!(std::fs::read_to_string(&saved).expect("backup"), old);
        assert!(
            line.contains(&saved.display().to_string())
                && line.contains("ae stop orchestrator")
                && !line.contains('\n'),
            "{line}"
        );
        // Run again: the file is current, so nothing is said and nothing is kept.
        assert_eq!(reconcile(&scratch.0, "9.9"), None);
        assert!(!scratch.0.join("orchestrator.config.pre-v9.9-3").exists());
    }

    #[test]
    fn reconcile_leaves_every_other_file_alone_and_names_the_repair() {
        let septenber = fixtures()
            .into_iter()
            .find(|(tag, _)| tag == "v2026.9.61")
            .expect("fixture")
            .1;
        let edited = septenber.replace("You are the ae orchestrator:", "Owner's prompt:");
        for (tag, bytes) in [
            ("stale", edited.as_bytes().to_vec()),
            ("nul", b"[prompt]\r\ninstructions = \0\xff".to_vec()),
            ("big", vec![b'x'; 1024 * 1024 + 1]),
        ] {
            let scratch = root_with(tag, Some(&bytes));
            let line = reconcile(&scratch.0, "9.9").unwrap_or_default();
            assert!(
                line.starts_with("left ")
                    && line.contains("ae stop orchestrator")
                    && line.contains(".pre-v9.9"),
                "{tag}: {line}"
            );
            assert_eq!(
                std::fs::read(scratch.0.join("orchestrator.config")).expect("file"),
                bytes,
                "{tag}"
            );
            assert!(!scratch.0.join("orchestrator.config.pre-v9.9").exists());
        }
        let scratch = root_with("absent", None);
        assert_eq!(reconcile(&scratch.0, "9.9"), None);
        let scratch = root_with("link", None);
        std::fs::write(scratch.0.join("dotfiles"), SEAT_TEMPLATE).expect("target");
        std::os::unix::fs::symlink(
            scratch.0.join("dotfiles"),
            scratch.0.join("orchestrator.config"),
        )
        .expect("link");
        let line = reconcile(&scratch.0, "9.9").unwrap_or_default();
        assert!(
            line.starts_with("left ") && line.contains("regular file"),
            "{line}"
        );
    }

    #[test]
    fn a_taken_backup_name_stops_the_replacement() {
        let old = fixtures()
            .into_iter()
            .find(|(tag, _)| tag == "v2026.9.61")
            .expect("fixture")
            .1;
        let scratch = root_with("exclusive", Some(old.as_bytes()));
        let path = scratch.0.join("orchestrator.config");
        let backup = scratch.0.join("orchestrator.config.pre-v9.9");
        std::fs::create_dir(&backup).expect("a directory holds the name");
        assert!(replace(&path, &old, SEAT_TEMPLATE, &backup).is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("file"), old);
        std::fs::remove_dir(&backup).expect("free");
        std::os::unix::fs::symlink(scratch.0.join("nowhere"), &backup).expect("dangling link");
        assert!(replace(&path, &old, SEAT_TEMPLATE, &backup).is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("file"), old);
    }

    #[test]
    fn a_directory_at_the_seat_config_path_gets_a_move_that_can_succeed() {
        let scratch = root_with("dir", None);
        let path = scratch.0.join("orchestrator.config");
        std::fs::create_dir(&path).expect("a directory");
        let line = reconcile(&scratch.0, "9.9").unwrap_or_default();
        assert!(
            line.starts_with("left ")
                && line.contains("it is a directory, not a file")
                && line.contains("sh -c 'mkdir \"$2\" && mv \"$1\" \"$2\"/ && ")
                // The brace group stays inside the sh -c string, which fish never parses.
                && line.contains("ae orchestrator; }' sh ")
                && !line.contains("mv -n")
                && !line.contains("ln ")
                && !line.contains('\n'),
            "{line}"
        );
        assert!(path.is_dir());
        // The printed move works on a directory, and never on a taken name.
        let backup = scratch.0.join("orchestrator.config.pre-v9.9");
        assert!(line.contains(&crate::words::shell_quote(&backup.to_string_lossy())));
        // A named instance's directory names the instance and prints no default-root ae command.
        let named = scratch.0.join("instances/blue");
        std::fs::create_dir_all(named.join("orchestrator.config")).expect("named directory");
        let line = reconcile(&named, "9.9").unwrap_or_default();
        assert!(
            line.contains("instance blue")
                && line.contains("sh -c 'mkdir \"$2\" && mv \"$1\" \"$2\"/' sh ")
                && !line.contains("mv -n")
                && !line.contains("ae stop orchestrator")
                && line.ends_with(".pre-v9.9'"),
            "{line}"
        );
    }

    #[test]
    fn the_context_notice_names_every_seat_in_one_line() {
        let seats = |session: &str, resume: &[&str], fresh: &[&str]| StaleSeats {
            instance: None,
            session: session.to_owned(),
            resume: resume.iter().map(|s| (*s).to_owned()).collect(),
            fresh: fresh.iter().map(|s| (*s).to_owned()).collect(),
        };
        assert_eq!(context_notice(&[]), None);
        assert_eq!(context_notice(&[seats("a", &[], &[])]), None);
        // A pair on two tools is ONE entry with a remedy for each affected slot.
        let line = context_notice(&[
            seats("a", &["main"], &["worker.0"]),
            seats("b", &["main", "worker.0"], &[]),
        ])
        .expect("a line");
        assert!(!line.contains('\n'));
        for part in [
            "4 running lead seats lack a current relay context stamp",
            "absent, malformed or older",
            "a: main needs a stop and resume (ae stop a; ae a --resume), worker.0 needs a fresh \
             conversation (a resume records no stamp for its tool)",
            "b: main,worker.0 need a stop and resume (ae stop b; ae b --resume)",
        ] {
            assert!(line.contains(part), "{part}: {line}");
        }
        assert_eq!(
            line.matches("a: ").count(),
            1,
            "one entry per session: {line}"
        );
        let mut blue = seats("a", &["main"], &["worker.0"]);
        blue.instance = Some("blue".to_owned());
        // Every root's seats make ONE line, each entry qualified by its root, and an
        // instance's entries print no ae command that would start the default root.
        let one = context_notice(&[seats("a", &["main"], &[]), blue]).expect("a line");
        assert!(
            !one.contains('\n')
                && one.contains("3 running lead seats lack")
                && one.contains("a: main needs a stop and resume (ae stop a; ae a --resume)")
                && one.contains(
                    "instance blue/a: main needs a stop and resume (with that instance's own ae \
                     command), worker.0 needs a fresh conversation"
                )
                && !one.contains("ae stop blue"),
            "{one}"
        );
        let single = context_notice(&[seats("a", &["main"], &[])]).expect("a line");
        assert!(single.contains("1 running lead seat lacks"), "{single}");
        let many: Vec<StaleSeats> = (0..30)
            .map(|n| seats(&format!("s{n}"), &["main"], &[]))
            .collect();
        let all = context_notice(&many).expect("a line");
        assert!(all.contains("30 running lead seats") && all.contains("s29: main needs"));
    }

    #[test]
    fn a_session_name_that_cannot_round_trip_through_both_commands_prints_no_command() {
        let seats = |session: &str, instance: Option<&str>| StaleSeats {
            instance: instance.map(str::to_owned),
            session: session.to_owned(),
            resume: vec!["main".to_owned()],
            fresh: vec!["worker.0".to_owned()],
        };
        assert_eq!(command_word("plain-1").as_deref(), Some("plain-1"));
        assert_eq!(command_word("my name").as_deref(), Some("'my name'"));
        assert_eq!(command_word("it's").as_deref(), Some(r"'it'\''s'"));
        assert_eq!(command_word("a;rm x").as_deref(), Some("'a;rm x'"));
        for refused in ["all", "-flag", "-", "two\nlines", "tab\there", ""] {
            assert_eq!(command_word(refused), None, "{refused:?}");
        }
        assert_eq!(command_word("allx").as_deref(), Some("allx"));
        let quoted = context_notice(&[seats("my name", None)]).expect("a line");
        assert!(
            quoted.contains(
                "'my name': main needs a stop and resume (ae stop 'my name'; ae 'my name' --resume)"
            ),
            "{quoted}"
        );
        // `ae stop all` stops the fleet: no command may carry that name, and the pair
        // still gets a remedy for each slot.
        for hostile in ["all", "-flag", "two\nlines"] {
            let line = context_notice(&[seats(hostile, None)]).expect("a line");
            assert!(
                !line.contains('\n')
                    && !line.contains("; ae ")
                    && !line.contains(&format!("ae stop {hostile}"))
                    && line.contains(&format!(
                        "{hostile:?}: main needs a stop and resume (by hand"
                    ))
                    && line.contains("cannot be given to ae stop or ae as an argument")
                    && line.contains("worker.0 needs a fresh conversation"),
                "{hostile:?}: {line}"
            );
        }
        let named = context_notice(&[seats("all", Some("blue"))]).expect("a line");
        assert!(
            named.contains("instance blue/\"all\": main needs a stop and resume (by hand")
                && !named.contains("; ae ")
                && !named.contains("ae stop all"),
            "{named}"
        );
    }

    /// The v2026.11.94 template, byte for byte: the last one before the
    /// chief-of-staff charter, which carries the bound owner words.
    const BEFORE_CHARTER: &str =
        include_str!("../../../tests/fixtures/orchestrator/orchexec-v2026.11.98.config");

    #[test]
    fn an_owner_prompt_with_the_owner_words_but_no_charter_is_left_alone_and_named() {
        let (body, tail) = BEFORE_CHARTER
            .rsplit_once("\n\"\"\"")
            .expect("the template's prompt block");
        let edited = format!("{body}\nAn owner note.\n\"\"\"{tail}");
        assert!(
            seat_prompt_is_current(&edited),
            "the relay keeps accepting it"
        );
        assert!(!edited.contains(crate::config::SEAT_CHARTER_MARKER));
        assert_ne!(classify(&edited), Verdict::Compatible);
        let scratch = root_with("charterless", Some(edited.as_bytes()));
        let path = scratch.0.join("orchestrator.config");
        let line = reconcile(&scratch.0, "9.9").expect("a repair line");
        assert_eq!(std::fs::read_to_string(&path).expect("file"), edited);
        assert!(!scratch.0.join("orchestrator.config.pre-v9.9").exists());
        assert!(
            line.starts_with(&format!("left {} unchanged: ", path.display()))
                && line.contains("charter")
                && line.contains("CHARTER UPDATE")
                && line.contains("ae stop orchestrator; ae orchestrator")
                && !line.contains('\n'),
            "{line}"
        );
        // The current prompt with an owner note carries the charter: nothing to say.
        let (body, tail) = SEAT_TEMPLATE
            .rsplit_once("\n\"\"\"")
            .expect("the template's prompt block");
        let current = root_with(
            "chartered",
            Some(format!("{body}\nAn owner note.\n\"\"\"{tail}").as_bytes()),
        );
        assert_eq!(reconcile(&current.0, "9.9"), None);
    }

    #[test]
    fn a_migration_that_raises_the_charter_names_the_charter_update_turn() {
        for (tag, old) in [
            (
                "v2026.9.61",
                fixtures()
                    .into_iter()
                    .find(|(tag, _)| tag == "v2026.9.61")
                    .expect("fixture")
                    .1,
            ),
            ("v2026.11.94", BEFORE_CHARTER.to_owned()),
        ] {
            let scratch = root_with(&format!("raised-{tag}"), Some(old.as_bytes()));
            let path = scratch.0.join("orchestrator.config");
            let line = reconcile(&scratch.0, "9.9").unwrap_or_else(|| panic!("{tag}: a line"));
            assert_eq!(
                std::fs::read_to_string(&path).expect("file"),
                SEAT_TEMPLATE,
                "{tag}"
            );
            assert!(
                line.contains(tag)
                    && line.contains("CHARTER UPDATE")
                    && line.contains("declares done")
                    && line.contains("ae stop orchestrator, then ae orchestrator")
                    && !line.contains('\n'),
                "{tag}: {line}"
            );
        }
    }

    #[test]
    fn the_restart_advice_names_the_charter_turn_only_when_the_charter_rose() {
        for named in [false, true] {
            let raised = restart(named, true);
            let flat = restart(named, false);
            assert!(raised.contains("CHARTER UPDATE") && raised.contains("declares done"));
            assert!(!flat.contains("CHARTER UPDATE"), "{flat}");
            for line in [raised, flat] {
                assert_eq!(line.contains("ae stop orchestrator"), !named, "{line}");
                assert_eq!(line.contains("that instance"), named, "{line}");
            }
        }
    }
}
