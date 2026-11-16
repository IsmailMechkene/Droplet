//! Droplet: tracks AI coding usage limits and says which tool to use next.
//!
//! One Rust process owns all state, files and network access. The notch is the
//! only always-alive webview; the settings window exists only while open.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clock;
mod commands;
mod config;
mod forecast;
mod i18n;
mod net;
mod notch;
mod providers;
mod scheduler;
mod settings;
mod state;
mod suggest;
mod win;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use tauri::RunEvent;

use state::Core;

/// %APPDATA%\Droplet — config, history and the network log live here.
fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Droplet")
}

fn main() {
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let config = config::load(&dir);
    let core = Arc::new(Core {
        i18n: RwLock::new(i18n::I18n::new(config.general.language, &win::system_locale())),
        config: RwLock::new(config),
        dir,
        providers: Mutex::new(HashMap::new()),
        net: net::Net::new(),
        scheduler: Mutex::new(None),
        pick: Mutex::new(None),
    });

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {}))
        .manage(core.clone())
        .manage(notch::Notch(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::get_snapshot,
            commands::set_setting,
            commands::refresh,
            commands::card_opened,
            commands::notch_shape,
            commands::quit,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            notch::create(&handle, &core.config())?;
            scheduler::start(handle);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to start Droplet");

    // Droplet keeps running with no window open (tray only, or notch hidden).
    app.run(|_app, event| {
        if let RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
