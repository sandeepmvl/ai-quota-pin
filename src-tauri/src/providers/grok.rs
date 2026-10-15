//! Grok (grok.com CLI sign-in): ~/.grok/auth.json -> cli-chat-proxy.grok.com/v1/billing.
use super::*;

pub struct Grok;
fn auth_path() -> PathBuf { env_or_home("GROK_HOME", ".grok").join("auth.json") }

impl Provider for Grok {
    fn id(&self) -> &'static str { "grok" }
    fn name(&self) -> &'static str { "Grok" }
    fn interval(&self) -> u64 { 300 }
    fn hint(&self) -> &'static str { "Run `grok login`" }
    fn detect(&self) -> bool { auth_path().exists() }
    fn fetch(&self) -> Usage {
        let a = read_json(&auth_path());
        let Some(t) = a.as_ref().and_then(|a| a.as_object()?.values().find_map(|e| e["key"].as_str().map(String::from)))
            else { return Usage::signin(self) };
        let r = agent().get("https://cli-chat-proxy.grok.com/v1/billing?format=credits")
            .set("Authorization", &format!("Bearer {t}")).set("Accept", "application/json").set("X-XAI-Token-Auth", "xai-grok-cli");
        match get_json(r) {
            Ok(b) => {
                let c = if b["config"].is_object() { &b["config"] } else { &b };
                let per = &c["currentPeriod"];
                let weekly = per["type"].as_str().unwrap_or("").to_uppercase().contains("WEEK");
                let w = Window { label: if weekly { "Week" } else { "Period" }.into(), used: Some(num(&c["creditUsagePercent"]).unwrap_or(0.0)),
                    detail: None, resets_at: ts(&per["end"]).or_else(|| ts(&c["billingPeriodEnd"])) };
                let used: Vec<String> = c["productUsage"].as_array().into_iter().flatten()
                    .filter_map(|p| { let u = num(&p["usagePercent"])?; (u > 0.0).then(|| format!("{} {}%", p["product"].as_str().unwrap_or("?"), u.round())) }).collect();
                Usage::ok(self, vec![w], (!used.is_empty()).then(|| used.join(", ")))
            }
            Err(e) if e == "HTTP 401" => Usage::signin(self),
            Err(e) => Usage::na(self, e),
        }
    }
}
