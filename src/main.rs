mod autostart;
mod i18n;
mod notify;
mod openvpn;
mod preflight;
mod profiles;
mod secrets;
mod settings;
mod theme;
mod tray;
mod updater;

use std::cell::{Cell, RefCell};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, SharedString, VecModel};

use i18n::{Lang, Strings};

slint::include_modules!();

const MAX_LOG_LINES: usize = 2000;
/// How many traffic samples the sparkline keeps.
const MAX_SAMPLES: usize = 40;
const RECONNECT_DELAY: Duration = Duration::from_secs(5);
const MAX_RECONNECTS: u8 = 3;
const TOAST_TIME: Duration = Duration::from_secs(4);
const TOAST_ERROR_TIME: Duration = Duration::from_secs(7);
/// A profile fetched from a link is a text file; anything bigger isn't one.
const MAX_PROFILE_BYTES: u64 = 1024 * 1024;

/// Everything that reaches the UI thread from other threads.
enum Msg {
    /// An event from the openvpn session with this id.
    Vpn(u64, openvpn::Event),
    Tray(tray::Command),
    UpdateChecked {
        status: updater::UpdateStatus,
        silent: bool,
    },
    UpdateProgress(updater::Progress),
    /// A saved password read from the keyring for the session with this id.
    SavedPassword {
        session: u64,
        req: openvpn::AuthRequest,
        result: Result<Option<String>, String>,
    },
    /// A password was saved to (`saved`) or removed from the keyring.
    PasswordStored {
        file: PathBuf,
        saved: bool,
        result: Result<(), String>,
    },
    /// The `pkexec kill` fallback for a session whose management connection is
    /// gone. `Ok` means the signal was sent; the process still has to exit.
    Killed(Result<(), String>),
    /// A profile downloaded from a link, ready to import.
    Fetched {
        name: String,
        result: Result<String, String>,
    },
}

struct App {
    store: profiles::Store,
    settings: settings::Settings,
    /// What the profile search box currently holds.
    query: String,
    session: Option<openvpn::Session>,
    // Bumped per connection so events from an old openvpn process are ignored.
    session_id: u64,
    active: Option<usize>,
    connected_at: Option<Instant>,
    auth: Option<openvpn::AuthRequest>,
    auth_failed: bool,
    /// A password read from the keyring, kept only while the 2FA code is typed.
    saved_password: Option<String>,
    /// A crtext challenge waiting for its response.
    cr_challenge: Option<openvpn::Challenge>,
    // Passwords to save in the keyring once the connection proves they work.
    pending_save: Vec<(String, String)>,
    tx: mpsc::Sender<Msg>,
    tray: Option<ksni::blocking::Handle<tray::Tray>>,
    tray_state: tray::State,
    release: Option<updater::Release>,
    install: Option<updater::Install>,
    /// The palette in use, resolved from the theme setting.
    dark: bool,
    /// When the toast should disappear.
    toast_until: Option<Instant>,
    /// A reconnect waiting for its delay, with the attempt number.
    retry: Option<(Instant, u8)>,
    /// The profile the retry belongs to.
    retry_profile: Option<usize>,
    /// Whether the last connection reached CONNECTED, so a drop is worth retrying.
    was_connected: bool,
    /// Traffic samples for the sparkline, oldest first.
    samples: Vec<f32>,
    last_bytes: (u64, u64),
    last_sample: Instant,
    /// Every log line, so the filter can rebuild the visible list.
    log_all: Vec<LogLine>,
    log_filter: String,
    /// Whether the "still running in the tray" hint was already shown.
    tray_hint_shown: bool,
    /// The profile the profile window is showing, so its check knows what to look at.
    viewing: Option<usize>,
}

/// A window that is built the first time it is opened.
///
/// On Wayland the compositor maps a Slint window as soon as it exists, so a
/// window created up front shows up in the dock even when it is never opened.
struct LazyWindow<T> {
    window: RefCell<Option<T>>,
    build: Box<dyn Fn() -> Result<T, slint::PlatformError>>,
}

impl<T: ComponentHandle> LazyWindow<T> {
    fn new(build: impl Fn() -> Result<T, slint::PlatformError> + 'static) -> Rc<Self> {
        Rc::new(Self {
            window: RefCell::new(None),
            build: Box::new(build),
        })
    }

    /// The window, built on the first call.
    fn open(&self) -> Option<T> {
        if let Some(window) = self.window.borrow().as_ref() {
            return Some(window.clone_strong());
        }
        match (self.build)() {
            Ok(window) => {
                *self.window.borrow_mut() = Some(window.clone_strong());
                Some(window)
            }
            Err(e) => {
                eprintln!("ether-ovpn: cannot create a window: {e}");
                None
            }
        }
    }

    /// The window if it was already opened, without building it.
    fn opened(&self) -> Option<T> {
        self.window.borrow().as_ref().map(|window| window.clone_strong())
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    // Wayland compositors find the window's icon through the desktop entry named by the app id.
    let app_id = std::env::var("FLATPAK_ID").unwrap_or_else(|_| "ether-ovpn".into());
    slint::set_xdg_app_id(app_id)?;
    let (tx, rx) = mpsc::channel::<Msg>();
    let tray = {
        let tx = tx.clone();
        tray::spawn(move |command| {
            let _ = tx.send(Msg::Tray(command));
        })
    };
    let mut settings = settings::Settings::load();
    // The autostart entry is the source of truth: the user may have removed it
    // from the desktop's own startup settings.
    settings.autostart = autostart::enabled();
    let dark = settings.dark();
    let (store, store_error) = profiles::Store::load();
    let app = Rc::new(RefCell::new(App {
        store,
        settings,
        query: String::new(),
        session: None,
        session_id: 0,
        active: None,
        connected_at: None,
        auth: None,
        auth_failed: false,
        saved_password: None,
        cr_challenge: None,
        pending_save: Vec::new(),
        tx: tx.clone(),
        tray,
        tray_state: tray::State::default(),
        release: None,
        install: None,
        dark,
        toast_until: None,
        retry: None,
        retry_profile: None,
        was_connected: false,
        samples: Vec::new(),
        last_bytes: (0, 0),
        last_sample: Instant::now(),
        log_all: Vec::new(),
        log_filter: String::new(),
        tray_hint_shown: false,
        viewing: None,
    }));

    let profile_model = Rc::new(VecModel::<ProfileInfo>::default());
    let log_model = Rc::new(VecModel::<LogLine>::default());
    let sample_model = Rc::new(VecModel::<f32>::default());
    let profile_line_model = Rc::new(VecModel::<ProfileLine>::default());
    // arboard serves the copied text for as long as the clipboard object lives.
    let clipboard = Rc::new(RefCell::new(None::<arboard::Clipboard>));

    let log_window = LazyWindow::new({
        let app = app.clone();
        let log_model = log_model.clone();
        let clipboard = clipboard.clone();
        let ui_weak = ui.as_weak();
        move || {
            let window = LogWindow::new()?;
            {
                let app = app.borrow();
                apply_theme(&window, app.dark);
                i18n::apply(&window, app.settings.lang());
                window.set_frame(app.settings.frame);
            }
            window.set_lines(ModelRc::from(log_model.clone()));
            window.on_copy_all({
                let app = app.clone();
                let clipboard = clipboard.clone();
                move || {
                    let text = log_text(&app.borrow().log_all);
                    let mut clipboard = clipboard.borrow_mut();
                    if clipboard.is_none() {
                        *clipboard = arboard::Clipboard::new().ok();
                    }
                    if let Some(clipboard) = clipboard.as_mut() {
                        let _ = clipboard.set_text(text);
                    }
                }
            });
            window.on_copy_diagnostics({
                let ui_weak = ui_weak.clone();
                let app = app.clone();
                let clipboard = clipboard.clone();
                move || {
                    let Some(ui) = ui_weak.upgrade() else { return };
                    let mut app = app.borrow_mut();
                    let text = diagnostics(&app);
                    let mut clipboard = clipboard.borrow_mut();
                    if clipboard.is_none() {
                        *clipboard = arboard::Clipboard::new().ok();
                    }
                    let copied = clipboard
                        .as_mut()
                        .is_some_and(|clipboard| clipboard.set_text(text).is_ok());
                    let t = Strings::new(app.settings.lang());
                    let message = if copied {
                        t.diagnostics_copied
                    } else {
                        t.diagnostics_failed
                    };
                    toast(&ui, &mut app, message, !copied);
                }
            });
            window.on_filter_changed({
                let window_weak = window.as_weak();
                let app = app.clone();
                let log_model = log_model.clone();
                move |query| {
                    let Some(window) = window_weak.upgrade() else { return };
                    let mut app = app.borrow_mut();
                    app.log_filter = query.to_string();
                    rebuild_log(&window, &log_model, &app);
                }
            });
            window.on_save({
                let ui_weak = ui_weak.clone();
                let app = app.clone();
                let saving = Rc::new(Cell::new(false));
                move || {
                    if saving.replace(true) {
                        return;
                    }
                    let title = Strings::new(app.borrow().settings.lang()).save_log_title;
                    let text = log_text(&app.borrow().log_all);
                    let saving = saving.clone();
                    let ui_weak = ui_weak.clone();
                    let app = app.clone();
                    let _ = slint::spawn_local(async move {
                        let file = rfd::AsyncFileDialog::new()
                            .set_title(title)
                            .set_file_name("ether-ovpn.log")
                            .save_file()
                            .await;
                        saving.set(false);
                        let Some(file) = file else { return };
                        if let Err(e) = std::fs::write(file.path(), text) {
                            let Some(ui) = ui_weak.upgrade() else { return };
                            let mut app = app.borrow_mut();
                            let t = Strings::new(app.settings.lang());
                            toast(&ui, &mut app, i18n::fill(t.log_save_failed, &[&e.to_string()]), true);
                        }
                    });
                }
            });
            window.on_clear({
                let log_model = log_model.clone();
                let app = app.clone();
                move || {
                    let mut app = app.borrow_mut();
                    app.log_all.clear();
                    log_model.set_vec(Vec::new());
                }
            });
            window.on_close_window({
                let window_weak = window.as_weak();
                move || {
                    if let Some(window) = window_weak.upgrade() {
                        let _ = window.hide();
                    }
                }
            });
            // Lines may have arrived before the window existed, so ask the view
            // to scroll to the end the way a new line would.
            window.set_count(1);
            Ok(window)
        }
    });

    let profile_window = LazyWindow::new({
        let app = app.clone();
        let clipboard = clipboard.clone();
        let profile_line_model = profile_line_model.clone();
        move || {
            let window = ProfileWindow::new()?;
            {
                let app = app.borrow();
                apply_theme(&window, app.dark);
                i18n::apply(&window, app.settings.lang());
                window.set_frame(app.settings.frame);
            }
            window.set_lines(ModelRc::from(profile_line_model.clone()));
            window.on_check({
                let window_weak = window.as_weak();
                let app = app.clone();
                move || {
                    let Some(window) = window_weak.upgrade() else { return };
                    let (profile, t) = {
                        let app = app.borrow();
                        let Some(profile) = app.viewing.and_then(|i| app.store.profiles.get(i)).cloned() else {
                            return;
                        };
                        (profile, Strings::new(app.settings.lang()))
                    };
                    window.set_checking(true);
                    window.set_checks(ModelRc::from(Rc::new(VecModel::<CheckLine>::default())));
                    let window_weak = window.as_weak();
                    // Name lookups and socket probes block, so they stay off the UI thread.
                    std::thread::spawn(move || {
                        let rows: Vec<CheckLine> = preflight::run(&profile).iter().map(|c| check_line(c, &t)).collect();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window_weak.upgrade() {
                                window.set_checking(false);
                                window.set_checks(ModelRc::from(Rc::new(VecModel::from(rows))));
                            }
                        });
                    });
                }
            });
            window.on_copy({
                let app = app.clone();
                let clipboard = clipboard.clone();
                move || {
                    let app = app.borrow();
                    let Some(profile) = app.viewing.and_then(|i| app.store.profiles.get(i)) else {
                        return;
                    };
                    let Ok(text) = std::fs::read_to_string(&profile.file) else {
                        return;
                    };
                    let mut clipboard = clipboard.borrow_mut();
                    if clipboard.is_none() {
                        *clipboard = arboard::Clipboard::new().ok();
                    }
                    if let Some(clipboard) = clipboard.as_mut() {
                        let _ = clipboard.set_text(text);
                    }
                }
            });
            window.on_close_window({
                let window_weak = window.as_weak();
                move || {
                    if let Some(window) = window_weak.upgrade() {
                        let _ = window.hide();
                    }
                }
            });
            Ok(window)
        }
    });

    ui.set_profiles(ModelRc::from(profile_model.clone()));
    ui.set_samples(ModelRc::from(sample_model.clone()));
    ui.set_version(updater::current_version().into());
    apply_theme(&ui, app.borrow().dark);
    let lang = app.borrow().settings.lang();
    i18n::apply(&ui, lang);
    ui.set_lang(lang.code().into());
    ui.set_status_text(Strings::new(lang).state_idle.into());
    sync_settings(&ui, &app.borrow());
    sync_profiles(&ui, &app.borrow().store, &profile_model, &app.borrow().query);
    if !app.borrow().store.profiles.is_empty() {
        ui.set_selected(0);
    }
    update_card_name(&ui, &app.borrow());

    ui.on_show_log({
        let log_window = log_window.clone();
        move || {
            if let Some(window) = log_window.open() {
                let _ = window.show();
            }
        }
    });

    ui.on_set_theme({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let log_window = log_window.clone();
        let profile_window = profile_window.clone();
        move |mode| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            app.settings.set_theme(&mode);
            app.dark = app.settings.dark();
            apply_theme(&ui, app.dark);
            if let Some(window) = log_window.opened() {
                apply_theme(&window, app.dark);
            }
            if let Some(window) = profile_window.opened() {
                apply_theme(&window, app.dark);
            }
            ui.set_theme_mode(app.settings.theme_mode().into());
            if let Err(e) = app.settings.save() {
                let t = Strings::new(app.settings.lang());
                toast(&ui, &mut app, i18n::fill(t.theme_save_failed, &[&e.to_string()]), true);
            }
        }
    });

    ui.on_set_language({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let profile_model = profile_model.clone();
        let log_window = log_window.clone();
        let profile_window = profile_window.clone();
        move |code| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            let lang = Lang::from_code(&code);
            app.settings.set_lang(lang);
            if let Err(e) = app.settings.save() {
                let t = Strings::new(lang);
                toast(
                    &ui,
                    &mut app,
                    i18n::fill(t.language_save_failed, &[&e.to_string()]),
                    true,
                );
            }
            i18n::apply(&ui, lang);
            if let Some(window) = log_window.opened() {
                i18n::apply(&window, lang);
            }
            if let Some(window) = profile_window.opened() {
                i18n::apply(&window, lang);
            }
            ui.set_lang(lang.code().into());
            // The status line is a live message, so only reset it when nothing runs.
            if ui.get_conn_state() == ConnState::Idle {
                ui.set_status_text(Strings::new(lang).state_idle.into());
            }
            sync_profiles(&ui, &app.store, &profile_model, &app.query);
            update_card_name(&ui, &app);
        }
    });

    ui.on_set_autostart({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move |on| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            match autostart::set(on) {
                Ok(()) => {
                    app.settings.autostart = on;
                    save_settings(&ui, &mut app);
                }
                Err(e) => {
                    let t = Strings::new(app.settings.lang());
                    toast(&ui, &mut app, i18n::fill(t.autostart_failed, &[&e.to_string()]), true);
                }
            }
            ui.set_autostart(app.settings.autostart);
        }
    });

    ui.on_set_auto_connect({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move |on| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            app.settings.auto_connect = on;
            save_settings(&ui, &mut app);
        }
    });

    ui.on_set_notifications({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move |on| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            app.settings.notifications = on;
            save_settings(&ui, &mut app);
        }
    });

    ui.on_set_frame({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let log_window = log_window.clone();
        let profile_window = profile_window.clone();
        move |on| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            app.settings.frame = on;
            ui.set_frame(on);
            if let Some(window) = log_window.opened() {
                window.set_frame(on);
            }
            if let Some(window) = profile_window.opened() {
                window.set_frame(on);
            }
            save_settings(&ui, &mut app);
        }
    });

    ui.on_set_channel({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let tx = tx.clone();
        move |code| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            let channel = updater::Channel::from_code(&code);
            app.settings.set_channel(channel);
            ui.set_channel(channel.code().into());
            save_settings(&ui, &mut app);
            // Switching channels should offer the other channel's release now
            // rather than at the next launch, but never over a running install.
            if matches!(ui.get_update_state(), UpdateState::Idle | UpdateState::Failed) {
                ui.set_update_state(UpdateState::Checking);
                ui.set_update_note("".into());
                check_for_updates(tx.clone(), channel, false);
            }
        }
    });

    ui.on_search_changed({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let profile_model = profile_model.clone();
        move |query| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            app.query = query.to_string();
            sync_profiles(&ui, &app.store, &profile_model, &app.query);
        }
    });

    ui.on_select_profile({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move |i| {
            let Some(ui) = ui_weak.upgrade() else { return };
            ui.set_selected(i);
            update_card_name(&ui, &app.borrow());
        }
    });

    // Arrow keys walk the visible rows, which the filtered list owns, not the UI.
    ui.on_move_selection({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move |delta| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let app = app.borrow();
            let needle = app.query.trim().to_lowercase();
            let visible: Vec<usize> = app
                .store
                .profiles
                .iter()
                .enumerate()
                .filter(|(_, p)| needle.is_empty() || p.name.to_lowercase().contains(&needle))
                .map(|(index, _)| index)
                .collect();
            if visible.is_empty() {
                return;
            }
            let current = visible.iter().position(|index| *index as i32 == ui.get_selected());
            let next = match current {
                Some(position) => (position as i32 + delta).clamp(0, visible.len() as i32 - 1) as usize,
                None => 0,
            };
            ui.set_selected(visible[next] as i32);
            update_card_name(&ui, &app);
        }
    });

    ui.on_request_remove({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move |i| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let app = app.borrow();
            let Some(profile) = app.store.profiles.get(i as usize) else {
                return;
            };
            let t = Strings::new(app.settings.lang());
            ui.set_confirm_title(i18n::fill(t.remove_title, &[&profile.name]).into());
            ui.set_confirm_saved_password(profile.saved_password);
            ui.set_confirm_remove(i);
        }
    });

    ui.on_request_rename({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move |i| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let app = app.borrow();
            let Some(profile) = app.store.profiles.get(i as usize) else {
                return;
            };
            ui.set_rename_index(i);
            ui.set_rename_name(profile.name.as_str().into());
            ui.set_rename_error("".into());
            ui.set_rename_visible(true);
        }
    });

    ui.on_rename_profile({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let profile_model = profile_model.clone();
        move |i, name| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            let t = Strings::new(app.settings.lang());
            match app.store.rename(i as usize, &name) {
                Ok(()) => {
                    ui.set_rename_visible(false);
                    sync_profiles(&ui, &app.store, &profile_model, &app.query);
                    update_card_name(&ui, &app);
                }
                Err(profiles::RenameError::Empty) => ui.set_rename_error(t.rename_empty.into()),
                Err(profiles::RenameError::Taken) => ui.set_rename_error(t.rename_taken.into()),
                Err(profiles::RenameError::Save(e)) => ui.set_rename_error(e.into()),
            }
        }
    });

    ui.on_view_profile({
        let ui_weak = ui.as_weak();
        let profile_window = profile_window.clone();
        let app = app.clone();
        let line_model = profile_line_model.clone();
        move |index| {
            let Some(window) = profile_window.open() else { return };
            // The borrow ends before the window is touched: showing it can run
            // bindings that call back into the app.
            let (name, remote, text, read_error) = {
                let mut app = app.borrow_mut();
                let Some(profile) = app.store.profiles.get(index as usize).cloned() else {
                    return;
                };
                app.viewing = Some(index as usize);
                match std::fs::read_to_string(&profile.file) {
                    Ok(text) => (profile.name, profile.remote, text, None),
                    Err(e) => (profile.name, profile.remote, String::new(), Some(e.to_string())),
                }
            };
            if let Some(e) = read_error
                && let Some(ui) = ui_weak.upgrade()
            {
                let mut app = app.borrow_mut();
                let t = Strings::new(app.settings.lang());
                toast(&ui, &mut app, i18n::fill(t.profile_read_failed, &[&e]), true);
            }
            window.set_profile_name(name.as_str().into());
            window.set_profile_remote(remote.as_str().into());
            window.set_has_risky(!profiles::risky_directives(&text).is_empty());
            window.set_checking(false);
            window.set_checks(ModelRc::from(Rc::new(VecModel::<CheckLine>::default())));
            line_model.set_vec(profile_lines(&text));
            let _ = window.show();
        }
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
            let t = Strings::new(app.borrow().settings.lang());
            let (ui_weak, app, profile_model, picking) =
                (ui_weak.clone(), app.clone(), profile_model.clone(), picking.clone());
            let _ = slint::spawn_local(async move {
                let files = rfd::AsyncFileDialog::new()
                    .set_title(t.import_title)
                    .add_filter(t.import_filter, &["ovpn", "conf"])
                    .pick_files()
                    .await;
                picking.set(false);
                let (Some(files), Some(ui)) = (files, ui_weak.upgrade()) else {
                    return;
                };
                if files.is_empty() {
                    return;
                }
                let paths: Vec<PathBuf> = files.iter().map(|file| file.path().to_path_buf()).collect();
                import_paths(&ui, &mut app.borrow_mut(), &profile_model, &paths);
            });
        }
    });

    ui.on_import_clipboard({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let profile_model = profile_model.clone();
        let clipboard = clipboard.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            let t = Strings::new(app.settings.lang());
            let text = {
                let mut clipboard = clipboard.borrow_mut();
                if clipboard.is_none() {
                    *clipboard = arboard::Clipboard::new().ok();
                }
                clipboard.as_mut().and_then(|clipboard| clipboard.get_text().ok())
            };
            let Some(text) = text.filter(|text| !profiles::remote_of(text).is_empty()) else {
                toast(&ui, &mut app, t.import_clipboard_empty, true);
                return;
            };
            match app.store.import_text("pasted", &text) {
                Ok(index) => {
                    sync_profiles(&ui, &app.store, &profile_model, &app.query);
                    ui.set_selected(index as i32);
                    update_card_name(&ui, &app);
                    ui.set_add_visible(false);
                    let scripts = profiles::risky_directives(&text);
                    let message = if scripts.is_empty() {
                        t.import_pasted.to_string()
                    } else {
                        i18n::fill(t.import_scripts, &[&scripts.join(", ")])
                    };
                    toast(&ui, &mut app, message, !scripts.is_empty());
                }
                Err(profiles::SkipReason::Duplicate) => toast(&ui, &mut app, t.import_duplicate, true),
                Err(profiles::SkipReason::NotProfile) => toast(&ui, &mut app, t.import_not_ovpn, true),
                Err(profiles::SkipReason::Failed(e)) => toast(&ui, &mut app, e, true),
                Err(profiles::SkipReason::SaveFailed(e)) => {
                    toast(&ui, &mut app, i18n::fill(t.profiles_save_failed, &[&e]), true)
                }
            }
        }
    });

    ui.on_import_url({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let tx = tx.clone();
        move |url| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            let url = url.to_string();
            let t = Strings::new(app.settings.lang());
            if !url.starts_with("https://") {
                // A profile can carry `up` and `plugin`, which run as root, so it
                // must not travel over a link anyone on the way can rewrite.
                let message = if url.starts_with("http://") {
                    t.import_url_needs_https
                } else {
                    t.import_url_not_profile
                };
                toast(&ui, &mut app, message, true);
                return;
            }
            ui.set_add_visible(false);
            toast(&ui, &mut app, t.downloading_profile, false);
            let tx = tx.clone();
            std::thread::spawn(move || {
                let name = name_from_url(&url);
                let result = fetch_profile(&url);
                let _ = tx.send(Msg::Fetched { name, result });
            });
        }
    });

    ui.on_files_dropped({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let profile_model = profile_model.clone();
        move |data| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let paths: Vec<PathBuf> = data
                .file_paths()
                .map(|paths| paths.map(Path::to_path_buf).collect())
                .unwrap_or_default();
            if paths.is_empty() {
                return;
            }
            import_paths(&ui, &mut app.borrow_mut(), &profile_model, &paths);
        }
    });

    ui.on_remove_profile({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let profile_model = profile_model.clone();
        move |i| {
            let Some(ui) = ui_weak.upgrade() else { return };
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
                Err(e) => {
                    let t = Strings::new(app.settings.lang());
                    toast(&ui, &mut app, i18n::fill(t.remove_failed, &[&e.to_string()]), true);
                }
            }
            if let Some(a) = app.active.filter(|a| *a > i) {
                app.active = Some(a - 1);
            }
            ui.set_active(app.active.map_or(-1, |a| a as i32));
            ui.set_status_for(app.active.map_or(-1, |a| a as i32));
            sync_profiles(&ui, &app.store, &profile_model, &app.query);
            let len = app.store.profiles.len();
            ui.set_selected(if len == 0 { -1 } else { i.min(len - 1) as i32 });
            update_card_name(&ui, &app);
        }
    });

    ui.on_connect({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let log_model = log_model.clone();
        let sample_model = sample_model.clone();
        let tx = tx.clone();
        move |i| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            if app.session.is_some() {
                return;
            }
            let Some(profile) = app.store.profiles.get(i as usize).cloned() else {
                return;
            };
            app.session_id += 1;
            let id = app.session_id;
            let tx = tx.clone();
            app.log_all.clear();
            log_model.set_vec(Vec::new());
            app.samples.clear();
            sample_model.set_vec(Vec::new());
            app.last_bytes = (0, 0);
            app.last_sample = Instant::now();
            app.was_connected = false;
            reset_stats(&ui);
            ui.set_status_for(i);
            match openvpn::Session::start(
                &profile.file,
                profile.workdir.as_deref(),
                profile.saved_password,
                move |ev| {
                    let _ = tx.send(Msg::Vpn(id, ev));
                },
            ) {
                Ok(session) => {
                    app.session = Some(session);
                    app.active = Some(i as usize);
                    app.auth_failed = false;
                    app.pending_save.clear();
                    ui.set_active(i);
                    ui.set_conn_state(ConnState::Connecting);
                    let t = Strings::new(app.settings.lang());
                    ui.set_status_text(t.waiting_admin.into());
                    update_card_name(&ui, &app);
                }
                Err(e) => {
                    let t = Strings::new(app.settings.lang());
                    ui.set_conn_state(ConnState::Error);
                    ui.set_status_text(i18n::fill(t.start_failed, &[&e.to_string()]).into());
                }
            }
        }
    });

    ui.on_disconnect({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            app.was_connected = false;
            app.retry = None;
            let Some(session) = app.session.clone() else { return };
            if session.disconnect() {
                ui.set_conn_state(ConnState::Disconnecting);
                let t = Strings::new(app.settings.lang());
                ui.set_status_text(t.disconnecting.into());
            } else {
                // The management connection is gone, so the only way left to
                // stop the tunnel is to kill the child we started. pkexec
                // waits on the polkit prompt, so it runs off this thread and
                // reports back through the channel.
                let tx = app.tx.clone();
                thread::spawn(move || {
                    let result = session.kill().map_err(|e| e.to_string());
                    let _ = tx.send(Msg::Killed(result));
                });
                ui.set_conn_state(ConnState::Disconnecting);
                let t = Strings::new(app.settings.lang());
                ui.set_status_text(t.disconnecting.into());
            }
        }
    });

    ui.on_submit_auth({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move |username, password, code| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            ui.set_auth_visible(false);
            ui.set_auth_password("".into());
            ui.set_auth_code("".into());

            // A crtext challenge is answered on its own, without a password prompt.
            if app.cr_challenge.take().is_some() {
                if let Some(session) = &app.session {
                    session.send_cr_response(&clean_code(&code));
                }
                let t = Strings::new(app.settings.lang());
                ui.set_status_text(t.checking_code.into());
                return;
            }

            let Some(req) = app.auth.take() else { return };
            // With a saved password the dialog only asked for the code.
            let from_keyring = app.saved_password.take();
            let typed_password = from_keyring.is_none();
            let username = match &from_keyring {
                Some(_) => app
                    .active
                    .and_then(|a| app.store.profiles.get(a))
                    .map(|p| p.username.clone())
                    .unwrap_or_default(),
                None => username.trim().to_string(),
            };
            let password = from_keyring.unwrap_or_else(|| password.to_string());
            if let Some(session) = &app.session {
                session.send_auth(&req, &username, &password, &clean_code(&code));
            }
            let saved = app
                .active
                .and_then(|a| app.store.profiles.get(a))
                .filter(|p| p.saved_password)
                .map(|p| p.file.clone());
            if typed_password {
                if ui.get_auth_remember() {
                    app.pending_save.push((req.realm.clone(), password));
                } else if let Some(file) = saved {
                    forget_password(&app.tx, file);
                }
            }
            let mut renamed = false;
            if req.needs_username
                && let Some(profile) = app.active.and_then(|a| app.store.profiles.get_mut(a))
                && profile.username != username
            {
                profile.username = username;
                renamed = true;
            }
            if renamed {
                save_profiles(&ui, &mut app);
            }
            let t = Strings::new(app.settings.lang());
            ui.set_status_text(t.signing_in.into());
        }
    });

    ui.on_change_password({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            app.saved_password = None;
            let Some(req) = app.auth.clone() else { return };
            show_auth_dialog(&ui, &mut app, req, "");
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
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            ui.set_auth_visible(false);
            ui.set_auth_password("".into());
            ui.set_auth_code("".into());
            app.auth = None;
            app.saved_password = None;
            app.cr_challenge = None;
            app.was_connected = false;
            app.retry = None;
            let Some(session) = app.session.clone() else { return };
            if session.disconnect() {
                ui.set_conn_state(ConnState::Disconnecting);
                let t = Strings::new(app.settings.lang());
                ui.set_status_text(t.disconnecting.into());
            } else {
                // The management connection is gone, so the only way left to
                // stop the tunnel is to kill the child we started. pkexec
                // waits on the polkit prompt, so it runs off this thread and
                // reports back through the channel.
                let tx = app.tx.clone();
                thread::spawn(move || {
                    let result = session.kill().map_err(|e| e.to_string());
                    let _ = tx.send(Msg::Killed(result));
                });
                ui.set_conn_state(ConnState::Disconnecting);
                let t = Strings::new(app.settings.lang());
                ui.set_status_text(t.disconnecting.into());
            }
        }
    });

    ui.on_check_updates({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let tx = tx.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            ui.set_update_state(UpdateState::Checking);
            ui.set_update_note("".into());
            check_for_updates(tx.clone(), app.borrow().settings.channel(), false);
        }
    });

    ui.on_install_update({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        let tx = tx.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut app = app.borrow_mut();
            let Some(release) = app.release.clone() else { return };
            match updater::Install::detect() {
                Ok(install) => {
                    ui.set_update_state(UpdateState::Downloading);
                    ui.set_update_progress(0.0);
                    let t = Strings::new(app.settings.lang());
                    ui.set_update_text(i18n::fill(t.downloading, &[&release.version]).into());
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
            let Some(ui) = ui_weak.upgrade() else { return };
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
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_update_state(UpdateState::Idle);
            }
        }
    });

    // With a tray icon, closing the window keeps Ether (and the tunnel) running.
    ui.window().on_close_requested({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else {
                return CloseRequestResponse::HideWindow;
            };
            hide_to_tray(&ui, &mut app.borrow_mut());
            CloseRequestResponse::HideWindow
        }
    });

    // The title bar's × does what the compositor's close button does.
    ui.on_hide_window({
        let ui_weak = ui.as_weak();
        let app = app.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            hide_to_tray(&ui, &mut app.borrow_mut());
        }
    });

    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), {
        let ui_weak = ui.as_weak();
        let log_window = log_window.clone();
        let app = app.clone();
        let log_model = log_model.clone();
        let sample_model = sample_model.clone();
        let profile_model = profile_model.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            // Only an already opened log window is updated: building it here
            // would bring back the empty window this change removes.
            let log = log_window.opened();
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    Msg::Vpn(id, event) => {
                        let mut app = app.borrow_mut();
                        if id == app.session_id {
                            handle_event(&ui, log.as_ref(), &mut app, &log_model, &sample_model, event);
                        }
                    }
                    // Run through the UI's own callbacks, which borrow `app` themselves.
                    Msg::Tray(command) => handle_tray_command(&ui, command),
                    Msg::UpdateChecked { status, silent } => {
                        handle_update_status(&ui, &mut app.borrow_mut(), status, silent)
                    }
                    Msg::UpdateProgress(progress) => {
                        handle_update_progress(&ui, progress, app.borrow().settings.lang())
                    }
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
                                save_profiles(&ui, &mut app);
                                sync_profiles(&ui, &app.store, &profile_model, &app.query);
                            }
                            Err(e) => {
                                let t = Strings::new(app.settings.lang());
                                let template = if saved {
                                    t.keyring_save_failed
                                } else {
                                    t.keyring_forget_failed
                                };
                                toast(&ui, &mut app, i18n::fill(template, &[&e.to_string()]), true);
                            }
                        }
                    }
                    Msg::Killed(result) => {
                        // A kill that went through still has to be confirmed by
                        // the process exiting, which arrives as Event::Exited.
                        if let Err(e) = result {
                            let mut app = app.borrow_mut();
                            let t = Strings::new(app.settings.lang());
                            toast(&ui, &mut app, i18n::fill(t.stop_failed, &[&e]), true);
                        }
                    }
                    Msg::Fetched { name, result } => {
                        let mut app = app.borrow_mut();
                        let t = Strings::new(app.settings.lang());
                        match result {
                            Ok(text) if !profiles::remote_of(&text).is_empty() => match app
                                .store
                                .import_text(&name, &text)
                            {
                                Ok(index) => {
                                    sync_profiles(&ui, &app.store, &profile_model, &app.query);
                                    ui.set_selected(index as i32);
                                    update_card_name(&ui, &app);
                                    let scripts = profiles::risky_directives(&text);
                                    let message = if scripts.is_empty() {
                                        i18n::import_summary(app.settings.lang(), 1, &[])
                                    } else {
                                        i18n::fill(t.import_scripts, &[&scripts.join(", ")])
                                    };
                                    toast(&ui, &mut app, message, !scripts.is_empty());
                                }
                                Err(profiles::SkipReason::Duplicate) => toast(&ui, &mut app, t.import_duplicate, true),
                                Err(profiles::SkipReason::NotProfile) => toast(&ui, &mut app, t.import_not_ovpn, true),
                                Err(profiles::SkipReason::Failed(e)) => toast(&ui, &mut app, e, true),
                                Err(profiles::SkipReason::SaveFailed(e)) => {
                                    toast(&ui, &mut app, i18n::fill(t.profiles_save_failed, &[&e]), true)
                                }
                            },
                            Ok(_) => toast(&ui, &mut app, t.import_url_not_profile, true),
                            Err(e) => toast(&ui, &mut app, i18n::fill(t.import_url_failed, &[&e]), true),
                        }
                    }
                }
            }
            {
                let mut app = app.borrow_mut();
                if let Some(since) = app.connected_at {
                    ui.set_duration(format_duration(since.elapsed()).into());
                }
                if app.toast_until.is_some_and(|until| Instant::now() >= until) {
                    app.toast_until = None;
                    ui.set_toast("".into());
                }
            }
            // A retry calls back into the UI, which borrows `app` itself, so the
            // decision is made and the borrow released first.
            let retry = {
                let mut app = app.borrow_mut();
                match app.retry {
                    Some((at, attempt)) if Instant::now() >= at => {
                        app.retry = None;
                        app.retry_profile.map(|index| (index, attempt))
                    }
                    _ => None,
                }
            };
            if let Some((index, attempt)) = retry {
                let t = Strings::new(app.borrow().settings.lang());
                let text = i18n::fill(
                    t.reconnecting_attempt,
                    &[&attempt.to_string(), &MAX_RECONNECTS.to_string()],
                );
                ui.set_status_text(text.into());
                ui.invoke_connect(index as i32);
            }
            sync_tray(&ui, &mut app.borrow_mut());
        }
    });

    if let Some(e) = store_error {
        let t = Strings::new(app.borrow().settings.lang());
        toast(
            &ui,
            &mut app.borrow_mut(),
            i18n::fill(t.profiles_migration_failed, &[&e.to_string()]),
            true,
        );
    }
    ui.show()?;
    // Restore the saved size only once the window is mapped: a size set before
    // the window is shown loses to the layout's preferred size.
    if let Some((width, height)) = app.borrow().settings.window_size() {
        ui.window()
            .set_size(slint::LogicalSize::new(width as f32, height as f32));
    }
    check_for_updates(tx.clone(), app.borrow().settings.channel(), true);
    // "Connect on launch" waits for the window, so the prompt has somewhere to appear.
    let auto_connect = app.borrow().settings.auto_connect;
    let last_used = app.borrow().store.last_used();
    if auto_connect && let Some(index) = last_used {
        ui.set_selected(index as i32);
        ui.invoke_connect(index as i32);
    }
    slint::run_event_loop_until_quit()?;

    // Nothing is left to control the tunnel once Ether exits, so take it down.
    if let Some(session) = &app.borrow().session {
        session.disconnect();
    }
    save_window_size(&ui, &mut app.borrow_mut());
    Ok(())
}

/// Whether Ether runs inside a Flatpak sandbox, where host programs and
/// files have to be reached through `flatpak-spawn --host` and /run/host.
pub fn sandboxed() -> bool {
    std::env::var_os("FLATPAK_ID").is_some()
}

/// Switches the palette of `ui`.
///
/// The widgets are the app's own, so this is all it takes. Globals are per
/// component tree, so every window the app opens needs this call: a window
/// built later starts with the default palette.
fn apply_theme<C>(ui: &C, dark: bool)
where
    C: ComponentHandle,
    for<'a> Theme<'a>: slint::Global<'a, C>,
{
    ui.global::<Theme>().set_dark(dark);
}

/// Pushes the settings the settings dialog shows.
fn sync_settings(ui: &AppWindow, app: &App) {
    ui.set_theme_mode(app.settings.theme_mode().into());
    ui.set_autostart(app.settings.autostart);
    ui.set_auto_connect(app.settings.auto_connect);
    ui.set_notifications(app.settings.notifications);
    ui.set_frame(app.settings.frame);
    ui.set_channel(app.settings.channel().code().into());
}

/// Hides the window, or quits when there is no tray icon to bring it back.
fn hide_to_tray(ui: &AppWindow, app: &mut App) {
    save_window_size(ui, app);
    if app.tray.is_none() {
        let _ = slint::quit_event_loop();
        return;
    }
    if !app.tray_hint_shown {
        app.tray_hint_shown = true;
        if app.settings.notifications {
            notify::send(Strings::new(app.settings.lang()).notify_tray, "");
        }
    }
    let _ = ui.hide();
}

fn save_window_size(ui: &AppWindow, app: &mut App) {
    let size = ui.window().size();
    let scale = ui.window().scale_factor();
    if scale <= 0.0 {
        return;
    }
    let width = (size.width as f32 / scale).round() as u32;
    let height = (size.height as f32 / scale).round() as u32;
    if width == 0 || height == 0 {
        return;
    }
    if app.settings.window_size() != Some((width, height)) {
        app.settings.set_window_size(width, height);
        // The window is on its way out, so a toast would never be seen.
        if let Err(e) = app.settings.save() {
            eprintln!("ether-ovpn: cannot save the window size: {e}");
        }
    }
}

fn check_for_updates(tx: mpsc::Sender<Msg>, channel: updater::Channel, silent: bool) {
    updater::spawn_check(channel, move |status| {
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
            let t = Strings::new(app.settings.lang());
            let password = match updater::Install::detect() {
                Ok(install) if install.asks_for_password() => t.update_password_note,
                _ => "",
            };
            let prerelease = if release.prerelease {
                t.update_prerelease_note
            } else {
                ""
            };
            let text = i18n::fill(t.update_available, &[&release.version, updater::current_version()]);
            ui.set_update_text(format!("{text}{prerelease}{password}").into());
            app.release = Some(release);
        }
        UpdateStatus::UpToDate => {
            ui.set_update_state(UpdateState::Idle);
            if !silent {
                ui.set_update_note(Strings::new(app.settings.lang()).up_to_date.into());
            }
        }
        UpdateStatus::Failed(e) => {
            ui.set_update_state(UpdateState::Idle);
            if !silent {
                let t = Strings::new(app.settings.lang());
                ui.set_update_note(i18n::fill(t.update_check_failed, &[&e]).into());
            }
        }
    }
}

fn handle_update_progress(ui: &AppWindow, progress: updater::Progress, lang: Lang) {
    use updater::Progress;
    match progress {
        Progress::Downloading(fraction) => ui.set_update_progress(fraction),
        Progress::Installing => {
            ui.set_update_state(UpdateState::Installing);
            ui.set_update_text(Strings::new(lang).installing.into());
        }
        Progress::Done => {
            ui.set_update_state(UpdateState::Done);
            let t = Strings::new(lang);
            let note = if ui.get_active() >= 0 { t.update_done_busy } else { "" };
            ui.set_update_text(format!("{}{note}", t.update_done).into());
        }
        Progress::Failed(e) => {
            ui.set_update_state(UpdateState::Failed);
            ui.set_update_text(i18n::fill(Strings::new(lang).update_failed, &[&e]).into());
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
    let lang = app.settings.lang();
    let t = Strings::new(lang);
    let profile_name = app
        .active
        .and_then(|a| app.store.profiles.get(a))
        .map(|p| p.name.as_str());
    let status_text = match (status, profile_name) {
        (tray::Status::Connected, Some(name)) => i18n::fill(t.tray_connected_to, &[name]),
        (tray::Status::Idle, _) => t.tray_disconnected.to_string(),
        _ => ui.get_status_text().to_string(),
    };
    let update = match ui.get_update_state() {
        UpdateState::Available => app.release.as_ref().map(|r| r.version.clone()),
        _ => None,
    };
    let last = app
        .store
        .last_used()
        .and_then(|i| app.store.profiles.get(i).map(|p| (i, p.name.clone())));
    let state = tray::State {
        status,
        status_text,
        profiles: app.store.profiles.iter().map(|p| p.name.clone()).collect(),
        busy,
        update,
        lang,
        last,
    };
    if state != app.tray_state {
        tray::set_state(handle, state.clone());
        app.tray_state = state;
    }
}

fn handle_event(
    ui: &AppWindow,
    log_window: Option<&LogWindow>,
    app: &mut App,
    log_model: &VecModel<LogLine>,
    sample_model: &VecModel<f32>,
    event: openvpn::Event,
) {
    use openvpn::Event;
    let t = Strings::new(app.settings.lang());
    match event {
        Event::Log(line) => {
            let level = log_level(&line);
            app.log_all.push(LogLine {
                text: line.as_str().into(),
                level,
            });
            if app.log_all.len() > MAX_LOG_LINES {
                app.log_all.remove(0);
            }
            if matches_filter(&line, &app.log_filter) {
                if log_model.row_count() >= MAX_LOG_LINES {
                    log_model.remove(0);
                }
                log_model.push(LogLine {
                    text: line.as_str().into(),
                    level,
                });
            }
            if let Some(log_window) = log_window {
                log_window.set_count(log_window.get_count() + 1);
            }
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
                    app.was_connected = true;
                    app.retry = None;
                    let file = app
                        .active
                        .and_then(|a| app.store.profiles.get(a))
                        .map(|p| p.file.clone());
                    if let Some(file) = file {
                        let tx = app.tx.clone();
                        save_passwords(&tx, file, std::mem::take(&mut app.pending_save));
                    }
                    if let Some(index) = app.active {
                        let touched = app.store.touch(index);
                        report_save(ui, app, touched, t.profiles_save_failed);
                    }
                    if app.settings.notifications {
                        let name = app
                            .active
                            .and_then(|a| app.store.profiles.get(a))
                            .map(|p| p.name.clone())
                            .unwrap_or_default();
                        notify::send(i18n::fill(t.notify_connected, &[&name]), "");
                    }
                    (ConnState::Connected, t.state_connected.to_string())
                }
                "RECONNECTING" => {
                    app.connected_at = None;
                    (ConnState::Connecting, t.reconnecting.to_string())
                }
                "EXITING" => (ConnState::Disconnecting, t.disconnecting.to_string()),
                other => (
                    ConnState::Connecting,
                    i18n::fill(t.connecting_to, &[&other.to_lowercase().replace('_', " ")]),
                ),
            };
            ui.set_conn_state(state);
            ui.set_status_text(text.into());
        }
        Event::Bytes { rx, tx } => {
            ui.set_rx(format_bytes(rx).into());
            ui.set_tx(format_bytes(tx).into());
            push_sample(app, sample_model, rx);
        }
        Event::NeedAuth(req) => {
            let profile = app.active.and_then(|a| app.store.profiles.get(a));
            // After a failed attempt the saved password is evidently wrong, so ask instead.
            if let Some(file) = profile
                .filter(|p| p.saved_password && !app.auth_failed)
                .map(|p| p.file.clone())
            {
                ui.set_status_text(t.signing_in_saved.into());
                let (tx, session) = (app.tx.clone(), app.session_id);
                std::thread::spawn(move || {
                    let result = secrets::load(&file, &req.realm);
                    let _ = tx.send(Msg::SavedPassword { session, req, result });
                });
                return;
            }
            let error = match (app.auth_failed, profile.is_some_and(|p| p.saved_password)) {
                (true, true) => t.saved_password_failed,
                (true, false) => t.auth_failed,
                _ => "",
            };
            show_auth_dialog(ui, app, req, error);
        }
        Event::Challenge(challenge) => show_code_dialog(ui, app, challenge),
        Event::AuthPending { timeout } => {
            let text = match timeout {
                Some(seconds) => i18n::fill(t.auth_pending_timeout, &[&seconds.to_string()]),
                None => t.auth_pending.to_string(),
            };
            ui.set_status_text(text.into());
        }
        Event::AuthFailed => {
            app.auth_failed = true;
            app.pending_save.clear();
        }
        Event::Lost => {
            // The process may have died at the same moment, and Exited then owns
            // the state; without a session there is nothing left to report.
            if app.session.is_none() {
                return;
            }
            ui.set_conn_state(ConnState::Error);
            ui.set_status_text(t.connection_lost.into());
            if app.settings.notifications {
                notify::send(t.connection_lost, "");
            }
        }
        Event::Exited(error) => {
            let was_active = app.active;
            let dropped = app.was_connected;
            app.session = None;
            app.active = None;
            app.connected_at = None;
            app.auth = None;
            app.saved_password = None;
            app.cr_challenge = None;
            app.was_connected = false;
            ui.set_active(-1);
            ui.set_auth_visible(false);
            reset_stats(ui);
            match error {
                Some(message) => {
                    ui.set_conn_state(ConnState::Error);
                    ui.set_status_text(message.clone().into());
                    if app.settings.notifications {
                        notify::send(i18n::fill(t.notify_failed, &[&message]), "");
                    }
                    // A tunnel that was up and then died is worth another try.
                    if dropped {
                        let attempt = app.retry.map_or(1, |(_, attempt)| attempt + 1);
                        if attempt <= MAX_RECONNECTS {
                            app.retry = Some((Instant::now() + RECONNECT_DELAY, attempt));
                            app.retry_profile = was_active;
                        } else {
                            app.retry = None;
                            toast(ui, app, t.reconnect_gave_up, true);
                        }
                    }
                }
                None => {
                    ui.set_conn_state(ConnState::Idle);
                    ui.set_status_text(t.state_idle.into());
                    if app.settings.notifications {
                        notify::send(t.notify_disconnected, "");
                    }
                }
            }
        }
    }
}

/// Keeps the sparkline fed: one sample per bytecount message, scaled to the
/// busiest sample in the window so the shape is readable at any speed.
fn push_sample(app: &mut App, model: &VecModel<f32>, rx: u64) {
    let now = Instant::now();
    let seconds = now.duration_since(app.last_sample).as_secs_f32().max(0.1);
    let rate = rx.saturating_sub(app.last_bytes.0) as f32 / seconds;
    app.last_bytes = (rx, app.last_bytes.1);
    app.last_sample = now;
    app.samples.push(rate);
    if app.samples.len() > MAX_SAMPLES {
        app.samples.remove(0);
    }
    let peak = app.samples.iter().copied().fold(0.0f32, f32::max).max(1.0);
    let scaled: Vec<f32> = app.samples.iter().map(|sample| sample / peak).collect();
    model.set_vec(scaled);
}

/// 0 for ordinary output, 1 for warnings, 2 for errors.
fn log_level(line: &str) -> i32 {
    if line.contains("ERROR") || line.contains("FATAL") {
        2
    } else if line.contains("WARNING") || line.contains("WARN") {
        1
    } else {
        0
    }
}

fn matches_filter(line: &str, filter: &str) -> bool {
    filter.is_empty() || line.to_lowercase().contains(&filter.to_lowercase())
}

fn rebuild_log(log_window: &LogWindow, model: &VecModel<LogLine>, app: &App) {
    let rows: Vec<LogLine> = app
        .log_all
        .iter()
        .filter(|line| matches_filter(&line.text, &app.log_filter))
        .cloned()
        .collect();
    model.set_vec(rows);
    log_window.set_count(log_window.get_count() + 1);
}

/// Imports files picked in the dialog or dropped onto the window.
fn import_paths(ui: &AppWindow, app: &mut App, model: &VecModel<ProfileInfo>, paths: &[PathBuf]) {
    let report = app.store.import_many(paths);
    let lang = app.settings.lang();
    let t = Strings::new(lang);
    let reasons: Vec<String> = report
        .skipped
        .iter()
        .map(|(_, reason)| match reason {
            profiles::SkipReason::Duplicate => t.import_duplicate.to_string(),
            profiles::SkipReason::NotProfile => t.import_not_ovpn.to_string(),
            profiles::SkipReason::Failed(e) => e.clone(),
            profiles::SkipReason::SaveFailed(e) => i18n::fill(t.profiles_save_failed, &[e]),
        })
        .collect();
    if !report.imported.is_empty() || !reasons.is_empty() {
        let mut summary = i18n::import_summary(lang, report.imported.len(), &reasons);
        if !report.scripts.is_empty() {
            summary.push(' ');
            summary.push_str(&i18n::fill(t.import_scripts, &[&report.scripts.join(", ")]));
        }
        if let Some(e) = &report.save_error {
            summary.push(' ');
            summary.push_str(&i18n::fill(t.profiles_save_failed, &[e]));
        }
        toast(
            ui,
            app,
            summary,
            report.imported.is_empty() || !report.scripts.is_empty() || report.save_error.is_some(),
        );
    }
    sync_profiles(ui, &app.store, model, &app.query);
    if let Some(first) = report.imported.first() {
        ui.set_selected(*first as i32);
    }
    update_card_name(ui, app);
}

fn show_auth_dialog(ui: &AppWindow, app: &mut App, req: openvpn::AuthRequest, error: &str) {
    // With a password from the keyring only the 2FA code is missing.
    let t = Strings::new(app.settings.lang());
    let code_only = app.saved_password.is_some();
    let title = if code_only {
        t.enter_code.to_string()
    } else if req.needs_username {
        t.sign_in_vpn.to_string()
    } else {
        i18n::fill(t.enter_password, &[&req.realm.to_lowercase()])
    };
    let profile = app.active.and_then(|a| app.store.profiles.get(a));
    ui.set_auth_title(title.into());
    ui.set_auth_needs_username(req.needs_username && !code_only);
    ui.set_auth_needs_password(!code_only);
    ui.set_auth_needs_code(req.challenge.is_some());
    ui.set_auth_challenge_text(req.challenge.as_ref().map(challenge_text).unwrap_or_default().into());
    ui.set_auth_password_saved(code_only);
    ui.set_auth_username(profile.map(|p| p.username.clone()).unwrap_or_default().into());
    ui.set_auth_remember(profile.is_some_and(|p| p.saved_password));
    ui.set_auth_error(error.into());
    ui.set_auth_code("".into());
    ui.set_auth_visible(true);
    ui.set_status_text(t.waiting_credentials.into());
    // The window may be hidden in the tray; the prompt needs to be seen.
    let _ = ui.show();
    app.auth = Some(req);
}

/// A crtext challenge arrives while authentication is pending, so there is no
/// password prompt to attach it to.
fn show_code_dialog(ui: &AppWindow, app: &mut App, challenge: openvpn::Challenge) {
    let t = Strings::new(app.settings.lang());
    ui.set_auth_title(t.enter_code.into());
    ui.set_auth_needs_username(false);
    ui.set_auth_needs_password(false);
    ui.set_auth_needs_code(true);
    ui.set_auth_challenge_text(challenge_text(&challenge).into());
    ui.set_auth_password_saved(false);
    ui.set_auth_error("".into());
    ui.set_auth_code("".into());
    ui.set_auth_visible(true);
    ui.set_status_text(t.waiting_code.into());
    let _ = ui.show();
    app.cr_challenge = Some(challenge);
}

fn challenge_text(challenge: &openvpn::Challenge) -> &str {
    match challenge {
        openvpn::Challenge::Static { text, .. }
        | openvpn::Challenge::Dynamic { text, .. }
        | openvpn::Challenge::CrText { text, .. } => text,
    }
}

fn use_saved_password(
    ui: &AppWindow,
    app: &mut App,
    req: openvpn::AuthRequest,
    result: Result<Option<String>, String>,
) {
    let username = app
        .active
        .and_then(|a| app.store.profiles.get(a))
        .map(|p| p.username.clone())
        .unwrap_or_default();
    match result {
        Ok(Some(password)) if !req.needs_username || !username.is_empty() => {
            if req.challenge.is_some() {
                // The code is still needed, so keep the password for the dialog.
                app.saved_password = Some(password);
                show_auth_dialog(ui, app, req, "");
            } else if let Some(session) = &app.session {
                session.send_auth(&req, &username, &password, "");
                let t = Strings::new(app.settings.lang());
                ui.set_status_text(t.signing_in.into());
            }
        }
        Ok(_) => show_auth_dialog(ui, app, req, ""),
        Err(e) => {
            let t = Strings::new(app.settings.lang());
            let message = i18n::fill(t.saved_password_read_failed, &[&e]);
            show_auth_dialog(ui, app, req, &message)
        }
    }
}

fn save_passwords(tx: &mpsc::Sender<Msg>, file: PathBuf, passwords: Vec<(String, String)>) {
    if passwords.is_empty() {
        return;
    }
    let tx = tx.clone();
    std::thread::spawn(move || {
        let result = passwords
            .iter()
            .try_for_each(|(realm, password)| secrets::save(&file, realm, password));
        let _ = tx.send(Msg::PasswordStored {
            file,
            saved: true,
            result,
        });
    });
}

fn forget_password(tx: &mpsc::Sender<Msg>, file: PathBuf) {
    let tx = tx.clone();
    std::thread::spawn(move || {
        let result = secrets::forget_all(&file);
        let _ = tx.send(Msg::PasswordStored {
            file,
            saved: false,
            result,
        });
    });
}

/// Rebuilds the visible list, keeping each row's index in the store.
fn sync_profiles(ui: &AppWindow, store: &profiles::Store, model: &VecModel<ProfileInfo>, query: &str) {
    let needle = query.trim().to_lowercase();
    let rows: Vec<ProfileInfo> = store
        .profiles
        .iter()
        .enumerate()
        .filter(|(_, p)| needle.is_empty() || p.name.to_lowercase().contains(&needle))
        .map(|(index, p)| ProfileInfo {
            index: index as i32,
            name: p.name.as_str().into(),
            saved_password: p.saved_password,
            remote: p.remote.as_str().into(),
            initials: initials(&p.name).into(),
        })
        .collect();
    model.set_vec(rows);
    ui.set_profile_count(store.profiles.len() as i32);
}

/// One or two letters for the row's avatar: "Work laptop" becomes "WL".
fn initials(name: &str) -> String {
    let mut words = name.split_whitespace().filter_map(|word| word.chars().next());
    let first = words.next().unwrap_or('?');
    match words.next() {
        Some(second) => format!("{first}{second}").to_uppercase(),
        None => first.to_uppercase().to_string(),
    }
}

/// Keeps the status card's profile name and server in step with the selection.
fn update_card_name(ui: &AppWindow, app: &App) {
    let index = if ui.get_active() >= 0 {
        ui.get_active()
    } else {
        ui.get_selected()
    };
    let profile = usize::try_from(index).ok().and_then(|i| app.store.profiles.get(i));
    ui.set_card_name(profile.map(|p| p.name.as_str()).unwrap_or("").into());
    ui.set_card_remote(profile.map(|p| p.remote.as_str()).unwrap_or("").into());
}

/// Saves the settings, telling the user when the write fails: a setting that
/// quietly doesn't stick is worse than a toast.
fn save_settings(ui: &AppWindow, app: &mut App) {
    let result = app.settings.save();
    let template = Strings::new(app.settings.lang()).settings_save_failed;
    report_save(ui, app, result, template);
}

/// The same for the profile list.
fn save_profiles(ui: &AppWindow, app: &mut App) {
    let result = app.store.save();
    let template = Strings::new(app.settings.lang()).profiles_save_failed;
    report_save(ui, app, result, template);
}

/// Reports a failed write, if any, with the message the caller picked.
fn report_save(ui: &AppWindow, app: &mut App, result: io::Result<()>, template: &str) {
    if let Err(e) = result {
        toast(ui, app, i18n::fill(template, &[&e.to_string()]), true);
    }
}

/// Shows a message over the window for a few seconds.
fn toast(ui: &AppWindow, app: &mut App, text: impl Into<SharedString>, error: bool) {
    ui.set_toast(text.into());
    ui.set_toast_error(error);
    let time = if error { TOAST_ERROR_TIME } else { TOAST_TIME };
    app.toast_until = Some(Instant::now() + time);
}

/// Splits a profile into numbered lines, marking the ones that run a program.
fn profile_lines(text: &str) -> Vec<ProfileLine> {
    let risky = profiles::risky_directives(text);
    text.lines()
        .enumerate()
        .map(|(i, line)| {
            let word = line.split_whitespace().next().unwrap_or("");
            ProfileLine {
                number: (i + 1) as i32,
                text: line.into(),
                risky: risky.iter().any(|d| d == word),
            }
        })
        .collect()
}

/// Turns one pre-connect check into a row the profile window can show.
fn check_line(check: &preflight::Check, t: &Strings) -> CheckLine {
    let label = match check.kind {
        preflight::Kind::File => t.check_file,
        preflight::Kind::Remote => t.check_remote,
        preflight::Kind::Files => t.check_files,
        preflight::Kind::Dns => t.check_dns,
        preflight::Kind::Port => t.check_port,
    };
    let (state, detail) = match &check.outcome {
        preflight::Outcome::Passed => (0, t.check_passed.to_string()),
        preflight::Outcome::Skipped(_) => (2, t.check_udp_skipped.to_string()),
        preflight::Outcome::Failed(detail) if detail.is_empty() => {
            let detail = match check.kind {
                preflight::Kind::Remote => t.check_remote_missing,
                _ => t.check_failed,
            };
            (1, detail.to_string())
        }
        preflight::Outcome::Failed(detail) => (1, detail.clone()),
    };
    CheckLine {
        label: label.into(),
        detail: detail.into(),
        state,
    }
}

fn log_text(lines: &[LogLine]) -> String {
    lines.iter().map(|line| format!("{}\n", line.text)).collect()
}

/// The block the "Copy diagnostics" button puts on the clipboard.
fn diagnostics(app: &App) -> String {
    let profile = app.active.and_then(|i| app.store.profiles.get(i));
    let mut out = String::new();
    out.push_str(&format!("Ether {}\n", updater::current_version()));
    out.push_str(&format!("OS: {}\n", os_release()));
    out.push_str(&format!("OpenVPN: {}\n", openvpn_version()));
    if let Some(profile) = profile {
        out.push_str(&format!("Profile: {} ({})\n", profile.name, profile.remote));
    }
    out.push_str(&format!("Profiles: {}\n", app.store.profiles.len()));
    out.push_str("---\n");
    let start = app.log_all.len().saturating_sub(50);
    for line in &app.log_all[start..] {
        out.push_str(&line.text);
        out.push('\n');
    }
    out
}

fn os_release() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix("PRETTY_NAME=")
                    .map(|name| name.trim_matches('"').to_string())
            })
        })
        .unwrap_or_else(|| std::env::consts::OS.to_string())
}

fn openvpn_version() -> String {
    let program = ["/usr/sbin/openvpn", "/usr/bin/openvpn"]
        .into_iter()
        .find(|path| Path::new(path).exists());
    let Some(program) = program else {
        return "not installed".to_string();
    };
    std::process::Command::new(program)
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.lines().next().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Downloads a profile from a link, with the same limits as any other text file.
fn fetch_profile(url: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("User-Agent", format!("ether-ovpn/{}", updater::current_version()))
        .call()
        .map_err(|e| e.to_string())?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_PROFILE_BYTES)
        .read_to_string()
        .map_err(|e| e.to_string())
}

/// The last path segment of a link, without a profile extension.
fn name_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("profile");
    // Only a profile extension is dropped: "example.com" is a host, not a name.
    let stem = last
        .strip_suffix(".ovpn")
        .or_else(|| last.strip_suffix(".conf"))
        .unwrap_or(last);
    if stem.is_empty() {
        "profile".to_string()
    } else {
        stem.to_string()
    }
}

/// 2FA codes are often pasted with spaces or a stray newline.
fn clean_code(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

fn reset_stats(ui: &AppWindow) {
    ui.set_local_ip("—".into());
    ui.set_remote("—".into());
    ui.set_rx("—".into());
    ui.set_tx("—".into());
    ui.set_duration("—".into());
}

fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_initials_from_the_name() {
        assert_eq!(initials("Work laptop"), "WL");
        assert_eq!(initials("office"), "O");
        assert_eq!(initials("  home   server  "), "HS");
        assert_eq!(initials(""), "?");
        // Non-ASCII names keep their own letters.
        assert_eq!(initials("офис"), "О");
    }

    #[test]
    fn names_a_downloaded_profile_after_its_link() {
        assert_eq!(name_from_url("https://example.com/files/work.ovpn"), "work");
        assert_eq!(name_from_url("https://example.com/files/work.ovpn?token=1"), "work");
        assert_eq!(name_from_url("https://example.com/files/"), "files");
        assert_eq!(name_from_url("https://example.com"), "example.com");
        assert_eq!(name_from_url("https://example.com/"), "example.com");
    }

    #[test]
    fn strips_whitespace_from_a_2fa_code() {
        assert_eq!(clean_code(" 123 456\n"), "123456");
        assert_eq!(clean_code("123456"), "123456");
        assert_eq!(clean_code(""), "");
    }

    #[test]
    fn marks_warnings_and_errors_in_the_log() {
        assert_eq!(log_level("2025-01-01 WARNING: bad option"), 1);
        assert_eq!(log_level("2025-01-01 ERROR: cannot open"), 2);
        assert_eq!(log_level("2025-01-01 FATAL: exiting"), 2);
        assert_eq!(log_level("2025-01-01 Initialization Sequence Completed"), 0);
    }

    #[test]
    fn filters_the_log_case_insensitively() {
        assert!(matches_filter("TLS Error: handshake failed", ""));
        assert!(matches_filter("TLS Error: handshake failed", "tls"));
        assert!(matches_filter("TLS Error: handshake failed", "HANDSHAKE"));
        assert!(!matches_filter("TLS Error: handshake failed", "dns"));
    }

    #[test]
    fn formats_traffic_and_time() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_duration(Duration::from_secs(0)), "00:00:00");
        assert_eq!(format_duration(Duration::from_secs(3661)), "01:01:01");
    }
}
