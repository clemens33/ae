//! The cycle's publication steps: the look, the windows, the fleet strip and the per-pane writes.

use super::{
    CYCLE_LOOK_OPTIONS, Carry, Cycle, CycleLookRead, Look, Mark, MotionState, MotionVerdict,
    NON_AGENT_PANES, OptionScope, PaneMark, Published, PublishedOrchestratorStrip, StagedFleet,
    Timestamp, adoption_writes, entry_mut, enumerate_adoption, menu_open_expired,
    orchestrator_id_for, overlay_current_rank, procs, theme, tmux, transport, watch_segment,
    window_agents_line,
};

impl Cycle<'_> {
    /// Rewrite the LAYOUT when the look has moved under it.
    ///
    /// The values this daemon publishes follow the look every cycle, but the
    /// two status lines and the per-window styles are written once, at launch.
    /// So a `@ae_palette` or `@ae_look` changed on a live session would leave
    /// the bar half in the old look for as long as the session ran. The stamp
    /// is what the layout was written FOR; when it and the live look disagree,
    /// the layout is written again — or taken off, which unsets the session
    /// options and hands the user's own global status line back.
    ///
    /// `stamped` is the stamp the cycle's look read found ([`Cycle::look`]); a
    /// stamp that is unset or did not answer reads as empty, as it always has.
    pub(super) fn reconcile_look(
        &self,
        session_id: &str,
        look: &Look,
        stamped: Option<&str>,
        motion: &mut MotionState,
    ) {
        if stamped.unwrap_or_default() == look.stamp() {
            return;
        }
        // The strip value may be unchanged while tmux's layout was removed or
        // repainted. Forget the local equality proof so this cycle republishes
        // it into the new surface.
        motion.published_fleet = None;
        // `&=`, never `&&`: every option is attempted even after one fails, and
        // the STAMP is only advanced when all of them landed. A stamp written
        // over a partial repaint would tell every later cycle the work was
        // done — which is how a session keeps ae's borders after `theme = off`.
        let mut ok = true;
        if look.drawn {
            for (option, value) in theme::layout_options(look) {
                ok &= transport::publish_option(
                    self.server,
                    OptionScope::Session,
                    session_id,
                    &option,
                    &value,
                );
            }
        } else {
            for option in theme::LAYOUT_OPTIONS {
                ok &=
                    transport::clear_option(self.server, OptionScope::Session, session_id, option);
            }
        }
        ok &= self.reconcile_windows(look);
        if !ok {
            return;
        }
        let _ = transport::publish_option(
            self.server,
            OptionScope::Session,
            session_id,
            theme::LOOK_STAMP_OPTION,
            &look.stamp(),
        );
    }

    /// The window half of [`Cycle::reconcile_look`]: restamp every window in
    /// the new look, or unset the options the old one wrote.
    fn reconcile_windows(&self, look: &Look) -> bool {
        // An enumeration that did not RUN is not an empty session: reporting
        // success here would advance the stamp over windows nobody looked at.
        let Some(panes) = transport::observe_window_panes(self.server, self.session) else {
            return false;
        };
        let mut ok = true;
        let mut done: Vec<&str> = Vec::new();
        for pane in &panes {
            if done.contains(&pane.window_id.as_str()) {
                continue;
            }
            done.push(&pane.window_id);
            if look.drawn {
                ok &= crate::session_launch::stamp_window(self.server, &pane.window_id, look);
            } else {
                for option in theme::window_option_names() {
                    ok &= transport::clear_option(
                        self.server,
                        OptionScope::Window,
                        &pane.window_id,
                        &option,
                    );
                }
            }
        }
        ok
    }

    /// The look this session is drawn in, as its own options declare it, with
    /// the session options the rest of the cycle reads beside it: ONE process.
    pub(super) fn look(&self) -> CycleLookRead {
        let read = transport::observe_session_look(self.server, self.session, &CYCLE_LOOK_OPTIONS);
        let mut options = read.options.into_iter();
        let mut next = || options.next().flatten();
        CycleLookRead {
            look: read
                .look
                .map(|read| Look::read(&read.icons, &read.palette, &read.drawn, &read.motion)),
            stamp: next(),
            opened: [next(), next()],
        }
    }

    /// Publish this cycle's verdicts as tmux user options.
    ///
    /// Nothing is sent here: every value joins the cycle's ONE batch
    /// ([`flush_cycle`]), and every value is asserted again each cycle. The
    /// only immediate write is the clear of an expired menu marker, judged on
    /// `opened`, the markers the cycle's look read found.
    pub(super) fn publish(
        &self,
        published: &Published<'_>,
        opened: &[Option<String>; 2],
        carry: &mut Carry,
        table: Option<&[procs::Proc]>,
    ) {
        let Some(session_id) = carry.pending.session_id().map(str::to_owned) else {
            return;
        };
        for (option, opened) in [theme::MENU_OPEN_OPTION, theme::SETTINGS_OPEN_OPTION]
            .into_iter()
            .zip(opened)
        {
            if let Some(opened) = opened
                && menu_open_expired(opened, Timestamp::now().epoch(), self.knobs.interval_secs)
            {
                let _ =
                    transport::clear_option(self.server, OptionScope::Session, &session_id, option);
            }
        }
        let look = published.look;
        let set = |name: &str, value: &str| {
            tmux::CycleOp::Set(tmux::OptionWrite::new(
                OptionScope::Session,
                &session_id,
                name,
                value,
            ))
        };
        let unset = |name: &str| tmux::CycleOp::Unset {
            scope: OptionScope::Session,
            target: session_id.clone(),
            name: name.to_owned(),
        };
        let mut ops = vec![set(tmux::WATCHDOG_STATUS_OPTION, &watch_segment(published))];
        // THE SESSION'S OWN ATTENTION, published as three facts: the glyph it
        // draws with, the rank another session's strip sorts on, and the style
        // its name segment is drawn in. Any session on this server can read
        // them, which is what makes the strip one tmux call rather than a walk
        // of every session's state.
        ops.push(set(
            theme::ATTENTION_GLYPH_OPTION,
            published.attention.glyph(look.icons),
        ));
        ops.push(set(
            theme::ATTENTION_RANK_OPTION,
            &published.attention.rank().to_string(),
        ));
        ops.push(set(
            theme::ATTENTION_STYLE_OPTION,
            &theme::attention_style(&look.palette, published.attention),
        ));
        ops.push(match published.agents {
            Some(agents) => set(theme::AGENTS_OPTION, agents),
            None => unset(theme::AGENTS_OPTION),
        });
        // The GOAL, ahead of the path on the right: what this session is for
        // outranks where its files are, and the path is the fact the reader's
        // own shell prompt already carries.
        ops.push(
            match self
                .goal
                .as_deref()
                .map(str::trim)
                .filter(|g| !g.is_empty())
            {
                Some(goal) => set(
                    theme::GOAL_OPTION,
                    &format!(" {}", theme::bar_text(goal, theme::GOAL_WIDTH)),
                ),
                None => unset(theme::GOAL_OPTION),
            },
        );
        // The core THIS daemon runs on. An upgrade restarts the daemon on the
        // new core, so the value moves with the install and never with a launch.
        ops.push(set(theme::VERSION_OPTION, &crate::shape::version_line()));
        carry.pending.ops.extend(ops);
        // The windows first: they decide this cycle's verdicts, and the fleet
        // stages a copy of the motion state that must already hold them.
        self.publish_windows(published, &mut carry.motion, &mut carry.pending.ops);
        self.publish_fleet(look, carry, table, &session_id, published.attention);
    }

    /// The fleet strip: every ae session on THIS server, as each one's own
    /// watchdog described itself — plus the strip of every same-server ae
    /// session that has no live watchdog to describe it.
    ///
    /// ONE listing answers both, and the adoption scan hangs off it: this is
    /// the read that happens every cycle whatever the ticker is doing, so it is
    /// the only place a daemon with no targets — and therefore no tick of its
    /// own — can ever discover its first one.
    fn publish_fleet(
        &self,
        look: &Look,
        carry: &mut Carry,
        table: Option<&[procs::Proc]>,
        session_id: &str,
        attention: Mark,
    ) {
        let Some(mut sessions) = transport::observe_fleet_listing(self.server) else {
            return;
        };
        // This cycle's own rank, whatever the listing caught of it.
        overlay_current_rank(&mut sessions, session_id, attention);
        // A listing that did not answer is not evidence that a peer is gone, so
        // the enumeration is simply not run and the adoption already held
        // stands until the next cycle.
        if let Some(next) =
            enumerate_adoption(self.server, self.session, &sessions, table, &carry.adoption)
        {
            carry.adoption = next;
        }
        let orchestrator_id = orchestrator_id_for(&sessions, self.session);
        let mut next = carry.motion.clone();
        next.replace_fleet(&sessions, &carry.adoption.known, self.session);
        // ONCE per verdict cycle, from the content this cycle read: a config
        // edit reaches a running session here, and the ticker inherits it.
        next.set_fleet_order(&self.fleet_order);
        let mut writes = Vec::new();
        next.push_fleet_write(&mut writes, look, None);
        if let Some(target) = next.fleet_target.clone() {
            let orchestrator = next
                .fleet
                .iter()
                .find(|row| row.name == crate::orchestrator::ORCHESTRATOR_SESSION)
                .cloned();
            let strip_changed = next.push_orchestrator_strip_write(
                &mut writes,
                look,
                &target,
                orchestrator.as_ref(),
                None,
            );
            if strip_changed
                && matches!(
                    &next.published_orchestrator_strip,
                    PublishedOrchestratorStrip::Unset
                )
                && !transport::clear_option(
                    self.server,
                    OptionScope::Session,
                    &target,
                    theme::ORCHESTRATOR_STRIP_OPTION,
                )
            {
                // The fleet strip is independent and still gets published;
                // restore the cache so the failed unset is retried next cycle.
                next.published_orchestrator_strip
                    .clone_from(&carry.motion.published_orchestrator_strip);
            }
            let id_changed = next.push_orchestrator_id_write(&mut writes, &target, orchestrator_id);
            if id_changed
                && orchestrator_id.is_none()
                && !transport::clear_option(
                    self.server,
                    OptionScope::Session,
                    &target,
                    theme::ORCHESTRATOR_ID_OPTION,
                )
            {
                // The fleet strip is independent and still gets published;
                // restore the cache so the failed unset is retried next cycle.
                next.published_orchestrator_id
                    .clone_from(&carry.motion.published_orchestrator_id);
            }
        }
        // The adopted strips ride the SAME batch: one tmux process carries
        // this session's own line and every line it is filling for a peer.
        let mut adoption = carry.adoption.clone();
        writes.extend(adoption_writes(&mut adoption, &sessions, &next.fleet_order));
        // STAGED, not committed: the batch runs at the cycle's end, and the
        // memories advance only once its writes are proven landed.
        let start = carry.pending.ops.len();
        carry
            .pending
            .ops
            .extend(writes.into_iter().map(tmux::CycleOp::Set));
        carry.pending.fleet = Some(StagedFleet {
            motion: next,
            adoption,
            ops: start..carry.pending.ops.len(),
        });
    }

    /// Per-window marks and per-pane state, grouped from the SAME per-pane
    /// verdicts in pane order — and the theme, restamped on any window that
    /// appeared since the launch dressed the session.
    fn publish_windows(
        &self,
        published: &Published<'_>,
        motion: &mut MotionState,
        ops: &mut Vec<tmux::CycleOp>,
    ) {
        let Some(panes) = transport::observe_window_panes(self.server, self.session) else {
            return;
        };
        let look = published.look;
        let mut windows: Vec<(String, Vec<(String, Mark)>)> = Vec::new();
        let mut verdicts = Vec::new();
        for pane in &panes {
            let agents = entry_mut(&mut windows, &pane.window_id);
            let Some(agent) = pane.agent.as_deref().filter(|name| !name.is_empty()) else {
                continue;
            };
            // The DRAWN name, BEFORE the agent filter and every cycle: a
            // session upgraded in place carries an identity and no label, and
            // the border format reads the label — so an unbackfilled pane, the
            // monitor's own included, would draw a blank title from the moment
            // the look reached it.
            ops.push(tmux::CycleOp::Set(tmux::OptionWrite::new(
                OptionScope::Pane,
                &pane.pane_id,
                theme::AGENT_LABEL_OPTION,
                &theme::agent_label(agent),
            )));
            if NON_AGENT_PANES.contains(&agent) {
                continue;
            }
            let found = published
                .by_pane
                .iter()
                .find(|entry| entry.pane == pane.pane_id);
            let mark = found.map_or(Mark::Idle, |entry| entry.verdict.mark());
            agents.push((theme::agent_label(agent), mark));
            ops.extend(
                Self::pane_state_writes(&pane.pane_id, found, look)
                    .into_iter()
                    .map(tmux::CycleOp::Set),
            );
            if let Some(entry) = found {
                verdicts.push(MotionVerdict {
                    pane: pane.pane_id.clone(),
                    window: pane.window_id.clone(),
                    verdict: entry.verdict,
                });
            }
        }
        for (window_id, agents) in &windows {
            let line = window_agents_line(agents, look, None);
            ops.push(if line.is_empty() {
                tmux::CycleOp::Unset {
                    scope: OptionScope::Window,
                    target: window_id.clone(),
                    name: theme::WINDOW_AGENTS_OPTION.to_owned(),
                }
            } else {
                tmux::CycleOp::Set(tmux::OptionWrite::new(
                    OptionScope::Window,
                    window_id,
                    theme::WINDOW_AGENTS_OPTION,
                    &line,
                ))
            });
        }
        // A window created after the launch — by a spawn on an older core, or
        // by the human — carries no stamp, so it is dressed here rather than
        // left on the user's global window table.
        let mut dressed: Vec<&str> = Vec::new();
        for pane in &panes {
            if pane.theme == theme::window_stamp(look) || dressed.contains(&pane.window_id.as_str())
            {
                continue;
            }
            dressed.push(&pane.window_id);
            crate::session_launch::stamp_window(self.server, &pane.window_id, look);
        }
        motion.replace_verdicts(verdicts);
    }

    /// One pane's border state: its mark, and the word behind it. Nothing
    /// when the pane has no verdict this cycle.
    pub(super) fn pane_state_writes(
        pane: &str,
        found: Option<&PaneMark>,
        look: &Look,
    ) -> Vec<tmux::OptionWrite> {
        let Some(entry) = found else {
            return Vec::new();
        };
        let mark = entry.verdict.mark();
        let state = theme::pane_state(
            &look.palette,
            mark,
            mark.glyph(look.icons),
            entry.verdict.reason(),
        );
        // The ACCENT alone, for the active border: a style option is
        // format-expanded, so the border colour follows the pane it belongs to
        // without a style written per pane.
        vec![
            tmux::OptionWrite::new(OptionScope::Pane, pane, theme::PANE_STATE_OPTION, &state),
            tmux::OptionWrite::new(
                OptionScope::Pane,
                pane,
                theme::PANE_ACCENT_OPTION,
                look.palette.accent(mark),
            ),
            tmux::OptionWrite::new(
                OptionScope::Pane,
                pane,
                theme::OBSERVED_OPTION,
                &entry.observed,
            ),
        ]
    }

    /// The recorded binary for a slot, or `None` when the roster has none —
    /// which the dead check must read as UNKNOWN, never as absent.
    /// The profile the roster records for `slot`, or `""`.
    pub(super) fn seat_profile(&self, slot: &str) -> String {
        self.roster
            .iter()
            .find(|entry| entry.slot == slot)
            .and_then(|entry| entry.profile.clone())
            .unwrap_or_default()
    }

    pub(super) fn agent_bin(&self, slot: &str) -> Option<String> {
        self.roster
            .iter()
            .find(|entry| entry.slot == slot)
            .and_then(|entry| entry.binary.clone())
            .filter(|binary| !binary.is_empty())
    }
}
