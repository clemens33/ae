//! The menu and confirmation modals: opening, keys, mouse, actions and painting.

use super::{App, Buffer, Instant, Key, Mouse, MouseKind, Request, draw, menu, theme};

impl App {
    /// The menu on `card`, drawn by the frame input read at `origin` was made
    /// on, or the hint row says none opened; that read is spent either way.
    /// Opening selects nothing.
    pub(super) fn open_menu(&mut self, card: Option<menu::Card>, origin: Instant) -> bool {
        self.spend(origin);
        let Some(card) = card else {
            let line = "menu not opened: no card is on screen for it".to_owned();
            self.flash = Some((line, Instant::now()));
            return true;
        };
        if card.items().is_empty() {
            let line = format!(
                "menu not opened: {} has nothing to start or stop",
                card.name
            );
            self.flash = Some((line, Instant::now()));
            return true;
        }
        self.unselect();
        self.modal = Some(menu::Modal::Menu(menu::Menu::new(card)));
        true
    }

    /// `s` on the selection: the menu's Start-or-Stop item on the card the
    /// frame drew — Start a stopped session, or ask the Stop question of a
    /// running one.
    pub(super) fn toggle(&mut self, origin: Instant) -> bool {
        let card = (self.model.selected()).and_then(|name| self.layout.cards.get(name));
        let opened = self.open_menu(card.cloned(), origin);
        if let Some(menu::Modal::Menu(menu)) = &self.modal
            && let Some(item) = menu.card.toggle_item()
        {
            self.act(item);
        }
        opened
    }

    /// A right press: the menu on the card the frame on screen at `origin`
    /// drew under it. One made on a frame the bound dropped opens none.
    pub(super) fn menu_press(&mut self, mouse: Mouse, origin: Instant) -> bool {
        let ended = self.model.let_go();
        if self.lost.is_some_and(|lost| origin <= lost) || self.zoom_gone(origin) {
            self.spend(origin);
            let line = "menu not opened: its frame is gone".to_owned();
            self.flash = Some((line, Instant::now()));
            return true;
        }
        let layout = self.layout_at(origin);
        let Some(draw::Hit::Session(name)) = layout.hit(mouse) else {
            return ended;
        };
        let card = layout.cards.get(&name).cloned();
        self.open_menu(card, origin)
    }

    /// Input the open modal takes only once a frame showing it was written
    /// before the input was read: earlier input is dropped, and the hint row
    /// says to press again.
    fn unseen(&mut self, origin: Instant) -> bool {
        let Some(modal) = &self.modal else {
            return false;
        };
        if modal.shown_at().is_some_and(|shown| origin > shown) {
            return false;
        }
        let word = modal.word();
        let line = format!("not taken: the {word} was not on screen yet - press again");
        self.flash = Some((line, Instant::now()));
        true
    }

    /// A key with a modal open: nothing under the modal sees it.
    pub(super) fn modal_key(&mut self, key: &Key, origin: Instant) -> bool {
        if self.unseen(origin) {
            return true;
        }
        let Some(modal) = self.modal.as_mut() else {
            return false;
        };
        let step = modal.key(key);
        self.stepped(step, origin)
    }

    /// A mouse report with a modal open: motion and the wheel die, a press
    /// acts on the frame it was made on.
    pub(super) fn modal_mouse(&mut self, mouse: Mouse, origin: Instant) -> bool {
        let right = mouse.kind == MouseKind::Menu;
        if !right && mouse.kind != MouseKind::Click {
            return false;
        }
        if self.unseen(origin) {
            return true;
        }
        if self.lost.is_some_and(|lost| origin <= lost) || self.zoom_gone(origin) {
            let line = "click not taken: its frame is gone".to_owned();
            self.flash = Some((line, Instant::now()));
            return true;
        }
        let hit = self.layout_at(origin).hit(mouse);
        let Some(modal) = &self.modal else {
            return false;
        };
        let step = modal.press(right, hit.as_ref());
        self.stepped(step, origin)
    }

    /// Carry out what input read at `origin` did to the modal: one that ends
    /// it spends that read.
    fn stepped(&mut self, step: menu::Step, origin: Instant) -> bool {
        if step.ends() {
            self.spend(origin);
        }
        match step {
            menu::Step::Kept => return false,
            menu::Step::Moved => {}
            menu::Step::Close => self.close_modal(),
            menu::Step::Act(item) => self.act(item),
            menu::Step::Affirm => self.affirm(),
        }
        true
    }

    /// Take the open modal down. A menu still waiting for its dialog's rows
    /// tells the reader to drop the read.
    fn close_modal(&mut self) {
        let waiting =
            matches!(&self.modal, Some(menu::Modal::Menu(menu)) if menu.reading.is_some());
        self.modal = None;
        if waiting && let Some(ask) = &self.ask {
            let _ = ask.send(Request::Records(None));
        }
    }

    /// `item` of the open menu: a dialog is asked of the reader; else its
    /// card's refusal, else Start or the Stop or End question. The menu
    /// closes either way, but for a dialog, which it waits on.
    fn act(&mut self, item: menu::Item) {
        if let Some(dialog) = item.dialog() {
            return self.ask_records(item, dialog);
        }
        let Some(menu::Modal::Menu(menu)) = self.modal.take() else {
            return;
        };
        let (name, card) = (menu.card.name.clone(), menu.card);
        if let Some(why) = card.refusal() {
            let line = format!("refused: {} {name}: {why}; nothing done", item.word());
            return self.tell(&name, line);
        }
        match item {
            menu::Item::Start => {
                let line = (self.start(&card))
                    .unwrap_or_else(|why| format!("refused: Start {name}: {why}; nothing started"));
                self.tell(&name, line);
            }
            menu::Item::Stop => match self.question(card) {
                Ok(question) => self.modal = Some(menu::Modal::Question(question)),
                Err(line) => self.tell(&name, line),
            },
            menu::Item::End => match self.end_question(card) {
                Ok(question) => self.modal = Some(menu::Modal::End(question)),
                Err(why) => {
                    let line = format!("refused: End {name}: {why}; nothing ended");
                    self.tell(&name, line);
                }
            },
            menu::Item::Activity | menu::Item::Memos => {}
        }
    }

    /// Paint the open modal over the frame. One that does not fit whole is
    /// taken down, the hint row says what it needed, and every key read so
    /// far is spent: nothing it held can act.
    pub(super) fn paint_modal(&mut self, look: Option<&theme::Look>, buf: &mut Buffer) {
        let Some(modal) = &self.modal else {
            return;
        };
        let icons = look.is_none_or(|look| look.icons);
        match menu::paint(modal, draw::Paint::of(look), icons, buf, &mut self.layout) {
            Ok(()) => self.layout.cursor = None,
            Err(why) => {
                self.modal = None;
                self.spend(Instant::now());
                self.flash = Some((why, Instant::now()));
                self.again = true;
            }
        }
    }
}
