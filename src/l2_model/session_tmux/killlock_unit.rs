//! The kill-server lock's planner: pure functions of a server and the alias
//! listing tmux printed. The behavioural contract (a real installed server, a
//! real publish) is `tests/it/tmuxlock_spec.rs`; these pin the decisions it
//! cannot see one at a time.

use super::*;

fn own() -> ServerId {
    crate::instance::own_server()
}

fn guard(spelling: &str) -> String {
    format!("{spelling}={KILL_SERVER_REFUSAL}")
}

/// The words of every planned write, joined, in spelling order.
fn plan(server: &ServerId, listing: &str) -> Vec<String> {
    kill_lock_writes(server, listing)
        .iter()
        .map(|write| write.as_args().join(" "))
        .collect()
}

const DEFAULTS: &str = "command-alias[0] split-pane=split-window\n\
    command-alias[1] splitp=split-window\n\
    command-alias[2] \"server-info=show-messages -JT\"\n\
    command-alias[3] \"info=show-messages -JT\"\n\
    command-alias[4] \"choose-window=choose-tree -w\"\n\
    command-alias[5] \"choose-session=choose-tree -s\"\n";

#[test]
fn a_server_without_a_guard_gets_all_four_spellings_appended() {
    let writes = plan(&own(), DEFAULTS);
    assert_eq!(writes.len(), 4);
    for (write, spelling) in writes.iter().zip(KILL_SERVER_SPELLINGS) {
        assert_eq!(
            *write,
            format!("-L ae set-option -sa command-alias {}", guard(spelling)),
            "{spelling} is appended, never written to a fixed index"
        );
    }
}

#[test]
fn the_four_spellings_are_exactly_the_unambiguous_kill_server_names() {
    assert_eq!(
        KILL_SERVER_SPELLINGS,
        ["kill-server", "kill-serve", "kill-serv", "kill-ser"]
    );
    // `kill-se` and `kill-s` also match `kill-session`: tmux refuses them, so
    // an alias on either would only shadow a command the lock must not touch.
    for ambiguous in ["kill-se", "kill-s"] {
        assert!(!KILL_SERVER_SPELLINGS.contains(&ambiguous));
    }
}

#[test]
fn a_present_guard_is_rewritten_in_place_at_its_own_index() {
    let listing = format!(
        "{DEFAULTS}command-alias[9] \"kill-server=\\\"old\\\"\"\n\
         command-alias[12] kill-serve=display-message\n\
         command-alias[40] kill-serv=x\n\
         command-alias[41] kill-ser=y\n"
    );
    let writes = plan(&own(), &listing);
    for (write, (spelling, index)) in writes.iter().zip([
        ("kill-server", 9),
        ("kill-serve", 12),
        ("kill-serv", 40),
        ("kill-ser", 41),
    ]) {
        assert_eq!(
            *write,
            format!(
                "-L ae set-option -s command-alias[{index}] {}",
                guard(spelling)
            )
        );
    }
}

#[test]
fn a_users_own_same_name_alias_at_the_lowest_index_is_displaced() {
    // tmux runs the LOWEST index of a name: the guard at 9 would lose to the
    // user's entry at 3, so the entry at 3 is the one that is overwritten.
    let listing = format!(
        "{DEFAULTS}command-alias[3] kill-server=display-message\n\
         command-alias[9] kill-server=whatever\n"
    );
    let writes = plan(&own(), &listing);
    assert_eq!(
        writes[0],
        format!(
            "-L ae set-option -s command-alias[3] {}",
            guard("kill-server")
        )
    );
}

#[test]
fn an_entry_of_another_name_is_never_written_whatever_its_index() {
    // Occupying the index a previous guard used, or any other, is not a collision.
    let listing = format!(
        "{DEFAULTS}command-alias[6] mine=list-sessions\ncommand-alias[7] \"kill-sessionx=a\"\n"
    );
    for write in plan(&own(), &listing) {
        assert!(
            write.contains("-sa command-alias"),
            "no write may name an index when no same-name entry exists: {write}"
        );
        assert!(!write.contains("mine"), "{write}");
    }
}

#[test]
fn a_prefix_of_a_name_is_not_that_name() {
    // `kill-servers` and `kill-server-x` are the user's; only the whole name
    // before `=` is a match.
    let listing = format!(
        "{DEFAULTS}command-alias[8] kill-servers=a\ncommand-alias[9] \"kill-server-x=b\"\n"
    );
    for write in plan(&own(), &listing) {
        assert!(write.contains("-sa command-alias"), "{write}");
    }
}

fn socket_of(server: &ServerId) -> Option<String> {
    match server {
        ServerId::Selected(Selector::Name(name)) if name == "ae" => Some("/run/own".to_owned()),
        ServerId::Selected(Selector::Name(name)) if name == "ae-dev" => Some("/run/dev".to_owned()),
        ServerId::Selected(Selector::Socket(path)) => path.to_str().map(str::to_owned),
        _ => None,
    }
}

fn guards(server: &ServerId) -> bool {
    let mut sockets = crate::fleet_read::SocketPaths::asking(socket_of);
    kill_lock_guards(server, &mut sockets)
}

#[test]
fn the_instances_own_server_is_guarded_by_name_or_by_proven_socket() {
    assert!(guards(&own()), "the name `ae` is the instance's own server");
    // A running server is recorded by its socket path, not by the launch name.
    assert!(guards(&ServerId::Selected(Selector::Socket(
        PathBuf::from("/run/own")
    ))));
}

#[test]
fn no_other_server_is_guarded() {
    for server in [
        ServerId::Ambient,
        ServerId::Selected(Selector::Name("ae-dev".to_owned())),
        ServerId::Selected(Selector::Name("other".to_owned())),
        ServerId::Selected(Selector::Socket(PathBuf::from("/run/dev"))),
        ServerId::Selected(Selector::Socket(PathBuf::from("/tmp/test/sock"))),
    ] {
        assert!(!guards(&server), "{server:?} stays killable");
    }
}

#[test]
fn an_unproven_socket_is_never_taken_for_the_instances_own() {
    // The own name answers nothing (no server): equivalence is unknown, and
    // unknown is not equal even when the spellings match.
    fn silent(_: &ServerId) -> Option<String> {
        None
    }
    let mut sockets = crate::fleet_read::SocketPaths::asking(silent);
    let socket = ServerId::Selected(Selector::Socket(PathBuf::from("/run/own")));
    assert!(!kill_lock_guards(&socket, &mut sockets));
}

#[test]
fn the_ambient_server_is_neither_queried_nor_written() {
    assert!(kill_lock_query(&ServerId::Ambient).is_none());
    assert!(kill_lock_writes(&ServerId::Ambient, DEFAULTS).is_empty());
    let query = kill_lock_query(&own());
    assert_eq!(
        query.as_ref().map(|query| query.as_args().join(" ")),
        Some("-L ae show-options -s command-alias".to_owned())
    );
}

#[test]
fn unreadable_listing_lines_are_ignored() {
    let listing = "garbage\ncommand-alias[x] kill-server=a\ncommand-alias[-1] kill-server=a\n\
        command-alias[5 kill-server=a\ncommand-alias[7] noequals\n\
        command-alias[99999999999] kill-server=a\n";
    for write in plan(&own(), listing) {
        assert!(write.contains("-sa command-alias"), "{write}");
    }
}

#[test]
fn a_listing_line_gives_its_index_and_name_quoted_or_not() {
    assert_eq!(alias_entry("command-alias[3] a=b"), Some((3, "a")));
    assert_eq!(
        alias_entry("command-alias[10] \"kill-server=display-message -p x\""),
        Some((10, "kill-server"))
    );
    assert_eq!(alias_entry("  command-alias[0] x=y"), Some((0, "x")));
    assert_eq!(alias_entry("terminal-features[0] a=b"), None);
    assert_eq!(alias_entry("command-alias[0]"), None);
}

#[test]
fn the_refusal_is_inert_text_that_names_the_supported_stop() {
    assert!(KILL_SERVER_REFUSAL.contains("ae stop"));
    assert!(KILL_SERVER_REFUSAL.starts_with('"') && KILL_SERVER_REFUSAL.ends_with('"'));
    let inner = &KILL_SERVER_REFUSAL[1..KILL_SERVER_REFUSAL.len() - 1];
    // tmux would read any of these inside the alias: a command separator, a
    // format, a variable, an escape or a second quote.
    for forbidden in [';', '#', '$', '\\', '"', '\'', '`', '{', '}', '\n'] {
        assert!(!inner.contains(forbidden), "{forbidden:?} in {inner}");
    }
}

#[test]
fn an_alias_is_one_set_option_word_so_the_message_is_one_tmux_argument() {
    let write = kill_lock_writes(&own(), DEFAULTS);
    for (write, spelling) in write.iter().zip(KILL_SERVER_SPELLINGS) {
        let words = write.as_args();
        assert_eq!(
            words.last().map(String::as_str),
            Some(guard(spelling).as_str())
        );
        assert_eq!(words.len(), 6, "-L ae set-option -sa command-alias <alias>");
    }
}

#[test]
fn a_publish_names_its_link_only_where_one_exists() {
    let home = std::env::temp_dir().join(format!("ae-killlock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let root = home.join(".ae");
    assert!(std::fs::create_dir_all(&root).is_ok());
    assert_eq!(publish_link(&root), None, "no link, no installed home");
    let link = home.join(".local").join("bin").join("ae");
    assert!(std::fs::create_dir_all(link.parent().unwrap_or(&home)).is_ok());
    assert!(std::fs::write(&link, b"").is_ok());
    assert_eq!(publish_link(&root), Some(link));
    let _ = std::fs::remove_dir_all(&home);
}
