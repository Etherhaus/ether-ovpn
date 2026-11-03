mod openvpn;
mod profiles;
mod secrets;
mod tray;
mod updater;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, SharedString, VecModel};

slint::include_modules!();

const MAX_LOG_LINES: usize = 2000;
const EMPTY_NOTICE: &str = "Import a .ovpn file to get started. Download it from your OpenVPN server.";

/// Everything that reaches the UI thread from other threads.
enum Msg {
    /// An event from the openvpn session with this id.
    Vpn(u64, openvpn::Event),
    Tray(tray::Command),
    UpdateChecked { status: updater::UpdateStatus, silent: bool },
    UpdateProgress(updater::Progress),
    /// A saved password read from the keyring for the session with this id.
    SavedPassword { session: u64, req: openvpn::AuthRequest, result: Result<Option<String>, String> },
    /// A password was saved to (`saved`) or removed from the keyring.
    PasswordStored { file: PathBuf, saved: bool, result: Result<(), String> },
}

struct App {
    store: profiles::Store,
    session: Option<openvpn::Session>,
    // Bumped per connection so events from an old openvpn process are ignored.
    session_id: u64,
    active: Option<usize>,
    connected_at: Option<Instant>,
    auth: Option<openvpn::AuthRequest>,
    auth_failed: bool,
    // Passwords to save in the keyring once the connection proves they work.
    pending_save: Vec<(String, String)>,
    tx: mpsc::Sender<Msg>,
    tray: Option<ksni::blocking::Handle<tray::Tray>>,
    tray_state: tray::State,
    release: Option<updater::Release>,
    install: Option<updater::Install>,
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    // Wayland compositors find the window's icon through the desktop entry named by the app id.
    let app_id = std::env::var("FLATPAK_ID").unwrap_or_else(|_| "vpndeck".into());
    slint::set_xdg_app_id(app_id)?;
    let (tx, rx) = mpsc::channel::<Msg>();
    let tray = {
        let tx = tx.clone();
        tray::spawn(move |command| {
            let _ = tx.send(Msg::Tray(command));
        })
    };
    let app = Rc::new(RefCell::new(App {
        store: profiles::Store::load(),
        session: None,
        session_id: 0,
        active: None,
        connected_at: None,
        auth: None,
        auth_failed: false,
        pending_save: Vec::new(),
        tx: tx.clone(),
        tray,
        tray_state: tray::State::default(),
        release: None,
        install: None,
    }));

    let log_window = LogWindow::new()?;
    let profile_model = Rc::new(VecModel::<ProfileInfo>::default());
    let log_model = Rc::new(VecModel::<SharedString>::default());
    ui.set_profiles(ModelRc::from(profile_model.clone()));
    log_window.set_lines(ModelRc::from(log_model.clone()));
    ui.set_version(updater::current_version().into());
    sync_profiles(&ui, &app.borrow().store, &profile_model);
    if !app.borrow().store.profiles.is_empty() {
        ui.set_selected(0);
    }

    ui.on_show_log({
        let log_weak = log_window.as_weak();
        move || {
            let _ = log_weak.unwrap().show();
        }
    });

    // arboard serves the copied text for as long as the clipboard object lives.
    let clipboard = Rc::new(RefCell::new(None::<arboard::Clipboard>));
    log_window.on_copy_all({
        let log_model = log_model.clone();
        move || {
            let mut clipboard = clipboard.borrow_mut();
            if clipboard.is_none() {
                *clipboard = arboard::Clipboard::new().ok();
            }
            if let Some(clipboard) = clipboard.as_mut() {
                let _ = clipboard.set_text(log_text(&log_model));
            }
        }
    });

    // File dialogs run async so the event loop keeps going while they're open.
    log_window.on_save({
        let log_model = log_model.clone();
        let saving = Rc::new(Cell::new(false));
        move || {
            if saving.replace(true) {
                return;
            }
            let (log_model, saving) = (log_model.clone(), saving.clone());
            let _ = slint::spawn_local(async move {
                let file = rfd::AsyncFileDialog::new().set_title("Save log").set_file_name("vpndeck.log").save_file().await;
                saving.set(false);
                if let Some(file) = file {
                    let _ = std::fs::write(file.path(), log_text(&log_model));
                }
            });
        }
    });

    log_window.on_clear({
        let log_model = log_model.clone();
        move || log_model.set_vec(Vec::new())
    });

    ui.on_import_profile({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let profile_model = profile_model.clone();
        let picking = Rc::new(Cell::new(false));
        move || {
            if picking.replace(true) {
                return;
            }
            let (ui_weak, app, profile_model, picking) = (ui_weak.clone(), app.clone(), profile_model.clone(), picking.clone());
            let _ = slint::spawn_local(async move {
                let file = rfd::AsyncFileDialog::new()
                    .set_title("Import OpenVPN profile")
                    .add_filter("OpenVPN profile", &["ovpn", "conf"])
                    .pick_file()
                    .await;
                picking.set(false);
                let (Some(file), Some(ui)) = (file, ui_weak.upgrade()) else { return };
                let path = file.path();
                let mut app = app.borrow_mut();
                match app.store.import(path) {
                    Ok(i) => {
                        sync_profiles(&ui, &app.store, &profile_model);
                        ui.set_selected(i as i32);
                    }
                    Err(e) => ui.set_notice(format!("Could not import {}: {e}", path.display()).into()),
                }
            });
        }
    });

    ui.on_remove_profile({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let profile_model = profile_model.clone();
        move |i| {
            let ui = ui_weak.unwrap();
            let mut app = app.borrow_mut();
            let i = i as usize;
            if i >= app.store.profiles.len() || app.active == Some(i) {
                return;
            }
            match app.store.remove(i) {
                Ok(profile) if profile.saved_password => {
                    std::thread::spawn(move || secrets::forget_all(&profile.file));
                }
                Ok(_) => {}
                Err(e) => ui.set_notice(format!("Could not remove the profile: {e}").into()),
            }
            if let Some(a) = app.active.filter(|a| *a > i) {
                app.active = Some(a - 1);
            }
            ui.set_active(app.active.map_or(-1, |a| a as i32));
            ui.set_status_for(app.active.map_or(-1, |a| a as i32));
            sync_profiles(&ui, &app.store, &profile_model);
            let len = app.store.profiles.len();
            ui.set_selected(if len == 0 { -1 } else { i.min(len - 1) as i32 });
        }
    });

    ui.on_connect({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let log_model = log_model.clone();
        let tx = tx.clone();
        move |i| {
            let ui = ui_weak.unwrap();
            let mut app = app.borrow_mut();
            if app.session.is_some() {
                return;
            }
            let Some(profile) = app.store.profiles.get(i as usize).cloned() else { return };
            app.session_id += 1;
            let id = app.session_id;
            let tx = tx.clone();
            log_model.set_vec(Vec::new());
            reset_stats(&ui);
            ui.set_status_for(i);
            match openvpn::Session::start(&profile.file, profile.workdir.as_deref(), move |ev| {
                let _ = tx.send(Msg::Vpn(id, ev));
            }) {
                Ok(session) => {
                    app.session = Some(session);
                    app.active = Some(i as usize);
                    app.auth_failed = false;
                    app.pending_save.clear();
                    ui.set_active(i);
                    ui.set_conn_state(ConnState::Connecting);
                    ui.set_status_text("Waiting for administrator authorization…".into());
                }
                Err(e) => {
                    ui.set_conn_state(ConnState::Error);
                    ui.set_status_text(format!("Could not start OpenVPN: {e}").into());
                }
            }
        }
    });

    ui.on_disconnect({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move || {
            let ui = ui_weak.unwrap();
            if let Some(session) = &app.borrow().session {
                session.disconnect();
                ui.set_conn_state(ConnState::Disconnecting);
                ui.set_status_text("Disconnecting…".into());
            }
        }
    });

    ui.on_submit_auth({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move |username, password| {
            let ui = ui_weak.unwrap();
            let mut app = app.borrow_mut();
            ui.set_auth_visible(false);
            ui.set_auth_password("".into());
            let Some(req) = app.auth.take() else { return };
            if let Some(session) = &app.session {
                session.send_auth(&req, &username, &password);
            }
            let saved = app.active.and_then(|a| app.store.profiles.get(a)).filter(|p| p.saved_password).map(|p| p.file.clone());
            if ui.get_auth_remember() {
                app.pending_save.push((req.realm.clone(), password.to_string()));
            } else if let Some(file) = saved {
                forget_password(&app.tx, file);
            }
            if req.needs_username
                && let Some(profile) = app.active.and_then(|a| app.store.profiles.get_mut(a))
                && profile.username != username.as_str()
            {
                profile.username = username.to_string();
                let _ = app.store.save();
            }
            ui.set_status_text("Signing in…".into());
        }
    });

    ui.on_forget_password({
        let app = app.clone();
        move |i| {
            let app = app.borrow();
            if let Some(profile) = app.store.profiles.get(i as usize) {
                forget_password(&app.tx, profile.file.clone());
            }
        }
    });

    ui.on_cancel_auth({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move || {
            let ui = ui_weak.unwrap();
            let mut app = app.borrow_mut();
            ui.set_auth_visible(false);
            ui.set_auth_password("".into());
            app.auth = None;
            if let Some(session) = &app.session {
                session.disconnect();
                ui.set_conn_state(ConnState::Disconnecting);
                ui.set_status_text("Disconnecting…".into());
            }
        }
    });

    ui.on_check_updates({
        let ui_weak = ui.as_weak();
        let tx = tx.clone();
        move || {
            let ui = ui_weak.unwrap();
            ui.set_update_state(UpdateState::Checking);
            ui.set_update_note("".into());
            check_for_updates(tx.clone(), false);
        }
    });

    ui.on_install_update({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let tx = tx.clone();
        move || {
            let ui = ui_weak.unwrap();
            let mut app = app.borrow_mut();
            let Some(release) = app.release.clone() else { return };
            match updater::Install::detect() {
                Ok(install) => {
                    ui.set_update_state(UpdateState::Downloading);
                    ui.set_update_progress(0.0);
                    ui.set_update_text(format!("Downloading VPNdeck {}…", release.version).into());
                    let tx = tx.clone();
                    updater::spawn_install(install.clone(), release.assets, move |p| {
                        let _ = tx.send(Msg::UpdateProgress(p));
                    });
                    app.install = Some(install);
                }
                Err(e) => {
                    ui.set_update_state(UpdateState::Failed);
                    ui.set_update_text(e.into());
                }
            }
        }
    });

    ui.on_open_release_notes({
        let app = app.clone();
        move || {
            if let Some(release) = &app.borrow().release {
                updater::open_url(&release.url);
            }
        }
    });

    ui.on_restart_app({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move || {
            let ui = ui_weak.unwrap();
            let app = app.borrow();
            let Some(install) = &app.install else { return };
            if let Err(e) = install.relaunch() {
                ui.set_update_state(UpdateState::Failed);
                ui.set_update_text(e.into());
                return;
            }
            let _ = slint::quit_event_loop();
        }
    });

    ui.on_dismiss_update({
        let ui_weak = ui.as_weak();
        move || ui_weak.unwrap().set_update_state(UpdateState::Idle)
    });

    // With a tray icon, closing the window keeps vpndeck (and the tunnel) running.
    ui.window().on_close_requested({
        let app = app.clone();
        move || {
            if app.borrow().tray.is_none() {
                let _ = slint::quit_event_loop();
            }
            CloseRequestResponse::HideWindow
        }
    });

    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), {
        let ui_weak = ui.as_weak();
        let log_weak = log_window.as_weak();
        let app = app.clone();
        let log_model = log_model.clone();
        let profile_model = profile_model.clone();
        move || {
            let ui = ui_weak.unwrap();
            let log_window = log_weak.unwrap();
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    Msg::Vpn(id, event) => {
                        let mut app = app.borrow_mut();
                        if id == app.session_id {
                            handle_event(&ui, &log_window, &mut app, &log_model, event);
                        }
                    }
                    // Run through the UI's own callbacks, which borrow `app` themselves.
                    Msg::Tray(command) => handle_tray_command(&ui, command),
                    Msg::UpdateChecked { status, silent } => handle_update_status(&ui, &mut app.borrow_mut(), status, silent),
                    Msg::UpdateProgress(progress) => handle_update_progress(&ui, progress),
                    Msg::SavedPassword { session, req, result } => {
                        let mut app = app.borrow_mut();
                        if session == app.session_id {
                            use_saved_password(&ui, &mut app, req, result);
                        }
                    }
                    Msg::PasswordStored { file, saved, result } => {
                        let mut app = app.borrow_mut();
                        match result {
                            Ok(()) => {
                                if let Some(profile) = app.store.profiles.iter_mut().find(|p| p.file == file) {
                                    profile.saved_password = saved;
                                }
                                let _ = app.store.save();
                                sync_profiles(&ui, &app.store, &profile_model);
                            }
                            Err(e) => {
                                let action = if saved { "save" } else { "forget" };
                                ui.set_notice(format!("Couldn't {action} the password in your keyring: {e}").into());
                            }
                        }
                    }
                }
            }
            let mut app = app.borrow_mut();
            if let Some(since) = app.connected_at {
                ui.set_duration(format_duration(since.elapsed()).into());
            }
            sync_tray(&ui, &mut app);
        }
    });

    ui.show()?;
    check_for_updates(tx.clone(), true);
    slint::run_event_loop_until_quit()?;

    // Nothing is left to control the tunnel once vpndeck exits, so take it down.
    if let Some(session) = &app.borrow().session {
        session.disconnect();
    }
    Ok(())
}

/// Whether vpndeck runs inside a Flatpak sandbox, where host programs and
/// files have to be reached through `flatpak-spawn --host` and /run/host.
pub fn sandboxed() -> bool {
    std::env::var_os("FLATPAK_ID").is_some()
}

fn check_for_updates(tx: mpsc::Sender<Msg>, silent: bool) {
    updater::spawn_check(move |status| {
        let _ = tx.send(Msg::UpdateChecked { status, silent });
    });
}

fn handle_tray_command(ui: &AppWindow, command: tray::Command) {
    match command {
        tray::Command::ShowWindow => {
            let _ = ui.show();
            ui.window().set_minimized(false);
        }
        tray::Command::Connect(i) => {
            ui.set_selected(i as i32);
            ui.invoke_connect(i as i32);
        }
        tray::Command::Disconnect => ui.invoke_disconnect(),
        tray::Command::CheckUpdates => {
            let _ = ui.show();
            ui.invoke_check_updates();
        }
        tray::Command::Quit => {
            let _ = slint::quit_event_loop();
        }
    }
}

fn handle_update_status(ui: &AppWindow, app: &mut App, status: updater::UpdateStatus, silent: bool) {
    use updater::UpdateStatus;
    // A silent check at startup must not clobber an install that's already under way.
    if silent && ui.get_update_state() != UpdateState::Idle {
        return;
    }
    match status {
        UpdateStatus::Available(release) => {
            ui.set_update_state(UpdateState::Available);
            ui.set_update_note("".into());
            let password = match updater::Install::detect() {
                Ok(install) if install.asks_for_password() => " You'll be asked for your password.",
                _ => "",
            };
            ui.set_update_text(
                format!("VPNdeck {} is available (you have {}).{password}", release.version, updater::current_version()).into(),
            );
            app.release = Some(release);
        }
        UpdateStatus::UpToDate => {
            ui.set_update_state(UpdateState::Idle);
            if !silent {
                ui.set_update_note("You're up to date.".into());
            }
        }
        UpdateStatus::Failed(e) => {
            ui.set_update_state(UpdateState::Idle);
            if !silent {
                ui.set_update_note(format!("Couldn't check: {e}").into());
            }
        }
    }
}

fn handle_update_progress(ui: &AppWindow, progress: updater::Progress) {
    use updater::Progress;
    match progress {
        Progress::Downloading(fraction) => ui.set_update_progress(fraction),
        Progress::Installing => {
            ui.set_update_state(UpdateState::Installing);
            ui.set_update_text("Installing the update…".into());
        }
        Progress::Done => {
            ui.set_update_state(UpdateState::Done);
            let note = if ui.get_active() >= 0 { " Restarting will disconnect the VPN." } else { "" };
            ui.set_update_text(format!("The update is installed. Restart VPNdeck to start using it.{note}").into());
        }
        Progress::Failed(e) => {
            ui.set_update_state(UpdateState::Failed);
            ui.set_update_text(format!("The update failed: {e}").into());
        }
    }
}

/// Pushes the current status to the tray icon when anything it shows has changed.
fn sync_tray(ui: &AppWindow, app: &mut App) {
    let Some(handle) = &app.tray else { return };
    let busy = app.session.is_some();
    let status = match ui.get_conn_state() {
        ConnState::Connected => tray::Status::Connected,
        ConnState::Connecting | ConnState::Disconnecting => tray::Status::Connecting,
        ConnState::Error => tray::Status::Error,
        ConnState::Idle => tray::Status::Idle,
    };
    let profile_name = app.active.and_then(|a| app.store.profiles.get(a)).map(|p| p.name.as_str());
    let status_text = match (status, profile_name) {
        (tray::Status::Connected, Some(name)) => format!("Connected to {name}"),
        (tray::Status::Idle, _) => "Disconnected".to_string(),
        _ => ui.get_status_text().to_string(),
    };
    let update = match ui.get_update_state() {
        UpdateState::Available => app.release.as_ref().map(|r| r.version.clone()),
        _ => None,
    };
    let state = tray::State {
        status,
        status_text,
        profiles: app.store.profiles.iter().map(|p| p.name.clone()).collect(),
        busy,
        update,
    };
    if state != app.tray_state {
        tray::set_state(handle, state.clone());
        app.tray_state = state;
    }
}

fn handle_event(ui: &AppWindow, log_window: &LogWindow, app: &mut App, log_model: &VecModel<SharedString>, event: openvpn::Event) {
    use openvpn::Event;
    match event {
        Event::Log(line) => {
            if log_model.row_count() >= MAX_LOG_LINES {
                log_model.remove(0);
            }
            log_model.push(line.into());
            log_window.set_count(log_window.get_count() + 1);
        }
        Event::State { name, local_ip, remote } => {
            if !local_ip.is_empty() {
                ui.set_local_ip(local_ip.into());
            }
            if !remote.is_empty() {
                ui.set_remote(remote.into());
            }
            if ui.get_conn_state() == ConnState::Disconnecting {
                return;
            }
            let (state, text) = match name.as_str() {
                "CONNECTED" => {
                    app.connected_at.get_or_insert_with(Instant::now);
                    let file = app.active.and_then(|a| app.store.profiles.get(a)).map(|p| p.file.clone());
                    if let Some(file) = file {
                        save_passwords(&app.tx, file, std::mem::take(&mut app.pending_save));
                    }
                    (ConnState::Connected, "Connected".to_string())
                }
                "RECONNECTING" => {
                    app.connected_at = None;
                    (ConnState::Connecting, "Reconnecting…".to_string())
                }
                "EXITING" => (ConnState::Disconnecting, "Disconnecting…".to_string()),
                other => (ConnState::Connecting, format!("Connecting… ({})", other.to_lowercase().replace('_', " "))),
            };
            ui.set_conn_state(state);
            ui.set_status_text(text.into());
        }
        Event::Bytes { rx, tx } => {
            ui.set_rx(format_bytes(rx).into());
            ui.set_tx(format_bytes(tx).into());
        }
        Event::NeedAuth(req) => {
            let profile = app.active.and_then(|a| app.store.profiles.get(a));
            // After a failed attempt the saved password is evidently wrong, so ask instead.
            if let Some(file) = profile.filter(|p| p.saved_password && !app.auth_failed).map(|p| p.file.clone()) {
                ui.set_status_text("Signing in with your saved password…".into());
                let (tx, session) = (app.tx.clone(), app.session_id);
                std::thread::spawn(move || {
                    let result = secrets::load(&file, &req.realm);
                    let _ = tx.send(Msg::SavedPassword { session, req, result });
                });
                return;
            }
            let error = match (app.auth_failed, profile.is_some_and(|p| p.saved_password)) {
                (true, true) => "Your saved password didn't work. Enter it again.",
                (true, false) => "That didn't work. Check your username and password and try again.",
                _ => "",
            };
            show_auth_dialog(ui, app, req, error);
        }
        Event::AuthFailed => {
            app.auth_failed = true;
            app.pending_save.clear();
        }
        Event::Exited(error) => {
            app.session = None;
            app.active = None;
            app.connected_at = None;
            app.auth = None;
            ui.set_active(-1);
            ui.set_auth_visible(false);
            reset_stats(ui);
            match error {
                Some(message) => {
                    ui.set_conn_state(ConnState::Error);
                    ui.set_status_text(message.into());
                }
                None => {
                    ui.set_conn_state(ConnState::Idle);
                    ui.set_status_text("Disconnected".into());
                }
            }
        }
    }
}

fn show_auth_dialog(ui: &AppWindow, app: &mut App, req: openvpn::AuthRequest, error: &str) {
    let title = if req.needs_username {
        "Sign in to the VPN".to_string()
    } else {
        format!("Enter the {} password", req.realm.to_lowercase())
    };
    let profile = app.active.and_then(|a| app.store.profiles.get(a));
    ui.set_auth_title(title.into());
    ui.set_auth_needs_username(req.needs_username);
    ui.set_auth_username(profile.map(|p| p.username.clone()).unwrap_or_default().into());
    ui.set_auth_remember(profile.is_some_and(|p| p.saved_password));
    ui.set_auth_error(error.into());
    ui.set_auth_visible(true);
    ui.set_status_text("Waiting for your credentials…".into());
    // The window may be hidden in the tray; the prompt needs to be seen.
    let _ = ui.show();
    app.auth = Some(req);
}

fn use_saved_password(ui: &AppWindow, app: &mut App, req: openvpn::AuthRequest, result: Result<Option<String>, String>) {
    let username = app.active.and_then(|a| app.store.profiles.get(a)).map(|p| p.username.clone()).unwrap_or_default();
    match result {
        Ok(Some(password)) if !req.needs_username || !username.is_empty() => {
            if let Some(session) = &app.session {
                session.send_auth(&req, &username, &password);
            }
            ui.set_status_text("Signing in…".into());
        }
        Ok(_) => show_auth_dialog(ui, app, req, ""),
        Err(e) => show_auth_dialog(ui, app, req, &format!("Couldn't read your saved password: {e}")),
    }
}

fn save_passwords(tx: &mpsc::Sender<Msg>, file: PathBuf, passwords: Vec<(String, String)>) {
    if passwords.is_empty() {
        return;
    }
    let tx = tx.clone();
    std::thread::spawn(move || {
        let result = passwords.iter().try_for_each(|(realm, password)| secrets::save(&file, realm, password));
        let _ = tx.send(Msg::PasswordStored { file, saved: true, result });
    });
}

fn forget_password(tx: &mpsc::Sender<Msg>, file: PathBuf) {
    let tx = tx.clone();
    std::thread::spawn(move || {
        let result = secrets::forget_all(&file);
        let _ = tx.send(Msg::PasswordStored { file, saved: false, result });
    });
}

fn sync_profiles(ui: &AppWindow, store: &profiles::Store, model: &VecModel<ProfileInfo>) {
    model.set_vec(
        store
            .profiles
            .iter()
            .map(|p| ProfileInfo { name: p.name.as_str().into(), saved_password: p.saved_password })
            .collect::<Vec<_>>(),
    );
    ui.set_notice(if store.profiles.is_empty() { EMPTY_NOTICE.into() } else { "".into() });
}

fn log_text(model: &VecModel<SharedString>) -> String {
    model.iter().map(|line| format!("{line}\n")).collect()
}

fn reset_stats(ui: &AppWindow) {
    for set in [AppWindow::set_local_ip, AppWindow::set_remote, AppWindow::set_rx, AppWindow::set_tx, AppWindow::set_duration] {
        set(ui, "—".into());
    }
}

fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{n} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}
