//! The keys: decoded key routing by mode, the browse and compose paths, the fold and mark keys.

use std::hash::{DefaultHasher, Hash as _, Hasher as _};

use super::{
    App, Arm, EARLY_CAP, EARLY_NOTICE, Instant, Item, Key, Kind, Lane, PanelByte, QUIT_GUARD, Rc,
    lane, lines_word, model, peek_key, peek_keys, reader_key, reader_keys, settings_byte,
    settings_key, terminal_text,
};

/// Take decoded keys: `Some(redraw)`, or `None` to quit. From a wheel notch
/// the frame cannot bound yet on, and behind an ask the loop has not
/// delivered, every key is held in order; a held `^C` still quits at once,
/// unless it would lose an ask or a draft. Any key but a browsing letter or
/// `^C` ends the armed press.
pub(super) fn take_keys(app: &mut App, keyed: Vec<(Key, Instant)>) -> Option<bool> {
    if !keyed.is_empty() {
        // Whatever the human did, a view handed over later never yanks it.
        (app.keep, app.touched) = (None, true);
    }
    let mut redraw = false;
    let mut keyed = keyed.into_iter();
    while let Some((mut key, origin)) = keyed.next() {
        if !matches!(key, Key::Mouse(_)) {
            app.read_press = None;
        }
        // A pane view a key just asked for takes the terminal next: every key
        // after it, held ones included, was typed for the seat, never here.
        if app.pane_view.is_some() {
            app.deferred.clear();
            return Some(true);
        }
        // Only a draft takes a line break; everywhere else ^J is Enter.
        if key == Key::Newline && !app.composing() {
            key = Key::Enter;
        }
        // An open overlay or seat view owns its keys even before the first
        // fleet read lands; only plain navigation waits for the fleet.
        if !app.fleeted && !overlay_open(app) && key != Key::Interrupt {
            redraw |= early(app, &key, origin)?;
            continue;
        }
        if !app.deferred.is_empty() || !app.queued.is_empty() || app.unbounded(&key) {
            return hold(app, (key, origin), keyed);
        }
        // A read spent on a modal changes nothing more; an open modal takes
        // every key and press before any mode under it.
        if app.spent(origin) {
            continue;
        }
        if app.modal.is_some() {
            redraw |= match &key {
                Key::Mouse(mouse) => app.modal_mouse(*mouse, origin),
                key => app.modal_key(key, origin),
            };
            continue;
        }
        // Browsing here is not `Mode::Browse`: the seat view and the reader
        // count as browsing, the settings overlay does not.
        let browsing = !app.model.settings_open() && !app.composing() && app.held.is_none();
        if disarms(&key, browsing) {
            redraw |= app.armed.take().is_some();
        }
        // A selection a panel change left behind is gone before this key; Esc
        // with one still held only clears it.
        redraw |= app.stale_selection();
        if key == Key::Escape && app.select.take().is_some_and(|held| held.sel.moved()) {
            redraw = true;
            continue;
        }
        redraw |= route_key(app, key, origin)?;
    }
    Some(redraw)
}

/// Whether the settings overlay, the seat view or the reader is open: each
/// owns its keys.
fn overlay_open(app: &App) -> bool {
    app.model.settings_open() || app.model.peek_open() || app.model.reader_open()
}

/// Whether `key` ends the armed press: any key but `^C`, and but a letter
/// while `browsing`, where a Text run decides for itself byte by byte.
pub(super) fn disarms(key: &Key, browsing: bool) -> bool {
    *key != Key::Interrupt && !(browsing && matches!(key, Key::Text(_)))
}

/// Route one key that passed every gate: a mouse report acts on the frame
/// that drew it; a key or a paste ends a drag first, then acts as it always
/// has, in the mode that owns it. `Some(redraw)`, or `None` to quit.
fn route_key(app: &mut App, key: Key, origin: Instant) -> Option<bool> {
    if let Key::Mouse(mouse) = &key {
        return Some(app.mouse_made(*mouse, origin));
    }
    let ended = app.model.let_go();
    let acted = by_mode(app, key, origin)?;
    Some(acted || ended)
}

/// Act on a key that is no mouse report: a `^C` first clears a draft or only
/// arms a quit when the last clear guards it, anything else goes to the mode
/// that owns the key. Judged here, after the `^C` and the end of a drag, never
/// earlier. `Some(redraw)`, or `None` to quit.
fn by_mode(app: &mut App, key: Key, origin: Instant) -> Option<bool> {
    if key == Key::Interrupt && (app.clear_draft(origin) || app.guard_quit(origin)) {
        return Some(true);
    }
    let overlays = [
        app.model.settings_open(),
        app.model.peek_open(),
        app.model.reader_open(),
    ];
    match mode(overlays, app.composing(), app.held.is_some()) {
        Mode::Settings => settings_key(app, &key, origin),
        Mode::Peek => peek_key(app, &key, origin),
        Mode::Reader => reader_key(app, &key, origin),
        Mode::Composer => app.compose_routed(key, origin).map(|()| true),
        Mode::Held if app.panel_focused() && !app.orch.active => {
            app.in_panel(|app| app.retry(&key)).unwrap_or(Some(false))
        }
        Mode::Held => app.retry(&key),
        Mode::Browse => browse(app, &key, origin),
    }
}

/// Who takes a key, first of these that is true: the settings overlay, the
/// seat view, the file reader, the composer, a refused (held) entry, else the
/// browse keys. Not the `browsing` that decides what ends an armed press in
/// [`take_keys`]: that one counts the seat view and the reader as browsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Settings,
    Peek,
    Reader,
    Composer,
    Held,
    Browse,
}

/// The mode that owns a key given which of them is open (see [`Mode`]): the
/// three `overlays` are settings, seat view and reader, in that order.
pub(super) fn mode(overlays: [bool; 3], composing: bool, held: bool) -> Mode {
    let [settings, peek, reader] = overlays;
    if settings {
        Mode::Settings
    } else if peek {
        Mode::Peek
    } else if reader {
        Mode::Reader
    } else if composing {
        Mode::Composer
    } else if held {
        Mode::Held
    } else {
        Mode::Browse
    }
}

/// Hold `first` and every key after it (`rest`) in order behind what is
/// waiting: `Some(true)`, or `None` when a held `^C` quits at once.
fn hold(
    app: &mut App,
    first: (Key, Instant),
    rest: impl Iterator<Item = (Key, Instant)>,
) -> Option<bool> {
    app.deferred.push(first);
    app.deferred.extend(rest);
    let quit = held_quit(
        &app.deferred,
        app.queued.is_empty(),
        app.has_text(),
        app.cleared_at,
    );
    (!quit).then_some(true)
}

/// Whether the held keys quit at once: one is a `^C` the last clear (at
/// `cleared_at`) does not guard, judged at that key's own read, and no ask is
/// queued (a quit would lose it) and no draft has text (a quit would lose it).
pub(super) fn held_quit(
    held: &[(Key, Instant)],
    nothing_queued: bool,
    has_text: bool,
    cleared_at: Option<Instant>,
) -> bool {
    nothing_queued
        && !has_text
        && (held.iter())
            .any(|(key, origin)| *key == Key::Interrupt && !guarded_at(cleared_at, *origin))
}

/// Whether a ^C read at `origin` falls inside [`QUIT_GUARD`] of a clear at
/// `cleared_at`: the press's own time, never the clock when it is looked at.
pub(super) fn guarded_at(cleared_at: Option<Instant>, origin: Instant) -> bool {
    cleared_at.is_some_and(|at| origin.saturating_duration_since(at) <= QUIT_GUARD)
}

/// Take a key read before the first fleet read, a Text run byte by byte:
/// `q` presses its armed key now, and `,` and `?` open the settings now when
/// nothing waits, the rest of the run going to the overlay; a navigation key
/// waits for the fleet it navigates, and so does an opener behind one, and
/// every key after a waiting opener, which the overlay will own
/// ([`EARLY_CAP`], the oldest go first). Nothing else is taken, told once on
/// the hint row. Each of those ends an armed press. `Some(true)`, or `None`
/// to quit.
fn early(app: &mut App, key: &Key, origin: Instant) -> Option<bool> {
    let pieces = match key {
        Key::Text(bytes) => bytes.iter().map(|byte| Key::Text(vec![*byte])).collect(),
        other => vec![other.clone()],
    };
    for piece in pieces {
        // An earlier byte of this run opened the overlay: it owns the rest.
        if app.model.settings_open() {
            browse(app, &piece, origin)?;
            continue;
        }
        let behind = app.early.iter().any(|(key, _)| opens_settings(key));
        let now = (piece == Key::Text(b"q".to_vec()) && !behind)
            || (opens_settings(&piece) && app.early.is_empty());
        if now {
            browse(app, &piece, origin)?;
            continue;
        }
        app.armed = None;
        let nav = behind
            || opens_settings(&piece)
            || match &piece {
                Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Tab | Key::Escape => true,
                Key::Text(byte) => matches!(byte.as_slice(), [b'1'..=b'9' | b'!' | b'j' | b'k']),
                _ => false,
            };
        let evicted = nav && app.early.len() >= EARLY_CAP;
        if evicted {
            app.early.remove(0);
        }
        if nav {
            app.early.push((piece, origin));
        }
        if (!nav || evicted) && !std::mem::replace(&mut app.early_named, true) {
            app.flash = Some((EARLY_NOTICE.to_owned(), Instant::now()));
        }
    }
    Some(true)
}

/// Whether `key` is one byte that opens the settings while browsing: `,`, or
/// `?` on its keys.
fn opens_settings(key: &Key) -> bool {
    matches!(key, Key::Text(byte) if matches!(byte.as_slice(), [b',' | b'?']))
}

/// Take one key: `Some(redraw)`, or `None` to quit. A batched Text key
/// spells in byte order through the current mode: an `s` opens mid-chunk
/// and the rest routes modal, an `i` starts writing and the rest is draft.
pub(super) fn browse(app: &mut App, key: &Key, origin: Instant) -> Option<bool> {
    if let Key::Text(bytes) = key {
        let mut redraw = false;
        let mut rest = bytes.as_slice();
        while let Some((&byte, tail)) = rest.split_first() {
            if app.model.settings_open() {
                redraw |= settings_byte(app, byte);
                rest = tail;
                continue;
            }
            // The zoom is the frame's, not the overlay's: `X` reaches it through
            // the seat view and the reader as it does through the chat.
            if byte == b'X'
                && !app.orch.active
                && (app.model.peek_open() || app.model.reader_open())
            {
                redraw |= app.flip_zoom();
                rest = tail;
                continue;
            }
            if app.model.peek_open() {
                redraw |= peek_keys(app, model::browse_keys(&Key::Text(vec![byte])));
                rest = tail;
                continue;
            }
            if app.model.reader_open() {
                redraw |= reader_keys(app, model::browse_keys(&Key::Text(vec![byte])));
                rest = tail;
                continue;
            }
            if app.held.is_some() {
                // Writing was refused mid-chunk: the rest is HELD, swallowed.
                return Some(true);
            }
            if app.composing() {
                // Writing started mid-chunk: the rest is draft, whole, with
                // its original instant — never re-spelled per byte.
                app.compose_routed(Key::Text(rest.to_vec()), origin)
                    .map(|()| true)?;
                return Some(true);
            }
            redraw |= browse_byte(app, byte, origin)?;
            rest = tail;
        }
        return Some(redraw);
    }
    if !app.orch.active && app.desk_focused() && *key != Key::Interrupt {
        return Some(match key {
            Key::Escape => app.focus_desk(false),
            Key::PageUp | Key::PageDown => app.scroll_desk(*key == Key::PageUp, true),
            _ => false,
        });
    }
    // With the panel holding the keys, Esc hides it, the arrows step the
    // list (a full panel turns side first), and the rest is the panel's chat.
    let mut turned = false;
    if app.panel_focused() && !app.orch.active {
        match key {
            Key::Escape => {
                app.close_panel();
                return Some(true);
            }
            // The arrows are `j` / `k`: the list's, which turn a full panel
            // side to show the selection.
            Key::Up | Key::Down if matches!(app.panel_byte(b'j'), PanelByte::Select) => {
                turned = show_selection(app, &model::browse_keys(key));
            }
            Key::Up | Key::Down | Key::Interrupt => {}
            _ => {
                return app
                    .in_panel(|app| browse(app, key, origin))
                    .unwrap_or(Some(false));
            }
        }
    }
    if let Key::Pasted(bytes) = key {
        paste(app, bytes, origin);
        return Some(true);
    }
    if *key == Key::Enter && app.toggle_marked() {
        return Some(true);
    }
    let mut redraw = turned;
    for key in model::browse_keys(key) {
        let act = app.model.key(key, &app.fleet, app.can_compose());
        redraw |= apply(app, &act, origin)?;
    }
    Some(redraw)
}

/// List keys over a full panel: when one names a row, the selected one too,
/// the panel turns side and the main column takes the keys, so the session
/// shows. A key that names no row, a digit past the list or a step off either
/// end, leaves the panel full. Whether the panel turned, which owes a redraw.
fn show_selection(app: &mut App, keys: &[model::Key]) -> bool {
    let names = keys
        .iter()
        .any(|key| app.model.target(*key, &app.fleet).is_some());
    if names {
        app.reader_to_main();
    }
    names
}

/// A paste while browsing is never a command: a writable selection starts
/// writing with it as the draft, anything else says why it was not taken.
fn paste(app: &mut App, bytes: &[u8], origin: Instant) {
    if !app.can_compose() {
        let why = Some(app.read_line.as_str()).filter(|why| !why.is_empty());
        let why = format!("read-only · {}", why.unwrap_or("no session is selected"));
        return app.refuse_paste(&why);
    }
    app.write(true);
    if !app.composing() {
        let why = app.held.clone().unwrap_or_default();
        return app.refuse_paste(&why);
    }
    app.admitted = Some(origin);
    let _ = app.compose(Key::Pasted(bytes.to_vec()), origin);
}

/// One Text byte while browsing: `q` and `o` press their armed key, `,`
/// opens the settings and `?` its keys, `m` the menu on the selection's card,
/// `s` starts or stops the selection, `z` opens or closes the stopped group,
/// `Z` folds or shows the selection's spawned seats,
/// anything else decodes as it always has and ends the
/// armed press. A byte of a read the menu spent does nothing. Browsing on
/// entry; the caller routes the modes.
pub(super) fn browse_byte(app: &mut App, byte: u8, origin: Instant) -> Option<bool> {
    if app.spent(origin) {
        return Some(false);
    }
    let disarmed = !matches!(byte, b'q' | b'o') && app.armed.take().is_some();
    let mut turned = false;
    if !app.orch.active {
        if byte == b'B' {
            return Some(app.toggle_desk() || disarmed);
        }
        if byte == b'b' {
            return Some(app.toggle_desk_hidden() || disarmed);
        }
        if app.desk_focused() && !matches!(byte, b'O' | b'F' | b'X' | b'q' | b',' | b'?') {
            return Some(disarmed);
        }
        match byte {
            b'O' => {
                app.toggle_panel();
                return Some(true);
            }
            b'F' => return Some(app.flip_panel() || disarmed),
            b'X' => return Some(app.flip_zoom() || disarmed),
            b'!' => return Some(app.next_need() || disarmed),
            _ => {}
        }
        if app.panel_focused() {
            match app.panel_byte(byte) {
                PanelByte::Inert => return Some(disarmed),
                PanelByte::Select => {
                    turned = show_selection(app, &model::browse_keys(&Key::Text(vec![byte])));
                }
                PanelByte::Open => return Some(app.pane_key(origin)),
                PanelByte::Panel => {
                    let done = app.in_panel(|app| browse_byte(app, byte, origin));
                    if byte == b'r' && app.model.reader_open() {
                        app.reader_to_main();
                    }
                    return done.unwrap_or(Some(disarmed));
                }
                PanelByte::Main => {}
            }
        }
    }
    match byte {
        b'q' => return app.press(Arm::Quit, origin).is_none().then_some(true),
        b',' => {
            app.open_settings();
            return Some(true);
        }
        b's' => return Some(app.toggle(origin)),
        b'z' => return Some(app.toggle_stopped()),
        b'Z' => return Some(app.toggle_spawned()),
        b'?' => {
            app.open_settings();
            app.model.show_settings_tab(model::SettingsTab::Keys);
            return Some(true);
        }
        b'm' => {
            let card = (app.model.selected()).and_then(|name| app.layout.cards.get(name));
            return Some(app.open_menu(card.cloned(), origin));
        }
        _ => {}
    }
    let mut redraw = disarmed || turned;
    for key in model::browse_keys(&Key::Text(vec![byte])) {
        let act = app.model.key(key, &app.fleet, app.can_compose());
        redraw |= apply(app, &act, origin)?;
    }
    Some(redraw)
}

/// Carry out a browse action from either keys or a frame's mouse target.
pub(super) fn apply(app: &mut App, act: &model::Act, origin: Instant) -> Option<bool> {
    match act {
        model::Act::Quit => None,
        model::Act::Select(_) => {
            if app.model.peek_open() {
                app.close_peek();
            }
            if app.model.reader_open() {
                app.close_reader();
            }
            app.show();
            Some(true)
        }
        model::Act::Redraw => Some(true),
        model::Act::StatsScope => {
            app.flip_stats_scope();
            Some(true)
        }
        model::Act::StatsReset => {
            app.ask_reset();
            Some(true)
        }
        model::Act::StatsPage(next) => Some(app.stats_page(*next)),
        model::Act::StatsFilter(next) => Some(app.stats_filter(*next)),
        model::Act::Compose => {
            app.write(true);
            Some(true)
        }
        model::Act::Open => {
            app.open_key(origin);
            Some(true)
        }
        model::Act::SeatNext => Some(app.move_focus(true)),
        model::Act::SeatPrev => Some(app.move_focus(false)),
        model::Act::TurnOlder => Some(app.step_turn(true)),
        model::Act::TurnNewer => Some(app.step_turn(false)),
        model::Act::Copy => {
            app.copy();
            Some(true)
        }
        model::Act::Peek => {
            app.open_peek();
            Some(true)
        }
        model::Act::Read => {
            app.open_reader();
            Some(true)
        }
        // `O` is the panel's own control; the main column never selects the
        // orchestrator.
        model::Act::Orchestrator => {
            if !app.orch.active {
                app.toggle_panel();
            }
            Some(true)
        }
        model::Act::Notice(line) => {
            app.flash = Some((line.clone(), Instant::now()));
            Some(true)
        }
        model::Act::None => Some(false),
    }
}

impl App {
    /// The fingerprints of the lane's turns, in item order: built once for
    /// each lane, so a frame, a key and a click share one build.
    pub(super) fn keys(&mut self) -> Rc<[u64]> {
        let same = |(lane, _): &&(Rc<Lane>, Rc<[u64]>)| Rc::ptr_eq(lane, &self.lane);
        if let Some((_, keys)) = self.keyed.as_ref().filter(same) {
            return Rc::clone(keys);
        }
        let keys: Rc<[u64]> = self.lane.items.iter().map(lane::key).collect();
        self.keyed = Some((Rc::clone(&self.lane), Rc::clone(&keys)));
        keys
    }

    /// What names `item` for its fold across re-reads: the selected session,
    /// its incarnation and the turn's journal position, or its words when it
    /// has none. A changed body keeps it; another incarnation never shares it.
    pub(super) fn fold_key(&self, item: &Item) -> u64 {
        let name = self.model.selected().unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        (name, self.ids.get(name)).hash(&mut hasher);
        match item.record {
            Some(record) => (0_u8, record).hash(&mut hasher),
            None => (1_u8, item.micros, &item.kind, &item.body).hash(&mut hasher),
        }
        hasher.finish()
    }

    /// What names the ask `id` this app sent to session `name`.
    pub(super) fn sent_key(&self, name: &str, id: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        (name, self.ids.get(name), id).hash(&mut hasher);
        hasher.finish()
    }

    /// Each lane turn's fold key, whether it shows whole and whether it is an
    /// ask that went through: built per frame, from the sets alone.
    pub(super) fn fold_state(&self) -> (Vec<u64>, Vec<bool>, Vec<bool>) {
        let name = self.model.selected().unwrap_or_default();
        let items = &self.lane.items;
        let folds: Vec<u64> = items.iter().map(|item| self.fold_key(item)).collect();
        let open = folds
            .iter()
            .map(|key| self.expanded.contains(key))
            .collect();
        let sent = items.iter().map(|item| match &item.kind {
            Kind::Asked { id, .. } => self.sent.contains(&self.sent_key(name, id)),
            _ => false,
        });
        (folds, open, sent.collect())
    }

    /// Fold or unfold the turn with fold key `key`.
    pub(super) fn toggle_fold(&mut self, key: u64) {
        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
    }

    /// Enter on a marked turn that has a fold row: toggle it, the mark kept.
    /// A draft held anywhere is never put aside by it. Whether it toggled.
    fn toggle_marked(&mut self) -> bool {
        let Some(mark) = self.model.turn().filter(|_| !self.protects()) else {
            return false;
        };
        let fold = self.layout.folds.iter().find(|(turn, _)| *turn == mark);
        let Some(&(_, key)) = fold else {
            return false;
        };
        self.toggle_fold(key);
        true
    }

    /// `[` / `]`: mark the next older / newer turn the last frame produced and
    /// bring it into view; with none marked, the newest turn the frame shows.
    /// An edge is a no-op that keeps the mark. Whether anything moved.
    pub(super) fn step_turn(&mut self, older: bool) -> bool {
        let page = self.layout.page_rows;
        let scroll = self.model.scroll_rows(page);
        let turns = &self.layout.turns;
        let marked = (self.model.turn()).and_then(|mark| turns.iter().position(|t| t.0 == mark));
        let next = match marked {
            Some(at) if older => Some(at + 1),
            Some(at) => at.checked_sub(1),
            None => turns
                .iter()
                .position(|&(_, below, rows)| below < scroll + page && below + rows > scroll),
        };
        let Some(&(key, below, rows)) = next.and_then(|at| turns.get(at)) else {
            return false;
        };
        self.model.set_turn(Some(key));
        self.model.reveal(below, below + rows, page);
        true
    }

    /// `y`: the marked turn's body goes to the tmux buffer, and the hint row
    /// says what reached where.
    pub(super) fn copy(&mut self) {
        let line = self.copied();
        self.flash = Some((line, Instant::now()));
    }

    /// The marked turn, or the words that say why there is none. A mark the
    /// lane no longer holds is cleared.
    pub(super) fn marked_item(&mut self) -> Result<Item, String> {
        let Some(mark) = self.model.turn() else {
            return Err("no turn marked - click one or press [ ]".to_owned());
        };
        let keys = self.keys();
        let found = keys.iter().position(|key| *key == mark);
        let Some(item) = found.and_then(|at| self.lane.items.get(at)) else {
            self.model.set_turn(None);
            return Err("that turn changed or left the lane - mark it again".to_owned());
        };
        Ok(item.clone())
    }

    /// The words for a copy. A refusal before any tmux call is a plain hint;
    /// only a copy that was tried, or cannot exist, says `copy failed`.
    fn copied(&mut self) -> String {
        let item = match self.marked_item() {
            Ok(item) => item,
            Err(words) => return words,
        };
        let text = terminal_text(&item.body);
        if text.is_empty() {
            return "nothing to copy: that turn has no text".to_owned();
        }
        let preview = lane::preview_why(&item).map(|why| format!(" (preview only: {why})"));
        let count = format!("{}{}", lines_word(&text), preview.unwrap_or_default());
        self.ship(&text, &count)
    }
}
