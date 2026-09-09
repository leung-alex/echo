//! Target-only UIA subscriptions. Callbacks enqueue an invalidation and do not
//! retrieve event strings, read surrounding text, or retain another app's content.
use super::*;
use windows::{
    core::{implement, Interface, Ref, Result},
    Win32::{
        System::{Com::SAFEARRAY, Variant::VARIANT},
        UI::Accessibility::*,
    },
};

#[implement(
    IUIAutomationEventHandler,
    IUIAutomationPropertyChangedEventHandler,
    IUIAutomationTextEditTextChangedEventHandler
)]
struct Listener {
    shared: Arc<Shared>,
    sender: SyncSender<Request>,
    session: u64,
}
impl IUIAutomationEventHandler_Impl for Listener_Impl {
    fn HandleAutomationEvent(&self, _: Ref<IUIAutomationElement>, _: UIA_EVENT_ID) -> Result<()> {
        self.shared.dirty(&self.sender, self.session);
        Ok(())
    }
}
impl IUIAutomationPropertyChangedEventHandler_Impl for Listener_Impl {
    fn HandlePropertyChangedEvent(
        &self,
        _: Ref<IUIAutomationElement>,
        _: UIA_PROPERTY_ID,
        _: &VARIANT,
    ) -> Result<()> {
        self.shared.dirty(&self.sender, self.session);
        Ok(())
    }
}
impl IUIAutomationTextEditTextChangedEventHandler_Impl for Listener_Impl {
    fn HandleTextEditTextChangedEvent(
        &self,
        _: Ref<IUIAutomationElement>,
        change: TextEditChangeType,
        _: *const SAFEARRAY,
    ) -> Result<()> {
        if self.shared.active.load(Ordering::Acquire) == self.session
            && !self.shared.committing.load(Ordering::Acquire)
        {
            let observed_at = Instant::now();
            self.shared.dirty(&self.sender, self.session);
            let input_serial = self.shared.input_serial.load(Ordering::Acquire);
            // Finalized is an invalidation; only a fresh target read may prove
            // Clear and authorize the final committed query.
            let state = if change == TextEditChangeType_Composition {
                IME_ACTIVE
            } else {
                IME_UNKNOWN
            };
            if let Ok(mut evidence) = self.shared.composition_evidence.try_lock() {
                let update = composition::CompositionEvidence {
                    state,
                    source: composition::CompositionSource::TextEditEvent,
                    session: self.session,
                    input_serial,
                    observed_at,
                };
                if update.can_publish(
                    self.shared.active.load(Ordering::Acquire),
                    self.shared.input_serial.load(Ordering::Acquire),
                    *evidence,
                ) {
                    *evidence = Some(update);
                    self.shared.ime.store(state, Ordering::Release);
                }
            }
        }
        Ok(())
    }
}
pub(super) struct Subscription {
    uia: IUIAutomation,
    element: IUIAutomationElement,
    events: IUIAutomationEventHandler,
    registered: Vec<UIA_EVENT_ID>,
    property: Option<IUIAutomationPropertyChangedEventHandler>,
    composition: Option<(IUIAutomation3, IUIAutomationTextEditTextChangedEventHandler)>,
}
impl Subscription {
    pub(super) fn new(
        uia: &IUIAutomation,
        element: &IUIAutomationElement,
        shared: Arc<Shared>,
        sender: SyncSender<Request>,
        session: u64,
    ) -> Option<Self> {
        unsafe {
            let events: IUIAutomationEventHandler = Listener {
                shared,
                sender,
                session,
            }
            .into();
            let mut registered = Vec::new();
            for id in [
                UIA_Text_TextChangedEventId,
                UIA_Text_TextSelectionChangedEventId,
            ] {
                if uia
                    .AddAutomationEventHandler(id, element, TreeScope_Element, None, &events)
                    .is_ok()
                {
                    registered.push(id);
                }
            }
            let property = events
                .cast::<IUIAutomationPropertyChangedEventHandler>()
                .ok()
                .filter(|p| {
                    uia.AddPropertyChangedEventHandlerNativeArray(
                        element,
                        TreeScope_Element,
                        None,
                        p,
                        &[UIA_ValueValuePropertyId],
                    )
                    .is_ok()
                });
            let composition = uia.cast::<IUIAutomation3>().ok().and_then(|a| {
                let listener = events
                    .cast::<IUIAutomationTextEditTextChangedEventHandler>()
                    .ok()?;
                a.AddTextEditTextChangedEventHandler(
                    element,
                    TreeScope_Element,
                    TextEditChangeType_None,
                    None,
                    &listener,
                )
                .ok()?;
                Some((a, listener))
            });
            Some(Self {
                uia: uia.clone(),
                element: element.clone(),
                events,
                registered,
                property,
                composition,
            })
        }
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        unsafe {
            for id in &self.registered {
                let _ = self
                    .uia
                    .RemoveAutomationEventHandler(*id, &self.element, &self.events);
            }
            if let Some(p) = &self.property {
                let _ = self.uia.RemovePropertyChangedEventHandler(&self.element, p);
            }
            if let Some((uia, listener)) = &self.composition {
                let _ = uia.RemoveTextEditTextChangedEventHandler(&self.element, listener);
            }
        }
    }
}
