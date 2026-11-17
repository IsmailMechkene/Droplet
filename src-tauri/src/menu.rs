//! Native menus (notch right-click, tray) and the one handler for their items.

use std::sync::Arc;

use tauri::menu::{CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::scheduler::{Msg, Reason};
use crate::state::Core;
use crate::{notch, settings, settings_window};

/// Right-click menu of the notch:
/// Keep open · Refresh now · Rearrange · Lock position · Settings… · Quit.
pub fn notch_menu(app: &AppHandle, keep_open: bool, jiggle: bool) -> tauri::Result<Menu<Wry>> {
    let core = app.state::<Arc<Core>>();
    let config = core.config();
    let i18n = core.i18n.read().unwrap();
    let always = config.notch.show == crate::config::Show::Always;
    let keep = CheckMenuItemBuilder::with_id("keep_open", i18n.t("menu.keep_open")).checked(keep_open || always).enabled(!always).build(app)?;
    let rearrange = MenuItemBuilder::with_id("rearrange", i18n.t(if jiggle { "menu.done_rearranging" } else { "menu.rearrange" })).build(app)?;
    let lock = CheckMenuItemBuilder::with_id("lock", i18n.t("menu.lock")).checked(config.notch.locked).build(app)?;
    MenuBuilder::new(app)
        .item(&keep)
        .item(&MenuItemBuilder::with_id("refresh", i18n.t("menu.refresh")).build(app)?)
        .item(&rearrange)
        .item(&lock)
        .item(&MenuItemBuilder::with_id("settings", i18n.t("menu.settings")).build(app)?)
        .item(&PredefinedMenuItem::separator(app)?)
        .item(&MenuItemBuilder::with_id("quit", i18n.t("menu.quit")).build(app)?)
        .build()
}

/// Runs a menu item, whichever menu it came from.
pub fn handle(app: &AppHandle, id: &str) {
    let core = app.state::<Arc<Core>>();
    match id {
        "refresh" => core.send(Msg::RefreshAll(Reason::Manual)),
        "rearrange" | "keep_open" => {
            let _ = app.emit_to(notch::LABEL, "notch-command", id);
        }
        "lock" => {
            let locked = core.config().notch.locked;
            let _ = settings::change(app, "notch.locked", (!locked).into());
        }
        "settings" => settings_window::open(app, None),
        "netlog" => {
            let _ = crate::netlog::open(&core.dir, &core.config(), &core.net.log_snapshot());
        }
        "quit" => app.exit(0),
        _ => {}
    }
}
