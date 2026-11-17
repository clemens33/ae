//! The settings overlay: how it opens and closes, its stats, reset and tools reads with their answers, its mouse and
//! its keys.

use super::{
    App, Instant, Key, Mouse, MouseKind, Request, Timestamp, browse_byte, draw, model, settings,
};

impl App {
    /// Fold one settings answer. Only the open overlay's own generation
    /// paints: a late answer for a closed overlay is dropped. Quota rides
    /// every answer; config, about and instructions ride the first per open,
    /// then stay.
    pub(super) fn settled(
        &mut self,
        generation: u64,
        quota: Vec<crate::quota::view::Scope>,
        config: Option<settings::ConfigView>,
        about: Option<settings::AboutFacts>,
        instructions: Option<settings::InstructionsView>,
    ) {
        if !self.model.settings_generation(generation) {
            return;
        }
        // A refresh that failed outright keeps the body already held, marked.
        let held = (self.settings_bodies.quota.as_ref())
            .is_some_and(|held| *held != crate::quota::view::unavailable());
        if quota == crate::quota::view::unavailable() && held && self.quota_at.is_some() {
            self.settings_bodies.quota_kept = Some(String::new());
            self.quota_failed = true;
        } else {
            self.settings_bodies.quota = Some(quota);
            self.settings_bodies.quota_kept = None;
            self.quota_at = Some(Timestamp::now().epoch());
            self.quota_failed = false;
        }
        if let Some(config) = config {
            self.settings_bodies.config = config;
        }
        if let Some(about) = about {
            self.settings_bodies.about = Some(about);
        }
        if let Some(instructions) = instructions {
            self.settings_bodies.instructions = instructions;
        }
    }

    /// Open the overlay on a fresh generation and ask the reader for it.
    pub(super) fn open_settings(&mut self) {
        if self.model.settings_open() {
            return;
        }
        // The reader captures the session it was last told is selected, and a
        // drain tells it only after the keys: tell it before the open.
        self.focus();
        self.settings_generation = self.settings_generation.wrapping_add(1);
        self.model.open_settings(self.settings_generation);
        // An earlier open's quota stays drawn, marked kept, until this open's own answer.
        let quota = self.settings_bodies.quota.take();
        let live_seats = self.settings_bodies.live_seats.take();
        self.settings_bodies = settings::SettingsBodies {
            quota_kept: quota.as_ref().map(|_| String::new()),
            quota,
            live_seats,
            ..settings::SettingsBodies::default()
        };
        self.quota_failed = false;
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Settings {
                open: true,
                generation: self.settings_generation,
            });
        }
    }

    /// Spell a kept quota body's line from the one epoch, so its age moves with the clock.
    fn spell_kept(&mut self) {
        if let (Some(kept), Some(at)) = (&mut self.settings_bodies.quota_kept, self.quota_at) {
            let age = crate::quota::age_label(Timestamp::now().epoch().saturating_sub(at));
            *kept = if self.quota_failed {
                format!("refresh failed · showing values from {age} ago")
            } else {
                format!("last updated {age} — refreshing")
            };
        }
    }

    /// What the overlay owes before its frame.
    pub(super) fn settings_due(&mut self) {
        self.spell_kept();
        self.ask_stats();
        self.ask_tools();
    }

    /// Tell the reader, before a frame, when the Statistics tab became shown
    /// or stopped being shown under the open overlay: shown, it watches the
    /// store; hidden, it reads nothing for it. A repaint asks nothing.
    pub(super) fn ask_stats(&mut self) {
        let shown = (self.model.settings())
            .filter(|open| open.tab == model::SettingsTab::Statistics)
            .map(|open| open.generation);
        if shown == self.stats_shown {
            return;
        }
        let (generation, now) = match (shown, self.stats_shown) {
            (Some(generation), _) => (generation, true),
            (None, Some(generation)) => (generation, false),
            (None, None) => return,
        };
        self.stats_shown = shown;
        self.ask_stats_for(generation, now);
    }

    /// Tell the reader the Statistics tab of `generation` is `shown` or not.
    fn ask_stats_for(&self, generation: u64, shown: bool) {
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Stats { generation, shown });
        }
    }

    /// What the store shows, for the open it was read for and no other. A
    /// reset's line stays until the next reset or the overlay closes.
    pub(super) fn stats_landed(
        &mut self,
        generation: u64,
        glance: &crate::stats_store::disk::Glance,
    ) {
        if !self.model.settings_generation(generation) {
            return;
        }
        let (view, mut store) = settings::StoreLine::of(glance);
        let stats = &mut self.settings_bodies.stats;
        store.reset = stats.store.reset.take();
        stats.view = view;
        stats.store = store;
    }

    /// `r` on the Statistics tab: a reset owed to the upkeep, for this open.
    pub(super) fn ask_reset(&mut self) {
        // The store reset belongs to the Spawns page alone.
        if self.settings_bodies.stats.page != settings::StatsPage::Spawns {
            return;
        }
        let shown = (self.model.settings())
            .filter(|open| open.tab == model::SettingsTab::Statistics)
            .map(|open| open.generation);
        if let Some(generation) = shown {
            self.stats_reset = Some(generation);
            self.settings_bodies.stats.store.reset = Some(settings::ResetLine::Requested);
        }
    }

    /// The reset owed to the upkeep, handed over once.
    pub(super) const fn take_reset(&mut self) -> Option<u64> {
        self.stats_reset.take()
    }

    /// No upkeep thread took the reset of `generation`: it waits for one.
    pub(super) fn reset_waits(&mut self, generation: u64) {
        if self.model.settings_generation(generation) {
            self.settings_bodies.stats.store.reset = Some(settings::ResetLine::Waiting);
        }
    }

    /// What became of the reset of `generation`, drawn only under that open.
    /// A reset that took drops the old numbers, and the watch reads the
    /// store again at once.
    pub(super) fn reset_landed(
        &mut self,
        generation: u64,
        outcome: Result<crate::stats_store::disk::Reset, crate::stats_store::disk::Refused>,
    ) {
        use crate::stats_store::disk::Reset;
        use settings::{ResetLine, ScopeLine};
        if !self.model.settings_generation(generation) {
            return;
        }
        let stats = &mut self.settings_bodies.stats;
        let line = match outcome {
            Ok(Reset::Rebuilt { .. } | Reset::Empty) => {
                stats.view = None;
                stats.store.all = ScopeLine::Building;
                stats.store.this = ScopeLine::Building;
                if self.stats_shown == Some(generation) {
                    self.ask_stats_for(generation, true);
                }
                ResetLine::Rebuilt
            }
            Ok(Reset::Busy) => ResetLine::Busy,
            Err(why) => ResetLine::Refused(why.describe()),
        };
        self.settings_bodies.stats.store.reset = Some(line);
    }

    /// The orchestrator view the reader read for `generation`, drawn only
    /// under that open.
    pub(super) fn orchestrator_resolved(
        &mut self,
        generation: u64,
        view: settings::OrchestratorView,
    ) {
        if self.model.settings_generation(generation) {
            self.settings_bodies.orchestrator = view;
        }
    }

    /// `o` on the Config tab: open the orchestrator profile picker on the
    /// configured profile. A row that cannot be edited opens nothing: its
    /// reason is already drawn.
    pub(super) fn open_orchestrator_picker(&mut self) -> bool {
        let settings::OrchestratorView::Ready(ready) = &self.settings_bodies.orchestrator else {
            return false;
        };
        // One write at a time, whatever overlay asked for the first: while it
        // runs the picker stays shut and the row says what is being written.
        if let Some((_, profile)) = &self.orchestrator.inflight {
            self.settings_bodies.orchestrator_note =
                Some(settings::OrchestratorNote::Saving(profile.clone()));
            return true;
        }
        let at = ready
            .choices
            .iter()
            .position(|choice| *choice == ready.configured)
            .unwrap_or(0);
        self.settings_bodies.orchestrator_picker = Some(at);
        self.orchestrator.reveal = true;
        true
    }

    /// One key with the picker open on the Config tab: whether it was the
    /// picker's, and whether it changed the screen. Navigation never saves;
    /// Enter queues the highlighted choice once and closes the picker.
    fn orchestrator_picker_key(&mut self, key: model::Key) -> Option<bool> {
        let at = self.settings_bodies.orchestrator_picker?;
        let last = match &self.settings_bodies.orchestrator {
            settings::OrchestratorView::Ready(ready) => ready.choices.len().saturating_sub(1),
            _ => 0,
        };
        match key {
            model::Key::Up => {
                self.settings_bodies.orchestrator_picker = Some(at.saturating_sub(1));
                self.orchestrator.reveal = true;
            }
            model::Key::Down => {
                self.settings_bodies.orchestrator_picker = Some((at + 1).min(last));
                self.orchestrator.reveal = true;
            }
            model::Key::Compose => self.queue_orchestrator_pick(at),
            model::Key::Esc | model::Key::Quit => self.settings_bodies.orchestrator_picker = None,
            model::Key::Open => return Some(false),
            _ => return None,
        }
        Some(true)
    }

    /// Queue the `at`th offered profile for the writer thread and close the
    /// picker. The note says it is being saved until the answer lands.
    fn queue_orchestrator_pick(&mut self, at: usize) {
        self.settings_bodies.orchestrator_picker = None;
        let settings::OrchestratorView::Ready(ready) = &self.settings_bodies.orchestrator else {
            return;
        };
        let (Some(profile), Some(open)) = (ready.choices.get(at), self.model.settings()) else {
            return;
        };
        self.orchestrator.pick = Some(settings::OrchestratorPick {
            generation: open.generation,
            profile: profile.clone(),
            seen: ready.seen.clone(),
            global: ready.global.clone(),
        });
        self.orchestrator.inflight = Some((open.generation, profile.clone()));
        self.settings_bodies.orchestrator_note =
            Some(settings::OrchestratorNote::Saving(profile.clone()));
    }

    /// The orchestrator profile pick owed to the writer thread, handed once.
    pub(super) const fn take_orchestrator_pick(&mut self) -> Option<settings::OrchestratorPick> {
        self.orchestrator.pick.take()
    }

    /// What became of the pick of `generation`, drawn only under that open.
    /// A write asks the reader for the file again, so the row and the
    /// offered profiles show what is on disk now.
    pub(super) fn orchestrator_landed(
        &mut self,
        generation: u64,
        outcome: Result<crate::config::Applied, crate::config::ApplyRefusal>,
    ) {
        use settings::OrchestratorNote;
        // The write ended, whichever overlay is open now.
        let inflight = self.orchestrator.inflight.take();
        if !self.model.settings_generation(generation) {
            return;
        }
        let asked = inflight
            .filter(|(started, _)| *started == generation)
            .map(|(_, profile)| profile)
            .unwrap_or_default();
        self.settings_bodies.orchestrator_note = None;
        let (note, reread) = match outcome {
            Ok(applied) if applied.changed => {
                let kept = applied
                    .backup
                    .map(|path| format!("; the previous file is kept as {}", path.display()))
                    .unwrap_or_default();
                (
                    OrchestratorNote::Saved(format!("orchestrator profile saved: {asked}{kept}")),
                    true,
                )
            }
            Ok(_) => (
                OrchestratorNote::Unchanged(format!(
                    "orchestrator profile unchanged: {asked} is already configured, nothing was written"
                )),
                false,
            ),
            Err(refusal) => (OrchestratorNote::Refused(refusal.to_string()), false),
        };
        self.settings_bodies.orchestrator_note = Some(note);
        if reread && let Some(ask) = &self.ask {
            // The reader forgets its watches on a repeat open, so this
            // overlay asks for the Statistics and tool pages again.
            let _ = ask.send(Request::Settings {
                open: true,
                generation,
            });
            self.stats_shown = None;
            self.tools_asked = None;
        }
    }

    /// Show the other scope of the read already held.
    pub(super) fn flip_stats_scope(&mut self) {
        let stats = &mut self.settings_bodies.stats;
        stats.scope = stats.scope.flip();
        // A tool page waits for the other scope's own answer.
        if stats.page != settings::StatsPage::Spawns {
            stats.tools = None;
        }
    }

    /// Left or Right on the Statistics tab: the inner tab one step along the
    /// row, wrapping. See [`Self::select_stats_tab`].
    fn stats_tab(&mut self, next: bool) -> bool {
        let shown = settings::StatsTab::of(self.settings_bodies.stats.page);
        self.select_stats_tab(shown.step(next))
    }

    /// Show the inner tab `tab` on its first page unless it is the one shown
    /// (a click on the shown title changes nothing). A new tab draws
    /// `loading` with no filter until its own answer lands.
    fn select_stats_tab(&mut self, tab: settings::StatsTab) -> bool {
        let stats = &mut self.settings_bodies.stats;
        if settings::StatsTab::of(stats.page) == tab {
            return false;
        }
        stats.page = tab.first_page();
        stats.filter = crate::tool_stats::Filter::All;
        stats.tools = None;
        true
    }

    /// `n` or `p` on the Statistics tab: the next or previous page. A new
    /// page draws `loading` with no filter until its own answer lands.
    pub(super) fn stats_page(&mut self, next: bool) -> bool {
        use settings::StatsPage;
        let stats = &mut self.settings_bodies.stats;
        let order = [
            StatsPage::Spawns,
            StatsPage::Skills,
            StatsPage::Mcp,
            StatsPage::Tools,
        ];
        let at = order
            .iter()
            .position(|page| *page == stats.page)
            .unwrap_or(0);
        let step = if next { 1 } else { order.len() - 1 };
        stats.page = order[(at + step) % order.len()];
        stats.filter = crate::tool_stats::Filter::All;
        stats.tools = None;
        true
    }

    /// `]` or `[` on a tool page: the next or previous filter the shown
    /// answer offers (`all` first). The numbers stay until the filtered
    /// answer lands.
    pub(super) fn stats_filter(&mut self, next: bool) -> bool {
        let stats = &mut self.settings_bodies.stats;
        let Some(view) = &stats.tools else {
            return false;
        };
        let filters = &view.filters;
        if filters.is_empty() {
            return false;
        }
        let at = filters
            .iter()
            .position(|filter| *filter == stats.filter)
            .unwrap_or(0);
        let step = if next { 1 } else { filters.len() - 1 };
        stats.filter = filters[(at + step) % filters.len()].clone();
        true
    }

    /// The tool page the open Statistics tab shows, as the reader is asked
    /// for it: `None` on the Spawns page or with the tab not shown. `m` asks
    /// for this session: the selection's recorded id, else home's.
    fn tools_ask(&self) -> Option<(u64, crate::tool_stats::Ask)> {
        use crate::tool_stats::{Ask, Page, Scope};
        let open = self
            .model
            .settings()
            .filter(|open| open.tab == model::SettingsTab::Statistics)?;
        let stats = &self.settings_bodies.stats;
        let page = match stats.page {
            settings::StatsPage::Spawns => return None,
            settings::StatsPage::Skills => Page::Skills,
            settings::StatsPage::Mcp => Page::Mcp,
            settings::StatsPage::Tools => Page::Tools,
        };
        let scope = match stats.scope {
            settings::StatsScope::All => Scope::All,
            settings::StatsScope::ThisSession => {
                let name = self.model.selected().or(self.home.as_deref());
                Scope::This(name.and_then(|name| self.ids.get(name)).cloned())
            }
        };
        Some((
            open.generation,
            Ask {
                page,
                scope,
                filter: stats.filter.clone(),
            },
        ))
    }

    /// Tell the reader, before a frame, which tool page is shown, or that
    /// none is any more. A repaint asks nothing.
    fn ask_tools(&mut self) {
        let shown = self.tools_ask();
        if shown == self.tools_asked {
            return;
        }
        let request = match (&shown, &self.tools_asked) {
            (Some((generation, ask)), _) => Request::Tools {
                generation: *generation,
                shown: true,
                ask: ask.clone(),
            },
            (None, Some((generation, ask))) => Request::Tools {
                generation: *generation,
                shown: false,
                ask: ask.clone(),
            },
            (None, None) => return,
        };
        self.tools_asked = shown;
        if let Some(ask) = &self.ask {
            let _ = ask.send(request);
        }
    }

    /// A tool-statistics answer: kept only for the open overlay and the page
    /// it shows now (a stale or another page's answer is dropped).
    pub(super) fn tools_landed(&mut self, generation: u64, view: Box<crate::tool_stats::View>) {
        if !self.model.settings_generation(generation) {
            return;
        }
        if self.tools_ask().is_some_and(|(_, ask)| ask == view.ask) {
            self.settings_bodies.stats.tools = Some(view);
        }
    }

    /// Close the overlay and tell the reader to stop reading for it. It
    /// always tells: the model may already hold the close the key caused.
    pub(super) fn close_settings(&mut self) {
        self.model.close_settings();
        // The close below drops the reader's watches with the overlay.
        self.stats_shown = None;
        self.tools_asked = None;
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Settings {
                open: false,
                generation: self.settings_generation,
            });
        }
    }

    /// A mouse event with the overlay open: the wheel scrolls its body
    /// wherever it lands, a press on the gear or the close label closes, one
    /// on a tab title shows that tab, one on an inner title shows that inner
    /// tab, and any other press dies.
    pub(super) fn settings_mouse(&mut self, mouse: Mouse) -> bool {
        match mouse.kind {
            MouseKind::WheelUp => {
                self.model.settings_wheel(-model::WHEEL_ROWS.cast_signed());
                true
            }
            MouseKind::WheelDown => {
                self.model.settings_wheel(model::WHEEL_ROWS.cast_signed());
                true
            }
            MouseKind::Click => match self.layout.hit(mouse) {
                Some(draw::Hit::Settings | draw::Hit::SettingsClose) => {
                    self.close_settings();
                    true
                }
                Some(draw::Hit::SettingsTab(tab)) => {
                    self.model.show_settings_tab(tab) == model::Act::Redraw
                }
                Some(draw::Hit::StatsTab(tab)) => self.select_stats_tab(tab),
                _ => false,
            },
            MouseKind::Drag | MouseKind::Release | MouseKind::Menu => false,
        }
    }
}

/// Take one key with the overlay open: `Some(redraw)`, or `None` to quit.
/// The overlay routes before composing; a close it causes tells the reader.
/// Bytes after a mid-chunk close resume the actual mode: the suspended
/// composer when writing, browse otherwise.
pub(super) fn settings_key(app: &mut App, key: &Key, origin: Instant) -> Option<bool> {
    if *key == Key::Interrupt {
        return None;
    }
    if let Key::Text(bytes) = key {
        let mut redraw = false;
        let mut rest = bytes.as_slice();
        while let Some((&byte, tail)) = rest.split_first() {
            if !app.model.settings_open() {
                if app.composing() {
                    // The rest of the run goes back to the draft whole, with
                    // its original instant — never re-spelled per byte.
                    app.compose(Key::Text(rest.to_vec()), origin)
                        .map(|()| true)?;
                    return Some(true);
                }
                redraw |= browse_byte(app, byte, origin)?;
                rest = tail;
                continue;
            }
            redraw |= settings_byte(app, byte);
            rest = tail;
        }
        return Some(redraw);
    }
    let mut redraw = false;
    if let Key::Left | Key::Right = key {
        // An arrow steps the Statistics tab's inner row and means nothing on
        // any other tab.
        return Some(app.model.arrows_step_stats() && app.stats_tab(*key == Key::Right));
    }
    for key in model::browse_keys(key) {
        // The overlay branch returns no Quit, Select or Compose; a close
        // shows as the state it leaves behind, told below.
        redraw |= settings_act(app, key);
    }
    if !app.model.settings_open() {
        app.close_settings();
        redraw = true;
    }
    Some(redraw)
}

/// One Text byte with the overlay open: `,` closes with its request, the
/// rest decodes modal. The overlay is open on entry; the caller resumes the
/// actual mode after a close.
pub(super) fn settings_byte(app: &mut App, byte: u8) -> bool {
    if byte == b',' {
        app.close_settings();
        return true;
    }
    if byte == b'?' {
        app.model.show_settings_tab(model::SettingsTab::Keys);
        return true;
    }
    let mut redraw = false;
    for key in model::browse_keys(&Key::Text(vec![byte])) {
        redraw |= settings_act(app, key);
    }
    if !app.model.settings_open() {
        app.close_settings();
        redraw = true;
    }
    redraw
}

/// One decoded key with the overlay open: whether it changed the screen.
pub(super) fn settings_act(app: &mut App, key: model::Key) -> bool {
    let on_config =
        (app.model.settings()).is_some_and(|open| open.tab == model::SettingsTab::Config);
    if on_config {
        if let Some(redraw) = app.orchestrator_picker_key(key) {
            return redraw;
        }
        if key == model::Key::Open {
            return app.open_orchestrator_picker();
        }
    }
    let redraw = settings_act_model(app, key);
    // A picker belongs to the Config tab: leaving it closes the picker.
    let on_config =
        (app.model.settings()).is_some_and(|open| open.tab == model::SettingsTab::Config);
    if !on_config && app.settings_bodies.orchestrator_picker.take().is_some() {
        return true;
    }
    redraw
}

/// One decoded key the model decides, with the overlay open.
fn settings_act_model(app: &mut App, key: model::Key) -> bool {
    match app.model.key(key, &app.fleet, app.can_compose()) {
        model::Act::Redraw => true,
        model::Act::StatsScope => {
            app.flip_stats_scope();
            true
        }
        model::Act::StatsReset => {
            app.ask_reset();
            true
        }
        model::Act::StatsPage(next) => app.stats_page(next),
        model::Act::StatsFilter(next) => app.stats_filter(next),
        _ => false,
    }
}

/// What a review of the orchestrator profile picker found: one write at a time,
/// a selection that stays on screen, refusals that reach the end of a narrow
/// panel, the launch reader's own validation, and a recorded profile that is
/// never claimed absent when it is only unknown.
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "tests own scratch config files and UI channels"
)]
mod review_tests {
    use std::fmt::Write as _;
    use std::fs;
    use std::sync::mpsc;
    use std::time::Instant;

    use ratatui_core::buffer::Buffer;
    use ratatui_core::layout::Rect;

    use crate::app::loader::{Answer, Wake};
    use crate::app::settings::{OrchestratorView, resolve_orchestrator};
    use crate::app::tests::Root;
    use crate::app::{App, drain, take_keys};
    use crate::config::{self, Applied, ApplyRefusal};
    use crate::console::input::{Key, Keys};

    fn key(app: &mut App, key: Key) {
        assert!(take_keys(app, vec![(key, Instant::now())]).is_some());
    }

    fn text(app: &mut App, line: &str) {
        key(app, Key::Text(line.as_bytes().to_vec()));
    }

    fn frame(app: &mut App, width: u16, height: u16) -> String {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        app.frame(&mut buf);
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn land(app: &mut App, generation: u64, outcome: Result<Applied, ApplyRefusal>) {
        let (tx, rx) = mpsc::channel();
        tx.send(Wake::OrchestratorPick {
            generation,
            outcome,
        })
        .expect("completion");
        assert!(drain(app, &mut Keys::app(), &rx, &mut None).is_some());
    }

    /// An app on the Config tab over `count` profiles `p00..`, p00 configured.
    fn fixture(tag: &str, count: usize) -> (Root, App) {
        let root = Root::new(tag);
        let mut config = String::from("[profiles]\n");
        for at in 0..count {
            let _ = writeln!(config, "p{at:02} = /bin/echo {at}");
        }
        config.push_str("[roster]\norchestrator = p00\n");
        fs::write(root.0.join("config"), config).expect("config");
        let mut app = App::new(None, None, None, None);
        app.fleeted = true;
        text(&mut app, ",2");
        app.settings_bodies.config =
            crate::app::settings::resolve_config(None, &root.0.join("config"));
        app.settings_bodies.orchestrator =
            resolve_orchestrator(None, &root.0.join("config"), Some(&root.0));
        (root, app)
    }

    #[test]
    fn a_second_pick_waits_for_the_first_write_and_the_saved_line_names_the_write_that_landed() {
        let (root, mut app) = fixture("orchprof-rev-race", 3);
        text(&mut app, "oj");
        key(&mut app, Key::Enter);
        let first = app.take_orchestrator_pick().expect("first queued");
        assert_eq!(first.profile, "p01");
        text(&mut app, "ojj");
        key(&mut app, Key::Enter);
        assert!(
            app.take_orchestrator_pick().is_none(),
            "one write at a time: a pick made while the first is writing is not queued"
        );
        let actual = config::apply_roster_row(
            &root.0.join("config"),
            &first.seen,
            "orchestrator",
            &first.profile,
            19,
        )
        .expect("first real apply");
        land(&mut app, first.generation, Ok(actual));
        let shown = frame(&mut app, 220, 60);
        assert!(shown.contains("orchestrator profile saved: p01"), "{shown}");
        assert!(!shown.contains("saved: p02"), "{shown}");
        app.settings_bodies.orchestrator =
            resolve_orchestrator(None, &root.0.join("config"), Some(&root.0));
        text(&mut app, "ojj");
        key(&mut app, Key::Enter);
        let second = app
            .take_orchestrator_pick()
            .expect("a pick after the landing");
        assert_eq!(second.profile, "p02");
    }

    #[test]
    fn a_write_still_running_blocks_a_new_pick_across_a_reopened_overlay() {
        let (root, mut app) = fixture("orchprof-rev-reopen", 3);
        text(&mut app, "oj");
        key(&mut app, Key::Enter);
        let first = app.take_orchestrator_pick().expect("first queued");
        key(&mut app, Key::Escape);
        assert!(!app.model.settings_open());
        text(&mut app, ",2");
        app.settings_bodies.orchestrator =
            resolve_orchestrator(None, &root.0.join("config"), Some(&root.0));
        text(&mut app, "ojj");
        key(&mut app, Key::Enter);
        assert!(
            app.take_orchestrator_pick().is_none(),
            "the first write has not landed: no second write starts"
        );
        let shown = frame(&mut app, 220, 60);
        assert!(
            shown.contains("saving p01"),
            "the reopened overlay says a write runs: {shown}"
        );
        assert!(!shown.contains("Orchestrator profile"), "{shown}");
        let before = frame(&mut app, 220, 60);
        land(
            &mut app,
            first.generation,
            Ok(Applied {
                changed: true,
                backup: None,
            }),
        );
        assert_eq!(
            frame(&mut app, 220, 60),
            before,
            "an old overlay's landing paints nothing in the new one"
        );
        text(&mut app, "o");
        assert!(
            frame(&mut app, 220, 60).contains("Orchestrator profile"),
            "the landing ended the write: the picker opens again"
        );
    }

    #[test]
    fn the_highlighted_choice_stays_on_screen_in_a_short_panel() {
        let (_root, mut app) = fixture("orchprof-rev-scroll", 50);
        text(&mut app, "o");
        frame(&mut app, 90, 20);
        for _ in 0..25 {
            text(&mut app, "j");
            frame(&mut app, 90, 20);
        }
        let shown = frame(&mut app, 90, 20);
        assert!(
            shown.contains("> p25"),
            "the active choice is visible: {shown}"
        );
        for _ in 0..25 {
            text(&mut app, "k");
            frame(&mut app, 90, 20);
        }
        assert!(frame(&mut app, 90, 20).contains("> p00"));
    }

    #[test]
    fn opening_the_picker_scrolls_a_scrolled_config_tab_to_the_highlighted_choice() {
        let (_root, mut app) = fixture("orchprof-rev-reveal", 50);
        frame(&mut app, 90, 20);
        for _ in 0..3 {
            key(&mut app, Key::PageDown);
            frame(&mut app, 90, 20);
        }
        text(&mut app, "o");
        let shown = frame(&mut app, 90, 20);
        assert!(
            shown.contains("> p00"),
            "the preselected choice is visible: {shown}"
        );
        assert!(shown.contains("Orchestrator profile"), "{shown}");
    }

    #[test]
    fn a_refusal_and_the_backup_it_names_are_reachable_in_a_narrow_panel() {
        let (_root, mut app) = fixture("orchprof-rev-width", 3);
        text(&mut app, "oj");
        key(&mut app, Key::Enter);
        let first = app.take_orchestrator_pick().expect("first queued");
        let backup = std::path::PathBuf::from(
            "/scratch/a-long-but-valid-config-directory/config.1234567890.bak",
        );
        let reason = "the replacement copy has a different owner or group; edit the file by hand";
        land(
            &mut app,
            first.generation,
            Err(ApplyRefusal::Publish {
                why: reason.into(),
                backup: Some(backup),
            }),
        );
        let mut seen = String::new();
        for _ in 0..8 {
            seen.push_str(&frame(&mut app, 90, 20));
            key(&mut app, Key::PageDown);
        }
        assert!(seen.contains("config.1234567890.bak"), "{seen}");
        assert!(seen.contains("edit the file by hand"), "{seen}");
    }

    #[test]
    fn the_reader_answer_is_drawn_for_the_open_it_was_read_for_and_for_no_other() {
        let (root, mut app) = fixture("orchprof-rev-answer", 3);
        let generation = app.model.settings().expect("open").generation;
        let ready = resolve_orchestrator(None, &root.0.join("config"), Some(&root.0));
        app.settings_bodies.orchestrator = OrchestratorView::Loading;
        app.answer(Answer::OrchestratorProfile {
            generation: generation + 1,
            view: ready.clone(),
        });
        let shown = frame(&mut app, 220, 60);
        assert!(
            shown.contains("orchestrator: loading"),
            "an answer for another open is dropped: {shown}"
        );
        app.answer(Answer::OrchestratorProfile {
            generation,
            view: ready,
        });
        let shown = frame(&mut app, 220, 60);
        assert!(
            shown.contains("orchestrator = p00"),
            "the answer for this open is drawn: {shown}"
        );
    }

    #[test]
    fn o_with_the_picker_open_changes_nothing_and_does_not_reset_the_highlight() {
        let (_root, mut app) = fixture("orchprof-rev-reopen-key", 3);
        text(&mut app, "o");
        text(&mut app, "j");
        assert!(frame(&mut app, 220, 60).contains("> p01"));
        text(&mut app, "o");
        let shown = frame(&mut app, 220, 60);
        assert!(
            shown.contains("> p01") && !shown.contains("> p00"),
            "a second o is the picker's and moves nothing: {shown}"
        );
    }

    #[test]
    fn a_config_the_launch_reader_refuses_offers_nothing() {
        let root = Root::new("orchprof-rev-client");
        let path = root.0.join("config");
        let bytes = "[clients]\ncc = claude config_home=/client\n[profiles]\ngood = /bin/echo good\nbad = CLAUDE_CONFIG_DIR=/other cc\n[roster]\norchestrator = good\n";
        fs::write(&path, bytes).expect("config");
        let refusal =
            config::read_identity(Some(&path), None).expect_err("the launch owner refuses");
        match resolve_orchestrator(None, &path, Some(&root.0)) {
            OrchestratorView::Unreadable(why) => {
                assert!(why.contains("CLAUDE_CONFIG_DIR"), "{why} vs {refusal}");
            }
            other => {
                panic!("the launch reader refuses this config, so nothing is offered: {other:?}")
            }
        }
    }

    #[test]
    fn a_saved_record_that_cannot_be_read_is_unknown_not_absent() {
        let (root, mut app) = fixture("orchprof-rev-meta", 3);
        let dir = root.0.join("sessions/orchestrator");
        fs::create_dir_all(dir.join("meta")).expect("an existing record whose meta is no file");
        app.settings_bodies.orchestrator =
            resolve_orchestrator(Some(&dir), &root.0.join("config"), Some(&root.0));
        let shown = frame(&mut app, 220, 60);
        assert!(!shown.contains("(no session)"), "{shown}");
        assert!(
            shown.contains("recorded orchestrator = (unknown"),
            "{shown}"
        );
        fs::remove_dir_all(dir.join("meta")).expect("reset");
        fs::write(
            dir.join("meta"),
            "schema=2\nsession=orchestrator\nprofile.main=\n",
        )
        .expect("meta");
        app.settings_bodies.orchestrator =
            resolve_orchestrator(Some(&dir), &root.0.join("config"), Some(&root.0));
        let shown = frame(&mut app, 220, 60);
        assert!(
            !shown.contains("(no session)"),
            "an empty record is not no session: {shown}"
        );
        assert!(
            shown.contains("recorded orchestrator = (unknown"),
            "{shown}"
        );
    }
}

/// The Config tab keeps its effective-value rows where they always were: the
/// body starts at `session:`, and the orchestrator block follows them. Existing
/// acceptance reads the top of the tab as its scroll oracle.
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "tests own scratch config files and UI channels"
)]
mod placement_tests {
    use std::fs;
    use std::time::Instant;

    use ratatui_core::buffer::Buffer;
    use ratatui_core::layout::Rect;

    use crate::app::settings::resolve_orchestrator;
    use crate::app::tests::Root;
    use crate::app::{App, take_keys};
    use crate::console::input::Key;

    fn text(app: &mut App, line: &str) {
        let key = Key::Text(line.as_bytes().to_vec());
        assert!(take_keys(app, vec![(key, Instant::now())]).is_some());
    }

    fn rows(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        app.frame(&mut buf);
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect()
    }

    #[test]
    fn the_config_tab_starts_at_its_session_row_and_ends_with_the_orchestrator_block() {
        let root = Root::new("orchprof-place");
        let path = root.0.join("config");
        fs::write(
            &path,
            "[profiles]\na = /bin/echo a\nb = /bin/echo b\n[roster]\norchestrator = a\n",
        )
        .expect("config");
        let mut app = App::new(None, None, None, None);
        app.fleeted = true;
        text(&mut app, ",2");
        app.settings_bodies.config = crate::app::settings::resolve_config(None, &path);
        app.settings_bodies.orchestrator = resolve_orchestrator(None, &path, Some(&root.0));
        let shown = rows(&mut app, 150, 40);
        let body = &shown[3..];
        assert!(
            body[0].contains("session:"),
            "the tab starts at its top: {shown:#?}"
        );
        let workers = body
            .iter()
            .position(|row| row.contains("workers = "))
            .expect("a workers row");
        let orchestrator = body
            .iter()
            .position(|row| row.contains("orchestrator = a"))
            .expect("the orchestrator row");
        assert!(
            workers < orchestrator,
            "the orchestrator block follows the effective values: {shown:#?}"
        );
    }
}
