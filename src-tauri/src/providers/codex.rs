//! Codex CLI: newest ~/.codex/sessions/YYYY/MM/DD/*.jsonl `rate_limits` (local, free),
//! falling back to chatgpt.com/backend-api/wham/usage with ~/.codex/auth.json.
use super::*;
use std::io::{Read, Seek, SeekFrom};

pub struct Codex;

fn home_dir() -> PathBuf { env_or_home("CODEX_HOME", ".codex") }

fn windows(rl: &Value) -> Vec<Window> {
    ["primary", "secondary", "primary_window", "secondary_window"].iter().filter_map(|k| {
        let w = rl.get(*k)?.as_object()?;
        let mins = w.get("window_minutes").and_then(num)
            .or_else(|| w.get("limit_window_seconds").and_then(num).map(|s| s / 60.0)).unwrap_or(300.0);
        let label = if mins >= 6.0 * 1440.0 { "Week".into() } else { format!("{}h", (mins / 60.0).round()) };
        let mut win = Window { label, used: w.get("used_percent").and_then(num), detail: None, resets_at: w.get("resets_at").or(w.get("reset_at")).and_then(ts) };
        if win.resets_at.map_or(false, |r| r < now()) { win.used = Some(0.0); win.resets_at = None; }
        Some(win)
    }).collect()
}

fn newest_session() -> Option<(PathBuf, i64)> {
    let mut best: Option<(PathBuf, i64)> = None;
    for back in 0..8 {
        let d = chrono::Local::now() - chrono::Duration::days(back);
        let dir = home_dir().join("sessions").join(d.format("%Y/%m/%d").to_string());
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let m = e.metadata().ok().and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
            if e.path().extension().map_or(false, |x| x == "jsonl") && best.as_ref().map_or(true, |b| m > b.1) { best = Some((e.path(), m)); }
        }
        if best.is_some() { break; }
    }
    best
}

fn local() -> Option<(Vec<Window>, Option<String>, i64)> {
    let (p, mtime) = newest_session()?;
    let mut f = std::fs::File::open(p).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(128 * 1024))).ok()?;
    let mut s = String::new();
    let mut buf = Vec::new(); f.read_to_end(&mut buf).ok()?; s.push_str(&String::from_utf8_lossy(&buf));
    for line in s.lines().rev().filter(|l| l.contains("\"rate_limits\"")) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let rl = if v["payload"]["rate_limits"].is_object() { &v["payload"]["rate_limits"] } else { &v["rate_limits"] };
        let w = windows(rl);
        if !w.is_empty() { return Some((w, rl["plan_type"].as_str().map(String::from), mtime)); }
    }
    None
}

impl Provider for Codex {
    fn id(&self) -> &'static str { "codex" }
    fn name(&self) -> &'static str { "Codex" }
    fn interval(&self) -> u64 { 60 }
    fn hint(&self) -> &'static str { "Run `codex login`" }
    fn detect(&self) -> bool { home_dir().exists() }
    fn fetch(&self) -> Usage {
        let loc = local();
        // Fresh local data (< 30 min old) costs no network at all.
        if let Some((w, plan, m)) = &loc { if now() - m < 1800 { return Usage::ok(self, w.clone(), plan.clone()); } }
        let auth = read_json(&home_dir().join("auth.json"));
        let tok = auth.as_ref().and_then(|a| a["tokens"]["access_token"].as_str());
        let remote = tok.ok_or_else(|| "not signed in".to_string()).and_then(|t| {
            let mut r = agent().get("https://chatgpt.com/backend-api/wham/usage").set("Authorization", &format!("Bearer {t}"));
            if let Some(acc) = auth.as_ref().and_then(|a| a["tokens"]["account_id"].as_str()) { r = r.set("ChatGPT-Account-Id", acc); }
            get_json(r)
        });
        match (remote, loc) {
            (Ok(b), _) => Usage::ok(self, windows(&b["rate_limit"]), b["plan_type"].as_str().map(String::from)),
            (Err(_), Some((w, plan, _))) => Usage::ok(self, w, plan.map(|p| format!("{p} · from last session"))),
            (Err(e), None) if e == "not signed in" || e == "HTTP 401" => Usage::signin(self),
            (Err(e), None) => Usage::na(self, e),
        }
    }
}
