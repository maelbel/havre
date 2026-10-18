//! Havre Desktop: a window onto your Havre server, with what a browser tab
//! can't do — a tray icon with the unread count, new-mail notifications,
//! opening at login, and being the computer's mail app for `mailto:` links.
//!
//! How it fits together:
//! - First launch shows the bundled "Connect to your Havre" page (`ui/`).
//!   Its `connect` command checks the address and remembers it
//!   (`config.rs`), then the window opens the server's own pages. All your
//!   data stays on the server; the app keeps nothing but its address.
//! - The server's pages may call a handful of commands (`notify`,
//!   `set_unread`, `open_external`, `desktop_info`) and nothing else. That
//!   permission is given at run time to that one address
//!   (`server_capability`), because it is only known once you type it.
//! - Every other site opens in your browser (`allowed`), so sign-ins with
//!   Google or Microsoft happen where your passwords and passkeys are.
//!
//! What it teaches: Tauri's command / permission model, sharing state
//! between threads with `Mutex`, and platform differences behind `cfg!`.

mod config;

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::{DownloadEvent, NewWindowResponse};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_deep_link::DeepLinkExt as _;
use tauri_plugin_notification::NotificationExt as _;
use tauri_plugin_opener::OpenerExt as _;

/// The one window's label (capabilities refer to it by this name).
const MAIN: &str = "main";
const TRAY: &str = "tray";

/// What the app shares between its threads (commands run on a thread
/// pool, menu clicks on the main thread): hence the `Mutex`es.
#[derive(Default)]
struct Shared {
    /// The Havre server, once connected.
    server: Mutex<Option<Url>>,
    /// A `mailto:` link that arrived before a server was connected.
    pending_mailto: Mutex<Option<String>>,
    /// How many server capabilities were added (each needs its own name).
    capabilities: Mutex<u32>,
}

impl Shared {
    fn server(&self) -> Option<Url> {
        self.server.lock().unwrap().clone()
    }
}

// ---- Commands (called from JavaScript with `invoke`) ------------------------------------

/// "Connect": checks that `address` is a Havre server, remembers it and
/// opens it. Returns the address as it was understood.
#[tauri::command]
async fn connect(app: AppHandle, address: String) -> Result<String, String> {
    let url = config::normalize(&address)?;
    check_server(&url).await?;
    config::save(&app, &config::Config { server: Some(url.to_string()) })?;
    use_server(&app, url.clone());
    let pending = app.state::<Shared>().pending_mailto.lock().unwrap().take();
    let target = match pending {
        Some(mailto) => compose_url(&url, &mailto),
        None => url.clone(),
    };
    if let Some(window) = app.get_webview_window(MAIN) {
        window.navigate(target).map_err(|e| e.to_string())?;
    }
    Ok(url.to_string())
}

/// Asks the server who it is. `/api/auth/status` answers even when signed
/// out, and only Havre answers it with `signed_in`.
async fn check_server(url: &Url) -> Result<(), String> {
    // "havre.home.lan:8080", as typed (the port matters in the messages).
    let host = match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
        None => url.host_str().unwrap_or_default().to_owned(),
    };
    let status = url.join("api/auth/status").map_err(|e| e.to_string())?;
    let client =
        reqwest::Client::builder().timeout(Duration::from_secs(6)).build().map_err(|e| e.to_string())?;
    let response = client.get(status).send().await.map_err(|e| {
        if e.is_timeout() {
            format!(
                "{host} didn't answer in time. Is Havre running, and is this computer on the same network?"
            )
        } else if e.is_connect() {
            format!("Can't reach {host}. Check the address and the port (Havre's default is 8080).")
        } else {
            format!("Can't reach {host}: {e}")
        }
    })?;
    let body: serde_json::Value =
        response.json().await.map_err(|_| format!("{host} answers, but it isn't a Havre server."))?;
    if body.get("signed_in").is_none() {
        return Err(format!("{host} answers, but it isn't a Havre server."));
    }
    Ok(())
}

#[derive(Serialize)]
struct DesktopInfo {
    version: String,
    server: Option<String>,
    platform: &'static str,
}

/// Lets the web pages know they run in the app (and which version).
#[tauri::command]
fn desktop_info(app: AppHandle) -> DesktopInfo {
    DesktopInfo {
        version: app.package_info().version.to_string(),
        server: app.state::<Shared>().server().map(|u| u.to_string()),
        platform: std::env::consts::OS,
    }
}

/// A system notification ("New mail from Claire"). Lengths are capped: the
/// text comes from emails, which anyone can send you.
#[tauri::command]
fn notify(app: AppHandle, title: String, body: String) -> Result<(), String> {
    #[cfg(debug_assertions)]
    eprintln!("notify: {title} — {body}");
    let cut = |s: String, n: usize| s.chars().take(n).collect::<String>();
    app.notification().builder().title(cut(title, 120)).body(cut(body, 300)).show().map_err(|e| e.to_string())
}

/// The unread count: in the tray icon's tooltip and, where the system has
/// one, on the app's icon (macOS dock, some Linux desktops).
#[tauri::command]
fn set_unread(app: AppHandle, count: u32) {
    #[cfg(debug_assertions)]
    eprintln!("set_unread: {count}");
    let tip = match count {
        0 => "Havre".to_owned(),
        1 => "Havre · 1 unread".to_owned(),
        n => format!("Havre · {n} unread"),
    };
    if let Some(tray) = app.tray_by_id(TRAY) {
        let _ = tray.set_tooltip(Some(tip));
    }
    if let Some(window) = app.get_webview_window(MAIN) {
        // Windows has no badge: this does nothing there (the tooltip stays).
        let _ = window.set_badge_count(if count == 0 { None } else { Some(count as i64) });
    }
}

/// Opens a web address in the default browser (sign-in pages, links).
#[tauri::command]
fn open_external(app: AppHandle, url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|_| "Not a web address.".to_owned())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only web addresses can be opened.".into());
    }
    app.opener().open_url(parsed.as_str(), None::<&str>).map_err(|e| e.to_string())
}

// ---- The server ----------------------------------------------------------------------------

/// Remembers the server and lets its pages use our commands.
fn use_server(app: &AppHandle, url: Url) {
    let shared = app.state::<Shared>();
    *shared.server.lock().unwrap() = Some(url.clone());
    let mut count = shared.capabilities.lock().unwrap();
    *count += 1;
    if let Err(err) = app.add_capability(server_capability(&url, *count)) {
        eprintln!("could not allow {url}: {err}");
    }
}

/// What the server's pages may do: four commands, in the main window only.
/// (Changing server adds another; the old one is gone at the next start,
/// and the window can't open the old server anyway: see `allowed`.)
fn server_capability(url: &Url, n: u32) -> tauri::ipc::CapabilityBuilder {
    SERVER_PERMISSIONS.iter().fold(
        tauri::ipc::CapabilityBuilder::new(format!("server-{n}")).remote(server_pattern(url)).window(MAIN),
        |cap, permission| cap.permission(*permission),
    )
}

/// The commands the server's pages may call. Not `connect`: only the
/// bundled page chooses the server.
const SERVER_PERMISSIONS: [&str; 4] =
    ["allow-desktop-info", "allow-notify", "allow-set-unread", "allow-open-external"];

/// Every page of that server (an URL pattern).
fn server_pattern(url: &Url) -> String {
    format!("{}*", url.as_str())
}

/// The bundled "Connect" page's address (Windows serves app files over
/// http://tauri.localhost, the others over tauri://localhost).
fn connect_page() -> Url {
    let url =
        if cfg!(windows) { "http://tauri.localhost/index.html" } else { "tauri://localhost/index.html" };
    Url::parse(url).expect("a valid address")
}

/// `mailto:claire@x.fr?subject=Hi` → the server's Mail page, which opens a
/// new message with it (see `DesktopBridge.vue` in the web app).
fn compose_url(server: &Url, mailto: &str) -> Url {
    let mut url = server.join("mail").expect("a valid path");
    url.query_pairs_mut().append_pair("compose", mailto);
    url
}

/// Where the window may go: the bundled pages and the server. Anything
/// else (a link in an email, a sign-in page) opens in the browser.
fn allowed(app: &AppHandle, url: &Url) -> bool {
    let local = url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost");
    let inert = matches!(url.scheme(), "about" | "data" | "blob");
    if local || inert {
        return true;
    }
    if let Some(server) = app.state::<Shared>().server()
        && config::same_origin(&server, url)
    {
        return true;
    }
    if matches!(url.scheme(), "http" | "https" | "mailto") {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
    false
}

// ---- mailto: ----------------------------------------------------------------------------------

/// A `mailto:` link was opened (Havre being the mail app, or a link in a page).
fn open_mailto(app: &AppHandle, mailto: &str) {
    show(app);
    match app.state::<Shared>().server() {
        Some(server) => {
            if let Some(window) = app.get_webview_window(MAIN) {
                let _ = window.navigate(compose_url(&server, mailto));
            }
        }
        // Not connected yet: it opens once you are.
        None => *app.state::<Shared>().pending_mailto.lock().unwrap() = Some(mailto.to_owned()),
    }
}

/// "Make Havre the default mail app". The installers declare that Havre can
/// open `mailto:`; choosing it is the system's (and your) decision.
fn make_default_mail_app(app: &AppHandle) {
    if cfg!(target_os = "linux") {
        // Writes ~/.local/share/applications/… and runs `xdg-mime default`.
        let _ = app.deep_link().register("mailto");
        let _ = app.notification().builder().title("Havre").body("Havre now opens mailto: links.").show();
    } else if cfg!(windows) {
        // Windows only lets you choose in Settings › Default apps.
        let _ = app.deep_link().register("mailto");
        let _ = app.opener().open_url("ms-settings:defaultapps", None::<&str>);
    } else {
        let _ = app
            .notification()
            .builder()
            .title("Havre")
            .body("In Mail › Settings › General, choose Havre as the default email reader.")
            .show();
    }
}

// ---- Window and tray --------------------------------------------------------------------------

fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn build_window(app: &AppHandle, start: WebviewUrl) -> tauri::Result<WebviewWindow> {
    let nav = app.clone();
    let popup = app.clone();
    let window = WebviewWindowBuilder::new(app, MAIN, start)
        .title("Havre")
        .inner_size(1280.0, 820.0)
        .min_inner_size(380.0, 520.0)
        .on_navigation(move |url| allowed(&nav, url))
        // target="_blank" links and pop-ups: in the browser, never a new window.
        .on_new_window(move |url, _features| {
            // `allowed` already sent other sites to the browser; the server's
            // own pages asked for a new window go there too.
            if allowed(&popup, &url) && matches!(url.scheme(), "http" | "https") {
                let _ = popup.opener().open_url(url.as_str(), None::<&str>);
            }
            NewWindowResponse::Deny
        })
        // Attachments go to the Downloads folder, under their own name.
        .on_download(|webview, event| {
            if let DownloadEvent::Requested { url, destination } = event {
                let name = destination
                    .file_name()
                    .map(|n| n.to_os_string())
                    .or_else(|| url.path_segments().and_then(|mut s| s.next_back()).map(Into::into))
                    .unwrap_or_else(|| "attachment".into());
                if let Ok(dir) = webview.path().download_dir() {
                    *destination = dir.join(name);
                }
            }
            true
        })
        .build()?;
    // Closing the window keeps Havre in the tray (Quit is in its menu).
    let handle = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = handle.hide();
        }
    });
    Ok(window)
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let at_login = app.autolaunch().is_enabled().unwrap_or(false);
    let open = MenuItem::with_id(app, "open", "Open Havre", true, None::<&str>)?;
    let write = MenuItem::with_id(app, "write", "New message", true, None::<&str>)?;
    let login = CheckMenuItem::with_id(app, "login", "Open at login", true, at_login, None::<&str>)?;
    let default = MenuItem::with_id(app, "default", "Make Havre the default mail app", true, None::<&str>)?;
    let server = MenuItem::with_id(app, "server", "Change server…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Havre", true, None::<&str>)?;
    let sep = || PredefinedMenuItem::separator(app);
    let menu = Menu::with_items(app, &[&open, &write, &sep()?, &login, &default, &server, &sep()?, &quit])?;

    TrayIconBuilder::with_id(TRAY)
        .icon(app.default_window_icon().cloned().expect("the app has an icon"))
        .tooltip("Havre")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open" => show(app),
            "write" => open_mailto(app, "mailto:"),
            "login" => {
                let auto = app.autolaunch();
                let on = auto.is_enabled().unwrap_or(false);
                let _ = if on { auto.disable() } else { auto.enable() };
                let _ = login.set_checked(auto.is_enabled().unwrap_or(false));
            }
            "default" => make_default_mail_app(app),
            "server" => {
                show(app);
                if let Some(window) = app.get_webview_window(MAIN) {
                    let _ = window.navigate(connect_page());
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        // A left click on the icon brings the window back.
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

// ---- Start ------------------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // First: a second launch (or a mailto: link) goes to the running app.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| show(app)))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(Shared::default())
        .invoke_handler(tauri::generate_handler![connect, desktop_info, notify, set_unread, open_external])
        .setup(|app| {
            let handle = app.handle().clone();
            let server = config::load(&handle).server.and_then(|s| config::normalize(&s).ok());
            if let Some(url) = &server {
                use_server(&handle, url.clone());
            }

            // Started by a mailto: link?
            let mailto = app
                .deep_link()
                .get_current()
                .ok()
                .flatten()
                .and_then(|urls| urls.into_iter().find(|u| u.scheme() == "mailto"))
                .map(|u| u.to_string());
            let start = match (&server, &mailto) {
                (Some(s), Some(m)) => WebviewUrl::External(compose_url(s, m)),
                (Some(s), None) => WebviewUrl::External(s.clone()),
                (None, _) => {
                    *app.state::<Shared>().pending_mailto.lock().unwrap() = mailto;
                    WebviewUrl::App("index.html".into())
                }
            };
            build_window(&handle, start)?;
            build_tray(&handle)?;

            // mailto: links while running.
            let links = handle.clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    if url.scheme() == "mailto" {
                        open_mailto(&links, url.as_str());
                    }
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while starting Havre")
        .run(|_app, _event| {
            // macOS: clicking the Dock icon brings the hidden window back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                show(_app);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailto_goes_to_compose() {
        let server = Url::parse("http://localhost:8080/").unwrap();
        let url = compose_url(&server, "mailto:claire@example.com?subject=Hello%20there");
        assert_eq!(url.path(), "/mail");
        let (key, value) = url.query_pairs().next().unwrap();
        assert_eq!(key, "compose");
        assert_eq!(value, "mailto:claire@example.com?subject=Hello%20there");
    }

    #[test]
    fn the_server_may_call_four_commands() {
        let server = Url::parse("http://havre.home.lan:8080/").unwrap();
        assert_eq!(server_pattern(&server), "http://havre.home.lan:8080/*");
        assert!(!SERVER_PERMISSIONS.contains(&"allow-connect"));
        // Building it must not panic (the names are checked by Tauri).
        let _ = server_capability(&server, 1);
    }
}
