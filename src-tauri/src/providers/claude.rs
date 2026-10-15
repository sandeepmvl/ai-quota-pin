//! Claude Code: OAuth token from macOS Keychain ("Claude Code-credentials") or
//! ~/.claude/.credentials.json (Linux/Windows) -> api.anthropic.com/api/oauth/usage.
//!
//! The usage endpoint is rate limited, so this provider is deliberately gentle:
//! - polled at most every 3 min, and the last good answer is cached on disk so app restarts don't refetch;
//! - HTTP 429 honours Retry-After, otherwise exponential backoff 5 → 10 → 20 → 40 → 60 min;
//! - while backing off the UI keeps the last known values marked "stale · retrying in Xm";
//! - with no cached values, a local estimate from ~/.claude/projects/**/*.jsonl is shown instead.
use super::*;
use std::sync::Mutex;

const MIN_GAP: i64 = 180;

#[derive(Default)]
struct St { token: Option<String>, next_at: i64, backoff: i64, last: Option<Usage> }

#[derive(Default)]
pub struct Claude { st: Mutex<St> }

fn token_from(v: &Value) -> Option<String> { v["claudeAiOauth"]["accessToken"].as_str().map(String::from) }
fn dir() -> PathBuf { env_or_home("CLAUDE_CONFIG_DIR", ".claude") }
fn cache_path() -> PathBuf { dirs::config_dir().unwrap_or_default().join("ai-quota-pin/claude-usage.json") }

fn read_token() -> Option<String> {
    if let Some(t) = read_json(&dir().join(".credentials.json")).as_ref().and_then(token_from) { return Some(t); }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"]).output().ok()?;
        return serde_json::from_slice::<Value>(&out.stdout).ok().as_ref().and_then(token_from);
    }
    #[allow(unreachable_code)]
    None
}

fn parse(b: &Value) -> Vec<Window> {
    [("five_hour", "5h"), ("seven_day", "Week"), ("seven_day_opus", "Opus wk"), ("seven_day_sonnet", "Sonnet wk")]
        .iter().filter(|(k, _)| b[*k].is_object())
        .map(|(k, l)| Window { label: l.to_string(), used: num(&b[*k]["utilization"]), resets_at: ts(&b[*k]["resets_at"]), detail: None })
        .collect()
}

/// Rough local fallback: tokens Claude Code logged in the last 5 hours (ccusage-style JSONL scan,
/// limited to files touched in that window so it stays cheap).
fn local_estimate() -> Option<Window> {
    let since = std::time::SystemTime::now() - std::time::Duration::from_secs(5 * 3600);
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(5);
    let (mut total, mut msgs, mut first): (u64, u64, i64) = (0, 0, i64::MAX);
    for proj in std::fs::read_dir(dir().join("projects")).ok()?.flatten() {
        for f in std::fs::read_dir(proj.path()).into_iter().flatten().flatten() {
            if f.path().extension().map_or(true, |x| x != "jsonl") { continue; }
            if f.metadata().ok().and_then(|m| m.modified().ok()).map_or(true, |m| m < since) { continue; }
            let Ok(text) = std::fs::read_to_string(f.path()) else { continue };
            for line in text.lines().filter(|l| l.contains("\"usage\"")) {
                let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                let Some(t) = ts(&v["timestamp"]).filter(|t| *t >= cutoff.timestamp()) else { continue };
                if v["type"] == "assistant" { msgs += 1; }
                first = first.min(t);
                let u = &v["message"]["usage"];
                total += ["input_tokens", "output_tokens", "cache_creation_input_tokens"].iter().filter_map(|k| u[*k].as_u64()).sum::<u64>();
            }
        }
    }
    // The 5h window starts at the first message after the previous one expired; approximate with the
    // earliest message in the last 5 hours.
    (total > 0).then(|| Window { label: "5h est.".into(), used: None, resets_at: Some(first + 5 * 3600),
        detail: Some(format!("{:.1}M tok · {msgs} msgs", total as f64 / 1e6)) })
}

impl Claude {
    fn waiting(&self, st: &St, why: &str) -> Usage {
        let mins = ((st.next_at - now()) as f64 / 60.0).ceil().max(1.0);
        let note = format!("{why} · retrying in {mins}m");
        match &st.last {
            Some(u) => { let mut u = u.clone(); u.status = "stale".into(); u.note = Some(format!("stale · {note}")); u }
            None => match local_estimate() {
                Some(w) => { let mut u = Usage::ok(self, vec![w], Some(format!("est. · {note}"))); u.status = "stale".into(); u }
                None => Usage::na(self, note),
            },
        }
    }
}

impl Provider for Claude {
    fn id(&self) -> &'static str { "claude" }
    fn name(&self) -> &'static str { "Claude" }
    fn interval(&self) -> u64 { MIN_GAP as u64 }
    fn hint(&self) -> &'static str { "Run `claude` and log in (/login)" }
    fn detect(&self) -> bool { dir().exists() }
    fn fetch(&self) -> Usage {
        let mut st = self.st.lock().unwrap();
        // Disk cache: a restart within MIN_GAP reuses the last answer instead of calling the API.
        if st.last.is_none() {
            if let Some(c) = read_json(&cache_path()) {
                let at = c["at"].as_i64().unwrap_or(0);
                let w = parse(&c["body"]);
                if !w.is_empty() { let mut u = Usage::ok(self, w, None); u.updated_at = at; st.last = Some(u); }
                if now() - at < MIN_GAP { st.next_at = at + MIN_GAP; }
            }
        }
        if now() < st.next_at {
            return match &st.last { Some(u) if st.backoff == 0 => u.clone(), _ => self.waiting(&st, "rate limited") };
        }
        for attempt in 0..2 {
            if st.token.is_none() { st.token = read_token(); }
            let Some(t) = st.token.clone() else { return Usage::signin(self) };
            st.next_at = now() + MIN_GAP;
            let res = agent().get("https://api.anthropic.com/api/oauth/usage")
                .set("Authorization", &format!("Bearer {t}")).set("anthropic-beta", "oauth-2025-04-20").call();
            match res {
                Ok(r) => {
                    let Ok(b) = r.into_json::<Value>() else { return Usage::na(self, "bad JSON") };
                    st.backoff = 0;
                    let _ = std::fs::create_dir_all(cache_path().parent().unwrap());
                    let _ = std::fs::write(cache_path(), serde_json::to_vec(&serde_json::json!({ "at": now(), "body": b })).unwrap());
                    let u = Usage::ok(self, parse(&b), None);
                    st.last = Some(u.clone());
                    return u;
                }
                Err(ureq::Error::Status(401, _)) if attempt == 0 => { st.token = None; }
                Err(ureq::Error::Status(401, _)) => return Usage::signin(self),
                Err(ureq::Error::Status(429, r)) => {
                    st.backoff = if st.backoff == 0 { 300 } else { (st.backoff * 2).min(3600) };
                    let ra = r.header("retry-after").and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(0);
                    st.next_at = now() + ra.max(st.backoff);
                    return self.waiting(&st, "rate limited");
                }
                Err(e) => {
                    st.backoff = if st.backoff == 0 { 300 } else { (st.backoff * 2).min(3600) };
                    st.next_at = now() + st.backoff;
                    let why = match e { ureq::Error::Status(c, _) => format!("HTTP {c}"), _ => "offline".into() };
                    return self.waiting(&st, &why);
                }
            }
        }
        Usage::signin(self)
    }
}
