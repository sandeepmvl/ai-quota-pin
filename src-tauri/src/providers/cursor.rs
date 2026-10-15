//! Cursor: access token from Cursor's state.vscdb (SQLite, read-only) ->
//! cursor.com/api/usage-summary (same call the Cursor dashboard makes).
use super::*;
use base64::Engine;

pub struct Cursor;

fn db() -> PathBuf {
    #[cfg(target_os = "macos")] let base = home().join("Library/Application Support");
    #[cfg(target_os = "windows")] let base = dirs::config_dir().unwrap_or_default();
    #[cfg(all(unix, not(target_os = "macos")))] let base = dirs::config_dir().unwrap_or_default();
    base.join("Cursor/User/globalStorage/state.vscdb")
}

fn token() -> Option<String> {
    let c = rusqlite::Connection::open_with_flags(db(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    c.query_row("SELECT value FROM ItemTable WHERE key='cursorAuth/accessToken'", [], |r| r.get::<_, String>(0)).ok()
}

impl Provider for Cursor {
    fn id(&self) -> &'static str { "cursor" }
    fn name(&self) -> &'static str { "Cursor" }
    fn interval(&self) -> u64 { 300 }
    fn hint(&self) -> &'static str { "Open Cursor and sign in" }
    fn detect(&self) -> bool { db().exists() }
    fn fetch(&self) -> Usage {
        let Some(t) = token() else { return Usage::signin(self) };
        let sub = t.split('.').nth(1)
            .and_then(|p| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(p.trim_end_matches('=')).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v["sub"].as_str().map(|s| s.rsplit('|').next().unwrap_or(s).to_string()));
        let Some(uid) = sub else { return Usage::na(self, "unreadable token") };
        let r = agent().get("https://cursor.com/api/usage-summary")
            .set("Cookie", &format!("WorkosCursorSessionToken={uid}%3A%3A{t}"));
        match get_json(r) {
            Ok(b) => {
                let p = &b["individualUsage"]["plan"];
                let used = num(&p["used"]).zip(num(&p["limit"])).filter(|(_, l)| *l > 0.0).map(|(u, l)| u / l * 100.0);
                let w = Window { label: "Month".into(), used: used.or_else(|| num(&p["totalPercentUsed"])), detail: None, resets_at: ts(&b["billingCycleEnd"]) };
                Usage::ok(self, vec![w], b["membershipType"].as_str().map(String::from))
            }
            Err(e) if e == "HTTP 401" || e == "HTTP 403" => Usage::signin(self),
            Err(e) => Usage::na(self, e),
        }
    }
}
