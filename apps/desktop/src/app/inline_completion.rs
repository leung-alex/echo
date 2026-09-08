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
    #[allow(dead_code)] // Read by the isolated native-test diagnostics, not normal telemetry.
    pub backend: &'static str,
}
impl App {
    pub(super) fn inline_active(&self) -> bool {
        self.inline_ui.ticket.is_some() || self.inline_ui.unavailable
    }
    pub(super) fn stop_inline(&mut self) -> bool {
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
        }
    }
    pub(super) fn inline_event(&mut self, event: InlineEvent) {
        match event {
            InlineEvent::Started(start) => {
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
                self.report("Type in your input · Enter replaces this query · Esc cancels · F6 independent search", false);
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
                self.inline_ui = InlineUi {
                    unavailable: true,
                    suspended: true,
                    ..Default::default()
                };
                self.surface.set_query(String::new());
                self.window.set_query("".into());
                self.window.set_inline_mode(true);
                self.handle(Event::Activated(
                    session,
                    Context::QuickInsert,
                    Ok(crate::events::ActivationResult {
                        target: None,
                        anchor: Some(anchor),
                    }),
                ));
                self.report(format!("Input stays active · {reason} · F6: search in Echo (no automatic replacement)"),false);
            }
            InlineEvent::Changed {
                ticket,
                query,
                anchor,
                composing,
                mut suspended,
            } => {
                if self.inline_ui.ticket.is_none() || ticket.session != self.session.epoch {
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
                    self.report("IME state is unavailable · Enter is protected · F6 opens independent search", false);
                } else if state_changed {
                    self.report(
                        "Enter replaces the query · Esc keeps typed text · F6 independent search",
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
                    self.inline_fallback(reason);
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
    pub(super) fn inline_fallback(&mut self, reason: String) {
        let snapshot = echo_windows::focus::FocusSnapshot::capture();
        let still_original = self.activation_focus.as_ref().is_some_and(|old| {
            old.window_id == snapshot.window_id
                && old.process_id == snapshot.process_id
                && old.process_started_at == snapshot.process_started_at
                && snapshot.still_current()
        });
        if !self.stop_inline() {
            return;
        }
        self.popup_placement = None;
        if !still_original {
            self.activation_focus = None;
            self.dismiss();
            return;
        }
        self.capture_pending = true;
        self.activation_focus = Some(snapshot.clone());
        self.compatibility_notice = Some(format!(
            "Independent search: {reason}. Existing composer text is kept, not replaced."
        ));
        if !self.send(Work::Begin(
            self.session.epoch,
            Context::QuickInsert,
            Some(snapshot),
        )) {
            self.capture_pending = false;
            self.dismiss();
        }
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
