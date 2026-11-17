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

    /// The orchestrator profile pick owed to the writer thread, handed once.
    pub(super) const fn take_orchestrator_pick(&mut self) -> Option<settings::OrchestratorPick> {
        None
    }

    /// What became of the pick of `generation`, drawn only under that open.
    pub(super) fn orchestrator_landed(
        &mut self,
        _generation: u64,
        _outcome: Result<crate::config::Applied, crate::config::ApplyRefusal>,
    ) {
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
