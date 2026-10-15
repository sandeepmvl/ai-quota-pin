//! Gemini CLI: detects ~/.gemini/oauth_creds.json. Google exposes no stable public
//! quota endpoint for the CLI's free/Code Assist tiers yet, so this reports sign-in
//! state only. PRs welcome (see CONTRIBUTING.md).
use super::*;

pub struct Gemini;
fn creds() -> PathBuf { env_or_home("GEMINI_DIR", ".gemini").join("oauth_creds.json") }

impl Provider for Gemini {
    fn id(&self) -> &'static str { "gemini" }
    fn name(&self) -> &'static str { "Gemini" }
    fn interval(&self) -> u64 { 600 }
    fn hint(&self) -> &'static str { "Run `gemini` and choose Login with Google" }
    fn detect(&self) -> bool { creds().exists() }
    fn fetch(&self) -> Usage {
        if creds().exists() { Usage::na(self, "signed in · no public quota API") } else { Usage::signin(self) }
    }
}
