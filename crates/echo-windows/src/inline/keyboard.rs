//! The hook thread never reads text, calls UIA/COM, searches, or waits for a reply.
//! It only verifies the current HWND, consumes session control keys, and enqueues.
use super::*;
use std::{cell::RefCell, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{Accessibility::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
const WAKE: u32 = WM_APP + 61;
thread_local! { static LOCAL: RefCell<Option<Local>> = const { RefCell::new(None) }; }
enum Command {
    Arm(u64, SyncSender<Result<(), String>>),
    Disarm(u64),
    Stop,
}
struct Owner {
    sender: SyncSender<Command>,
    thread: u32,
    stopped: Arc<AtomicBool>,
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
        let worker = std::thread::Builder::new()
            .name("echo-inline-input-hook".into())
            .spawn(move || unsafe {
                let mut message: MSG = std::mem::zeroed();
                PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
                let tid = GetCurrentThreadId();
                LOCAL.with(|slot| {
                    *slot.borrow_mut() = Some(Local {
                        shared,
                        sender,
                        hook: null_mut(),
                        events: Vec::new(),
                        down: [false; 256],
                        swallowed: [false; 256],
                        ime_enter: false,
                        ime_window: 0,
                        session: 0,
                        retiring: false,
                    })
                });
                let _ = ready.send(tid);
                while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    if message.message == WAKE {
                        let mut stop = false;
                        while let Ok(command) = receiver.try_recv() {
                            LOCAL.with(|slot| {
                                let mut slot = slot.borrow_mut();
                                let state = slot.as_mut().unwrap();
                                match command {
                                    Command::Arm(id, reply) => {
                                        let _ = reply.send(state.arm(id));
                                    }
                                    Command::Disarm(id) => {
                                        if state.session == id {
                                            state.retire();
                                        }
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
                                    && !state.ime_enter
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
                LOCAL.with(|slot| {
                    if let Some(mut state) = slot.borrow_mut().take() {
                        state.clear();
                    }
                });
            })
            .map_err(|e| e.to_string())?;
        let thread = started
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Inline hook thread did not start")?;
        Ok(Self {
            owner: Arc::new(Owner {
                sender: commands,
                thread,
                stopped,
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
        let _ = self.send(Command::Disarm(session));
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
    ime_enter: bool,
    ime_window: isize,
    session: u64,
    retiring: bool,
}
impl Local {
    unsafe fn arm(&mut self, id: u64) -> Result<(), String> {
        if self.shared.requested.load(Ordering::Acquire) != id {
            return Err("Inline activation was cancelled".into());
        }
        self.clear_events();
        self.session = id;
        self.ime_window = 0;
        self.shared.ime_ui.store(IME_UNKNOWN, Ordering::Release);
        self.retiring = false;
        for (i, value) in self.down.iter_mut().enumerate() {
            *value = GetAsyncKeyState(i as i32) < 0;
        }
        if self.hook.is_null() {
            self.hook = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard),
                GetModuleHandleW(std::ptr::null()),
                0,
            );
            if self.hook.is_null() {
                return Err(format!(
                    "Windows refused the inline keyboard hook ({})",
                    GetLastError()
                ));
            }
        }
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
                self.clear();
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
            self.shared.ime.store(IME_ACTIVE, Ordering::Release);
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
    }
    unsafe fn retire(&mut self) {
        self.clear_events();
        self.retiring = true;
        if !self.swallowed.iter().any(|v| *v) && !self.ime_enter {
            self.clear();
        }
    }
    unsafe fn target_current(&self) -> bool {
        let root = self.shared.window.load(Ordering::Acquire) as HWND;
        if root.is_null() || GetForegroundWindow() != root {
            return false;
        }
        // Chromium/WPF virtual text elements may retain their UIA identity while
        // their native focus proxy HWND changes. The MTA validates that actual
        // element; a low-level hook must not confuse the proxy with the input.
        if !self.shared.native_identity.load(Ordering::Acquire) {
            return true;
        }
        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        GetGUIThreadInfo(self.shared.thread.load(Ordering::Acquire), &mut info) != 0
            && info.hwndFocus as isize == self.shared.focus.load(Ordering::Acquire)
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
    let handled = LOCAL.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else { return false; };
        let Some(state) = slot.as_mut() else { return false; };
        let key = input.vkCode as usize; let repeat = state.down[key]; state.down[key] = down;
        if state.swallowed[key] {
            if up { state.swallowed[key] = false; PostThreadMessageW(GetCurrentThreadId(), WAKE, 0, 0); }
            return true;
        }
        if key == VK_RETURN as usize && state.ime_enter {
            if up { state.ime_enter = false; PostThreadMessageW(GetCurrentThreadId(), WAKE, 0, 0); return false; }
            // The first Enter went to the IME; repeats must not send the composer.
            return true;
        }
        let session = state.shared.active.load(Ordering::Acquire);
        if state.retiring || session == 0 || session != state.session { return false; }
        if !state.target_current() {
            state.shared.cancel(session, "Input focus changed; inline completion cancelled");
            state.retire(); return false;
        }
        let composition = state.shared.ime.load(Ordering::Acquire);
        if down && key == VK_RETURN as usize {
            if composition == IME_ACTIVE { state.ime_enter = true; state.shared.input_changed(&state.sender, session); return false; }
            state.swallowed[key] = true;
            if !repeat && !state.mods() && !state.shift() && state.shared.can_confirm() {
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
                state.swallowed[key] = true; state.shared.cancel(session, "Inline completion cancelled; typed query kept"); state.retire(); return true;
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
            if state.shared.may_compose.load(Ordering::Acquire) && down && !state.mods() {
                state.shared.ime.store(if state.shared.ime_ui.load(Ordering::Acquire)==IME_ACTIVE {IME_ACTIVE} else {IME_UNKNOWN}, Ordering::Release);
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
                state.shared.ime.store(IME_ACTIVE, Ordering::Release);
            } else if (legacy_hide || event == EVENT_OBJECT_IME_HIDE)
                && state.ime_window == hwnd as isize
            {
                state.ime_window = 0;
                state.shared.ime_ui.store(IME_CLEAR, Ordering::Release);
                state.shared.ime.store(IME_UNKNOWN, Ordering::Release);
            } else {
                return;
            }
            state.shared.dirty(&state.sender, id);
            return;
        }
        if event == EVENT_SYSTEM_FOREGROUND || event == EVENT_OBJECT_FOCUS {
            if !state.target_current() {
                state.shared.cancel(
                    id,
                    if GetForegroundWindow() as isize != state.shared.window.load(Ordering::Acquire)
                    {
                        "Foreground application changed; inline completion cancelled"
                    } else {
                        "Native input focus changed; inline completion cancelled"
                    },
                );
                state.retire();
                return;
            }
        }
        if event == EVENT_OBJECT_FOCUS {
            state.shared.dirty(&state.sender, id);
        }
        let related = hwnd as isize == state.shared.focus.load(Ordering::Acquire)
            || hwnd as isize == state.shared.window.load(Ordering::Acquire);
        if related
            && (event != EVENT_OBJECT_LOCATIONCHANGE
                || object == OBJID_CARET
                || object == OBJID_WINDOW)
        {
            state.shared.dirty(&state.sender, id);
        }
    });
}
