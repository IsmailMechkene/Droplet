//! The settings schema. Every field has a default, so a partial file still loads.
//! Enums are checked by serde; numeric ranges are checked in `validate.rs`.

use serde::{Deserialize, Serialize};

use crate::providers::ProviderId;

pub const CURRENT_VERSION: u32 = 1;

macro_rules! string_enum {
    ($name:ident { $($variant:ident),+ $(,)? } default $default:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
        impl Default for $name {
            fn default() -> Self { $name::$default }
        }
    };
}

string_enum!(AppIcon { Tray, Hidden } default Tray);
string_enum!(Language { System, En, Fr } default System);
string_enum!(TrayIcons { PerProvider, Closest, None } default Closest);
string_enum!(Theme { System, Light, Dark } default System);
string_enum!(Size { S, M, L } default M);
string_enum!(WeeklyRing { Off, Inside, Outside } default Off);
string_enum!(ColorMode { Hard, Smooth } default Hard);
string_enum!(Glyphs { Mono, Color } default Mono);
string_enum!(Motion { System, Full, Reduced } default System);
string_enum!(SpringFeel { Calm, Default, Playful } default Default);
string_enum!(Show { Always, Hover, Hidden } default Hover);
string_enum!(Edge { Left, Right, Top, Bottom } default Right);
string_enum!(Screen { Primary, FollowMouse, Monitor } default Primary);
string_enum!(ShowAs { Peek, Toast, Both } default Peek);

impl Edge {
    pub fn is_vertical(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub general: General,
    pub providers: Providers,
    pub appearance: Appearance,
    pub notch: Notch,
    pub alerts: Alerts,
    pub forecast: Forecast,
    pub history: History,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: CURRENT_VERSION,
            general: General::default(),
            providers: Providers::default(),
            appearance: Appearance::default(),
            notch: Notch::default(),
            alerts: Alerts::default(),
            forecast: Forecast::default(),
            history: History::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    pub start_with_windows: bool,
    pub app_icon: AppIcon,
    pub language: Language,
    /// 3, 5, 10 or 15.
    pub refresh_minutes: u32,
    /// Like "Win+Shift+U". Empty means no shortcut.
    pub shortcut: String,
    pub tray_icons: TrayIcons,
}

impl Default for General {
    fn default() -> Self {
        General {
            start_with_windows: true,
            app_icon: AppIcon::Tray,
            language: Language::System,
            refresh_minutes: 5,
            shortcut: "Win+Shift+U".into(),
            tray_icons: TrayIcons::Closest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderCfg {
    pub enabled: bool,
    /// Include in "use this one now" suggestions.
    pub suggest: bool,
    /// Window id of the limit shown on the ring (see each provider module).
    pub main_limit: String,
    /// Antigravity only: explicit path to agy.exe. Empty means auto-detect.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub agy_path: String,
}

impl Default for ProviderCfg {
    fn default() -> Self {
        ProviderCfg { enabled: true, suggest: true, main_limit: String::new(), agy_path: String::new() }
    }
}

fn provider(main_limit: &str) -> ProviderCfg {
    ProviderCfg { main_limit: main_limit.into(), ..ProviderCfg::default() }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Providers {
    /// Shared by the notch, jiggle mode and the Accounts page.
    pub order: Vec<ProviderId>,
    pub claude: ProviderCfg,
    pub codex: ProviderCfg,
    pub cursor: ProviderCfg,
    pub opencode: ProviderCfg,
    pub kimi: ProviderCfg,
    pub antigravity: ProviderCfg,
}

impl Default for Providers {
    fn default() -> Self {
        Providers {
            order: ProviderId::ALL.to_vec(),
            claude: provider("session"),
            codex: provider("primary"),
            cursor: provider("total"),
            opencode: provider("rolling"),
            kimi: provider("5h"),
            // Off by default: reading it starts agy.exe (~200 MB for ~12 s).
            antigravity: ProviderCfg { enabled: false, suggest: false, ..provider("first") },
        }
    }
}

impl Providers {
    pub fn get(&self, id: ProviderId) -> &ProviderCfg {
        match id {
            ProviderId::Claude => &self.claude,
            ProviderId::Codex => &self.codex,
            ProviderId::Cursor => &self.cursor,
            ProviderId::Opencode => &self.opencode,
            ProviderId::Kimi => &self.kimi,
            ProviderId::Antigravity => &self.antigravity,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Appearance {
    pub theme: Theme,
    pub size: Size,
    pub show_remaining: bool,
    pub weekly_ring: WeeklyRing,
    pub pace_tick: bool,
    pub color_mode: ColorMode,
    /// 40..=95
    pub warn_at: u32,
    /// 50..=99 and greater than `warn_at`.
    pub crit_at: u32,
    pub glyphs: Glyphs,
    pub motion: Motion,
    pub spring_feel: SpringFeel,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            theme: Theme::System,
            size: Size::M,
            show_remaining: false,
            weekly_ring: WeeklyRing::Off,
            pace_tick: true,
            color_mode: ColorMode::Hard,
            warn_at: 70,
            crit_at: 90,
            glyphs: Glyphs::Mono,
            motion: Motion::System,
            spring_feel: SpringFeel::Default,
        }
    }
}

/// Where the notch sits along each edge, 0..=1. Each edge remembers its own spot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Positions {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

impl Default for Positions {
    fn default() -> Self {
        Positions { left: 0.5, right: 0.5, top: 0.5, bottom: 0.5 }
    }
}

impl Positions {
    pub fn get(&self, edge: Edge) -> f64 {
        match edge {
            Edge::Left => self.left,
            Edge::Right => self.right,
            Edge::Top => self.top,
            Edge::Bottom => self.bottom,
        }
    }

    pub fn set(&mut self, edge: Edge, value: f64) {
        let slot = match edge {
            Edge::Left => &mut self.left,
            Edge::Right => &mut self.right,
            Edge::Top => &mut self.top,
            Edge::Bottom => &mut self.bottom,
        };
        *slot = value.clamp(0.0, 1.0);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Notch {
    pub show: Show,
    pub edge: Edge,
    pub screen: Screen,
    /// Stable monitor device path, used when `screen` is `monitor` and remembered after a drag.
    pub monitor_id: String,
    pub position: Positions,
    pub locked: bool,
    pub hide_fullscreen: bool,
    /// 0 (never), 1, 5 or 15 minutes.
    pub collapse_after_min: u32,
    /// 0..=600
    pub hover_delay_ms: u32,
    pub high_contrast: bool,
}

impl Default for Notch {
    fn default() -> Self {
        Notch {
            show: Show::Hover,
            edge: Edge::Right,
            screen: Screen::Primary,
            monitor_id: String::new(),
            position: Positions::default(),
            locked: false,
            hide_fullscreen: true,
            collapse_after_min: 0,
            hover_delay_ms: 250,
            high_contrast: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QuietHours {
    pub enabled: bool,
    /// "HH:MM", local time.
    pub from: String,
    pub to: String,
}

impl Default for QuietHours {
    fn default() -> Self {
        QuietHours { enabled: false, from: "22:00".into(), to: "08:00".into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Alerts {
    pub warning: bool,
    pub critical: bool,
    pub run_out: bool,
    /// 15, 30 or 60: only warn when the limit would run out at least this long before its reset.
    pub run_out_lead_min: u32,
    pub reset: bool,
    pub reset_soon: bool,
    pub show_as: ShowAs,
    pub sound: bool,
    pub quiet: QuietHours,
}

impl Default for Alerts {
    fn default() -> Self {
        Alerts {
            warning: true,
            critical: true,
            run_out: true,
            run_out_lead_min: 30,
            reset: true,
            reset_soon: false,
            show_as: ShowAs::Peek,
            sound: false,
            quiet: QuietHours::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Forecast {
    pub show: bool,
    /// 30, 60 or 120.
    pub pace_window_min: u32,
    pub suggest: bool,
    /// 5..=40 percentage points.
    pub hysteresis_pts: u32,
}

impl Default for Forecast {
    fn default() -> Self {
        Forecast { show: true, pace_window_min: 60, suggest: true, hysteresis_pts: 15 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct History {
    /// 0 (off), 7, 30 or 90.
    pub retention_days: u32,
    pub weekly_summary: bool,
}

impl Default for History {
    fn default() -> Self {
        History { retention_days: 30, weekly_summary: false }
    }
}
