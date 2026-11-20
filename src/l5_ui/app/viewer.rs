//! The peek panel and the file reader: the two panes that show one seat capture or one file, with their reads, their
//! paint, their mouse and their keys.

use super::{
    App, Body, Buffer, Drawn, Instant, Key, Mouse, PeekAsk, ReadAsk, RefCell, Refusal, Request,
    Status, browse, draw, lane, lanemd, link, markdown, model, paneview, peek, reader, theme,
};

/// What the open peek panel holds of its pane, pinned to the model's
/// generation: a capture that landed for another is dropped.
#[derive(Debug, Default)]
pub(super) struct PeekPane {
    pub(super) generation: u64,
    /// The session and seat the panel was opened on.
    pub(super) target: Option<(String, String)>,
    /// The body on screen; `None` until the first answer lands.
    pub(super) shown: Option<Body>,
    /// A newer capture held back while the viewport is scrolled back.
    pub(super) held: Option<Body>,
    /// When the newest capture, held or shown, was taken.
    pub(super) at: Option<Instant>,
    /// The age in whole seconds the last frame painted.
    pub(super) painted: Option<u64>,
}

/// What the open reader holds of its file, pinned to the model's generation:
/// an answer that landed for another is dropped.
#[derive(Debug, Default)]
pub(super) struct ReaderPane {
    generation: u64,
    /// The file as the link named it, and the line it asked for.
    pub(super) path: String,
    line: Option<usize>,
    /// The turn's file occurrences, with the active position and resolution base.
    links: usize,
    pub(super) files: Vec<link::FileRef>,
    base: link::Base,
    pub(super) index: usize,
    /// The body on screen; `None` until the answer lands.
    body: Option<reader::Body>,
    /// The body's wrap, and the text width it was made for.
    flow: Option<(usize, reader::Flow)>,
    /// The `:line` jump waits for the first frame that wraps the body.
    jump: bool,
    /// Markdown keeps its parse across widths and both views across toggles.
    document: Option<markdown::Document>,
    rendered: Option<(usize, markdown::Rendered)>,
    raw: bool,
    /// A mode or width change restores this zero-based source anchor.
    anchor: Option<usize>,
}

impl ReaderPane {
    fn shown(&self) -> Option<markdown::Shown<'_>> {
        let (_, rendered) = self.rendered.as_ref()?;
        Some(if self.raw || rendered.refusal().is_some() {
            markdown::Shown::Raw(&self.flow.as_ref()?.1)
        } else {
            markdown::Shown::Rendered(rendered)
        })
    }

    fn source_line(&self, top: usize) -> usize {
        match self.shown() {
            Some(markdown::Shown::Rendered(r)) => r.line_of(top.min(r.rows().saturating_sub(1))),
            _ => self
                .flow
                .as_ref()
                .and_then(|(_, f)| f.display(top.min(f.rows().saturating_sub(1))).map(|r| r.0))
                .unwrap_or(0),
        }
    }

    fn reflow(&mut self, width: usize, top: usize) {
        let source = self.source_line(top);
        let Some((lines, gutter)) = self.body.as_ref().and_then(reader::Body::lines) else {
            return;
        };
        let room = width.saturating_sub(gutter);
        if self.flow.as_ref().is_none_or(|(held, _)| *held != room) {
            self.flow = Some((room, reader::flow(lines, room)));
        }
        if let Some(doc) = &self.document
            && self
                .rendered
                .as_ref()
                .is_none_or(|(held, _)| *held != width)
        {
            if self.rendered.is_some() {
                self.anchor.get_or_insert(source);
            }
            self.rendered = Some((width, doc.layout(width)));
        }
    }
}

impl PeekPane {
    /// The viewport is scrolled back over captured lines: a newer capture waits.
    fn paused(&self, back: usize) -> bool {
        back > 0 && matches!(self.shown, Some(Body::Lines(_)))
    }

    /// Whole seconds since the newest capture, at `now`.
    pub(super) fn age(&self, now: Instant) -> u64 {
        self.at
            .map_or(0, |at| now.saturating_duration_since(at).as_secs())
    }
}

impl App {
    /// Fold one peek capture. Only the open panel's own generation paints. A
    /// refusal or a failed capture replaces the body at once, scrolled back
    /// or not, and returns to the newest row; lines wait while the viewport
    /// is scrolled back over lines.
    pub(super) fn peeked(&mut self, generation: u64, at: Instant, body: Body) {
        if self.model.peek_generation() != Some(generation) {
            return;
        }
        self.peek_sync();
        self.peek.at = Some(at);
        if matches!(body, Body::Gap(_)) {
            self.model.peek_newest();
            self.peek.held = None;
            self.peek.shown = Some(body);
        } else {
            self.peek.held = Some(body);
        }
        self.peek_adopt();
    }

    /// Start the panel's state afresh when the model's generation is not the
    /// one it holds, and drop it once the panel is closed.
    pub(super) fn peek_sync(&mut self) {
        match self.model.peek_generation() {
            None => self.peek = PeekPane::default(),
            Some(generation) if generation != self.peek.generation => {
                self.peek = PeekPane {
                    generation,
                    ..PeekPane::default()
                };
            }
            Some(_) => {}
        }
    }

    /// Show the held capture unless the viewport is scrolled back over lines.
    fn peek_adopt(&mut self) {
        let back = self.model.peek_back().unwrap_or(0);
        if !self.peek.paused(back)
            && !self.holds_peek()
            && let Some(held) = self.peek.held.take()
        {
            self.peek.shown = Some(held);
        }
    }

    /// `v`: open the peek panel on the seat the last frame highlighted, as
    /// that frame drew it. A refusal is said on the hint row, opens nothing and
    /// asks the reader nothing; the proof against tmux is the reader's, every
    /// capture.
    pub(super) fn open_peek(&mut self) {
        let Some(session) = self.model.selected().map(str::to_owned) else {
            let line = "refused: peek: no session is selected".to_owned();
            self.flash = Some((line, Instant::now()));
            return;
        };
        let drawn = self.drawn.clone().filter(|drawn| drawn.session == session);
        let label = (drawn.as_ref())
            .and_then(|drawn| drawn.focused.clone())
            .unwrap_or_else(|| session.clone());
        match self.peek_ask(&session, drawn.as_ref()) {
            Ok(ask) => {
                if self.composing() {
                    self.write(false);
                }
                if self.model.reader_open() {
                    self.close_reader();
                }
                self.peek_generation = ask.generation;
                self.model.open_peek(ask.generation);
                self.peek = PeekPane {
                    generation: ask.generation,
                    target: Some((session, ask.seat.name.clone())),
                    ..PeekPane::default()
                };
                if let Some(tx) = &self.ask {
                    let _ = tx.send(Request::Peek(Some(ask)));
                }
            }
            Err(why) => {
                let line = format!("refused: peek {label}: {}", why.why());
                self.flash = Some((line, Instant::now()));
            }
        }
    }

    /// A click on a seat: inside tmux the peek panel; outside tmux the seat's
    /// own pane, interactive, through [`App::ask_pane_view`].
    pub(super) fn open_seat(&mut self) {
        if self.server.is_some() {
            self.open_peek();
            return;
        }
        let Some(session) = self.model.selected().map(str::to_owned) else {
            let line = "refused: view: no session is selected".to_owned();
            self.flash = Some((line, Instant::now()));
            return;
        };
        let drawn = self.drawn.clone().filter(|drawn| drawn.session == session);
        let label = (drawn.as_ref())
            .and_then(|drawn| drawn.focused.clone())
            .unwrap_or_else(|| session.clone());
        self.ask_pane_view(session, drawn.as_ref(), label);
    }

    /// Ask the loop to hand the terminal to the drawn seat's pane, or say why
    /// not on the hint row.
    fn ask_pane_view(&mut self, session: String, drawn: Option<&Drawn>, label: String) {
        let refuse = |why: String| (format!("refused: view {label}: {why}"), Instant::now());
        if (self.entry(&session)).is_some_and(|entry| entry.status == Status::Stopped) {
            self.flash = Some(refuse(Refusal::Stopped(session).why()));
            return;
        }
        let Some(dir) = self.dirs.get(&session).cloned() else {
            self.flash = Some(refuse(format!(
                "{session} has no record directory ae can read"
            )));
            return;
        };
        let Some(drawn) = drawn else {
            self.flash = Some(refuse("the view changed".to_owned()));
            return;
        };
        let seat = match drawn.seat() {
            Ok(seat) => seat.clone(),
            Err(why) => {
                self.flash = Some(refuse(why.why()));
                return;
            }
        };
        if self.composing() {
            self.write(false);
        }
        if self.model.reader_open() {
            self.close_reader();
        }
        let ask = paneview::Ask {
            session,
            uuid: drawn.uuid.clone(),
            dir,
            seat,
            label,
        };
        self.queue_pane_view(ask, paneview::Verb::View);
    }

    /// Queue `ask` for the loop to hand the terminal over once the frame is
    /// out, its refusals worded by `verb`. The one place that sets both, so a
    /// queued ask never carries another ask's words.
    pub(super) fn queue_pane_view(&mut self, ask: paneview::Ask, verb: paneview::Verb) {
        self.pane_view = Some(ask);
        self.pane_verb = verb;
    }

    /// What `session`'s drawn seat asks the reader to capture, or why the
    /// panel cannot open: the checks that need no tmux read.
    fn peek_ask(&self, session: &str, drawn: Option<&Drawn>) -> Result<Box<PeekAsk>, Refusal> {
        let Some(server) = self.server.clone() else {
            return Err(Refusal::NoTmux);
        };
        if (self.entry(session)).is_some_and(|entry| entry.status == Status::Stopped) {
            return Err(Refusal::Stopped(session.to_owned()));
        }
        let Some(dir) = self.dirs.get(session) else {
            let why = format!("{session} has no record directory ae can read");
            return Err(Refusal::NoSeat(why));
        };
        let Some(drawn) = drawn else {
            return Err(Refusal::NoSeat("the view changed".to_owned()));
        };
        Ok(Box::new(PeekAsk {
            generation: self.peek_generation.wrapping_add(1),
            session: session.to_owned(),
            uuid: drawn.uuid.clone(),
            dir: dir.clone(),
            seat: drawn.seat()?.clone(),
            server,
        }))
    }

    /// Close the panel and tell the reader to stop capturing. It always tells:
    /// the model may already hold the close the key caused.
    pub(super) fn close_peek(&mut self) {
        self.model.close_peek();
        self.peek = PeekPane::default();
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Peek(None));
        }
    }

    /// Fold the file the open reader asked for. Only the open reader's own
    /// generation lands: an answer for a closed or an older open is dropped.
    pub(super) fn read_landed(&mut self, generation: u64, body: reader::Body) {
        if self.model.reader_generation() != Some(generation)
            || self.reader.generation != generation
        {
            return;
        }
        self.reader.document = match &body {
            reader::Body::Text { lines, .. } if markdown::is_markdown(&self.reader.path) => {
                Some(markdown::Document::parse(&lines.join("\n")))
            }
            _ => None,
        };
        self.reader.rendered = None;
        self.reader.anchor = None;
        self.reader.body = Some(body);
        self.reader.flow = None;
        self.reader.jump = true;
    }

    /// The cache and lane width a turn's links are read with, the ones the last
    /// frame drew it with.
    fn md(&self) -> (&RefCell<lanemd::Cache>, usize) {
        (&self.md, usize::from(self.layout.lane_width))
    }

    /// `r`: open the reader on the first file link of the marked turn, read as
    /// the displayed lane parts, in the session's link base. A refusal is said
    /// on the hint row, opens nothing and asks the reader thread nothing.
    pub(super) fn open_reader(&mut self) {
        let item = match self.marked_item() {
            Ok(item) => item,
            Err(words) => {
                self.flash = Some((words, Instant::now()));
                return;
            }
        };
        let base = self.overview.base.clone();
        let found = self
            .keys()
            .iter()
            .position(|key| *key == lane::key(&item))
            .map(|at| lane::read_links(&self.lane.items, at, &base, Some(self.md())).1)
            .unwrap_or_default();
        self.read_file(found, 0, base);
    }

    /// Every activation and step uses the same reader generation and file door.
    pub(super) fn read_file(&mut self, found: Vec<link::FileRef>, index: usize, base: link::Base) {
        let Some(first) = found.get(index) else {
            self.flash = Some(("no file link in that turn".to_owned(), Instant::now()));
            return;
        };
        let ask = Box::new(ReadAsk {
            generation: self.reader_generation.wrapping_add(1),
            path: first.path.clone(),
            line: first.line,
            links: found.len(),
            base,
        });
        self.reader_generation = ask.generation;
        self.model.open_reader(ask.generation);
        // A visual file is not read: the panel holds the note at once, and the
        // reader thread is told to drop any ask for the file left behind.
        let visual = reader::visual(&ask.path);
        self.reader = ReaderPane {
            generation: ask.generation,
            path: ask.path.clone(),
            line: ask.line,
            links: ask.links,
            files: found,
            base: ask.base.clone(),
            index,
            body: visual.map(|kind| reader::Body::Gap(reader::visual_note(kind))),
            raw: ask.line.is_some(),
            ..ReaderPane::default()
        };
        if let Some(tx) = &self.ask {
            let _ = tx.send(Request::Read(visual.is_none().then_some(ask)));
        }
    }

    /// Resolve the press frame against the current turn before using its ordinal.
    pub(super) fn file_list(&mut self, hit: &draw::FileHit) -> Option<Vec<link::FileRef>> {
        if self.model.selected() != Some(hit.session.as_str())
            || self.overview.base != hit.link.base
        {
            return None;
        }
        let at = self.keys().iter().position(|key| *key == hit.link.turn)?;
        if self.fold_key(self.lane.items.get(at)?) != hit.fold {
            return None;
        }
        let (fingerprint, files) =
            lane::read_links(&self.lane.items, at, &hit.link.base, Some(self.md()));
        (fingerprint == hit.link.fingerprint && files.get(hit.link.index) == Some(&hit.link.file))
            .then_some(files)
    }

    pub(super) fn changed_read(&mut self) {
        self.flash = Some(("refused: read: the view changed".to_owned(), Instant::now()));
    }

    /// Close the reader and tell the reader thread to drop an unread ask. It
    /// always tells: the model may already hold the close the key caused.
    pub(super) fn close_reader(&mut self) {
        self.model.close_reader();
        self.reader = ReaderPane::default();
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Read(None));
        }
    }

    /// A mouse event with a seat viewed: the sidebar stays live — a press
    /// on the viewed session returns to its chat, one on another session
    /// or seat goes through the ordinary press (which closes or retargets
    /// the view) — the wheel scrolls what it lands on, and a press on the
    /// viewed body begins a selection.
    pub(super) fn peek_mouse(&mut self, mouse: Mouse, fresh: bool) -> bool {
        self.panel_mouse(mouse, fresh, false)
    }

    /// [`App::peek_mouse`] for the reader, which takes the same presses: its
    /// column has no close label, and the wheel scrolls the file.
    pub(super) fn reader_mouse(&mut self, mouse: Mouse, fresh: bool) -> bool {
        self.panel_mouse(mouse, fresh, true)
    }

    /// Paint the viewed seat's screen into the frame's chat column: its
    /// newest body with its age as of now, then keep the viewport inside
    /// what was drawn. The sidebar stands as drawn.
    pub(super) fn paint_peek(&mut self, look: Option<&theme::Look>, buf: &mut Buffer) {
        self.peek_sync();
        self.peek_adopt();
        let age = self.peek.age(Instant::now());
        let (session, seat) = match &self.peek.target {
            Some((session, seat)) => (session.as_str(), seat.as_str()),
            None => (self.model.selected().unwrap_or_default(), ""),
        };
        let view = peek::View {
            session,
            seat,
            body: self.peek.shown.as_ref(),
            age_secs: age,
            back: self.model.peek_back().unwrap_or(0),
            newer_waiting: self.peek.held.is_some(),
        };
        let bounds = draw::paint_peek_column(&view, look, buf, &mut self.layout);
        self.peek.painted = Some(age);
        self.model.set_peek_page_rows(bounds.visible);
        if self.deferred.is_empty() {
            self.model.clamp_peek(bounds.max_back);
        }
    }

    /// Paint the reader's file into the frame's chat column: wrapped to the
    /// width the panel leaves it, jumped to the line the link named once the
    /// body is first wrapped, then keep the viewport inside what was drawn.
    /// The sidebar stands as drawn.
    pub(super) fn paint_reader(&mut self, look: Option<&theme::Look>, buf: &mut Buffer) {
        let lane = self.layout.lane_area().unwrap_or_default();
        let width = usize::from(lane.width);
        let visible = usize::from(lane.height.saturating_sub(1));
        self.reader
            .reflow(width, self.model.reader_top().unwrap_or(0));
        if self.reader.jump
            && let Some((_, flow)) = &self.reader.flow
        {
            self.reader.jump = false;
            if let Some(line) = self.reader.line {
                let (_, max_top) = reader::window(flow.rows(), visible, 0);
                let top = reader::top_for(flow.row_of(line), visible, max_top);
                self.model.set_reader_top(top);
            }
        }
        if let Some(line) = self.reader.anchor.take() {
            let top = match self.reader.shown() {
                Some(markdown::Shown::Rendered(r)) => r.row_of(line.saturating_add(1)),
                Some(markdown::Shown::Raw(f)) => f.row_of(line.saturating_add(1)),
                None => 0,
            };
            self.model.set_reader_top(top);
        }
        let view = reader::View {
            path: &self.reader.path,
            line: self.reader.line,
            body: self.reader.body.as_ref(),
            flow: self.reader.flow.as_ref().map(|(_, flow)| flow),
            top: self.model.reader_top().unwrap_or(0),
            links: self.reader.links,
        };
        let bounds = if let (Some(reader::Body::Text { lines, .. }), Some(shown)) =
            (&self.reader.body, self.reader.shown())
        {
            let markdown = markdown::View {
                path: view.path,
                line: view.line,
                lines,
                shown,
                refusal: self.reader.rendered.as_ref().and_then(|(_, r)| r.refusal()),
                top: view.top,
                links: view.links,
            };
            draw::paint_markdown_column(self.reader.index, &markdown, look, buf, &mut self.layout)
        } else {
            draw::paint_reader_column(self.reader.index, &view, look, buf, &mut self.layout)
        };
        self.model.set_reader_page_rows(bounds.visible);
        if self.deferred.is_empty() {
            self.model.clamp_reader(bounds.max_back);
        }
    }
}

/// Take one key with the peek panel open: `Some(redraw)`, or `None` to quit.
/// The panel owns every key: it scrolls, it closes, and a paste or any other
/// key dies. Bytes after a mid-chunk close resume the actual mode.
pub(super) fn peek_key(app: &mut App, key: &Key, origin: Instant) -> Option<bool> {
    if *key == Key::Interrupt {
        return None;
    }
    if matches!(key, Key::Text(_)) {
        // `browse` feeds the open panel byte by byte and hands the rest of
        // the chunk to the mode it is really in once the panel closes.
        return browse(app, key, origin);
    }
    Some(peek_keys(app, model::browse_keys(key)))
}

/// Feed `keys` to the seat view; a close they cause tells the reader.
/// `y` copies no turn here: it only says how copying works in the view.
pub(super) fn peek_keys(app: &mut App, keys: Vec<model::Key>) -> bool {
    let mut redraw = false;
    for key in keys {
        if key == model::Key::Copy {
            app.flash = Some(("drag to copy text".to_owned(), Instant::now()));
            redraw = true;
            continue;
        }
        redraw |= app.model.key(key, &app.fleet, false) == model::Act::Redraw;
    }
    if !app.model.peek_open() {
        app.close_peek();
        redraw = true;
    }
    redraw
}

/// Take one key with the reader open: `Some(redraw)`, or `None` to quit.
/// The reader owns every key: it scrolls, it closes, and a paste or any
/// other key dies. Bytes after a mid-chunk close resume the actual mode.
pub(super) fn reader_key(app: &mut App, key: &Key, origin: Instant) -> Option<bool> {
    if *key == Key::Interrupt {
        return None;
    }
    if matches!(key, Key::Text(_)) {
        return browse(app, key, origin);
    }
    Some(reader_keys(app, model::browse_keys(key)))
}

/// Feed `keys` to the reader; a close they cause tells the reader thread.
/// `y` copies no turn here: it only says how copying works in the reader.
pub(super) fn reader_keys(app: &mut App, keys: Vec<model::Key>) -> bool {
    let mut redraw = false;
    for key in keys {
        if matches!(key, model::Key::SeatNext | model::Key::SeatPrev) {
            let next = if key == model::Key::SeatNext {
                app.reader.index.checked_add(1)
            } else {
                app.reader.index.checked_sub(1)
            };
            if let Some(next) = next.filter(|next| *next < app.reader.files.len()) {
                app.read_file(app.reader.files.clone(), next, app.reader.base.clone());
                redraw = true;
            }
            continue;
        }
        if key == model::Key::Mode {
            if app.reader.document.is_some() {
                if let Some(why) = app.reader.rendered.as_ref().and_then(|(_, r)| r.refusal()) {
                    app.flash = Some((why.to_owned(), Instant::now()));
                } else {
                    app.reader.anchor =
                        Some(app.reader.source_line(app.model.reader_top().unwrap_or(0)));
                    app.reader.raw = !app.reader.raw;
                }
                redraw = true;
            }
            continue;
        }
        if key == model::Key::Copy {
            app.flash = Some(("drag to copy text".to_owned(), Instant::now()));
            redraw = true;
            continue;
        }
        redraw |= app.model.key(key, &app.fleet, false) == model::Act::Redraw;
    }
    if !app.model.reader_open() {
        app.close_reader();
        redraw = true;
    }
    redraw
}
