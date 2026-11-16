//! The settings window: a second webview that exists only while it is open.
//! Closing it destroys the webview, which frees its WebView2 memory.

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "settings";
const SECTIONS: [&str; 8] = ["general", "accounts", "appearance", "notch", "alerts", "forecast", "privacy", "about"];

/// Opens the settings window (or focuses it), optionally on a given section.
pub fn open(app: &AppHandle, section: Option<&str>) {
    let section = section.filter(|s| SECTIONS.contains(s));
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.unminimize();
        let _ = window.set_focus();
        if let Some(section) = section {
            let _ = app.emit_to(LABEL, "settings-section", section);
        }
        return;
    }
    let start = format!("window.__DROPLET_SECTION__ = {:?};", section.unwrap_or("general"));
    let app = app.clone();
    // Building a webview blocks until the event loop answers, so it must not run
    // on the event-loop thread itself (menu clicks and shortcuts arrive there).
    std::thread::spawn(move || {
        // Hidden until the page has drawn its first frame ("ready"), so there is no white flash.
        let _ = WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::App("settings/index.html".into()))
            .title("Droplet Settings")
            .inner_size(780.0, 580.0)
            .min_inner_size(560.0, 420.0)
            .decorations(false)
            .shadow(true)
            .center()
            .visible(false)
            .initialization_script(&start)
            .build();
    });
}

pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn close(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.destroy();
    }
}

pub fn minimize(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.minimize();
    }
}

pub fn toggle_maximize(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = if window.is_maximized().unwrap_or(false) { window.unmaximize() } else { window.maximize() };
    }
}
