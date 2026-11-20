// Annotate Console — harness-agnostic UI annotations (Rust + Tauri).
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod app;
mod batch;
mod bridge;
mod capture;
mod commands;
mod control;
mod deliver;
mod flow;
mod geometry;
mod harness;
mod integrations;
mod mcp;
mod model;
mod paste;
mod paths;
mod platform;
mod receiver;
mod settings;
mod shortcut;

use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Manager, RunEvent, WindowEvent};

const HELP: &str =
    "Annotate — whole-app screenshots, highlighted areas and notes for any AI harness

Usage:
  annotate                         Open the Annotate Console
  annotate open [--target ID] [--message TEXT]
                                   Open (or retarget) the Console
  annotate capture                 Start a capture (for compositor keybindings)
  annotate quit                    Close the Console, discarding unsent drafts
  annotate wait [--project DIR] [--timeout SECS] [--harness ID] [--no-open]
                                   Register this terminal as a destination, wait for
                                   the user to Send, print the request (exit 2 on timeout)
  annotate mcp                     Run the MCP server (stdio)
  annotate harnesses [--json]      List destinations detected on this machine
  annotate integrations [list | install ID | uninstall ID | snippet]
                                   Manage harness integrations
  annotate --version
";

fn attach_console() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn AttachConsole(process: u32) -> i32;
        }
        unsafe {
            AttachConsole(u32::MAX);
        }
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("gui");
    if command != "gui" && !command.starts_with("-psn") {
        attach_console();
    }
    let code = match command {
        "gui" => {
            gui();
            0
        }
        _ if command.starts_with("-psn") => {
            gui();
            0
        }
        "--version" | "-V" | "version" => {
            println!("annotate {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            0
        }
        "open" => {
            let mut body = json!({});
            if let Some(t) = flag(&args, "--target") {
                body["target"] = json!(t);
            }
            if let Some(m) = flag(&args, "--message") {
                body["message"] = json!(m);
            }
            report(control::send_or_launch("/open", &body))
        }
        "capture" => report(control::send_or_launch("/capture", &json!({}))),
        "quit" => {
            if control::running() {
                report(control::send("/quit", &json!({})))
            } else {
                0
            }
        }
        "mcp" => {
            harness::adopt_login_path();
            report(mcp::run().map(|_| Value::Null))
        }
        "wait" => wait(&args),
        "harnesses" => {
            harness::adopt_login_path();
            let mut all: Vec<harness::Destination> = harness::live_sessions()
                .into_iter()
                .map(|(_, d)| d)
                .collect();
            all.extend(harness::static_destinations());
            if args.iter().any(|a| a == "--json") {
                println!("{}", serde_json::to_string_pretty(&all).unwrap());
            } else {
                for d in all {
                    let place = if d.project.is_empty() {
                        String::new()
                    } else {
                        format!("  {}", d.project)
                    };
                    println!("{:<10} {:<34} {}{}", d.group, d.label, d.id, place);
                }
            }
            0
        }
        "integrations" => {
            harness::adopt_login_path();
            match args.get(1).map(String::as_str).unwrap_or("list") {
                "list" => {
                    for i in integrations::list() {
                        println!(
                            "{:<14} {:<10} {:<28} {}",
                            i.id,
                            if i.installed {
                                "installed"
                            } else if i.available {
                                "available"
                            } else {
                                "-"
                            },
                            i.name,
                            i.description
                        );
                    }
                    0
                }
                "install" => report(
                    args.get(2)
                        .ok_or("Usage: annotate integrations install ID".to_string())
                        .and_then(|id| integrations::install(id))
                        .map(Value::String),
                ),
                "uninstall" | "remove" => report(
                    args.get(2)
                        .ok_or("Usage: annotate integrations uninstall ID".to_string())
                        .and_then(|id| integrations::uninstall(id))
                        .map(Value::String),
                ),
                "snippet" => {
                    println!("{}", integrations::mcp_snippet());
                    0
                }
                other => {
                    eprintln!("Unknown integrations command: {other}");
                    1
                }
            }
        }
        other => {
            eprintln!("Unknown command: {other}\n\n{HELP}");
            1
        }
    };
    std::process::exit(code);
}

fn report(result: Result<Value, String>) -> i32 {
    match result {
        Ok(Value::String(s)) => {
            println!("{s}");
            0
        }
        Ok(_) => 0,
        Err(e) => {
            eprintln!("annotate: {e}");
            1
        }
    }
}

/// `annotate wait`: a temporary destination for any harness with a shell.
fn wait(args: &[String]) -> i32 {
    let project = flag(args, "--project")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let project = project.canonicalize().unwrap_or(project);
    let timeout = flag(args, "--timeout")
        .and_then(|t| t.parse::<u64>().ok())
        .unwrap_or(900)
        .min(86_400);
    let harness_id = flag(args, "--harness").unwrap_or_else(|| "cli".into());
    let name = harness::harness_name(&harness_id);
    let receiver = match receiver::Receiver::start(
        &harness_id,
        &name,
        &receiver::project_for(&project),
        "annotate wait",
        json!([{ "id": "return", "label": "Return to waiting agent" }]),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("annotate: {e}");
            return 1;
        }
    };
    if !args.iter().any(|a| a == "--no-open") {
        if let Err(e) = control::send_or_launch("/open", &json!({ "target": receiver.record.id })) {
            eprintln!("annotate: {e}");
            return 1;
        }
        eprintln!("Annotate Console is open and aimed at this session. Waiting for the user to press Send…");
    }
    let packages = receiver.take(Duration::from_secs(timeout));
    if packages.is_empty() {
        eprintln!("No annotations were sent within {timeout} seconds.");
        return 2;
    }
    for p in packages {
        println!("{}", p.text);
        for item in &p.items {
            println!("\n{}", item.text);
        }
        println!("\nAll files: {} (ANNOTATIONS.md, annotations.json, N.png). View each screenshot before acting.", p.saved_to);
    }
    0
}

// ---------------------------------------------------------------- GUI

fn gui() {
    if control::running() {
        let _ = control::send("/open", &json!({}));
        return;
    }
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(commands::handler())
        .setup(|tauri_app| {
            let handle = tauri_app.handle().clone();
            let config = tauri_app
                .path()
                .app_config_dir()
                .map_err(|e| e.to_string())?;
            let settings_file = config.join("settings.json");
            let bounds_file = config.join("console-size.json");
            let settings = settings::load(&settings_file);
            let default_shortcut = std::env::var("ANNOTATE_HOTKEY")
                .or_else(|_| std::env::var("PI_ANNOTATE_HOTKEY"))
                .ok()
                .filter(|s| settings::validate_shortcut(s).is_ok_and(|v| !v.is_empty()))
                .unwrap_or_else(|| "CommandOrControl+Shift+A".into());
            let inner = app::Inner {
                capture_kind: settings.mark_tool.clone(),
                destination_id: settings.destination.clone(),
                settings,
                settings_file,
                bounds_file: bounds_file.clone(),
                drafts: vec![],
                selected_id: None,
                batch_id: uuid::Uuid::new_v4().to_string(),
                sending: false,
                capturing: false,
                saving: false,
                quitting: false,
                frozen: None,
                notice: String::new(),
                detail_id: None,
                pill_mode: "collapsed".into(),
                statics: vec![],
                live: vec![],
                attempts: Default::default(),
                shortcut: Default::default(),
                selection: None,
                cancel: None,
                console: Default::default(),
                default_shortcut,
                confirming: false,
            };
            tauri_app.manage(Arc::new(app::Shared {
                inner: Mutex::new(inner),
            }));

            // Initial Console bounds: user-resized timeline bounds persist; the pill starts collapsed.
            let areas = app::logical_areas(&handle);
            let (stored, stored_layout) = app::load_bounds(&bounds_file);
            let layout = app::lock(&handle).settings.console_layout.clone();
            let initial = if stored_layout.as_deref() == Some("timeline") && layout == "timeline" {
                geometry::console_bounds(stored, &areas, geometry::TIMELINE_MIN).ok()
            } else {
                geometry::layout_bounds(
                    stored,
                    &areas,
                    &layout,
                    "collapsed",
                    stored_layout.is_none(),
                )
                .ok()
            };
            let initial = initial.unwrap_or(model::Rect::new(100.0, 100.0, 320.0, 600.0));
            app::lock(&handle).console.bounds = Some(initial);
            let window = app::open_window(
                &handle,
                app::WindowSpec {
                    label: app::TOOLBAR,
                    page: "toolbar",
                    title: "Annotate Console",
                    bounds: initial,
                    canonical: false,
                    transparent: true,
                    focus: true,
                    resizable: layout != "pill",
                    passive: false,
                    visible: false,
                    taskbar: true,
                },
            )?;
            let _ = window;
            app::show_console(&handle);
            shortcut::init(&handle);
            start_control(&handle)?;
            start_discovery(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            let app = window.app_handle().clone();
            match (window.label(), event) {
                (app::TOOLBAR, WindowEvent::Moved(_) | WindowEvent::Resized(_)) => {
                    app::schedule_settle(&app)
                }
                (app::TOOLBAR, WindowEvent::CloseRequested { api, .. }) => {
                    api.prevent_close();
                    tauri::async_runtime::spawn(async move {
                        let _ = commands::quit(app).await;
                    });
                }
                (app::NOTE | app::OVERLAY, WindowEvent::CloseRequested { .. }) => {
                    flow::finish_capture(
                        &app,
                        Some("Capture cancelled. Existing annotations were kept.".into()),
                    );
                }
                (app::POPUP, WindowEvent::Destroyed) => {
                    app::lock(&app).detail_id = None;
                    app::changed(&app);
                }
                (_, WindowEvent::ScaleFactorChanged { .. }) => flow::displays_changed(&app),
                _ => {}
            }
        });
    let app = match builder.build(tauri::generate_context!()) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("Annotate Console failed to start: {e}");
            std::process::exit(1);
        }
    };
    app.run(|app, event| match event {
        RunEvent::ExitRequested {
            api, code: None, ..
        } => {
            // Cmd+Q / last window: confirm before discarding drafts.
            api.prevent_exit();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = commands::quit(app).await;
            });
        }
        RunEvent::Exit => {
            shortcut::release_all(app);
            if let Some(token) = app.try_state::<ControlToken>() {
                control::remove(&token.0);
            }
        }
        _ => {}
    });
}

struct ControlToken(String, #[allow(dead_code)] bridge::Server);

fn start_control(handle: &tauri::AppHandle) -> Result<(), String> {
    let app = handle.clone();
    let server = bridge::serve(64 * 1024, Arc::new(move |req| handle_control(&app, req)))?;
    control::write(&control::AppRecord {
        url: server.url.clone(),
        token: server.token.clone(),
        pid: std::process::id(),
    })?;
    let token = server.token.clone();
    handle.manage(ControlToken(token, server));
    Ok(())
}

fn handle_control(app: &tauri::AppHandle, req: bridge::Request) -> Result<Value, (u16, String)> {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/status") => Ok(json!({ "ok": true, "pid": std::process::id() })),
        ("POST", "/open") => {
            if let Some(target) = req.body["target"].as_str() {
                let id = if target.contains(':') {
                    target.to_string()
                } else {
                    format!("session:{target}")
                };
                // Pick up the new session immediately instead of waiting for the next poll.
                if !app::lock(app).destinations().iter().any(|d| d.id == id) {
                    let live = harness::live_sessions();
                    app::lock(app).live = live;
                }
                let mut inner = app::lock(app);
                if inner.destinations().iter().any(|d| d.id == id) {
                    if inner.busy() && inner.sending {
                        return Err((409, "A batch is being sent; retry shortly".into()));
                    }
                    inner.destination_id = id.clone();
                    let mut next = inner.settings.clone();
                    next.destination = id;
                    let _ = inner.save_settings(next);
                }
            }
            let message = req.body["message"]
                .as_str()
                .map(|m| m.chars().take(300).collect::<String>());
            let a = app.clone();
            let _ = app.run_on_main_thread(move || {
                let busy = { let inner = app::lock(&a); inner.capturing || inner.frozen.is_some() || inner.selection.is_some() };
                if !busy {
                    app::show_console(&a);
                }
                if let Some(m) = message {
                    app::status(&a, m);
                } else {
                    app::status(&a, "Annotate Console is aimed at the selected destination. Existing annotations were kept.");
                }
                app::changed(&a);
            });
            Ok(json!({ "ok": true }))
        }
        ("POST", "/capture") => {
            let a = app.clone();
            let _ = app.run_on_main_thread(move || shortcut::external_trigger(&a));
            Ok(json!({ "ok": true }))
        }
        ("POST", "/quit") => {
            let a = app.clone();
            let _ = app.run_on_main_thread(move || flow::quit_now(&a));
            Ok(json!({ "ok": true }))
        }
        _ => Err((404, "Not found".into())),
    }
}

/// Polls live sessions (fast) and installed harnesses (slow), choosing a default destination.
fn start_discovery(handle: &tauri::AppHandle) {
    let app = handle.clone();
    std::thread::spawn(move || {
        // Dock-launched apps get a minimal PATH; adopt the login shell's first.
        harness::adopt_login_path();
        harness::rescan();
        let statics = harness::static_destinations();
        app::lock(&app).statics = statics;
        let mut tick = 0u64;
        loop {
            if app::lock(&app).quitting {
                return;
            }
            if tick > 0 && tick % 40 == 0 {
                harness::rescan();
                let statics = harness::static_destinations();
                app::lock(&app).statics = statics;
            }
            let live = harness::live_sessions();
            let changed = {
                let mut inner = app::lock(&app);
                let before = (
                    inner
                        .live
                        .iter()
                        .map(|(_, d)| d.clone())
                        .collect::<Vec<_>>(),
                    inner.destination_id.clone(),
                    inner.statics.len(),
                );
                inner.live = live;
                if inner.destination_id.is_empty()
                    || (!inner.destination_id.starts_with("session:")
                        && inner.destination().is_none())
                {
                    if let Some(first) = inner.destinations().first() {
                        inner.destination_id = first.id.clone();
                    }
                }
                let after = (
                    inner
                        .live
                        .iter()
                        .map(|(_, d)| d.clone())
                        .collect::<Vec<_>>(),
                    inner.destination_id.clone(),
                    inner.statics.len(),
                );
                before != after || tick == 0
            };
            if changed {
                app::changed(&app);
            }
            tick += 1;
            std::thread::sleep(Duration::from_millis(1500));
        }
    });
}
