use super::{
    common::{error, wide},
    EventHandler, ShellEvent,
};
use std::{
    ptr::{null, null_mut},
    sync::mpsc::SyncSender,
};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::*, WindowsAndMessaging::*},
};
const TRAY_MESSAGE: u32 = WM_APP + 17;
struct Host {
    handler: EventHandler,
    icon: HICON,
    owned_icon: bool,
    taskbar: u32,
}
unsafe fn notify(hwnd: HWND, host: &Host, operation: u32) {
    let mut data: NOTIFYICONDATAW = std::mem::zeroed();
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = TRAY_MESSAGE;
    data.hIcon = host.icon;
    for (out, ch) in data.szTip.iter_mut().zip(wide("Echo — clipboard history")) {
        *out = ch;
    }
    Shell_NotifyIconW(operation, &data);
    if operation == NIM_ADD {
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        Shell_NotifyIconW(NIM_SETVERSION, &data);
    }
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Host;
    if !state.is_null() {
        let host = &*state;
        if msg == host.taskbar {
            notify(hwnd, host, NIM_ADD);
            return 0;
        }
        match msg {
            TRAY_MESSAGE => {
                let event = (l as u32) & 0xffff;
                if event == WM_LBUTTONDBLCLK || event == NIN_SELECT || event == (NIN_SELECT | 1) {
                    (host.handler)(ShellEvent::Open);
                }
                if event == WM_CONTEXTMENU || event == WM_RBUTTONUP {
                    let menu = CreatePopupMenu();
                    if menu.is_null() {
                        return 0;
                    }
                    for (id, label) in [
                        (1, "Open Echo"),
                        (2, "Favorites"),
                        (3, "Settings"),
                        (4, "Quit Echo"),
                    ] {
                        AppendMenuW(menu, MF_STRING, id, wide(label).as_ptr());
                    }
                    let mut point: POINT = std::mem::zeroed();
                    GetCursorPos(&mut point);
                    SetForegroundWindow(hwnd);
                    let id = TrackPopupMenu(
                        menu,
                        TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                        point.x,
                        point.y,
                        0,
                        hwnd,
                        null(),
                    ) as u32;
                    DestroyMenu(menu);
                    PostMessageW(hwnd, WM_NULL, 0, 0);
                    match id {
                        1 => (host.handler)(ShellEvent::Open),
                        2 => (host.handler)(ShellEvent::Favorites),
                        3 => (host.handler)(ShellEvent::Settings),
                        4 => (host.handler)(ShellEvent::Quit),
                        _ => {}
                    }
                }
                return 0;
            }
            WM_SETTINGCHANGE | WM_THEMECHANGED => {
                (host.handler)(ShellEvent::ThemeChanged);
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                return 0;
            }
            WM_DESTROY => {
                notify(hwnd, host, NIM_DELETE);
                PostQuitMessage(0);
                return 0;
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, msg, w, l)
}
pub(super) fn run(handler: EventHandler, ready: SyncSender<Result<isize, String>>) {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = wide("EchoNativeTrayHost");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            let _ = ready.send(Err(error()));
            return;
        }
        let mut icon = LoadImageW(
            instance,
            1_usize as *const u16,
            IMAGE_ICON,
            32,
            32,
            LR_DEFAULTCOLOR,
        ) as HICON;
        let owned_icon = !icon.is_null();
        if icon.is_null() {
            icon = LoadIconW(null_mut(), IDI_APPLICATION);
        }
        let mut host = Box::new(Host {
            handler,
            icon,
            owned_icon,
            taskbar: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
        });
        // A hidden top-level host, not HWND_MESSAGE: Explorer restart broadcasts must reach it.
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            wide("Echo native host").as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            (&mut *host as *mut Host).cast(),
        );
        if hwnd.is_null() {
            if host.owned_icon {
                DestroyIcon(host.icon);
            }
            let _ = ready.send(Err(error()));
            return;
        }
        notify(hwnd, &host, NIM_ADD);
        let _ = ready.send(Ok(hwnd as isize));
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        if IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
        if host.owned_icon {
            DestroyIcon(host.icon);
        }
        UnregisterClassW(class.as_ptr(), instance);
    }
}
