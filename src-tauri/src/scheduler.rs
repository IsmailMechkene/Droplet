//! The one worker thread. It polls providers one request at a time and sleeps
//! in between, so the app uses no CPU at rest.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::clock;
use crate::net;
use crate::providers::{self, FetchError, ProviderId, Usage};
use crate::state::{Core, Status};

/// Why a refresh happens. Antigravity is expensive, so it only reacts to some of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Startup,
    Timer,
    Manual,
    CardOpen,
}

pub enum Msg {
    RefreshAll(Reason),
    RefreshOne(ProviderId, Reason),
    /// Settings changed: re-evaluate the timer and redraw.
    ConfigChanged,
}

pub fn start(app: AppHandle) {
    let (tx, rx) = mpsc::channel();
    let core = app.state::<Arc<Core>>().inner().clone();
    *core.scheduler.lock().unwrap() = Some(tx);
    std::thread::Builder::new()
        .name("droplet-worker".into())
        .spawn(move || run(app, core, rx))
        .expect("start worker thread");
}

fn interval(core: &Core) -> Duration {
    // The config validator only allows 3, 5, 10 or 15; never poll faster than 3 minutes.
    Duration::from_secs(core.config.read().unwrap().general.refresh_minutes.max(3) as u64 * 60)
}

fn run(app: AppHandle, core: Arc<Core>, rx: Receiver<Msg>) {
    refresh_all(&app, &core, Reason::Startup);
    let mut last_poll = Instant::now();
    loop {
        let wait = (last_poll + interval(&core)).saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(Msg::RefreshAll(reason)) => {
                refresh_all(&app, &core, reason);
                last_poll = Instant::now();
            }
            Ok(Msg::RefreshOne(id, reason)) => refresh_one(&app, &core, id, reason),
            Ok(Msg::ConfigChanged) => publish(&app),
            Err(RecvTimeoutError::Timeout) => {
                refresh_all(&app, &core, Reason::Timer);
                last_poll = Instant::now();
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn refresh_all(app: &AppHandle, core: &Core, reason: Reason) {
    let order = core.config.read().unwrap().providers.order.clone();
    for id in order {
        refresh_one(app, core, id, reason);
    }
}

fn refresh_one(app: &AppHandle, core: &Core, id: ProviderId, reason: Reason) {
    let config = core.config();
    if !config.providers.get(id).enabled || !providers::is_available(id) {
        return;
    }
    // Opening a card only re-reads Antigravity; for the others it just refreshes the wording.
    if reason == Reason::CardOpen && id != ProviderId::Antigravity {
        publish(app);
        return;
    }
    let now = clock::now();
    {
        let mut all = core.providers.lock().unwrap();
        let state = all.entry(id).or_default();
        // Honour a 429 back-off even for manual refreshes.
        if now < state.retry_at {
            return;
        }
        state.loading = true;
    }
    publish(app);

    let result = providers::fetch(id, &core.net, &config);
    apply(core, id, result, clock::now());
    publish(app);
}

/// Records the outcome of one fetch. On failure the last reading is kept and
/// shown as stale; a number is never made up.
pub fn apply(core: &Core, id: ProviderId, result: Result<Usage, FetchError>, now: i64) {
    let mut all = core.providers.lock().unwrap();
    let state = all.entry(id).or_default();
    state.loading = false;
    state.checked_at = Some(now);
    match result {
        Ok(usage) => {
            for limit in &usage.limits {
                state.series.entry(limit.id.clone()).or_default().push(now, limit.used_pct, limit.resets_at);
            }
            state.usage = Some(usage);
            state.fetched_at = Some(now);
            state.status = Status::Ok;
            state.rate_limited = 0;
            state.retry_at = 0;
        }
        Err(FetchError::NotInstalled) => {
            state.status = Status::NotInstalled;
            state.usage = None;
        }
        Err(FetchError::SignIn) => {
            state.status = Status::SignIn;
            state.usage = None;
        }
        Err(FetchError::NoSubscription) => {
            state.status = Status::NoSubscription;
            state.usage = None;
        }
        Err(FetchError::RateLimited { retry_after }) => {
            state.rate_limited += 1;
            let wait = net::backoff_secs(state.rate_limited, retry_after);
            state.retry_at = now + wait as i64;
            state.status = Status::Error("HTTP 429 (rate limited)".into());
        }
        Err(FetchError::Failed(error)) => state.status = Status::Error(error),
    }
}

/// Sends the current snapshot to every open window.
pub fn publish(app: &AppHandle) {
    let core = app.state::<Arc<Core>>();
    let snapshot = core.snapshot(clock::now());
    let _ = app.emit("usage", &snapshot);
}
