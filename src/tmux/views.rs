//! The `_aeview` views: the view sessions an `ae app` pane opens on a session's panes, their markers, bindings, switches and census.

use super::{
    Duration, FIELD_SEPARATOR, SESSION_NAME_FORMAT, ServerId, is_session_id, pane_id_is_valid,
    server_args, session_id_is_valid, session_target,
};

/// The keys that take an `ae app` pane view back to the app. Neither is a key
/// a harness reads as an interrupt; `C-c` always reaches the pane.
pub const VIEW_RETURN_KEYS: [&str; 2] = ["F12", "C-]"];

/// What the pane view's client shows on its status line when it opens.
pub const VIEW_HINT: &str = "ae: F12 or C-] back to ae app (or prefix d)";

/// The session option naming the one client an `ae app` pane view attached.
pub const VIEW_CLIENT_OPTION: &str = "@ae_view_client";

/// The session option carrying the token of the app that owns that view.
pub const VIEW_TOKEN_OPTION: &str = "@ae_view_token";

/// The format [`view_marker_args`] reads: the view client, its owner token
/// and the session's current window.
const VIEW_MARKER_FORMAT: &str = "#{@ae_view_client} | #{@ae_view_token} | #{window_id}";

/// The session option holding the command an inside-tmux view runs to take
/// its client back to the app it came from: ae-built, from captured ids.
pub const VIEW_RETURN_OPTION: &str = "@ae_view_return";

/// `bind-key -n <key>`: the client the session's view marker names runs its
/// captured return when it has one (inside tmux) and detaches otherwise
/// (outside); every other client hands the same key to its pane.
#[must_use]
pub fn view_return_bind_args(server: &ServerId, key: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["bind-key", "-n", key, "if-shell", "-F"].map(ToOwned::to_owned));
    args.push(format!(
        "#{{&&:#{{{VIEW_CLIENT_OPTION}}},#{{==:#{{client_name}},#{{{VIEW_CLIENT_OPTION}}}}}}}"
    ));
    args.push(format!(
        "if-shell -F '#{{{VIEW_RETURN_OPTION}}}' \"run-shell -C '#{{{VIEW_RETURN_OPTION}}}'\" 'detach-client'"
    ));
    args.push(format!("send-keys {key}"));
    args
}

/// The name every transient pane-view session starts with. The leading `_`
/// fails ae's session grammar, so no inventory, fleet or resume path claims one.
pub const VIEW_SESSION_PREFIX: &str = "_aeview-";

/// The transient session a pane view with `token` attaches.
#[must_use]
pub fn view_session_name(token: &str) -> String {
    format!("{VIEW_SESSION_PREFIX}{token}")
}

/// Whether `name` is a transient pane-view session.
#[must_use]
pub fn is_view_session(name: &str) -> bool {
    name.starts_with(VIEW_SESSION_PREFIX)
}

/// The view session's own record of the real session it groups: the `$<n>`
/// id. tmux answers a `-t %pane` read, and an untargeted read from inside a
/// pane, for the MOST RECENTLY ACTIVE session holding the pane's window — the
/// view while a human watches — so a pane-keyed identity read that lands on a
/// view re-reads through this id.
pub const VIEW_OF_OPTION: &str = "@ae_view_of";

/// The view session's record of the ae session it shows: that session's
/// `@ae_session_uuid`. The tmux id dies with the real session; this survives
/// it, so a stop that finds the real session already gone can still name the
/// views that keep its windows alive.
pub const VIEW_UUID_OPTION: &str = "@ae_view_uuid";

/// Append the [`VIEW_UUID_OPTION`] stamp to a [`view_group_args`] chain, in
/// the same tmux invocation; `None` when `uuid` is not a canonical UUID.
#[must_use]
pub fn with_view_uuid(mut args: Vec<String>, view: &str, uuid: &str) -> Option<Vec<String>> {
    if !crate::archive::is_canonical_uuid(uuid) {
        return None;
    }
    let scope = format!("{}:", session_target(view));
    args.extend([";", "set-option", "-t", &scope, VIEW_UUID_OPTION, uuid].map(ToOwned::to_owned));
    Some(args)
}

/// What [`view_uuid_listing_args`] prints per session: id, name, then its
/// [`VIEW_UUID_OPTION`].
const VIEW_UUID_FORMAT: &str = "#{session_id} | #{session_name} | #{@ae_view_uuid}";

/// Every session on the server with the ae session it is a view of, if any.
#[must_use]
pub fn view_uuid_listing_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-sessions", "-F", VIEW_UUID_FORMAT].map(ToOwned::to_owned));
    args
}

/// The `(session id, session name, view uuid)` rows [`view_uuid_listing_args`]
/// printed, or `None` when the read failed or a row's id is not tmux's.
#[must_use]
pub fn interpret_view_uuids(
    succeeded: bool,
    stdout: &str,
) -> Option<Vec<(String, String, String)>> {
    if !succeeded {
        return None;
    }
    stdout
        .lines()
        .map(|line| {
            let (id, rest) = line.split_once(FIELD_SEPARATOR)?;
            let (name, uuid) = rest.rsplit_once(FIELD_SEPARATOR)?;
            is_session_id(id).then(|| (id.to_owned(), name.to_owned(), uuid.to_owned()))
        })
        .collect()
}

/// The ids of the pane views among `rows` recorded as views of the ae session
/// `uuid`. An empty `uuid` names nothing.
#[must_use]
pub fn orphan_view_ids(uuid: &str, rows: &[(String, String, String)]) -> Vec<String> {
    if uuid.is_empty() {
        return Vec::new();
    }
    rows.iter()
        .filter(|(_, name, of)| is_view_session(name) && of == uuid)
        .map(|(id, _, _)| id.clone())
        .collect()
}

/// `show-options -qv` of [`VIEW_OF_OPTION`] on exactly the view session `view`.
#[must_use]
pub fn view_of_args(server: &ServerId, view: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["show-options", "-qv", "-t"].map(ToOwned::to_owned));
    args.push(format!("{}:", session_target(view)));
    args.push(VIEW_OF_OPTION.to_owned());
    args
}

/// The real session id [`view_of_args`] printed: exactly one `$<digits>`
/// line, else `None`.
#[must_use]
pub fn view_of_value(stdout: &str) -> Option<String> {
    let mut lines = stdout.lines();
    let id = lines.next()?;
    (lines.next().is_none() && is_session_id(id)).then(|| id.to_owned())
}

/// `pane` addressed inside the session `session_id` — `$<n>:.%<m>` — or
/// `None` unless both are ids. tmux still answers this for a pane that is NOT
/// a member of that session, so it proves nothing on its own: see
/// [`view_membership_args`].
#[must_use]
pub fn pinned_pane_target(session_id: &str, pane: &str) -> Option<String> {
    let digits = pane.strip_prefix('%')?;
    (is_session_id(session_id)
        && !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit()))
    .then(|| format!("{session_id}:.{pane}"))
}

/// What [`view_membership_args`] prints per pane: the session's id and name,
/// then the pane id last.
const VIEW_MEMBERSHIP_FORMAT: &str = "#{session_id} | #{session_name} | #{pane_id}";

/// Every pane of every window of exactly the session `session_id`.
#[must_use]
pub fn view_membership_args(server: &ServerId, session_id: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-panes", "-s", "-t"].map(ToOwned::to_owned));
    args.push(format!("{}:", session_target(session_id)));
    args.extend(["-F", VIEW_MEMBERSHIP_FORMAT].map(ToOwned::to_owned));
    args
}

/// The name of `session_id` when [`view_membership_args`] listed `pane` among
/// its panes, every row naming that one session; else `None`. A view name is
/// never an answer.
#[must_use]
pub fn view_member_name(
    succeeded: bool,
    stdout: &str,
    session_id: &str,
    pane: &str,
) -> Option<String> {
    if !succeeded {
        return None;
    }
    let mut name: Option<&str> = None;
    let mut member = false;
    for line in stdout.lines() {
        let (id, rest) = line.split_once(FIELD_SEPARATOR)?;
        let (row_name, row_pane) = rest.rsplit_once(FIELD_SEPARATOR)?;
        if id != session_id || name.is_some_and(|seen| seen != row_name) {
            return None;
        }
        name = Some(row_name);
        member |= row_pane == pane;
    }
    let name = name.filter(|name| member && !name.is_empty() && !is_view_session(name))?;
    Some(name.to_owned())
}

/// Every window of exactly the session `session_id` — the windows a pane view
/// of it could still hold once it is killed.
#[must_use]
pub fn view_window_snapshot_args(server: &ServerId, session_id: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-windows", "-t"].map(ToOwned::to_owned));
    args.push(format!("{}:", session_target(session_id)));
    args.extend(["-F", "#{window_id}"].map(ToOwned::to_owned));
    args
}

/// The window ids [`view_window_snapshot_args`] printed: one `@<digits>` per
/// line and at least one, else `None`.
#[must_use]
pub fn interpret_window_snapshot(succeeded: bool, stdout: &str) -> Option<Vec<String>> {
    if !succeeded {
        return None;
    }
    let windows: Vec<String> = stdout.lines().map(ToOwned::to_owned).collect();
    (!windows.is_empty() && windows.iter().all(|window| is_window_id(window))).then_some(windows)
}

/// Whether `id` is spelled as tmux spells a window id: `@<digits>`.
fn is_window_id(id: &str) -> bool {
    id.strip_prefix('@')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// What [`view_holder_listing_args`] prints per window of every session: the
/// session's id and name, the window id, and the session's [`VIEW_OF_OPTION`].
const VIEW_HOLDER_FORMAT: &str = "#{session_id} | #{session_name} | #{window_id} | #{@ae_view_of}";

/// Every window of every session on the server, with who holds it.
#[must_use]
pub fn view_holder_listing_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-windows", "-a", "-F", VIEW_HOLDER_FORMAT].map(ToOwned::to_owned));
    args
}

/// One window a session holds: `(session id, session name, window id,
/// @ae_view_of)`.
pub type HolderRow = (String, String, String, String);

/// The rows [`view_holder_listing_args`] printed, or `None` when the read
/// failed or any row's ids are not tmux's own spelling — a listing ae cannot
/// read whole proves nothing about what survived.
#[must_use]
pub fn interpret_view_holders(succeeded: bool, stdout: &str) -> Option<Vec<HolderRow>> {
    if !succeeded {
        return None;
    }
    stdout
        .lines()
        .map(|line| {
            let (id, rest) = line.split_once(FIELD_SEPARATOR)?;
            let (rest, view_of) = rest.rsplit_once(FIELD_SEPARATOR)?;
            let (name, window) = rest.rsplit_once(FIELD_SEPARATOR)?;
            (is_session_id(id) && is_window_id(window)).then(|| {
                (
                    id.to_owned(),
                    name.to_owned(),
                    window.to_owned(),
                    view_of.to_owned(),
                )
            })
        })
        .collect()
}

/// The ids of the pane views among `rows` that keep the real session
/// `real`'s windows `windows` alive: a view name holding one of them, or one
/// recorded as a view of `real`. Each id once, first seen first; an unknown
/// (empty) `windows` decides nothing.
#[must_use]
pub fn views_holding(windows: &[String], real: &str, rows: &[HolderRow]) -> Vec<String> {
    if windows.is_empty() {
        return Vec::new();
    }
    let mut holders: Vec<String> = Vec::new();
    for (id, name, window, view_of) in rows {
        let holds = windows.contains(window) || view_of == real;
        if is_view_session(name) && holds && !holders.contains(id) {
            holders.push(id.clone());
        }
    }
    holders
}

/// `display-message -p` of the name of the session with id `session_id`.
#[must_use]
pub fn session_name_of_args(server: &ServerId, session_id: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", "-t"].map(ToOwned::to_owned));
    args.push(format!("{}:", session_target(session_id)));
    args.push(SESSION_NAME_FORMAT.to_owned());
    args
}

/// The moment, in Unix nanoseconds, a view session's name says it was minted
/// — the digits after the token's last `-` — or `None` for a name ae did not
/// mint.
#[must_use]
pub fn view_session_age_nanos(name: &str) -> Option<u64> {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let (pid, nanos) = name.strip_prefix(VIEW_SESSION_PREFIX)?.rsplit_once('-')?;
    (digits(pid) && digits(nanos)).then(|| nanos.parse().ok())?
}

/// Whether the view session `name` was minted at least `bound` before `now`
/// (Unix nanoseconds); a name ae did not mint, or one from the future, never is.
#[must_use]
pub fn view_session_expired(name: &str, now: u64, bound: Duration) -> bool {
    let bound = u64::try_from(bound.as_nanos()).unwrap_or(u64::MAX);
    view_session_age_nanos(name)
        .is_some_and(|born| now.checked_sub(born).is_some_and(|age| age >= bound))
}

/// Create the view session `view` in `session_id`'s group, detached, with its
/// own one-line status `line` (the theme's) labelled `session` and `label`:
/// it shares the real session's windows and none of its options, so no other
/// client's bar moves.
#[must_use]
pub fn view_group_args(
    server: &ServerId,
    session_id: &str,
    view: &str,
    session: &str,
    label: &str,
    line: &str,
) -> Vec<String> {
    let scope = format!("{}:", session_target(view));
    let shown = format!("{session} · {label}").replace('#', "##");
    let mut args = server_args(server);
    args.extend(
        [
            "new-session",
            "-d",
            "-s",
            view,
            "-t",
            &session_target(session_id),
        ]
        .map(ToOwned::to_owned),
    );
    for (option, value) in [
        ("status", "on".to_owned()),
        ("status-style", crate::theme::VIEW_STATUS_STYLE.to_owned()),
        ("status-format[0]", line.to_owned()),
        ("mouse", "on".to_owned()),
        (crate::theme::VIEW_LABEL_OPTION, shown.clone()),
        (VIEW_OF_OPTION, session_id.to_owned()),
    ] {
        args.extend([";", "set-option", "-t", &scope, option].map(ToOwned::to_owned));
        args.push(value);
    }
    args
}

/// Attach this terminal to the view session `view`, arm its destruction once
/// attached, mark the client in both sessions as the view `token` owns, then
/// land on `pane` only while it is still a member, else detach.
#[must_use]
pub fn view_attach_args(
    server: &ServerId,
    session_id: &str,
    view: &str,
    pane: &str,
    token: &str,
) -> Vec<String> {
    let exact = session_target(view);
    let real = format!("{session_id}:");
    let scope = format!("{exact}:");
    let mut args = server_args(server);
    args.extend(["attach-session", "-t", &exact].map(ToOwned::to_owned));
    args.extend(
        [";", "set-option", "-t", &scope, "destroy-unattached", "on"].map(ToOwned::to_owned),
    );
    for marked in [&real, &scope] {
        args.extend(
            [
                ";",
                "set-option",
                "-F",
                "-t",
                marked,
                VIEW_CLIENT_OPTION,
                "#{client_name}",
            ]
            .map(ToOwned::to_owned),
        );
        args.extend(
            [";", "set-option", "-t", marked, VIEW_TOKEN_OPTION, token].map(ToOwned::to_owned),
        );
    }
    args.extend([";", "run-shell", "-C", "-t", &scope].map(ToOwned::to_owned));
    args.push(view_landing(view, pane));
    args
}

/// The landing `run-shell -C` expands against the view session: the window
/// holding `pane` there, empty when it is no longer a member, which detaches.
fn view_landing(view: &str, pane: &str) -> String {
    view_landing_or(view, pane, "detach-client")
}

/// [`view_landing`] with `otherwise` run when `pane` is no longer a member.
fn view_landing_or(view: &str, pane: &str, otherwise: &str) -> String {
    let exact = session_target(view);
    let window = format!("#{{W:#{{P:#{{?#{{==:#{{pane_id}},{pane}}},#{{window_id}},}}}}}}");
    format!(
        "if-shell -F '{window}' \
         'select-window -t {exact}:{window} ; select-pane -t {exact}:{window}.{pane}' \
         \"{otherwise}\""
    )
}

/// Move the proven viewer `client` into the view session `view`, arm its
/// destruction once the client is in, mark it in both sessions as the view
/// `token` owns, store the captured return `ret`, then land on `pane` while
/// it is still a member — else run `ret`. Never `detach-client`: inside tmux
/// that would drop the human's terminal.
#[must_use]
pub fn view_switch_args(
    server: &ServerId,
    session_id: &str,
    view: &str,
    pane: &str,
    client: &str,
    token: &str,
    ret: &str,
) -> Vec<String> {
    let exact = session_target(view);
    let real = format!("{session_id}:");
    let scope = format!("{exact}:");
    let mut args = server_args(server);
    args.extend(["switch-client", "-c", client, "-t", &exact].map(ToOwned::to_owned));
    args.extend(
        [";", "set-option", "-t", &scope, "destroy-unattached", "on"].map(ToOwned::to_owned),
    );
    // The real session carries the return too: a client that left the view
    // another way and presses a return key there goes back to the app, never
    // detached.
    for marked in [&real, &scope] {
        args.extend(
            [";", "set-option", "-t", marked, VIEW_CLIENT_OPTION, client].map(ToOwned::to_owned),
        );
        args.extend(
            [";", "set-option", "-t", marked, VIEW_TOKEN_OPTION, token].map(ToOwned::to_owned),
        );
        args.extend(
            [";", "set-option", "-t", marked, VIEW_RETURN_OPTION, ret].map(ToOwned::to_owned),
        );
    }
    args.extend([";", "run-shell", "-C", "-t", &scope].map(ToOwned::to_owned));
    // `##`: the landing's own expansion leaves the reference for the inner
    // `run-shell -C`, so the stored command is never re-quoted.
    args.push(view_landing_or(
        view,
        pane,
        &format!("run-shell -C '##{{{VIEW_RETURN_OPTION}}}'"),
    ));
    args
}

/// The command an inside view runs to go back: switch to the app's session
/// `app_session`, select the app's `app_pane` in `app_window` only while it
/// is still that window's pane there — else nothing is selected — then
/// release the real session's marker while `token` still owns it. `None`
/// when an id is not tmux's own grammar.
///
/// The switch fires the app session's `client-session-changed` hook, which
/// tmux runs only after this command's queue pass; the guarded select waits
/// in `run-shell -C` for the next event-loop turn, so it lands after the hook
/// instead of under it.
#[must_use]
pub fn view_return_command(
    app_session: &str,
    app_window: &str,
    app_pane: &str,
    session_id: &str,
    token: &str,
) -> Option<String> {
    let proven = session_id_is_valid(app_session)
        && session_id_is_valid(session_id)
        && app_window
            .strip_prefix('@')
            .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
        && pane_id_is_valid(app_pane)
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-');
    proven.then(|| {
        format!(
            "switch-client -t {app_session} ; \
             run-shell -C {{ if-shell -F -t {app_session}:{app_window} '##{{P:##{{?##{{==:##{{pane_id}},{app_pane}}},1,}}}}' \
             'select-window -t {app_session}:{app_window} ; select-pane -t {app_session}:{app_window}.{app_pane}' }} ; \
             {}",
            view_clear_when(session_id, token)
        )
    })
}

/// `display-message -c <client>`: the session, window and pane the one
/// `client` is showing — the app an inside view returns to.
#[must_use]
pub fn view_client_place_args(server: &ServerId, client: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", "-c", client].map(ToOwned::to_owned));
    args.push("#{session_id} | #{window_id} | #{pane_id}".to_owned());
    args
}

/// What [`view_client_place_args`] printed: session, window and pane ids, or
/// `None` when tmux did not answer in that shape.
#[must_use]
pub fn interpret_view_client_place(
    succeeded: bool,
    stdout: &str,
) -> Option<(String, String, String)> {
    if !succeeded {
        return None;
    }
    let mut fields = stdout.lines().next()?.splitn(3, FIELD_SEPARATOR);
    let session = fields.next()?.to_owned();
    let window = fields.next()?.to_owned();
    let pane = fields.next()?.trim().to_owned();
    (session_id_is_valid(&session) && window.starts_with('@') && pane_id_is_valid(&pane))
        .then_some((session, window, pane))
}

/// Destroy exactly the view session `view`, never a prefix of another name.
#[must_use]
pub fn view_destroy_args(server: &ServerId, view: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["kill-session", "-t", &session_target(view)].map(ToOwned::to_owned));
    args
}

/// What [`view_census_args`] reads per session: its id, its name and whether
/// a client is attached.
const VIEW_CENSUS_FORMAT: &str = "#{session_id} | #{session_attached} | #{session_name}";

/// One session of [`view_census_args`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewCensusRow {
    /// The `$<n>` session id.
    pub id: String,
    /// Whether any client is attached.
    pub attached: bool,
    /// The session name.
    pub name: String,
}

/// `list-sessions`: every session's id, attachment and name, read by the pane
/// view's real-session watcher and its stale sweep.
#[must_use]
pub fn view_census_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-sessions", "-F", VIEW_CENSUS_FORMAT].map(ToOwned::to_owned));
    args
}

/// What [`view_census_args`] printed, or `None` when tmux did not answer or a
/// line breaks the shape — a census with a hole proves no absence.
#[must_use]
pub fn interpret_view_census(succeeded: bool, stdout: &str) -> Option<Vec<ViewCensusRow>> {
    if !succeeded {
        return None;
    }
    stdout
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.splitn(3, FIELD_SEPARATOR);
            let id = fields.next()?;
            let attached = fields.next()?.parse::<u32>().ok()? > 0;
            let name = fields.next()?;
            session_id_is_valid(id).then(|| ViewCensusRow {
                id: id.to_owned(),
                attached,
                name: name.to_owned(),
            })
        })
        .collect()
}

/// Unset the view marker and the captured return of `session_id` only while
/// `token` still owns them.
fn view_clear_when(session_id: &str, token: &str) -> String {
    format!(
        "if-shell -F -t {session_id}: '#{{==:#{{{VIEW_TOKEN_OPTION}}},{token}}}' \
         'set -u -t {session_id}: {VIEW_CLIENT_OPTION} ; set -u -t {session_id}: {VIEW_TOKEN_OPTION} ; \
         set -u -t {session_id}: {VIEW_RETURN_OPTION}'"
    )
}

/// The command a returning view runs: its own marker removed, never another's.
#[must_use]
pub fn view_release_command(session_id: &str, token: &str) -> String {
    view_clear_when(session_id, token)
}

/// The command clearing a marker whose client is gone, only while it still
/// carries the `seen` token it was judged stale by.
#[must_use]
pub fn view_stale_clear_command(session_id: &str, seen: &str) -> String {
    view_clear_when(session_id, seen)
}

/// The command putting `window`'s active pane back on `prev_pane` — only
/// while the incarnation is `uuid` and the window still shows the view's
/// `pane`. The window is shared with the view session, which is gone by then;
/// the real session's current window was never moved, so it is not restored.
#[must_use]
pub fn view_restore_command(
    session_id: &str,
    uuid: &str,
    window: &str,
    pane: &str,
    prev_pane: &str,
) -> String {
    format!(
        "if-shell -F -t {session_id}:{window} \
         '#{{&&:#{{==:#{{@ae_session_uuid}},{uuid}}},#{{==:#{{pane_id}},{pane}}}}}' \
         'select-pane -t {session_id}:{window}.{prev_pane}'"
    )
}

/// Run one tmux `command` string on `server`, its formats expanded against
/// `session_id` first.
#[must_use]
pub fn view_command_args(server: &ServerId, session_id: &str, command: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["run-shell", "-C", "-t"].map(ToOwned::to_owned));
    args.push(format!("{session_id}:"));
    args.push(command.to_owned());
    args
}

/// `display-message`: `session_id`'s view marker and current window.
#[must_use]
pub fn view_marker_args(server: &ServerId, session_id: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", "-t"].map(ToOwned::to_owned));
    args.push(format!("{session_id}:"));
    args.push(VIEW_MARKER_FORMAT.to_owned());
    args
}

/// What [`view_marker_args`] printed: the view client, the token and the
/// current window, or `None` when tmux did not answer in that shape.
#[must_use]
pub fn interpret_view_marker(succeeded: bool, stdout: &str) -> Option<(String, String, String)> {
    if !succeeded {
        return None;
    }
    let line = stdout.lines().next()?;
    let mut fields = line.rsplitn(3, FIELD_SEPARATOR);
    let window = fields.next()?.trim().to_owned();
    let token = fields.next()?.to_owned();
    let client = fields.next()?.to_owned();
    window.starts_with('@').then_some((client, token, window))
}

/// `display-message -p -t <target> '#{window_id} | #{pane_id}'`.
#[must_use]
pub fn view_place_args(server: &ServerId, target: &str) -> Vec<String> {
    let mut args = server_args(server);
    let format = "#{window_id} | #{pane_id}";
    args.extend(["display-message", "-p", "-t", target, format].map(ToOwned::to_owned));
    args
}

/// The window and pane [`view_place_args`] printed, or `None`.
#[must_use]
pub fn interpret_view_place(succeeded: bool, stdout: &str) -> Option<(String, String)> {
    if !succeeded {
        return None;
    }
    let (window, pane) = stdout.lines().next()?.split_once(FIELD_SEPARATOR)?;
    (window.starts_with('@') && pane.starts_with('%'))
        .then(|| (window.to_owned(), pane.trim().to_owned()))
}
