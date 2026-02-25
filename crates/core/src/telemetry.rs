use std::sync::{Mutex, OnceLock};

use regex::Regex;
use sentry::protocol::{Context, LogLevel, Value};
use sentry::{ClientInitGuard, ClientOptions, Level};
use std::collections::BTreeMap;
use uuid::Uuid;

static TELEMETRY: OnceLock<TelemetryState> = OnceLock::new();

struct TelemetryState {
    guard: Mutex<Option<ClientInitGuard>>,
    session_id: Uuid,
    build_version: Mutex<Option<String>>,
    scrub_unix_home: Regex,
    scrub_win_home: Regex,
    scrub_url: Regex,
}

fn state() -> &'static TelemetryState {
    TELEMETRY.get_or_init(|| TelemetryState {
        guard: Mutex::new(None),
        session_id: Uuid::new_v4(),
        build_version: Mutex::new(None),
        scrub_unix_home: Regex::new(r"/(Users|home)/[^/]+").expect("scrub_unix_home regex"),
        scrub_win_home: Regex::new(r"(?i)C:\\Users\\[^\\]+").expect("scrub_win_home regex"),
        scrub_url: Regex::new(r"https?://[^\s]+").expect("scrub_url regex"),
    })
}

pub fn configure_build_version(build_version: String) {
    *state().build_version.lock().expect("build_version lock") = Some(build_version);
}

fn env_hard_disabled() -> bool {
    matches!(
        std::env::var("FLEET_TELEMETRY").as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    )
}

pub fn apply_consent(consent: Option<bool>) {
    if env_hard_disabled() {
        disable();
        return;
    }

    match consent {
        Some(true) => enable(),
        _ => disable(),
    }
}

fn enable() {
    let s = state();
    if s.guard.lock().expect("telemetry guard lock").is_some() {
        return;
    }

    let dsn = match std::env::var("SENTRY_DSN") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => return,
    };

    let release = s
        .build_version
        .lock()
        .expect("build_version lock")
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    let scrub_unix = s.scrub_unix_home.clone();
    let scrub_win = s.scrub_win_home.clone();
    let scrub_url = s.scrub_url.clone();

    let scrub_unix_log = scrub_unix.clone();
    let scrub_win_log = scrub_win.clone();
    let scrub_url_log = scrub_url.clone();

    let guard = sentry::init((
        dsn,
        ClientOptions {
            release: Some(release.into()),
            send_default_pii: false,
            enable_logs: true,
            before_send: Some(std::sync::Arc::new(move |mut event| {
                scrub_event_message(&mut event, &scrub_win, &scrub_unix, &scrub_url);
                scrub_event_extra(&mut event, &scrub_win, &scrub_unix, &scrub_url);
                Some(event)
            })),
            before_send_log: Some(std::sync::Arc::new(move |mut log| {
                if matches!(log.level, LogLevel::Trace) {
                    return None;
                }
                if matches!(log.level, LogLevel::Debug) && !cfg!(debug_assertions) {
                    return None;
                }
                log.body = scrub_text(&log.body, &scrub_win_log, &scrub_unix_log, &scrub_url_log);
                for value in log.attributes.values_mut() {
                    if let Value::String(text) = &mut value.0 {
                        *text = scrub_text(text, &scrub_win_log, &scrub_unix_log, &scrub_url_log);
                    }
                }
                Some(log)
            })),
            ..Default::default()
        },
    ));

    *s.guard.lock().expect("telemetry guard lock") = Some(guard);
    capture_session_start();
}

fn disable() {
    let s = state();
    *s.guard.lock().expect("telemetry guard lock") = None;
}

pub fn session_id() -> Uuid {
    state().session_id
}

pub fn capture_session_start() {
    if state()
        .guard
        .lock()
        .expect("telemetry guard lock")
        .is_none()
    {
        return;
    }

    let build = state()
        .build_version
        .lock()
        .expect("build_version lock")
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    let build_tag = build.clone();
    sentry::configure_scope(|scope| {
        scope.set_tag("session_id", session_id().to_string());
        scope.set_tag("build_version", build_tag);
        let mut ctx = BTreeMap::new();
        ctx.insert(
            "session_id".to_string(),
            Value::String(session_id().to_string()),
        );
        ctx.insert("build_version".to_string(), Value::String(build));
        scope.set_context("fleet", Context::Other(ctx));
    });

    sentry::capture_message("fleet.session_start", Level::Info);
}

fn scrub_text(input: &str, scrub_win: &Regex, scrub_unix: &Regex, scrub_url: &Regex) -> String {
    let v = scrub_url.replace_all(input, "<redacted_url>");
    let v = scrub_win.replace_all(&v, "C:\\Users\\<redacted>");
    scrub_unix.replace_all(&v, "/home/<redacted>").to_string()
}

fn scrub_event_message(
    event: &mut sentry::protocol::Event<'static>,
    scrub_win: &Regex,
    scrub_unix: &Regex,
    scrub_url: &Regex,
) {
    if let Some(msg) = event.message.take() {
        event.message = Some(scrub_text(&msg, scrub_win, scrub_unix, scrub_url));
    }
    if let Some(logentry) = event.logentry.as_mut() {
        logentry.message = scrub_text(&logentry.message, scrub_win, scrub_unix, scrub_url);
    }
}

fn scrub_event_extra(
    event: &mut sentry::protocol::Event<'static>,
    scrub_win: &Regex,
    scrub_unix: &Regex,
    scrub_url: &Regex,
) {
    for value in event.extra.values_mut() {
        if let Value::String(text) = value {
            *text = scrub_text(text, scrub_win, scrub_unix, scrub_url);
        }
    }
}
