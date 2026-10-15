# Contributing to AI Quota Pin

PRs are welcome, especially new providers. Keep the app small: no new runtimes, no heavy crates, no telemetry.

## Adding a provider
1. Create `src-tauri/src/providers/<name>.rs` implementing the trait from `providers/mod.rs`:

```rust
use super::*;
pub struct Acme;
impl Provider for Acme {
    fn id(&self) -> &'static str { "acme" }
    fn name(&self) -> &'static str { "Acme" }
    fn interval(&self) -> u64 { 300 }               // seconds; >= 120 for network calls
    fn hint(&self) -> &'static str { "Run `acme login`" }
    fn detect(&self) -> bool { home().join(".acme").exists() }
    fn fetch(&self) -> Usage {
        let Some(token) = read_json(&home().join(".acme/auth.json")).and_then(|v| v["token"].as_str().map(String::from))
            else { return Usage::signin(self) };
        match get_json(agent().get("https://api.acme.dev/usage").set("Authorization", &format!("Bearer {token}"))) {
            Ok(b) => Usage::ok(self, vec![Window { label: "Week".into(), used: num(&b["pct"]), resets_at: ts(&b["reset"]) }], None),
            Err(e) if e == "HTTP 401" => Usage::signin(self),
            Err(e) => Usage::na(self, e),
        }
    }
}
```
2. Add `pub mod <name>;` and register it in `providers::all()`.
3. Document the data source, per-OS paths and sign-in hint in the README table.

### Rules
- **Never ask for passwords.** Read only what the provider's own CLI/app already stores locally. Missing/expired login → `Usage::signin`.
- **Never log, print, return or persist secrets.** Errors must be short and secret-free (`get_json` already does this).
- **Local first.** Prefer local session/log files; only call the network when local data is missing or stale.
- **Per-OS paths.** Use `home()`, `dirs::config_dir()`, and honour the tool's env override (e.g. `CODEX_HOME`).
  Gate OS-specific code with `#[cfg(target_os = ...)]`.
- `used` is percent *used* (0–100); `resets_at` is unix seconds.

## Dev loop
```bash
npm install && npm run dev
cd src-tauri && cargo clippy && cargo fmt
```
Release bundles for all OSes are built by `.github/workflows/release.yml` on a `v*` tag (draft release; a maintainer publishes it).
