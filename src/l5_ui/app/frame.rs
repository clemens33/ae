//! The frame: the columns, the composer, the cards and the in-place draw.

use super::{
    App, Buffer, CellDiffOption, HISTORY, Instant, Item, Kind, Position, Rc, SessionEntry, Size,
    Status, Terminal, Timestamp, Write, backend, console, draw, menu, theme,
};

/// One frame, drawn exactly as `Terminal::draw` draws it — autoresize, the
/// callback into the current buffer, the buffer diff, the cursor shown or
/// hidden, the buffers swapped, the backend flushed, in that order — except
/// that `paint` also answers the links the frame carries. The backend takes
/// them before the diff, and every cell it must write again — those a link
/// covers now and those one covered last frame — is marked so the diff cannot
/// skip it: a cell's symbol and style say nothing of the link the terminal
/// holds on it. With no runs and none before, no cell is marked and the bytes
/// are `Terminal::draw`'s.
pub(super) fn draw_frame<W: Write>(
    terminal: &mut Terminal<backend::Ansi<W>>,
    paint: impl FnOnce(&mut Buffer) -> (Option<Position>, Vec<backend::Run>),
) -> std::io::Result<()> {
    terminal.autoresize()?;
    let (cursor, runs) = paint(terminal.current_buffer_mut());
    let forced = terminal.backend_mut().links(runs);
    let buffer = terminal.current_buffer_mut();
    for at in forced {
        if let Some(cell) = buffer.cell_mut(at) {
            cell.set_diff_option(CellDiffOption::AlwaysUpdate);
        }
    }
    terminal.apply_buffer_with_cursor(cursor)?;
    Ok(())
}

impl App {
    /// The composer line for the selection.
    pub(super) fn composer(&self) -> draw::Composer<'_> {
        if self.model.peek_open() {
            let seat = self
                .peek
                .target
                .as_ref()
                .map_or("", |(_, seat)| seat.as_str());
            return draw::Composer::Viewing { seat };
        }
        if self.model.reader_open() {
            return draw::Composer::Reading {
                path: &self.reader.path,
            };
        }
        match (self.model.selected(), &self.held) {
            (None, _) => draw::Composer::NoHome,
            (Some(_), Some(why)) => draw::Composer::Held { why },
            (Some(_), None) if !self.read_only.is_empty() && !self.writes_here() => {
                draw::Composer::ReadOnly {
                    why: &self.read_line,
                }
            }
            (Some(name), None) => draw::Composer::Home {
                home: name,
                view: self.writes_here().then_some(&self.draft_view),
                draft: &self.draft,
            },
        }
    }

    /// The columns this frame stands in, and the selection's draft wrapped
    /// to the width its column leaves.
    fn frame_columns(&mut self, area: ratatui_core::layout::Rect) {
        let columns = self.columns();
        // A frame that zooms or unzooms, or hands the zoom from one column to
        // the other, puts other cells under the coordinates the last one drew:
        // no press made before it is published may land on them, a right press
        // on a hidden card least of all. The cut is monotonic and the app's
        // own, so neither the bound's drops nor a column swap can lower or hide
        // it; it moves to the replaced layout's instant and then to the
        // instant the frame is shown ([`App::frame_shown`]).
        if self.model.columns().surface() != columns.surface() {
            self.zoom_pending = true;
        }
        self.model.set_columns(columns);
        if self.model.selected().is_some() {
            let size = Size {
                width: draw::column_width(area, self.model.split(), columns),
                height: draw::composer_pane(area),
            };
            let shown = self.shown_input().map(|i| (i.bare_view(size), i.draft()));
            (self.draft_view, self.draft) = shown.unwrap_or_default();
        }
    }

    /// One frame into `buf`, then the chat scroll bounded by what it held.
    /// Before the fleet is read, the selection's header stands on what its
    /// name alone says.
    pub(super) fn frame(&mut self, buf: &mut Buffer) {
        if let Some(order) = self.pending.take() {
            self.fleet = std::mem::take(&mut self.fleet).arranged(&order);
        }
        self.frame_columns(buf.area);
        // An ask the lane shows needs no word of its own.
        let shown = |item: &Item, id: &str| matches!(&item.kind, Kind::Asked { id: ask, .. } | Kind::NotDelivered { id: ask, .. } if ask == id);
        let lane = Rc::clone(&self.lane);
        self.settling
            .take_if(|settling| lane.items.iter().any(|item| shown(item, &settling.id)));
        self.model.set_note(self.note());
        let selected = self.model.selected();
        let rows = self.fleet.rows.iter().map(|row| {
            let skip = row.home || selected == Some(row.name.as_str());
            let at = self.world.sessions.iter().find(|e| e.name == row.name);
            (row.name.as_str(), at.and_then(|e| e.answer_epoch), skip)
        });
        let unread = self.cue.frame(rows, Timestamp::now().epoch());
        self.model.set_unread(unread);
        self.model.set_stopped(self.stopped());
        self.sync_spawned();
        let keys = self.keys();
        let (look, zone) = self.dressed();
        let mut drawn = self.draw_main_column(&keys, look.as_ref(), zone, buf);
        let cards = drawn
            .sessions()
            .map(|name| (name.to_owned(), self.card(name)));
        drawn.cards = cards.collect();
        let replaced = std::mem::replace(&mut self.layout, drawn);
        let replaced_at = Instant::now();
        self.history.push((replaced_at, replaced));
        if self.zoom_pending {
            self.zoom_cut = self.zoom_cut.max(Some(replaced_at));
        }
        if self.history.len() > HISTORY {
            self.lost = Some(self.history.remove(0).0);
        }
        self.model
            .settle(self.layout.list_start, self.layout.body_top);
        self.frame_panel(buf);
        self.settle_chat_scroll();
        self.settings_due();
        if let Some(open) = self.model.settings() {
            self.layout.strip_for_settings();
            let revealed = std::mem::take(&mut self.orchestrator.reveal);
            self.layout.settings_reveal = revealed;
            draw::paint_settings(
                open.tab,
                open.scroll,
                &self.settings_bodies,
                look.as_ref(),
                buf,
                &mut self.layout,
            );
            self.model
                .set_settings_page_rows(self.layout.settings_page_rows);
            if revealed {
                self.model.set_settings_scroll(self.layout.settings_start);
            }
            if self.deferred.is_empty() {
                self.model
                    .clamp_settings_scroll(self.layout.settings_max_scroll);
            }
        }
        self.paint_in_place(look.as_ref(), buf);
        let drawn = self.model.selected().map(|name| self.freeze(name));
        self.drawn = drawn;
        self.paint_modal(look.as_ref(), buf);
        self.settle_selection(buf);
    }

    /// Held keys replay from the scroll they were read at. A handed-over
    /// scroll waits until the first read of its lane gives it a bound.
    pub(super) fn settle_chat_scroll(&mut self) {
        self.restored &= self.loading;
        if self.deferred.is_empty() && !self.restored {
            self.model.settle_scroll(self.layout.scroll);
        }
    }

    fn draw_main_column(
        &self,
        keys: &[u64],
        look: Option<&theme::Look>,
        zone: Option<&str>,
        buf: &mut Buffer,
    ) -> draw::Layout {
        let (folds, open, sent) = self.fold_state();
        let name = self.model.selected();
        let unread = name
            .filter(|_| !self.fleeted)
            .map(|name| SessionEntry::new(name, Status::Unknown));
        let selected = name.and_then(|name| self.entry(name)).or(unread.as_ref());
        let agents = name.and_then(|name| self.facts.get(name));
        let screen = draw::Screen {
            fleet: &self.fleet,
            model: &self.model,
            overview: &self.overview,
            selected,
            pair: &self.pair,
            agents,
            lane: &self.lane,
            composer: self.composer(),
            look: look.copied(),
            zone,
            now: Timestamp::now(),
        };
        let dest = name.and_then(|name| self.destination(name));
        let target = name.map(|name| (self.target(name), self.target_look(name, look)));
        let wait = draw::Wait {
            fleet: !self.fleeted,
            lane: self.loading,
            keys,
            folds: &folds,
            open: &open,
            sent: &sent,
            dest: dest.as_ref().map(draw::Dest::of),
            target: target.as_ref().map(|(target, look)| (target, *look)),
            confirming: self
                .writer_input()
                .is_some_and(console::input::Input::confirming),
            md: Some(&self.md),
            work: None,
            mic: None,
        };
        draw::draw_with_layout(&screen, wait, buf)
    }

    /// A caption borrows the column's look until the target's own look arrives.
    pub(super) fn target_look<'a>(
        &'a self,
        name: &str,
        fallback: Option<&'a theme::Look>,
    ) -> Option<&'a theme::Look> {
        self.looks
            .get(name)
            .map_or(fallback, |(look, _)| look.as_ref())
    }

    /// Paint whichever panel stands in the chat's place: the peek or the
    /// reader.
    fn paint_in_place(&mut self, look: Option<&theme::Look>, buf: &mut Buffer) {
        if self.model.peek_open() {
            self.paint_peek(look, buf);
        }
        if self.model.reader_open() {
            self.paint_reader(look, buf);
        }
    }

    /// The card `name` shows as the app reads it now: what a frame freezes.
    fn card(&self, name: &str) -> menu::Card {
        menu::Card::new(
            name,
            self.ids.get(name).map_or("", String::as_str),
            self.sole.get(name).map(String::as_str),
            self.entry(name).map(|entry| entry.status),
            self.facts.get(name),
        )
    }

    /// The layout on screen when input read at `origin` was made.
    pub(super) fn layout_at(&self, origin: Instant) -> &draw::Layout {
        (self.history.iter())
            .find(|(replaced, _)| origin <= *replaced)
            .map_or(&self.layout, |(_, layout)| layout)
    }

    /// Spend every key read at or before `origin` on the modal it changed.
    pub(super) fn spend(&mut self, origin: Instant) {
        self.cut = self.cut.max(Some(origin));
    }

    /// A key read at or before the last modal change: spent.
    pub(super) fn spent(&self, origin: Instant) -> bool {
        self.cut.is_some_and(|cut| origin <= cut)
    }

    /// The frame just written showed the open modal whole at `at`: the first
    /// such frame dates which input counts as made on it.
    pub(super) fn frame_shown(&mut self, at: Instant) {
        if std::mem::take(&mut self.zoom_pending) {
            self.zoom_cut = self.zoom_cut.max(Some(at));
        }
        if let Some(input) = self.writer_input_mut() {
            input.warning_painted(at);
        }
        if self.layout.shows_modal()
            && let Some(modal) = self.modal.as_mut()
        {
            modal.stamp(at);
        }
    }
}
