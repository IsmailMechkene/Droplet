//! The notch: the one always-alive webview, docked to a screen edge.
//!
//! Division of labour: the UI decides the window's *shape* (it knows the
//! content), Rust decides *where* it goes (monitor, edge, dock point) and does
//! all native window work. The window never takes focus when shown.

pub mod dock;
pub mod monitors;

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, GWL_STYLE, HWND_TOPMOST, SWP_NOACTIVATE,
    SWP_NOSENDCHANGING, SW_HIDE, SW_SHOWNOACTIVATE, WS_CAPTION, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX,
    WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
};

use crate::config::{Config, Edge, Screen, Show};
use dock::{Placement, Shape};
use monitors::Monitor;

pub const LABEL: &str = "notch";

pub struct NotchState {
    pub monitor: Monitor,
    pub edge: Edge,
    /// Dock point: the centre of the notch body along the edge, in physical screen pixels.
    pub point: f64,
    /// The last shape the UI asked for.
    pub shape: Option<Shape>,
    pub shown: bool,
    /// Hidden because a full-screen app is in front.
    pub suppressed: bool,
    pub dragging: bool,
}

pub struct Notch(pub Mutex<Option<NotchState>>);

/// The monitor the settings ask for, falling back to the primary one when a
/// saved monitor is no longer connected.
pub fn resolve_monitor(config: &Config) -> Option<Monitor> {
    let wanted = match config.notch.screen {
        Screen::Primary => None,
        Screen::Monitor => monitors::by_id(&config.notch.monitor_id),
        Screen::FollowMouse => {
            let (x, y) = monitors::cursor();
            monitors::at_point(x, y)
        }
    };
    wanted.or_else(monitors::primary).or_else(|| monitors::all().into_iter().next())
}

fn initial_state(config: &Config) -> Option<NotchState> {
    let monitor = resolve_monitor(config)?;
    let edge = config.notch.edge;
    let (start, len) = dock::along_range(monitor.work, edge);
    let point = dock::from_fraction(config.notch.position.get(edge), start, len);
    Some(NotchState { monitor, edge, point, shape: None, shown: false, suppressed: false, dragging: false })
}

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

fn hwnd(window: &WebviewWindow) -> Option<HWND> {
    window.hwnd().ok()
}

/// Creates the notch webview unless the setting is "Hidden" (then no webview exists at all).
pub fn create(app: &AppHandle, config: &Config) -> tauri::Result<()> {
    if config.notch.show == Show::Hidden || window(app).is_some() {
        return Ok(());
    }
    *app.state::<Notch>().0.lock().unwrap() = initial_state(config);

    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("notch/index.html".into()))
        .title("Droplet")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .visible(false)
        .inner_size(1.0, 1.0)
        .build()?;

    if let Some(hwnd) = hwnd(&window) {
        unsafe {
            // A bare popup: no caption or frame, so Windows applies no minimum size
            // and the collapsed tab can be a few pixels wide.
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            let strip = WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0;
            SetWindowLongPtrW(hwnd, GWL_STYLE, ((style & !strip) | WS_POPUP.0) as isize);
            // Never activate on click or show, and stay out of Alt+Tab.
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize);
        }
    }
    Ok(())
}

/// Destroys the notch webview, freeing its WebView2 processes.
pub fn destroy(app: &AppHandle) {
    if let Some(window) = window(app) {
        let _ = window.destroy();
    }
    *app.state::<Notch>().0.lock().unwrap() = None;
}

fn limits_for(state: &NotchState, shape: &Shape) -> (f64, f64) {
    let (start, len) = dock::along_range(state.monitor.work, state.edge);
    let scale = state.monitor.scale;
    dock::limits(start, len, shape.min_before * scale, shape.min_after * scale)
}

/// Moves and sizes the native window for the current state. Never activates it.
fn apply(window: &WebviewWindow, state: &mut NotchState) {
    let (Some(shape), Some(hwnd)) = (state.shape, hwnd(window)) else { return };
    if shape.edge != state.edge {
        return; // the UI has not laid out for the new edge yet
    }
    let (x, y, w, h) = dock::window_rect(state.monitor.work, state.edge, state.point, &shape, state.monitor.scale);
    unsafe {
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w.max(1), h.max(1), SWP_NOACTIVATE | SWP_NOSENDCHANGING);
        if !state.shown && !state.suppressed {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            state.shown = true;
        }
    }
}

pub fn placement_of(state: &NotchState) -> Placement {
    dock::placement(state.monitor.work, state.edge, state.point, state.monitor.scale)
}

/// The UI asks for a window shape. Returns where the notch is, for layout.
pub fn set_shape(app: &AppHandle, shape: Shape) -> Option<Placement> {
    let window = window(app)?;
    let notch = app.state::<Notch>();
    let mut guard = notch.0.lock().unwrap();
    let state = guard.as_mut()?;
    if shape.edge == state.edge {
        state.shape = Some(shape);
        if !state.dragging {
            let (lo, hi) = limits_for(state, &shape);
            state.point = dock::clamp(state.point, lo, hi);
        }
        apply(&window, state);
    }
    Some(placement_of(state))
}

pub fn placement(app: &AppHandle) -> Option<Placement> {
    app.state::<Notch>().0.lock().unwrap().as_ref().map(placement_of)
}

/// Re-reads monitor, edge and position from the settings (after a settings
/// change or a display change) and tells the UI to lay out again.
pub fn redock(app: &AppHandle, config: &Config) {
    let notch = app.state::<Notch>();
    let mut guard = notch.0.lock().unwrap();
    let Some(state) = guard.as_mut() else { return };
    let Some(fresh) = initial_state(config) else { return };
    state.monitor = fresh.monitor;
    state.edge = fresh.edge;
    state.point = fresh.point;
    let place = placement_of(state);
    if let Some(window) = window(app) {
        apply(&window, state);
    }
    drop(guard);
    let _ = app.emit_to(LABEL, "notch-dock", place);
}

/// Hides or re-shows the notch (full-screen apps), without touching focus.
pub fn set_suppressed(app: &AppHandle, suppressed: bool) {
    let notch = app.state::<Notch>();
    let mut guard = notch.0.lock().unwrap();
    let (Some(state), Some(window)) = (guard.as_mut(), window(app)) else { return };
    if state.suppressed == suppressed {
        return;
    }
    state.suppressed = suppressed;
    if let Some(hwnd) = hwnd(&window) {
        unsafe {
            if suppressed {
                let _ = ShowWindow(hwnd, SW_HIDE);
                state.shown = false;
            } else if state.shape.is_some() {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                state.shown = true;
            }
        }
    }
}

/// Applies a settings change to the notch window.
pub fn on_config_changed(app: &AppHandle, old: &Config, new: &Config) {
    let hidden = new.notch.show == Show::Hidden;
    if hidden {
        destroy(app);
        return;
    }
    if window(app).is_none() {
        let _ = create(app, new);
        return;
    }
    let moved = old.notch.edge != new.notch.edge
        || old.notch.screen != new.notch.screen
        || old.notch.monitor_id != new.notch.monitor_id
        || old.notch.position != new.notch.position;
    if moved {
        redock(app, new);
    }
}
