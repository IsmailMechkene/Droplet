//! Shared app state, owned by Rust. The UIs only ever see `Snapshot`: finished,
//! display-ready data with no credentials and no file contents.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Mutex, RwLock};

use serde::Serialize;

use crate::config::Config;
use crate::forecast::{self, Outlook, Series};
use crate::i18n::I18n;
use crate::net::Net;
use crate::providers::{Limit, ProviderId, Usage};
use crate::scheduler::Msg;
use crate::suggest::{self, Candidate};

/// What the last check of a provider found.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// Not checked yet since startup.
    Unknown,
    Ok,
    NotInstalled,
    SignIn,
    NoSubscription,
    /// The last check failed. An earlier reading, if any, is shown as stale.
    Error(String),
}

pub struct ProviderState {
    pub status: Status,
    /// The last successful reading. Kept through failures and shown as stale with its age.
    pub usage: Option<Usage>,
    pub fetched_at: Option<i64>,
    pub checked_at: Option<i64>,
    pub loading: bool,
    /// Readings per window id for the current period.
    pub series: HashMap<String, Series>,
    /// 24 hourly points of the main limit, oldest first (from history).
    pub spark: Vec<f64>,
    /// Consecutive HTTP 429 answers, and when the next attempt is allowed.
    pub rate_limited: u32,
    pub retry_at: i64,
}

impl Default for ProviderState {
    fn default() -> Self {
        ProviderState {
            status: Status::Unknown,
            usage: None,
            fetched_at: None,
            checked_at: None,
            loading: false,
            series: HashMap::new(),
            spark: Vec::new(),
            rate_limited: 0,
            retry_at: 0,
        }
    }
}

impl ProviderState {
    /// A reading exists but the latest check failed.
    pub fn is_stale(&self) -> bool {
        self.usage.is_some() && matches!(self.status, Status::Error(_))
    }

    pub fn has_fresh_reading(&self) -> bool {
        self.usage.is_some() && self.status == Status::Ok
    }
}

pub struct Core {
    /// %APPDATA%\Droplet
    pub dir: PathBuf,
    pub config: RwLock<Config>,
    pub i18n: RwLock<I18n>,
    pub providers: Mutex<HashMap<ProviderId, ProviderState>>,
    pub net: Net,
    pub scheduler: Mutex<Option<Sender<Msg>>>,
    /// The current "use this one now" pick, kept for hysteresis.
    pub pick: Mutex<Option<ProviderId>>,
}

impl Core {
    pub fn config(&self) -> Config {
        self.config.read().unwrap().clone()
    }

    pub fn send(&self, msg: Msg) {
        if let Some(tx) = self.scheduler.lock().unwrap().as_ref() {
            let _ = tx.send(msg);
        }
    }
}

// ---------------------------------------------------------------- views

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RowView {
    pub id: String,
    pub name: String,
    pub reset: String,
    pub pct: f64,
    pub caption: String,
    /// Screen-reader label for the row.
    pub label: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ForecastView {
    /// learning | onpace | safe | warn | danger | stale
    pub kind: &'static str,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MessageView {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProviderView {
    pub id: &'static str,
    pub name: &'static str,
    /// ok | stale | loading | na  (drives the ring)
    pub ring: &'static str,
    /// ok | stale | signin | nosub | error | waiting  (drives the card)
    pub card: &'static str,
    /// Enabled in settings. Disabled providers are only shown in jiggle mode.
    pub enabled: bool,
    pub loading: bool,
    /// Main limit, 0..=100.
    pub pct: f64,
    pub weekly: Option<f64>,
    /// Even pace of the main limit, 0..=1.
    pub pace: Option<f64>,
    /// Increases when the main limit starts a new period (plays the reset sweep).
    pub period: u32,
    pub plan: Option<String>,
    pub title: String,
    pub rows: Vec<RowView>,
    pub forecast: Option<ForecastView>,
    pub message: Option<MessageView>,
    pub spark: Vec<f64>,
    pub footer: String,
    /// Screen-reader label for the ring.
    pub label: String,
    /// When the main limit is forecast to run out, as local "HH:MM".
    pub out_at: Option<String>,
    /// When the main limit resets (Unix seconds), for the tray tooltip.
    pub main_resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SuggestionView {
    pub from: &'static str,
    pub to: &'static str,
    /// "Claude 92% (out at 15:40) →"
    pub a: String,
    /// "use Codex (12%)"
    pub b: String,
    pub line: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct Snapshot {
    /// Installed providers in the user's order.
    pub providers: Vec<ProviderView>,
    pub suggestion: Option<SuggestionView>,
}

/// The limit shown on the ring: the configured one, else the first reported.
pub fn main_limit<'a>(usage: &'a Usage, wanted: &str) -> Option<&'a Limit> {
    usage.limits.iter().find(|l| l.id == wanted).or_else(|| usage.limits.first())
}

/// Forecast for one limit of a provider.
pub fn outlook_for(state: &ProviderState, limit: &Limit, now: i64, pace_window_secs: i64) -> Outlook {
    match state.series.get(&limit.id) {
        Some(series) => forecast::outlook(&series.samples, limit.resets_at, limit.window_secs, now, pace_window_secs),
        None => Outlook::Learning,
    }
}

fn forecast_view(i18n: &I18n, outlook: Outlook, limit: &Limit, now: i64, crit_at: f64) -> ForecastView {
    let level = if limit.used_pct >= crit_at { "danger" } else { "warn" };
    let reset_in = limit.resets_at.map(|r| i18n.duration(r - now));
    match outlook {
        Outlook::Learning => ForecastView { kind: "learning", text: i18n.t("fc.learning") },
        Outlook::OnPace => ForecastView { kind: "onpace", text: i18n.t("fc.on_pace") },
        Outlook::WontRunOut => ForecastView { kind: "safe", text: i18n.t("fc.safe") },
        Outlook::AheadOfPace { by_pct } => {
            ForecastView { kind: "warn", text: i18n.f("fc.ahead", &[("pct", &format!("{by_pct:.0}"))]) }
        }
        Outlook::Exhausted => ForecastView {
            kind: "danger",
            text: match &reset_in {
                Some(time) => i18n.f("fc.exhausted", &[("time", time)]),
                None => i18n.t("fc.exhausted_plain"),
            },
        },
        Outlook::RunsOut { at, before_reset_secs } => {
            // A clock time is only unambiguous within the next day.
            let soon = at - now < 20 * 3600;
            let (at_text, in_text) = (i18n.clock(at), i18n.duration(at - now));
            let text = match (before_reset_secs, soon) {
                (Some(before), true) => i18n.f("fc.out_at", &[("at", &at_text), ("time", &i18n.duration(before))]),
                (Some(before), false) => i18n.f("fc.out_in", &[("in", &in_text), ("time", &i18n.duration(before))]),
                (None, true) => i18n.f("fc.out_at_plain", &[("at", &at_text)]),
                (None, false) => i18n.f("fc.out_in_plain", &[("in", &in_text)]),
            };
            ForecastView { kind: level, text }
        }
    }
}

fn age_text(i18n: &I18n, key_ago: &str, key_now: &str, at: Option<i64>, now: i64, tool: &str) -> String {
    match at {
        Some(at) if now - at >= 45 => i18n.f(key_ago, &[("time", &i18n.duration(now - at)), ("tool", tool)]),
        Some(_) => i18n.f(key_now, &[("tool", tool)]),
        None => i18n.f("card.checking", &[("tool", tool)]),
    }
}

fn provider_view(id: ProviderId, state: &ProviderState, config: &Config, i18n: &I18n, now: i64) -> ProviderView {
    let cfg = config.providers.get(id);
    let (name, tool) = (id.name(), id.tool());
    let crit = config.appearance.crit_at as f64;
    let pace_window = config.forecast.pace_window_min as i64 * 60;

    let mut view = ProviderView {
        id: id.key(),
        name,
        ring: "na",
        card: "waiting",
        enabled: cfg.enabled,
        loading: state.loading,
        pct: 0.0,
        weekly: None,
        pace: None,
        period: 0,
        plan: None,
        title: i18n.f("card.title", &[("name", name)]),
        rows: Vec::new(),
        forecast: None,
        message: None,
        spark: Vec::new(),
        footer: age_text(i18n, "card.checked", "card.checked_now", state.checked_at, now, tool),
        label: i18n.f("a11y.ring_error", &[("name", name)]),
        out_at: None,
        main_resets_at: None,
    };
    let message = |title_key: &str, body_key: &str, error: &str| {
        Some(MessageView {
            title: i18n.f(title_key, &[("tool", tool)]),
            body: i18n.f(body_key, &[("tool", tool), ("error", error)]),
        })
    };

    // A reading (fresh or stale) wins over any status message.
    let usage = match &state.usage {
        Some(usage) if matches!(state.status, Status::Ok | Status::Error(_)) => usage,
        _ => {
            match &state.status {
                Status::SignIn => {
                    view.card = "signin";
                    view.message = message("card.signin.title", "card.signin.body", "");
                    view.label = i18n.f("a11y.ring_signin", &[("name", name)]);
                }
                Status::NoSubscription => {
                    view.card = "nosub";
                    view.message = message("card.nosub.title", "card.nosub.body", "");
                    view.label = i18n.f("a11y.ring_nosub", &[("name", name)]);
                }
                Status::Error(error) => {
                    view.card = "error";
                    view.message = message("card.error.title", "card.error.body", error);
                }
                _ => {
                    view.message = message("card.waiting.title", "card.waiting.body", "");
                    view.label = i18n.f("a11y.ring_loading", &[("name", name)]);
                }
            }
            if state.loading {
                view.ring = "loading";
            }
            return view;
        }
    };

    let stale = state.is_stale();
    view.ring = if state.loading { "loading" } else if stale { "stale" } else { "ok" };
    view.card = if stale { "stale" } else { "ok" };
    view.plan = usage.plan.clone();
    view.spark = state.spark.clone();
    view.footer = if stale {
        age_text(i18n, "card.last_updated", "card.updated_now", state.fetched_at, now, tool)
    } else {
        age_text(i18n, "card.updated", "card.updated_now", state.fetched_at, now, tool)
    };

    for limit in &usage.limits {
        let pct = limit.used_pct.round();
        let (row_name, pct_text) = (i18n.t(&limit.label), format!("{pct:.0}"));
        view.rows.push(RowView {
            label: match limit.resets_at {
                Some(reset) => i18n.f("a11y.row", &[("name", &row_name), ("pct", &pct_text), ("time", &i18n.duration_long(reset - now))]),
                None => i18n.f("a11y.row_noreset", &[("name", &row_name), ("pct", &pct_text)]),
            },
            id: limit.id.clone(),
            name: i18n.t(&limit.label),
            reset: match limit.resets_at {
                Some(reset) => i18n.f("card.resets_in", &[("time", &i18n.duration(reset - now))]),
                None => i18n.t("card.resets_unknown"),
            },
            pct,
            caption: i18n.f("card.caption", &[("used", &format!("{pct:.0}")), ("left", &format!("{:.0}", 100.0 - pct))]),
        });
    }

    if let Some(main) = main_limit(usage, &cfg.main_limit) {
        view.pct = main.used_pct;
        view.pace = forecast::even_pace(main.resets_at, main.window_secs, now);
        view.main_resets_at = main.resets_at;
        view.period = state.series.get(&main.id).map_or(0, |s| s.period);
        // Show the main limit first in the card.
        if let Some(i) = view.rows.iter().position(|r| r.id == main.id) {
            let row = view.rows.remove(i);
            view.rows.insert(0, row);
        }
        let pct_text = format!("{:.0}", main.used_pct.round());
        view.label = if stale {
            let age = i18n.duration_long(now - state.fetched_at.unwrap_or(now));
            i18n.f("a11y.ring_stale", &[("name", name), ("pct", &pct_text), ("time", &age)])
        } else {
            match main.resets_at {
                Some(reset) => {
                    i18n.f("a11y.ring", &[("name", name), ("pct", &pct_text), ("time", &i18n.duration_long(reset - now))])
                }
                None => i18n.f("a11y.ring_noreset", &[("name", name), ("pct", &pct_text)]),
            }
        };
        if stale {
            view.forecast = Some(ForecastView { kind: "stale", text: i18n.t("fc.stale") });
        } else {
            let outlook = outlook_for(state, main, now, pace_window);
            if let Outlook::RunsOut { at, .. } = outlook {
                if at - now < 20 * 3600 {
                    view.out_at = Some(i18n.clock(at));
                }
            }
            view.forecast = Some(forecast_view(i18n, outlook, main, now, crit));
        }
        if !config.forecast.show {
            view.forecast = None;
        }
    }
    if let Some(weekly_id) = id.weekly_window() {
        view.weekly = usage.limits.iter().find(|l| l.id == weekly_id).map(|l| l.used_pct);
    }
    view
}

/// Providers that may be suggested: enabled, included, with a fresh reading.
fn candidates(providers: &HashMap<ProviderId, ProviderState>, config: &Config, now: i64) -> Vec<Candidate> {
    let pace_window = config.forecast.pace_window_min as i64 * 60;
    ProviderId::ALL
        .into_iter()
        .filter_map(|id| {
            let cfg = config.providers.get(id);
            let state = providers.get(&id)?;
            if !cfg.enabled || !cfg.suggest || !state.has_fresh_reading() || state.loading && state.usage.is_none() {
                return None;
            }
            let main = main_limit(state.usage.as_ref()?, &cfg.main_limit)?;
            Some(Candidate {
                id,
                used_pct: main.used_pct,
                runs_out_before_reset: outlook_for(state, main, now, pace_window).runs_out_before_reset(),
            })
        })
        .collect()
}

impl Core {
    /// Builds the display-ready snapshot and updates the suggestion pick.
    pub fn snapshot(&self, now: i64) -> Snapshot {
        let config = self.config();
        let i18n = self.i18n.read().unwrap();
        let providers = self.providers.lock().unwrap();

        let views: Vec<ProviderView> = config
            .providers
            .order
            .iter()
            .filter_map(|id| {
                let state = providers.get(id)?;
                // "Not installed" providers never appear in the notch.
                (state.status != Status::NotInstalled && state.status != Status::Unknown || state.loading)
                    .then(|| provider_view(*id, state, &config, &i18n, now))
            })
            .collect();

        let mut suggestion = None;
        let mut pick = self.pick.lock().unwrap();
        if config.forecast.suggest {
            let list = candidates(&providers, &config, now);
            let hysteresis = config.forecast.hysteresis_pts as f64;
            *pick = suggest::pick(&list, *pick, hysteresis);
            if let Some(sw) = suggest::switch(&list, *pick, config.appearance.warn_at as f64, hysteresis) {
                let pct = |id: ProviderId| list.iter().find(|c| c.id == id).map_or(0.0, |c| c.used_pct.round());
                let out_at = views.iter().find(|v| v.id == sw.from.key()).and_then(|v| v.out_at.clone());
                let (from_pct, to_pct) = (format!("{:.0}", pct(sw.from)), format!("{:.0}", pct(sw.to)));
                let mut args = vec![("from", sw.from.name()), ("from_pct", from_pct.as_str()), ("to", sw.to.name()), ("to_pct", to_pct.as_str())];
                let (a_key, line_key) = match &out_at {
                    Some(at) => {
                        args.push(("at", at.as_str()));
                        ("suggest.chip_a_out", "suggest.line_out")
                    }
                    None => ("suggest.chip_a", "suggest.line"),
                };
                suggestion = Some(SuggestionView {
                    from: sw.from.key(),
                    to: sw.to.key(),
                    a: i18n.f(a_key, &args),
                    b: i18n.f("suggest.chip_b", &args),
                    line: i18n.f(line_key, &args),
                });
            }
        } else {
            *pick = None;
        }
        Snapshot { providers: views, suggestion }
    }
}
