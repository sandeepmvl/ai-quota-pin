//! Provider plugin layer. Each provider implements [`Provider`] and is listed in [`all`].
//! Rules: read local files/CLIs first, never log or return secrets, keep network calls rare.
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

pub mod claude;
pub mod codex;
pub mod cursor;
pub mod gemini;
pub mod grok;

#[derive(Serialize, Clone, Debug, Default)]
pub struct Window {
    pub label: String,
    /// Percent used, 0–100.
    pub used: Option<f64>,
    /// Reset time, unix seconds.
    pub resets_at: Option<i64>,
    /// Free text shown instead of a bar when there is no percentage (e.g. a local estimate).
    pub detail: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Usage {
    pub id: String,
    pub name: String,
    pub windows: Vec<Window>,
    pub note: Option<String>,
    /// "ok" | "na" (error) | "signin" (not installed / not signed in / expired) | "stale"
    pub status: String,
    /// One-line fix for "signin", e.g. "Run `claude` and log in".
    pub hint: Option<String>,
    pub updated_at: i64,
}

impl Usage {
    pub fn ok(p: &dyn Provider, windows: Vec<Window>, note: Option<String>) -> Self {
        Usage { id: p.id().into(), name: p.name().into(), windows, note, status: "ok".into(), updated_at: now(), hint: None }
    }
    /// Login missing or expired. The app never asks for passwords; the user fixes it in the provider's own CLI.
    pub fn signin(p: &dyn Provider) -> Self {
        Usage { id: p.id().into(), name: p.name().into(), note: Some("Not signed in".into()), hint: Some(p.hint().into()), status: "signin".into(), updated_at: now(), ..Default::default() }
    }
    pub fn na(p: &dyn Provider, why: impl Into<String>) -> Self {
        Usage { id: p.id().into(), name: p.name().into(), note: Some(why.into()), status: "na".into(), updated_at: now(), ..Default::default() }
    }
}

pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    /// Seconds between polls (60–600). Network-backed providers should use >= 120.
    fn interval(&self) -> u64 { 120 }
    /// One-line sign-in hint shown when credentials are missing or expired.
    fn hint(&self) -> &'static str;
    /// Cheap check: is this tool installed / signed in on this machine?
    fn detect(&self) -> bool;
    fn fetch(&self) -> Usage;
}

pub fn all() -> Vec<Box<dyn Provider>> {
    vec![Box::new(claude::Claude::default()), Box::new(codex::Codex), Box::new(grok::Grok), Box::new(gemini::Gemini), Box::new(cursor::Cursor)]
}

// ---------- helpers ----------
pub fn now() -> i64 { chrono::Utc::now().timestamp() }
pub fn home() -> PathBuf { dirs::home_dir().unwrap_or_default() }
pub fn env_or_home(var: &str, rel: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| home().join(rel))
}
pub fn read_json(p: &std::path::Path) -> Option<Value> { serde_json::from_slice(&std::fs::read(p).ok()?).ok() }
pub fn num(v: &Value) -> Option<f64> { v.as_f64().or_else(|| v.as_str()?.parse().ok()) }
pub fn ts(v: &Value) -> Option<i64> {
    if let Some(n) = num(v) { return Some(if n > 1e12 { (n / 1000.0) as i64 } else { n as i64 }); }
    chrono::DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|d| d.timestamp())
}
pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).user_agent(concat!("ai-quota-pin/", env!("CARGO_PKG_VERSION"))).build()
}
/// GET JSON; Err carries a short, secret-free reason.
pub fn get_json(req: ureq::Request) -> Result<Value, String> {
    match req.call() {
        Ok(r) => r.into_json().map_err(|_| "bad JSON".into()),
        Err(ureq::Error::Status(c, _)) => Err(format!("HTTP {c}")),
        Err(_) => Err("offline".into()),
    }
}

/// Fake data for screenshots and UI work: run with `AIQP_DEMO=1`. Makes no file or network access.
pub fn demo() -> Vec<Usage> {
    let w = |l: &str, u: f64, h: i64| Window { label: l.into(), used: Some(u), resets_at: Some(now() + h * 3600), detail: None };
    let ok = |id: &str, name: &str, ws, note: Option<&str>| Usage { id: id.into(), name: name.into(), windows: ws, note: note.map(Into::into), status: "ok".into(), updated_at: now(), hint: None };
    vec![
        ok("claude", "Claude", vec![w("5h", 62.0, 2), w("Week", 38.0, 100)], None),
        ok("codex", "Codex", vec![w("5h", 21.0, 4), w("Week", 74.0, 60)], Some("plus")),
        ok("grok", "Grok", vec![w("Week", 93.0, 30)], None),
        Usage { id: "gemini".into(), name: "Gemini".into(), note: Some("Not signed in".into()), hint: Some("Run `gemini` and choose Login with Google".into()), status: "signin".into(), updated_at: now(), ..Default::default() },
    ]
}
