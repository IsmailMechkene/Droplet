//! Event-driven watchers, so nothing is polled:
//! - full-screen apps: an app-bar notification (ABN_FULLSCREENAPP) plus a
//!   foreground-window hook, used to hide the notch and bring it back,
//! - display changes: WM_DISPLAYCHANGE / work-area changes re-dock the notch.
//!
//! All of it hangs off one hidden window on the main thread.

use std::sync::{Arc, OnceLock};

use tauri::{AppHandle, Emitter, Manager};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_NEW, ABM_REMOVE, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClassNameW, GetForegroundWindow, GetWindowRect, RegisterClassW, EVENT_SYSTEM_FOREGROUND,
    WINDOW_EX_STYLE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP, WM_DESTROY, WM_DISPLAYCHANGE, WM_SETTINGCHANGE,
    WNDCLASSW, WS_POPUP,
};

use crate::config::Screen;
use crate::notch::{self, monitors, Notch};
use crate::state::Core;

static APP: OnceLock<AppHandle> = OnceLock::new();
const APPBAR_MESSAGE: u32 = WM_APP + 1;
const ABN_FULLSCREENAPP: usize = 2;
const SPI_SETWORKAREA: usize = 0x002F;

/// Creates the hidden window and registers the watchers. Call once, on the main thread.
pub fn start(app: &AppHandle) {
    if APP.set(app.clone()).is_err() {
        return;
    }
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let class = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: module.into(), lpszClassName: w!("DropletWatcher"), ..Default::default() };
        RegisterClassW(&class);
        // A real (never shown) top-level window: message-only windows do not receive display broadcasts.
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("DropletWatcher"), PCWSTR::null(), WS_POPUP, 0, 0, 0, 0, None, None, Some(module.into()), None) else {
            return;
        };
        let mut bar = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: hwnd, uCallbackMessage: APPBAR_MESSAGE, ..Default::default() };
        SHAppBarMessage(ABM_NEW, &mut bar);
        SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(foreground_changed), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        APPBAR_MESSAGE if wparam.0 == ABN_FULLSCREENAPP => evaluate(),
        WM_DISPLAYCHANGE => displays_changed(),
        WM_SETTINGCHANGE if wparam.0 == SPI_SETWORKAREA => displays_changed(),
        // Theme ("ImmersiveColorSet") or other system changes: the tray icons may need new ink.
        WM_SETTINGCHANGE => redraw_tray(),
        WM_DESTROY => {
            let mut bar = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: hwnd, ..Default::default() };
            SHAppBarMessage(ABM_REMOVE, &mut bar);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

unsafe extern "system" fn foreground_changed(_: HWINEVENTHOOK, _: u32, _: HWND, _: i32, _: i32, _: u32, _: u32) {
    evaluate();
}

/// True when the foreground window covers the whole monitor the notch is on
/// (a game, a video or a presentation), and is not the desktop itself.
fn fullscreen_app_in_front(notch_monitor: &monitors::Monitor) -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return false;
        }
        let mut class = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..len]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
            return false;
        }
        let mut window = RECT::default();
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetWindowRect(hwnd, &mut window).is_err() || !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() {
            return false;
        }
        let m = info.rcMonitor;
        let covers = window.left <= m.left && window.top <= m.top && window.right >= m.right && window.bottom >= m.bottom;
        let b = notch_monitor.bounds;
        covers && (m.left, m.top, m.right, m.bottom) == (b.left, b.top, b.right, b.bottom)
    }
}

/// Re-checks full-screen state and "follow mouse". Runs on foreground changes only.
fn evaluate() {
    let Some(app) = APP.get() else { return };
    let config = app.state::<Arc<Core>>().config();
    let monitor = match app.state::<Notch>().0.lock().unwrap().as_ref() {
        Some(state) if !state.dragging => state.monitor.clone(),
        _ => return,
    };
    if config.notch.screen == Screen::FollowMouse {
        let (x, y) = monitors::cursor();
        if monitors::at_point(x, y).is_some_and(|m| m.id != monitor.id) {
            notch::redock(app, &config);
            return;
        }
    }
    notch::set_suppressed(app, config.notch.hide_fullscreen && fullscreen_app_in_front(&monitor));
    notch::raise(app);
}

/// A monitor was added, removed or resized, or the taskbar moved: dock again
/// (falling back to the primary monitor if the saved one is gone).
fn displays_changed() {
    let Some(app) = APP.get() else { return };
    let config = app.state::<Arc<Core>>().config();
    notch::redock(app, &config);
    let _ = app.emit("monitors-changed", ());
    redraw_tray();
}

/// Taskbar theme or DPI changed: redraw the tray icons on the worker thread.
fn redraw_tray() {
    let Some(app) = APP.get() else { return };
    crate::tray::invalidate(app);
    app.state::<Arc<Core>>().send(crate::scheduler::Msg::ConfigChanged);
}

/// Called when settings change, so turning "hide for full-screen apps" off shows the notch again.
pub fn settings_changed() {
    evaluate();
}
