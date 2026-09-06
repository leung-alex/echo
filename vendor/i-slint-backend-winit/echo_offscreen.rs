//! Echo-only, opt-in adapter factory for a GPU offscreen component. No HWND is created.
use i_slint_core::window::WindowAdapter;
use std::{cell::RefCell,rc::Rc};
thread_local! { static NEXT:RefCell<Option<Rc<dyn WindowAdapter>>>=RefCell::new(None); }
/// Construct exactly one Slint component with the provided adapter, preserving the normal platform.
/// The override is scoped to this thread and restored even if construction fails or panics.
pub fn with_adapter<T>(adapter:Rc<dyn WindowAdapter>,create:impl FnOnce()->T)->T {
    struct Restore(Option<Rc<dyn WindowAdapter>>);
    impl Drop for Restore { fn drop(&mut self) {NEXT.with(|slot|*slot.borrow_mut()=self.0.take());} }
    let guard=Restore(NEXT.with(|slot|slot.replace(Some(adapter))));
    let result=create();drop(guard);result
}
pub(crate) fn take()->Option<Rc<dyn WindowAdapter>> {NEXT.with(|slot|slot.borrow_mut().take())}
