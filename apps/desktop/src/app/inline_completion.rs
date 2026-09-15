//! UI orchestration only. External text/ranges and keyboard ownership stay in
//! echo-windows; results still come from the retained-content domain service.
use super::*;
use echo_engine::InlineTicket;
use echo_windows::inline::InlineEvent;
#[derive(Default)]
pub(super) struct InlineUi {
    pub ticket: Option<InlineTicket>,
    pub pending: bool,
    pub unavailable: bool,
    pub composing: bool,
    pub suspended: bool,
    pub above: Option<bool>,
    pub editor_focus: bool,
    #[allow(dead_code)] // Read by the isolated native-test diagnostics, not normal telemetry.
    pub backend: &'static str,
}
impl InlineUi {
    fn accepts_ticket(&self, ticket: InlineTicket, session: u64) -> bool {
        self.ticket.is_some_and(|previous| {
            ticket.session == session
                && previous.session == session
                && ticket.revision >= previous.revision
                && ticket.input_serial >= previous.input_serial
        })
    }
}
impl App {
    pub(super) fn sync_inline_editor_focus(&mut self) {
        let editing = self.inline_active() && self.window.get_editor_open();
        if editing == self.inline_ui.editor_focus {
            return;
        }
        if editing {
            self.worker
                .inline
                .set_editor_active(self.session.epoch, true);
            let result = self
                .hook
                .as_ref()
                .ok_or_else(|| "Window hook unavailable".to_owned())
                .and_then(|hook| hook.set_inline_popup(false))
                .and_then(|_| {
                    self.hwnd
                        .ok_or_else(|| "Window handle unavailable".to_owned())
                })
                .and_then(shell::focus_window);
            if let Err(error) = result {
                self.window.set_editor_open(false);
                self.worker
                    .inline
                    .set_editor_active(self.session.epoch, false);
                self.report(format!("Could not focus the editor: {error}"), true);
                return;
            }
            self.inline_ui.editor_focus = true;
        } else {
            if let Some(hook) = &self.hook {
                if let Err(error) = hook.set_inline_popup(self.inline_active()) {
                    self.report(format!("Could not restore input mode: {error}"), true);
                    return;
                }
            }
            if let Some(snapshot) = &self.activation_focus {
                echo_windows::focus::restore_after_dismiss(snapshot);
            }
            self.inline_ui.editor_focus = false;
            self.worker
                .inline
                .set_editor_active(self.session.epoch, false);
        }
    }
    pub(super) fn inline_active(&self) -> bool {
        self.inline_ui.ticket.is_some() || self.inline_ui.unavailable
    }
    pub(super) fn stop_inline(&mut self) -> bool {
        let _timing = crate::popup_timing::span("inline_retirement");
        // Retire the visual lease before permitting a new host Enter. The
        // adapter still drains any already-consumed physical key-up afterward.
        if self.inline_active() || self.inline_ui.pending {
            if let Err(error) = self.window.hide() {
                self.report(
                    &format!("Inline window could not hide; Enter remains protected: {error}"),
                    true,
                );
                return false;
            }
        }
        self.inline_timer.stop();
        self.worker.inline.cancel(self.session.epoch);
        self.inline_ui = InlineUi::default();
        self.window.set_inline_mode(false);
        if let Some(hook) = &self.hook {
            if let Err(error) = hook.set_inline_popup(false) {
                self.report(&format!("Window mode could not be restored: {error}"), true);
                return false;
            }
        }
        true
    }
    pub(super) fn begin_inline(
        &mut self,
        epoch: u64,
        snapshot: echo_windows::focus::FocusSnapshot,
    ) {
        crate::popup_timing::mark("target_inspection_requested");
        self.inline_ui.pending = true;
        let hub = self.hub.clone();
        self.inline_timer.start(
            TimerMode::SingleShot,
            Duration::from_millis(650),
            move || {
                hub.post(Event::Command(Command::InlineTimeout(epoch)));
            },
        );
        if !self.send(Work::BeginInline(epoch, snapshot)) {
            self.dismiss();
            self.report(
                "Input inspection is busy. Focus stayed in your input; invoke again.",
                false,
            );
        } else {
        }
    }
    pub(super) fn inline_event(&mut self, event: InlineEvent) {
        if self.inline_ui.editor_focus {
            return;
        }
        match event {
            InlineEvent::Started(start) => {
                crate::popup_timing::mark("target_identified");
                if start.ticket.session != self.session.epoch || !self.inline_ui.pending {
                    self.worker.inline.cancel(start.ticket.session);
                    return;
                }
                self.inline_timer.stop();
                self.inline_ui = InlineUi {
                    ticket: Some(start.ticket),
                    backend: start.backend,
                    composing: start.composing,
                    suspended: start.suspended,
                    ..Default::default()
                };
                self.surface.set_query(start.query.clone());
                self.window.set_query(start.query.into());
                self.window.set_inline_mode(true);
                self.handle(Event::Activated(
                    start.ticket.session,
                    Context::QuickInsert,
                    Ok(crate::events::ActivationResult {
                        target: Some(start.target),
                        anchor: Some(start.anchor),
                    }),
                ));
                self.report("Type in your input · Enter replaces this query · Esc cancels · F6 browse and copy", false);
            }
            InlineEvent::Unavailable {
                session,
                reason,
                anchor,
            } => {
                if session != self.session.epoch || !self.inline_ui.pending {
                    return;
                }
                self.inline_timer.stop();
                let snapshot = self
                    .activation_focus
                    .clone()
                    .filter(echo_windows::focus::FocusSnapshot::still_current);
                if snapshot.is_none() || self.worker.inline.readiness()[0] != session {
                    self.dismiss();
                    return;
                }
                // Unavailable is published only after the adapter arms its
                // Enter/Esc/F6 guard. Keep that same session while checking an
                // ordinary paste target; never show an unshielded copy panel.
                self.inline_ui.pending = false;
                self.inline_ui.unavailable = true;
                self.popup_anchor = Some(anchor);
                self.popup_placement = None;
                self.surface.set_space(SpaceId::HISTORY);
                self.surface.set_query(String::new());
                self.window.set_query("".into());
                self.previews.clear();
                self.deck.show(SpaceId::HISTORY);
                self.render_navigation();
                self.pending_scroll = Some(0.0);
                self.compatibility_notice = Some(format!("Input filtering unavailable: {reason}"));
                self.capture_pending = true;
                self.set_busy();
                if !self.send(Work::Begin(session, Context::QuickInsert, snapshot)) {
                    self.dismiss();
                }
            }
            InlineEvent::Changed {
                ticket,
                query,
                anchor,
                composing,
                mut suspended,
            } => {
                if !self.inline_ui.accepts_ticket(ticket, self.session.epoch) {
                    return;
                }
                // A queued pre-paste observation cannot clear an unknown
                // delivery outcome published after it was produced.
                let outcome_unknown = self.worker.inline.replacement_outcome_unknown();
                suspended |= outcome_unknown;
                let state_changed = self.surface.query != query
                    || self.inline_ui.composing != composing
                    || self.inline_ui.suspended != suspended;
                self.inline_ui.ticket = Some(ticket);
                self.inline_ui.composing = composing;
                self.inline_ui.suspended = suspended;
                if let Some(mut anchor) = anchor {
                    // On a single line don't chase every character horizontally.
                    if let Some(previous) = self.popup_anchor {
                        if previous.source == anchor.source
                            && previous.geometry.work_area == anchor.geometry.work_area
                            && previous.geometry.target.y == anchor.geometry.target.y
                        {
                            anchor.geometry.target.x = previous.geometry.target.x;
                        }
                    }
                    self.popup_anchor = Some(anchor);
                }
                if self.surface.query != query {
                    self.command(Command::Query(query.clone()));
                    self.window.set_query(query.into());
                } else {
                    self.inline_results_ready();
                }
                if state_changed && outcome_unknown {
                    self.report("Replacement outcome is unknown. Check the input; Esc/F6 is required before another insertion.", true);
                } else if state_changed && composing {
                    self.report(
                        "Input method is composing · candidate keys stay with the input method",
                        false,
                    );
                } else if state_changed && suspended {
                    self.report("IME state is unavailable · Enter is protected · F6 opens history for copying", false);
                } else if state_changed {
                    self.report(
                        "Enter replaces the query · Esc keeps typed text · F6 browse and copy",
                        false,
                    );
                }
                self.prepare_window_geometry();
            }
            InlineEvent::Navigate { session, delta } => {
                if self.inline_active()
                    && session == self.session.epoch
                    && !self.inline_ui.composing
                    && !self.inline_ui.suspended
                {
                    self.keyboard(Intent::Move(delta));
                }
            }
            InlineEvent::SwitchSpace { session, delta } => {
                if self.inline_active()
                    && session == self.session.epoch
                    && !self.inline_ui.composing
                    && !self.inline_ui.suspended
                {
                    self.worker.inline.invalidate_results();
                    self.navigate(delta);
                }
            }
            InlineEvent::Confirm(ticket) => {
                if self.inline_ui.ticket != Some(ticket)
                    || !self.worker.inline.live(ticket)
                    || self.session.busy()
                    || !self.surface.ready
                    || self.surface.loading
                    || self.surface.selection.is_none()
                    || !self.deck.can_insert(self.surface.space)
                    || self.inline_ui.composing
                    || self.inline_ui.suspended
                {
                    if ticket.session == self.session.epoch {
                        self.report("Results changed; press Enter again on a current result. Nothing was sent.", false);
                    }
                    return;
                }
                self.execute(self.surface.selection.unwrap(), QuickInsertAction::Insert);
            }
            InlineEvent::Cancelled { session, reason } => {
                if session == self.session.epoch && (self.inline_active() || self.inline_ui.pending)
                {
                    self.activation_focus = None;
                    self.dismiss();
                    self.report(reason, false);
                }
            }
            InlineEvent::Compatibility { session, reason } => {
                if session == self.session.epoch && (self.inline_active() || self.inline_ui.pending)
                {
                    self.open_manual_history(reason);
                }
            }
            InlineEvent::Suspended { session, reason } => {
                if self.inline_active() && session == self.session.epoch {
                    self.inline_ui.suspended = true;
                    self.worker.inline.invalidate_results();
                    self.report(reason, false);
                }
            }
            InlineEvent::Notice { session, text } => {
                if self.inline_active() && session == self.session.epoch {
                    self.report(text, false);
                }
            }
        }
    }
    fn open_manual_history(&mut self, reason: String) {
        // A late provider response must not pull focus away from a new editor.
        let snapshot = echo_windows::focus::FocusSnapshot::capture();
        let still_original = self.activation_focus.as_ref().is_some_and(|old| {
            old.window_id == snapshot.window_id
                && old.process_id == snapshot.process_id
                && old.process_started_at == snapshot.process_started_at
                && snapshot.still_current()
        });
        let anchor = self.popup_anchor.clone();
        if !self.stop_inline() {
            return;
        }
        self.activation_focus = None;
        if !still_original {
            self.dismiss();
            return;
        }
        self.popup_placement = None;
        self.surface.set_query(String::new());
        self.window.set_query("".into());
        self.previews.clear();
        self.surface.set_space(SpaceId::HISTORY);
        self.deck.show(SpaceId::HISTORY);
        self.render_navigation();
        self.pending_scroll = Some(0.0);
        let epoch = self.session.activate(Context::QuickInsert);
        self.worker.epoch.store(epoch, Ordering::Release);
        self.compatibility_notice = Some(reason);
        self.handle(Event::Activated(
            epoch,
            Context::QuickInsert,
            Ok(crate::events::ActivationResult {
                target: None,
                anchor,
            }),
        ));
    }

    pub(super) fn inline_results_ready(&self) {
        if let Some(ticket) = self.inline_ui.ticket {
            self.worker.inline.results_ready(
                ticket,
                self.surface.ready
                    && !self.surface.loading
                    && !self.surface.dirty
                    && self.surface.selection.is_some()
                    && !self.session.busy()
                    && !self.window.get_modal()
                    && self.deck.can_insert(self.surface.space)
                    && !self.inline_ui.composing
                    && !self.inline_ui.suspended,
            );
        }
    }
    pub(super) fn execute_inline_item(&mut self, key: RowKey) {
        let Some(ticket) = self.inline_ui.ticket else {
            return;
        };
        if !self.worker.inline.live(ticket)
            || self.surface.loading
            || !self.surface.ready
            || self.inline_ui.composing
            || self.inline_ui.suspended
            || !self.deck.can_insert(self.surface.space)
        {
            self.report(
                "Input or results changed. Nothing was replaced or sent.",
                false,
            );
            return;
        }
        let Some(operation) = self.session.begin(QuickInsertAction::Insert) else {
            return;
        };
        self.worker.inline.invalidate_results();
        self.set_busy();
        self.report("Replacing the verified query range…", false);
        // Keep the non-activating popup alive until delivery is acknowledged.
        if !self.send(Work::ExecuteInline(operation, key, ticket)) {
            self.session.finish(operation, Err(()));
            self.set_busy();
            self.inline_results_ready();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_observations_never_regress_revision_or_input_serial() {
        let current = InlineTicket {
            session: 9,
            revision: 4,
            input_serial: 12,
        };
        let ui = InlineUi {
            ticket: Some(current),
            ..Default::default()
        };
        assert!(ui.accepts_ticket(current, 9));
        assert!(ui.accepts_ticket(
            InlineTicket {
                revision: 5,
                input_serial: 13,
                ..current
            },
            9
        ));
        assert!(!ui.accepts_ticket(
            InlineTicket {
                revision: 3,
                ..current
            },
            9
        ));
        assert!(!ui.accepts_ticket(
            InlineTicket {
                input_serial: 11,
                ..current
            },
            9
        ));
        assert!(!ui.accepts_ticket(
            InlineTicket {
                revision: 5,
                input_serial: 11,
                ..current
            },
            9
        ));
        assert!(!ui.accepts_ticket(
            InlineTicket {
                session: 8,
                ..current
            },
            9
        ));
        assert!(!ui.accepts_ticket(current, 10));
        assert!(!InlineUi::default().accepts_ticket(current, 9));
    }
}
