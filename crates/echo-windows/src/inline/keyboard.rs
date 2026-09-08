//! The hook thread never reads text, calls UIA/COM, searches, or waits for a reply.
//! It only verifies the current HWND, consumes session control keys, and enqueues.
use super::key_policy::{decide_key, KeyDecision, KeyState};
use super::*;
use std::{
    cell::{Cell, RefCell},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{Accessibility::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
const WAKE: u32 = WM_APP + 61;
const RETIRE_TIMER: usize = 62;
thread_local! { static LOCAL: RefCell<Option<Local>> = const { RefCell::new(None) }; }
// Separate from Local's borrow: reentrant callbacks must still own Enter.
#[derive(Clone, Copy, Default)]
struct GuardLease {
    session: u64,
    window: isize,
    consumed_enter: bool,
    ime_enter: bool,
    enter_down: bool,
}
thread_local! { static GUARD: Cell<GuardLease> = Cell::new(GuardLease::default()); }

unsafe fn reentrant_enter(down: bool) -> bool {
    GUARD.with(|cell| {
        let mut guard = cell.get();
        let owns = guard.consumed_enter
            || (down && guard.session != 0 && GetForegroundWindow() as isize == guard.window);
        if owns {
            guard.consumed_enter = down;
            cell.set(guard);
            PostThreadMessageW(GetCurrentThreadId(), WAKE, 0, 0);
        }
        owns
    })
}
enum Command {
    Arm(u64, SyncSender<Result<(), String>>),
    Stop,
}
struct Owner {
    sender: SyncSender<Command>,
    thread: u32,
    stopped: Arc<AtomicBool>,
    retire_requested: Arc<AtomicU64>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}
#[derive(Clone)]
pub(super) struct InputHook {
    owner: Arc<Owner>,
}
impl InputHook {
    pub(super) fn start(shared: Arc<Shared>, sender: SyncSender<Request>) -> Result<Self, String> {
        let (commands, receiver) = mpsc::sync_channel(8);
        let (ready, started) = mpsc::sync_channel(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stop = stopped.clone();
        let retire_requested = Arc::new(AtomicU64::new(0));
        let worker_retire = retire_requested.clone();
        let worker = std::thread::Builder::new()
            .name("echo-inline-input-hook".into())
            .spawn(move || unsafe {
                let mut message: MSG = std::mem::zeroed();
                PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
                let tid = GetCurrentThreadId();
                let timer = SetTimer(null_mut(), RETIRE_TIMER, 50, None);
                if timer == 0 {
                    let _ = ready.send(Err("Inline retirement watchdog did not start".to_string()));
                    return;
                }
                LOCAL.with(|slot| {
                    *slot.borrow_mut() = Some(Local {
                        shared,
                        sender,
                        hook: null_mut(),
                        events: Vec::new(),
                        down: [false; 256],
                        swallowed: [false; 256],
                        ime_window: 0,
                        session: 0,
                        retiring: false,
                    })
                });
                let _ = ready.send(Ok(tid));
                while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    if message.message == WAKE
                        || (message.message == WM_TIMER && message.wParam == timer)
                    {
                        LOCAL.with(|slot| {
                            if let Some(state) = slot.borrow_mut().as_mut() {
                                if !state.retiring
                                    && state.session != 0
                                    && worker_retire.load(Ordering::Acquire) == state.session
                                {
                                    state.shared.record("disarm-acknowledged", 0);
                                    let _ = worker_retire.compare_exchange(
                                        state.session,
                                        0,
                                        Ordering::AcqRel,
                                        Ordering::Acquire,
                                    );
                                    state.retire();
                                }
                            }
                        });
                        let mut stop = false;
                        while let Ok(command) = receiver.try_recv() {
                            LOCAL.with(|slot| {
                                let mut slot = slot.borrow_mut();
                                let state = slot.as_mut().unwrap();
                                match command {
                                    Command::Arm(id, reply) => {
                                        let _ = reply.send(state.arm(id));
                                    }
                                    Command::Stop => {
                                        state.clear();
                                        stop = true;
                                    }
                                }
                            });
                        }
                        // Disarm after the swallowed physical key-up, not after key-down.
                        LOCAL.with(|slot| {
                            if let Some(state) = slot.borrow_mut().as_mut() {
                                if state.retiring
                                    && !state.swallowed.iter().any(|v| *v)
                                    && !GUARD.with(|g| g.get().consumed_enter || g.get().ime_enter)
                                {
                                    state.clear();
                                }
                            }
                        });
                        if stop {
                            break;
                        }
                    } else {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                KillTimer(null_mut(), timer);
                LOCAL.with(|slot| {
                    if let Some(mut state) = slot.borrow_mut().take() {
                        state.clear();
                    }
                });
            })
            .map_err(|e| e.to_string())?;
        let thread = started
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Inline hook thread did not start")??;
        Ok(Self {
            owner: Arc::new(Owner {
                sender: commands,
                thread,
                stopped,
                retire_requested,
                worker: Mutex::new(Some(worker)),
            }),
        })
    }
    fn send(&self, command: Command) -> Result<(), String> {
        self.owner
            .sender
            .try_send(command)
            .map_err(|_| "Inline hook control queue is full")?;
        if unsafe { PostThreadMessageW(self.owner.thread, WAKE, 0, 0) } == 0 {
            return Err("Could not notify inline hook thread".into());
        }
        Ok(())
    }
    pub(super) fn arm(&self, session: u64) -> Result<(), String> {
        let (reply, response) = mpsc::sync_channel(1);
        self.send(Command::Arm(session, reply))?;
        response
            .recv_timeout(Duration::from_millis(200))
            .map_err(|_| "Inline hook did not acknowledge activation")?
    }
    pub(super) fn disarm(&self, session: u64) {
        // Retirement cannot be lost to the bounded command queue. The message
        // is a prompt wake; the independent timer also observes this request.
        self.owner
            .retire_requested
            .fetch_max(session, Ordering::AcqRel);
        unsafe {
            PostThreadMessageW(self.owner.thread, WAKE, 0, 0);
        }
    }
    pub(super) fn stop(&self) {
        self.owner.stopped.store(true, Ordering::Release);
        let _ = self.send(Command::Stop);
        unsafe {
            PostThreadMessageW(self.owner.thread, WAKE, 0, 0);
        }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.sender.try_send(Command::Stop);
        unsafe {
            PostThreadMessageW(self.thread, WAKE, 0, 0);
        }
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}
struct Local {
    shared: Arc<Shared>,
    sender: SyncSender<Request>,
    hook: HHOOK,
    events: Vec<HWINEVENTHOOK>,
    down: [bool; 256],
    swallowed: [bool; 256],
    ime_window: isize,
    session: u64,
    retiring: bool,
}
impl Local {
    unsafe fn arm(&mut self, id: u64) -> Result<(), String> {
        if self.shared.requested.load(Ordering::Acquire) != id {
            return Err("Inline activation was cancelled".into());
        }
        // Acquire the replacement before releasing a live registration. A
        // failed re-arm must not expose an already-consumed physical sequence.
        let replacement = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard),
            GetModuleHandleW(std::ptr::null()),
            0,
        );
        if replacement.is_null() {
            return Err(format!(
                "Windows refused the inline keyboard hook ({})",
                GetLastError()
            ));
        }
        if !self.hook.is_null() {
            UnhookWindowsHookEx(self.hook);
        }
        self.hook = replacement;
        self.clear_events();
        self.session = id;
        self.ime_window = 0;
        self.shared.ime_ui.store(IME_UNKNOWN, Ordering::Release);
        self.retiring = false;
        GUARD.with(|g| {
            let mut lease = g.get();
            lease.session = id;
            lease.window = self.shared.window.load(Ordering::Acquire);
            g.set(lease);
        });
        // Sample only after registration. A key-up in the old registration gap
        // must not become a modifier held for the lifetime of the new session.
        for (i, value) in self.down.iter_mut().enumerate() {
            *value = GetAsyncKeyState(i as i32) < 0;
        }
        GUARD.with(|g| {
            let mut lease = g.get();
            lease.enter_down = self.down[VK_RETURN as usize];
            g.set(lease);
        });
        for (start, end, pid) in [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, 0),
            (EVENT_OBJECT_FOCUS, EVENT_OBJECT_FOCUS, 0),
            (EVENT_OBJECT_IME_SHOW, EVENT_OBJECT_IME_CHANGE, 0),
            (EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE, 0),
            (
                EVENT_OBJECT_VALUECHANGE,
                EVENT_OBJECT_VALUECHANGE,
                self.shared.process.load(Ordering::Acquire),
            ),
            (
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                self.shared.process.load(Ordering::Acquire),
            ),
        ] {
            let hook = SetWinEventHook(
                start,
                end,
                null_mut(),
                Some(win_event),
                pid,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if hook.is_null() {
                // Observation failed, but the acknowledged keyboard lease
                // remains until the caller hides/cancels and it drains key-up.
                self.clear_events();
                return Err("Windows refused input-change observation".into());
            }
            self.events.push(hook);
        }
        // One activation-time inspection covers a candidate already visible.
        if let Some(candidate) =
            super::ime_window::visible(self.shared.focus.load(Ordering::Acquire) as HWND)
        {
            self.ime_window = candidate as isize;
            self.shared.ime_ui.store(IME_ACTIVE, Ordering::Release);
        }
        Ok(())
    }
    unsafe fn clear_events(&mut self) {
        for event in self.events.drain(..) {
            UnhookWinEvent(event);
        }
    }
    unsafe fn clear(&mut self) {
        self.clear_events();
        if !self.hook.is_null() {
            UnhookWindowsHookEx(self.hook);
            self.hook = null_mut();
        }
        self.retiring = false;
        self.session = 0;
        GUARD.with(|g| g.set(GuardLease::default()));
    }
    unsafe fn retire(&mut self) {
        self.clear_events();
        self.retiring = true;
        GUARD.with(|g| {
            let mut lease = g.get();
            lease.session = 0;
            g.set(lease);
        });
        if !self.swallowed.iter().any(|v| *v)
            && !GUARD.with(|g| g.get().consumed_enter || g.get().ime_enter)
        {
            self.clear();
        }
    }
    unsafe fn target_current(&self) -> bool {
        self.target_match() == Some(true)
    }
    unsafe fn target_match(&self) -> Option<bool> {
        let root = self.shared.window.load(Ordering::Acquire) as HWND;
        let foreground = GetForegroundWindow();
        if root.is_null() {
            return Some(false);
        }
        if foreground.is_null() {
            return None;
        }
        if foreground != root {
            return Some(false);
        }
        // Chromium/WPF virtual text elements may retain their UIA identity while
        // their native focus proxy HWND changes. The MTA validates that actual
        // element; a low-level hook must not confuse the proxy with the input.
        if !self.shared.native_identity.load(Ordering::Acquire) {
            return Some(true);
        }
        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if GetGUIThreadInfo(self.shared.thread.load(Ordering::Acquire), &mut info) == 0
            || info.hwndFocus.is_null()
        {
            return None;
        }
        Some(info.hwndFocus as isize == self.shared.focus.load(Ordering::Acquire))
    }
    fn mods(&self) -> bool {
        [
            VK_LCONTROL,
            VK_RCONTROL,
            VK_LMENU,
            VK_RMENU,
            VK_LWIN,
            VK_RWIN,
        ]
        .into_iter()
        .any(|vk| self.down[usize::from(vk)])
    }
    fn shift(&self) -> bool {
        [VK_LSHIFT, VK_RSHIFT]
            .into_iter()
            .any(|v| self.down[usize::from(v)])
    }
    unsafe fn reconcile_modifiers(&mut self) -> u32 {
        let mut mask = 0;
        for (bit, key) in [
            VK_LCONTROL,
            VK_RCONTROL,
            VK_LMENU,
            VK_RMENU,
            VK_LWIN,
            VK_RWIN,
            VK_LSHIFT,
            VK_RSHIFT,
        ]
        .into_iter()
        .enumerate()
        {
            let held = GetAsyncKeyState(key as i32) < 0;
            self.down[key as usize] = held;
            if held {
                mask |= 1 << bit;
            }
        }
        mask
    }
}
unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let input = &*(l as *const KBDLLHOOKSTRUCT);
    if input.dwExtraInfo == INJECTED_TAG || input.vkCode >= 256 {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let down = w as u32 == WM_KEYDOWN || w as u32 == WM_SYSKEYDOWN;
    let up = w as u32 == WM_KEYUP || w as u32 == WM_SYSKEYUP;
    if !down && !up {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let enter_repeat = if input.vkCode == VK_RETURN as u32 {
        GUARD.with(|g| {
            let mut lease = g.get();
            let was_down = lease.enter_down;
            lease.enter_down = down;
            g.set(lease);
            was_down
        })
    } else {
        false
    };
    if input.vkCode == VK_RETURN as u32 && GUARD.with(|g| g.get().consumed_enter) {
        return if reentrant_enter(down) { 1 } else { 0 };
    }
    if input.vkCode == VK_RETURN as u32 && GUARD.with(|g| g.get().ime_enter) {
        if up {
            GUARD.with(|g| {
                let mut lease = g.get();
                lease.ime_enter = false;
                g.set(lease);
            });
            PostThreadMessageW(GetCurrentThreadId(), WAKE, 0, 0);
            return CallNextHookEx(null_mut(), code, w, l);
        }
        return 1;
    }
    let handled = LOCAL.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return input.vkCode == VK_RETURN as u32 && reentrant_enter(down);
        };
        let Some(state) = slot.as_mut() else { return false; };
        let key = input.vkCode as usize;
        let repeat = if key == VK_RETURN as usize { enter_repeat } else { state.down[key] };
        state.down[key] = down;
        if state.swallowed[key] {
            if up { state.swallowed[key] = false; PostThreadMessageW(GetCurrentThreadId(), WAKE, 0, 0); }
            return true;
        }
        let session = state.shared.active.load(Ordering::Acquire);
        if state.retiring { return false; }
        if session == 0 || session != state.session {
            // Cancellation publication is not hook retirement. Until Disarm is
            // processed, the lease still protects Enter under the old popup.
            return key == VK_RETURN as usize && reentrant_enter(down);
        }
        let target_match = state.target_match();
        if target_match != Some(true) {
            let consume = down && key == VK_RETURN as usize;
            if consume { GUARD.with(|g| { let mut lease = g.get(); lease.consumed_enter = true; g.set(lease); }); }
            if target_match == Some(false) {
                state.shared.cancel(session, "Input focus changed; inline completion cancelled");
            } else {
                state.shared.dirty(&state.sender, session);
            }
            return consume;
        }
        if down && [VK_RETURN, VK_ESCAPE, VK_F6, VK_UP, VK_DOWN, VK_TAB].contains(&(key as u16)) {
            let modifiers = state.reconcile_modifiers();
            state.shared.record("control-key-state", (key as u32) | (modifiers << 8) | ((repeat as u32) << 16));
        }
        let composition = state.shared.verified_composition();
        if down && key == VK_RETURN as usize {
            let decision = decide_key(KeyState { guarded: true, consumed_sequence: false,
                ime_sequence: false, down, repeat, modified: state.mods() || state.shift(),
                composing: composition == IME_ACTIVE, ready: composition == IME_CLEAR && state.shared.can_confirm() });
            state.shared.record("enter-decision", decision as u32);
            if decision == KeyDecision::PassToVerifiedIme {
                GUARD.with(|g| { let mut lease = g.get(); lease.ime_enter = true; g.set(lease); });
                state.shared.input_changed(&state.sender, session); return false;
            }
            GUARD.with(|g| { let mut lease = g.get(); lease.consumed_enter = true; g.set(lease); });
            if decision == KeyDecision::ConsumeAndConfirm {
                let ticket = state.shared.ticket(); state.shared.selectable.store(false, Ordering::Release);
                (state.shared.callback)(InlineEvent::Confirm(ticket));
            } else {
                (state.shared.callback)(InlineEvent::Notice { session, text: if composition == IME_UNKNOWN {
                    "IME state is not verified. Finish composition or press F6 for independent search. Enter was not sent."
                } else { "Results are updating, empty, or a modifier is held. Enter was not sent; press it again when ready." }});
            }
            return true;
        }
        if down && composition != IME_ACTIVE && !state.mods() {
            if key == VK_ESCAPE as usize {
                state.swallowed[key] = true; state.shared.cancel(session, "Inline completion cancelled; typed query kept"); return true;
            }
            if key == VK_F6 as usize {
                state.swallowed[key] = true;
                (state.shared.callback)(InlineEvent::Compatibility { session, reason: "Independent search requested; the composer's typed query was kept".into() });
                return true;
            }
            if composition == IME_CLEAR && (key == VK_UP as usize || key == VK_DOWN as usize) && !state.shift() {
                state.swallowed[key] = true;
                (state.shared.callback)(InlineEvent::Navigate { session, delta: if key == VK_UP as usize { -1 } else { 1 } });
                return true;
            }
            if composition == IME_CLEAR && key == VK_TAB as usize {
                state.swallowed[key] = true;
                (state.shared.callback)(InlineEvent::SwitchSpace { session, delta: if state.shift() { -1 } else { 1 } });
                return true;
            }
        }
        // Notification only; not a key logger. Actual text is read from the pinned
        // provider after delivery. No virtual-key-to-character conversion exists.
        if down || (up && [VK_SHIFT, VK_LSHIFT, VK_RSHIFT, VK_CONTROL, VK_MENU].contains(&(key as u16))) {
            state.shared.record("input-invalidation", if down { 1 } else { 0 });
            if state.shared.may_compose.load(Ordering::Acquire) && down && !state.mods() {
                state.shared.ime.store(IME_UNKNOWN, Ordering::Release);
                let _=state.shared.ime_ui.compare_exchange(IME_CLEAR,IME_UNKNOWN,Ordering::AcqRel,Ordering::Acquire);
            }
            state.shared.input_changed(&state.sender, session);
        }
        false
    });
    if handled {
        1
    } else {
        CallNextHookEx(null_mut(), code, w, l)
    }
}
unsafe extern "system" fn win_event(
    _: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    #[cfg(feature = "native-test")]
    if super::diagnostics::PAUSE_WINDOW_EVENTS.load(Ordering::Acquire) {
        return;
    }
    LOCAL.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return;
        };
        let Some(state) = slot.as_mut() else {
            return;
        };
        let id = state.shared.active.load(Ordering::Acquire);
        if id == 0 || state.retiring {
            return;
        }
        state.shared.record("win-event", event);
        let legacy_show = event == EVENT_OBJECT_SHOW
            && object == OBJID_WINDOW
            && super::ime_window::candidate(
                hwnd,
                state.shared.focus.load(Ordering::Acquire) as HWND,
            );
        let legacy_hide = event == EVENT_OBJECT_HIDE && hwnd as isize == state.ime_window;
        // Documented IME lifecycle events also cover native editors whose
        // accessibility provider does not implement TextEditPattern. Observe
        // only during this session while its original input remains foreground.
        if legacy_show
            || legacy_hide
            || matches!(
                event,
                EVENT_OBJECT_IME_SHOW | EVENT_OBJECT_IME_HIDE | EVENT_OBJECT_IME_CHANGE
            )
        {
            if !state.target_current() || hwnd.is_null() {
                return;
            }
            if !legacy_hide && event != EVENT_OBJECT_IME_HIDE && IsWindowVisible(hwnd) != 0 {
                state.ime_window = hwnd as isize;
                state.shared.ime_ui.store(IME_ACTIVE, Ordering::Release);
            } else if (legacy_hide || event == EVENT_OBJECT_IME_HIDE)
                && state.ime_window == hwnd as isize
            {
                state.ime_window = 0;
                state.shared.ime_ui.store(IME_UNKNOWN, Ordering::Release);
                state.shared.ime.store(IME_UNKNOWN, Ordering::Release);
            } else {
                return;
            }
            state.shared.dirty(&state.sender, id);
            return;
        }
        if event == EVENT_SYSTEM_FOREGROUND || event == EVENT_OBJECT_FOCUS {
            let target_match = state.target_match();
            if target_match != Some(true) {
                // Windows may temporarily report no foreground window during
                // activation transitions. That is unknown identity, not proof
                // that another application took focus. Keep the lease while
                // the MTA retries; no insertion can pass current() meanwhile.
                if target_match.is_none() {
                    state.shared.record("foreground-unavailable", event);
                    let _ = state.sender.try_send(Request::Observe(id));
                    return;
                }
                let mut foreground_pid = 0;
                GetWindowThreadProcessId(GetForegroundWindow(), &mut foreground_pid);
                state.shared.record("foreground-lost", foreground_pid);
                state.shared.cancel(
                    id,
                    if GetForegroundWindow() as isize != state.shared.window.load(Ordering::Acquire)
                    {
                        "Foreground application changed; inline completion cancelled"
                    } else {
                        "Native input focus changed; inline completion cancelled"
                    },
                );
                return;
            }
        }
        if event == EVENT_OBJECT_FOCUS {
            // Accessibility providers may announce the same virtual editor
            // again during a TextPattern read. A focus notification is not a
            // text mutation. The MTA revalidates the original editor identity
            // and range; preflight and paste independently repeat that check.
            // Invalidating input_serial here makes those reads reject their
            // own confirmation ticket even when the input did not change.
            let _ = state.sender.try_send(Request::Observe(id));
            return;
        }
        let related = hwnd as isize == state.shared.focus.load(Ordering::Acquire)
            || hwnd as isize == state.shared.window.load(Ordering::Acquire);
        if related && event == EVENT_OBJECT_VALUECHANGE {
            state.shared.dirty(&state.sender, id);
        } else if related
            && event == EVENT_OBJECT_LOCATIONCHANGE
            && (object == OBJID_CARET || object == OBJID_WINDOW)
        {
            // Moving/blinking the caret is not text input. Re-read geometry
            // without invalidating an activation snapshot or confirmation token.
            let _ = state.sender.try_send(Request::Observe(id));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install_test_lease(callback: Arc<dyn Fn(InlineEvent) + Send + Sync>) -> Arc<Shared> {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let shared = Shared::new(callback);
        shared.active.store(1, Ordering::Release);
        shared.requested.store(1, Ordering::Release);
        shared
            .window
            .store(unsafe { GetForegroundWindow() } as isize, Ordering::Release);
        LOCAL.with(|slot| {
            *slot.borrow_mut() = Some(Local {
                shared: shared.clone(),
                sender,
                hook: null_mut(),
                events: Vec::new(),
                down: [false; 256],
                swallowed: [false; 256],
                ime_window: 0,
                session: 1,
                retiring: false,
            })
        });
        GUARD.with(|g| {
            g.set(GuardLease {
                session: 1,
                window: shared.window.load(Ordering::Acquire),
                ..GuardLease::default()
            })
        });
        shared
    }

    fn reset_test_lease() {
        LOCAL.with(|slot| {
            slot.borrow_mut().take();
        });
        GUARD.with(|g| g.set(GuardLease::default()));
    }

    #[test]
    fn full_delivery_queue_and_reentry_keep_every_enter_sequence_owned() {
        let (delivery, _receiver) = mpsc::sync_channel(1);
        delivery.send(()).unwrap();
        let rejected = Arc::new(AtomicU32::new(0));
        let count = rejected.clone();
        let shared = install_test_lease(Arc::new(move |_| {
            assert!(delivery.try_send(()).is_err());
            count.fetch_add(1, Ordering::Relaxed);
        }));
        let mut elapsed = Vec::new();
        for i in 0..1000 {
            shared.selectable.store(true, Ordering::Release);
            shared.range_valid.store(true, Ordering::Release);
            shared.ime.store(IME_CLEAR, Ordering::Release);
            *shared.composition_evidence.lock().unwrap() = Some(composition::CompositionEvidence {
                state: IME_CLEAR,
                source: composition::CompositionSource::TargetRead,
                session: 1,
                input_serial: 0,
                observed_at: Instant::now(),
            });
            let event = KBDLLHOOKSTRUCT {
                vkCode: VK_RETURN as u32,
                scanCode: 0x1c,
                flags: if i % 2 == 0 { LLKHF_EXTENDED } else { 0 },
                ..unsafe { std::mem::zeroed() }
            };
            let (down, up) = if i % 3 == 0 {
                (WM_SYSKEYDOWN, WM_SYSKEYUP)
            } else {
                (WM_KEYDOWN, WM_KEYUP)
            };
            let started = Instant::now();
            for message in [down, down, up] {
                let call = || unsafe {
                    keyboard(
                        HC_ACTION as i32,
                        message as usize,
                        &event as *const _ as isize,
                    )
                };
                let result = if i % 2 == 0 {
                    LOCAL.with(|slot| {
                        let _borrow = slot.borrow_mut();
                        call()
                    })
                } else {
                    call()
                };
                assert_eq!(result, 1);
            }
            elapsed.push(started.elapsed().as_micros());
            assert!(!GUARD.with(|g| g.get().consumed_enter));
        }
        assert_eq!(rejected.load(Ordering::Relaxed), 500);
        elapsed.sort_unstable();
        eprintln!(
            "1000 Enter sequences, queue full/reentrant: p50={}us p95={}us p99={}us max={}us",
            elapsed[499], elapsed[949], elapsed[989], elapsed[999]
        );
        assert!(
            elapsed[989] < 50_000,
            "Hook callback p99 exceeded the 50ms safety budget"
        );
        reset_test_lease();
    }

    #[test]
    fn unrelated_and_reordered_candidate_events_never_authorize_enter() {
        let shared = install_test_lease(Arc::new(|_| {}));
        for i in 0..100 {
            let hwnd = unsafe { GetForegroundWindow() };
            for event in [
                EVENT_OBJECT_IME_HIDE,
                EVENT_OBJECT_IME_SHOW,
                EVENT_OBJECT_IME_CHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_IME_SHOW,
                EVENT_OBJECT_IME_HIDE,
            ] {
                *shared.composition_evidence.lock().unwrap() =
                    Some(composition::CompositionEvidence {
                        state: IME_ACTIVE,
                        source: composition::CompositionSource::TextEditEvent,
                        session: if i % 2 == 0 { 2 } else { 1 },
                        input_serial: shared
                            .input_serial
                            .load(Ordering::Acquire)
                            .saturating_sub(1),
                        observed_at: Instant::now() - Duration::from_millis(251),
                    });
                unsafe {
                    win_event(null_mut(), event, hwnd, OBJID_WINDOW, 0, 0, 0);
                }
                assert_eq!(shared.verified_composition(), IME_UNKNOWN);
                let input = KBDLLHOOKSTRUCT {
                    vkCode: VK_RETURN as u32,
                    ..unsafe { std::mem::zeroed() }
                };
                for message in [WM_KEYDOWN, WM_KEYUP] {
                    assert_eq!(
                        unsafe {
                            keyboard(
                                HC_ACTION as i32,
                                message as usize,
                                &input as *const _ as isize,
                            )
                        },
                        1
                    );
                }
            }
        }
        reset_test_lease();
    }

    #[test]
    fn reentrant_protected_enter_must_not_reach_the_host() {
        // Exercise the actual callback while its state is borrowed, as during
        // a synchronous Win32 notification from arm/retire.
        GUARD.with(|g| {
            g.set(GuardLease {
                session: 1,
                window: unsafe { GetForegroundWindow() } as isize,
                consumed_enter: false,
                ime_enter: false,
                enter_down: false,
            })
        });
        LOCAL.with(|slot| {
            let _borrow = slot.borrow_mut();
            let event = KBDLLHOOKSTRUCT {
                vkCode: VK_RETURN as u32,
                ..unsafe { std::mem::zeroed() }
            };
            assert_eq!(
                unsafe {
                    keyboard(
                        HC_ACTION as i32,
                        WM_KEYDOWN as usize,
                        &event as *const _ as isize,
                    )
                },
                1
            );
        });
        GUARD.with(|g| g.set(GuardLease::default()));
    }

    #[test]
    fn cancellation_keeps_the_lease_until_retirement_for_all_enter_messages() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        LOCAL.with(|slot| {
            *slot.borrow_mut() = Some(Local {
                shared: Shared::new(Arc::new(|_| {})),
                sender,
                hook: null_mut(),
                events: Vec::new(),
                down: [false; 256],
                swallowed: [false; 256],
                ime_window: 0,
                session: 1,
                retiring: false,
            })
        });
        for iteration in 0..1000 {
            GUARD.with(|g| {
                g.set(GuardLease {
                    session: 1,
                    window: unsafe { GetForegroundWindow() } as isize,
                    consumed_enter: false,
                    ime_enter: false,
                    enter_down: false,
                })
            });
            let event = KBDLLHOOKSTRUCT {
                vkCode: VK_RETURN as u32,
                scanCode: 0x1c,
                flags: if iteration % 2 == 0 {
                    LLKHF_EXTENDED
                } else {
                    0
                },
                ..unsafe { std::mem::zeroed() }
            };
            let (down, up) = if iteration % 3 == 0 {
                (WM_SYSKEYDOWN, WM_SYSKEYUP)
            } else {
                (WM_KEYDOWN, WM_KEYUP)
            };
            for message in [down, down, up] {
                assert_eq!(
                    unsafe {
                        keyboard(
                            HC_ACTION as i32,
                            message as usize,
                            &event as *const _ as isize,
                        )
                    },
                    1
                );
            }
            assert!(!GUARD.with(|g| g.get().consumed_enter));
        }
        LOCAL.with(|slot| {
            slot.borrow_mut().take();
        });
        GUARD.with(|g| g.set(GuardLease::default()));
    }
}
