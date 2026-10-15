#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! AI Quota Pin: always-on-top sticky note for AI subscription limits.
mod providers;

use providers::{Provider, Usage};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewWindow};

#[derive(Default, Serialize, Deserialize, Clone)]
#[serde(default)]
struct Prefs { right: Option<i32>, top: Option<i32>, compact: bool, collapsed: bool, #[serde(default)] hidden: Vec<String> }

struct Shared { usage: Mutex<Vec<Usage>>, prefs: Mutex<Prefs>, wake: (Mutex<bool>, Condvar) }

fn prefs_path() -> std::path::PathBuf { dirs::config_dir().unwrap_or_default().join("ai-quota-pin/prefs.json") }
fn load_prefs() -> Prefs { std::fs::read(prefs_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default() }
fn save_prefs(p: &Prefs) {
    let path = prefs_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(path, serde_json::to_vec(p).unwrap());
}

#[tauri::command]
fn get_state(s: tauri::State<Arc<Shared>>) -> (Vec<Usage>, Prefs) { (s.usage.lock().unwrap().clone(), s.prefs.lock().unwrap().clone()) }

#[tauri::command]
fn refresh(s: tauri::State<Arc<Shared>>) { poke(&s) }

#[tauri::command]
fn set_mode(s: tauri::State<Arc<Shared>>, compact: Option<bool>, collapsed: Option<bool>, hide: Option<String>, show_all: Option<bool>) {
    let mut p = s.prefs.lock().unwrap();
    if let Some(c) = compact { p.compact = c; }
    if let Some(c) = collapsed { p.collapsed = c; }
    if let Some(id) = hide { if !p.hidden.contains(&id) { p.hidden.push(id); } }
    if show_all == Some(true) { p.hidden.clear(); poke(&s); }
    save_prefs(&p);
}

/// Resize to the UI's content while keeping the top-right corner pinned.
#[tauri::command]
fn fit(w: WebviewWindow, width: f64, height: f64) {
    let (Ok(pos), Ok(size), Ok(scale)) = (w.outer_position(), w.outer_size(), w.scale_factor()) else { return };
    let (nw, nh) = ((width * scale) as i32, (height * scale) as i32);
    if (nw - size.width as i32).abs() <= 1 && (nh - size.height as i32).abs() <= 1 { return; } // no-op: avoid relayout
    let right = pos.x + size.width as i32;
    let _ = w.set_size(LogicalSize::new(width, height));
    let _ = w.set_position(PhysicalPosition::new(right - nw, pos.y));
}

fn poke(s: &Shared) { *s.wake.0.lock().unwrap() = true; s.wake.1.notify_all(); }

fn default_corner(w: &WebviewWindow) -> Option<(i32, i32)> {
    let m = w.current_monitor().ok().flatten().or_else(|| w.primary_monitor().ok().flatten())?;
    let margin = (12.0 * m.scale_factor()) as i32;
    let top_inset = if cfg!(target_os = "macos") { (28.0 * m.scale_factor()) as i32 } else { 0 };
    Some((m.position().x + m.size().width as i32 - margin, m.position().y + margin + top_inset))
}

fn place(w: &WebviewWindow, p: &Prefs) {
    // Saved position is honoured on any monitor, but clamped back on-screen if that display is gone/resized.
    let on_screen = |r: i32, t: i32| w.available_monitors().unwrap_or_default().iter().any(|m| {
        let (mx, my) = (m.position().x, m.position().y);
        r - 20 >= mx && r <= mx + m.size().width as i32 && t >= my && t + 20 <= my + m.size().height as i32
    });
    let (right, top) = match (p.right, p.top) {
        (Some(r), Some(t)) if on_screen(r, t) => (r, t),
        _ => default_corner(w).unwrap_or((800, 40)),
    };
    let width = w.outer_size().map(|s| s.width as i32).unwrap_or(290);
    let _ = w.set_position(PhysicalPosition::new(right - width, top));
}

fn poll_loop(app: AppHandle, s: Arc<Shared>) {
    if std::env::var_os("AIQP_DEMO").is_some() {
        loop { let d = providers::demo(); *s.usage.lock().unwrap() = d.clone(); let _ = app.emit("usage", d); std::thread::sleep(Duration::from_secs(60)); }
    }
    let _ = &providers::all()[0].detect(); // keep detect() in the public plugin API
    let provs: Vec<Box<dyn Provider>> = providers::all(); // every provider gets a row; missing ones show a sign-in hint
    let mut last: Vec<Option<Instant>> = vec![None; provs.len()];
    *s.usage.lock().unwrap() = provs.iter().map(|p| Usage { id: p.id().into(), name: p.name().into(), status: "na".into(), note: Some("loading…".into()), ..Default::default() }).collect();
    loop {
        // Back off 10x when the user is idle/locked (> 10 min) — no wasted network while away.
        let idle = user_idle::UserIdle::get_time().map(|i| i.as_seconds()).unwrap_or(0) > 600;
        let forced = std::mem::take(&mut *s.wake.0.lock().unwrap());
        let mut changed = false;
        for (i, p) in provs.iter().enumerate() {
            let every = Duration::from_secs(p.interval() * if idle { 10 } else { 1 });
            if s.prefs.lock().unwrap().hidden.iter().any(|h| h == p.id()) { continue; }
            if !forced && last[i].map_or(false, |t| t.elapsed() < every) { continue; }
            last[i] = Some(Instant::now());
            let mut u = p.fetch();
            let mut all = s.usage.lock().unwrap();
            // Keep last good numbers on transient failure (429/offline), marked stale.
            if u.status == "na" && !all[i].windows.is_empty() && u.status != "signin" {
                let why = u.note.take(); u = all[i].clone(); u.status = "stale".into(); u.note = why;
            }
            all[i] = u; changed = true;
        }
        if changed {
            let all = s.usage.lock().unwrap().clone();
            let worst = all.iter().flat_map(|u| u.windows.iter().filter_map(|w| w.used)).fold(0.0, f64::max);
            if let Some(t) = app.tray_by_id("main") { let _ = t.set_tooltip(Some(format!("AI Quota Pin — max {worst:.0}% used"))); #[cfg(target_os = "macos")] let _ = t.set_title(Some(format!("{worst:.0}%"))); }
            let _ = app.emit("usage", all);
        }
        let g = s.wake.0.lock().unwrap();
        let _ = s.wake.1.wait_timeout_while(g, Duration::from_secs(30), |w| !*w);
    }
}

fn main() {
    let shared = Arc::new(Shared { usage: Mutex::new(vec![]), prefs: Mutex::new(load_prefs()), wake: (Mutex::new(false), Condvar::new()) });
    tauri::Builder::default()
        // Single instance: a second launch just shows the existing panel (avoids double polling / 429s).
        .plugin(tauri_plugin_single_instance::init(|app, _, _| { if let Some(w) = app.get_webview_window("main") { let _ = w.show(); } }))
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![get_state, refresh, set_mode, fit])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let w = app.get_webview_window("main").unwrap();
            let _ = w.set_visible_on_all_workspaces(true);
            place(&w, &shared.prefs.lock().unwrap());
            let _ = w.show();

            let toggle = MenuItem::with_id(app, "toggle", "Show / Hide", true, None::<&str>)?;
            let mode = MenuItem::with_id(app, "mode", "Compact / Expanded", true, None::<&str>)?;
            let refresh = MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
            let reset = MenuItem::with_id(app, "reset", "Reset position", true, None::<&str>)?;
            let showall = MenuItem::with_id(app, "showall", "Show hidden providers", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit AI Quota Pin", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &mode, &refresh, &reset, &showall, &PredefinedMenuItem::separator(app)?, &quit])?;
            let s2 = shared.clone();
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("AI Quota Pin").menu(&menu)
                .on_menu_event(move |app, e| {
                    let w = app.get_webview_window("main").unwrap();
                    match e.id.as_ref() {
                        "toggle" => { if w.is_visible().unwrap_or(false) { let _ = w.hide(); } else { let _ = w.show(); } }
                        "mode" => { let _ = app.emit("toggle-compact", ()); }
                        "refresh" => poke(&s2),
                        "reset" => { let mut p = s2.prefs.lock().unwrap(); p.right = None; p.top = None; save_prefs(&p); place(&w, &p); }
                        "showall" => { s2.prefs.lock().unwrap().hidden.clear(); save_prefs(&s2.prefs.lock().unwrap()); poke(&s2); let _ = app.emit("prefs", s2.prefs.lock().unwrap().clone()); }
                        "quit" => app.exit(0),
                        _ => {}
                    }
                })
                .build(app)?;

            let s3 = shared.clone();
            w.clone().on_window_event(move |e| if let tauri::WindowEvent::Moved(pos) = e {
                if let Ok(size) = w.outer_size() {
                    let mut p = s3.prefs.lock().unwrap();
                    let (r, t) = (Some(pos.x + size.width as i32), Some(pos.y));
                    if p.right != r || p.top != t { p.right = r; p.top = t; save_prefs(&p); }
                }
            });
            let (h, s4) = (app.handle().clone(), shared.clone());
            std::thread::Builder::new().name("poll".into()).spawn(move || poll_loop(h, s4))?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running AI Quota Pin");
}
