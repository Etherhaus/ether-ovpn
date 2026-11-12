//! Every user-visible string, in English and Russian.
//!
//! The table below is the single place where text lives: adding a language means
//! adding a column, and nothing else in the app changes. The Slint side reads the
//! strings from the `Labels` global, which [`apply`] fills in.

use slint::{ComponentHandle, Global};

use crate::Labels;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    En,
    Ru,
}

impl Lang {
    pub fn from_code(code: &str) -> Lang {
        match code {
            "ru" => Lang::Ru,
            _ => Lang::En,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ru => "ru",
        }
    }

    /// The desktop's language, used until the user picks one.
    pub fn from_env() -> Lang {
        let value = std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LC_MESSAGES"))
            .or_else(|_| std::env::var("LANG"))
            .unwrap_or_default();
        if value.to_lowercase().starts_with("ru") {
            Lang::Ru
        } else {
            Lang::En
        }
    }
}

/// Declares the string table and the code that pushes it into the UI.
///
/// Each row is `field, setter => (english, russian)`. The setter is the Slint
/// property setter, spelled out because the property names are kebab-case.
macro_rules! labels_set {
    ($labels:ident, $s:ident, $field:ident, $setter:ident) => {
        $labels.$setter($s.$field.into());
    };
    // Strings that only the Rust side shows have no Slint property.
    ($labels:ident, $s:ident, $field:ident) => {};
}

macro_rules! labels {
    ($($field:ident $(, $setter:ident)? => ($en:literal, $ru:literal)),* $(,)?) => {
        /// Every string the app shows, in one language.
        #[derive(Clone, Copy)]
        pub struct Strings {
            $(pub $field: &'static str,)*
        }

        impl Strings {
            pub fn new(lang: Lang) -> Strings {
                match lang {
                    Lang::En => Strings { $($field: $en,)* },
                    Lang::Ru => Strings { $($field: $ru,)* },
                }
            }
        }

        /// Fills the `Labels` global of `ui` with the strings of `lang`.
        ///
        /// Globals are per component tree, so every window the app opens needs
        /// this call: a window built later starts with empty strings.
        pub fn apply<C>(ui: &C, lang: Lang)
        where
            C: ComponentHandle,
            for<'a> Labels<'a>: Global<'a, C>,
        {
            let s = Strings::new(lang);
            let labels = ui.global::<Labels>();
            $(labels_set!(labels, s, $field $(, $setter)?);)*
        }
    };
}

labels! {
    // Header
    log, set_log => ("Log", "Журнал"),
    settings, set_settings => ("Settings", "Настройки"),

    // Update banner
    whats_new, set_whats_new => ("What's new", "Что нового"),
    update_now, set_update_now => ("Update now", "Обновить"),
    restart_app, set_restart_app => ("Restart Ether", "Перезапустить Ether"),
    dismiss, set_dismiss => ("Dismiss", "Скрыть"),

    // Status card
    state_connected, set_state_connected => ("Connected", "Подключено"),
    state_connecting, set_state_connecting => ("Connecting…", "Подключение…"),
    state_disconnecting, set_state_disconnecting => ("Disconnecting…", "Отключение…"),
    state_error, set_state_error => ("Couldn't connect", "Не удалось подключиться"),
    state_idle, set_state_idle => ("Not connected", "Не подключено"),
    choose_profile, set_choose_profile => ("Choose a profile below", "Выберите профиль ниже"),
    vpn_address, set_vpn_address => ("VPN address", "VPN-адрес"),
    server, set_server => ("Server", "Сервер"),
    received, set_received => ("Received", "Получено"),
    sent, set_sent => ("Sent", "Отправлено"),

    // Main button
    connect, set_connect => ("Connect", "Подключиться"),
    disconnect, set_disconnect => ("Disconnect", "Отключиться"),

    // Profiles
    profiles, set_profiles => ("Profiles", "Профили"),
    import_label, set_import_label => ("+ Import", "+ Импорт"),
    import_profiles, set_import_profiles => ("Import profiles", "Импортировать профили"),
    password_saved, set_password_saved => ("Password saved", "Пароль сохранён"),
    forget_password, set_forget_password => ("Forget password", "Забыть пароль"),
    remove, set_remove => ("Remove", "Удалить"),
    search_placeholder, set_search_placeholder => ("Search profiles", "Поиск по профилям"),
    search_empty, set_search_empty => ("No profiles match", "Профили не найдены"),

    // Add-profile dialog
    add_title, set_add_title => ("Add a profile", "Добавить профиль"),
    add_files, set_add_files => ("Choose files…", "Выбрать файлы…"),
    add_paste, set_add_paste => ("Paste from clipboard", "Вставить из буфера"),
    add_url, set_add_url => ("Import from link", "Импортировать по ссылке"),
    add_url_placeholder, set_add_url_placeholder => ("https://example.com/profile.ovpn", "https://example.com/profile.ovpn"),
    add_hint, set_add_hint => (
        "You can also drop .ovpn files onto the window.",
        "Можно перетащить .ovpn файлы прямо в окно."
    ),

    // Rename dialog
    rename_title, set_rename_title => ("Rename profile", "Переименовать профиль"),
    rename, set_rename => ("Rename", "Переименовать"),
    rename_empty, set_rename_empty => ("The name can't be empty.", "Имя не может быть пустым."),
    rename_taken, set_rename_taken => ("Another profile already has that name.", "Такое имя уже занято."),

    // Profile window
    view, set_view => ("View", "Просмотр"),
    view_title, set_view_title => ("Profile", "Профиль"),
    view_check, set_view_check => ("Check connection", "Проверить подключение"),
    view_checking, set_view_checking => ("Checking...", "Проверяю..."),
    view_copy, set_view_copy => ("Copy", "Копировать"),
    view_close, set_view_close => ("Close", "Закрыть"),
    view_risky, set_view_risky => (
        "This profile runs programs as root. Only connect if it came from your own administrator.",
        "Этот профиль запускает программы от root. Подключайтесь, только если он пришёл от вашего администратора."
    ),
    check_file, set_check_file => ("Profile file", "Файл профиля"),
    check_remote, set_check_remote => ("Server line", "Строка remote"),
    check_files, set_check_files => ("Certificate files", "Файлы сертификатов"),
    check_dns, set_check_dns => ("Name lookup", "Разрешение имени"),
    check_port, set_check_port => ("Server port", "Порт сервера"),
    check_remote_missing, set_check_remote_missing => ("no remote line", "строка remote не найдена"),
    check_udp_skipped, set_check_udp_skipped => ("UDP profile, not probed", "Профиль UDP, не проверяется"),
    check_passed, set_check_passed => ("ok", "ок"),
    check_failed, set_check_failed => ("failed", "ошибка"),

    // Empty state
    empty_title, set_empty_title => ("No profiles yet", "Профилей пока нет"),
    empty_hint, set_empty_hint => (
        "Import one or more .ovpn files. Download them from your OpenVPN server.",
        "Импортируйте один или несколько .ovpn файлов. Скачайте их с вашего сервера OpenVPN."
    ),

    // Footer
    check_updates, set_check_updates => ("Check for updates", "Проверить обновления"),
    checking, set_checking => ("Checking…", "Проверка…"),

    // Settings
    theme, set_theme => ("Theme", "Тема"),
    dark, set_dark => ("Dark", "Тёмная"),
    light, set_light => ("Light", "Светлая"),
    system, set_system => ("System", "Как в системе"),
    language, set_language => ("Language", "Язык"),
    startup, set_startup => ("Startup", "Запуск"),
    autostart, set_autostart => ("Start with the system", "Запускать при входе"),
    auto_connect, set_auto_connect => ("Connect on launch", "Подключаться при запуске"),
    notifications, set_notifications => ("Notifications", "Уведомления"),
    notify_on, set_notify_on => ("Show connection notifications", "Показывать уведомления"),
    window, set_window => ("Window", "Окно"),
    frame, set_frame => ("Window frame", "Обрамление окна"),
    close, set_close => ("Close", "Закрыть"),

    // Remove confirmation
    remove_title => ("Remove “{}”?", "Удалить «{}»?"),
    remove_body_saved, set_remove_body_saved => (
        "The profile and its saved password will be deleted. You'll need the .ovpn file to add it again.",
        "Профиль и сохранённый пароль будут удалены. Чтобы добавить его снова, понадобится .ovpn файл."
    ),
    remove_body, set_remove_body => (
        "The profile will be deleted. You'll need the .ovpn file to add it again.",
        "Профиль будет удалён. Чтобы добавить его снова, понадобится .ovpn файл."
    ),
    cancel, set_cancel => ("Cancel", "Отмена"),

    // Sign-in dialog
    password_saved_hint, set_password_saved_hint => (
        "Password saved — only the 2FA code is needed",
        "Пароль сохранён — нужен только код 2FA"
    ),
    username, set_username => ("Username", "Имя пользователя"),
    password, set_password => ("Password", "Пароль"),
    code, set_code => ("2FA code", "Код 2FA"),
    show_password, set_show_password => ("Show password", "Показать пароль"),
    remember_password, set_remember_password => ("Remember password", "Запомнить пароль"),
    change_password, set_change_password => ("Change password", "Сменить пароль"),
    sign_in, set_sign_in => ("Sign in", "Войти"),
    continue_button, set_continue_button => ("Continue", "Продолжить"),

    // Log window
    log_title, set_log_title => ("Ether log", "Журнал Ether"),
    connection_log, set_connection_log => ("Connection log", "Журнал подключения"),
    copy_all, set_copy_all => ("Copy all", "Копировать всё"),
    save, set_save => ("Save…", "Сохранить…"),
    clear, set_clear => ("Clear", "Очистить"),
    log_empty, set_log_empty => (
        "Nothing logged yet. OpenVPN's output appears here while you connect.",
        "Записей пока нет. Вывод OpenVPN появится здесь при подключении."
    ),
    log_filter_placeholder, set_log_filter_placeholder => ("Filter the log", "Фильтр по журналу"),
    log_nothing_found, set_log_nothing_found => ("Nothing matches the filter.", "Ничего не найдено."),
    diagnostics, set_diagnostics => ("Copy diagnostics", "Скопировать диагностику"),

    // Messages from the Rust side
    import_scripts => (
        "This profile runs programs as root: {}.",
        "Этот профиль запускает программы от root: {}."
    ),
    import_pasted => ("Imported from the clipboard.", "Импортировано из буфера обмена."),
    import_clipboard_empty => (
        "The clipboard doesn't hold an OpenVPN profile.",
        "В буфере нет профиля OpenVPN."
    ),
    import_url_failed => ("Couldn't download the profile: {}", "Не удалось скачать профиль: {}"),
    import_url_not_profile => ("That link isn't an OpenVPN profile.", "По ссылке не профиль OpenVPN."),
    import_url_needs_https => (
        "Only https:// links are accepted: a profile fetched over http can be changed on the way.",
        "Принимаются только ссылки https://: профиль, скачанный по http, можно подменить в пути."
    ),
    notify_connected => ("Connected to {}", "Подключено к {}"),
    notify_disconnected => ("Disconnected", "Отключено"),
    notify_failed => ("Connection failed: {}", "Подключение не удалось: {}"),
    connection_lost => (
        "Lost the connection to OpenVPN. The tunnel may still be up — stop it with sudo pkill openvpn.",
        "Связь с OpenVPN потеряна. Туннель мог остаться поднятым — остановите его командой sudo pkill openvpn."
    ),
    disconnect_failed => (
        "OpenVPN isn't answering, so it can't be stopped from here. Stop it with sudo pkill openvpn.",
        "OpenVPN не отвечает, отсюда его не остановить. Остановите командой sudo pkill openvpn."
    ),
    settings_save_failed => ("Couldn't save the settings: {}", "Не удалось сохранить настройки: {}"),
    profiles_save_failed => (
        "Couldn't save the profile list: {}",
        "Не удалось сохранить список профилей: {}"
    ),
    log_save_failed => ("Couldn't save the log: {}", "Не удалось сохранить журнал: {}"),
    profile_read_failed => ("Couldn't read the profile: {}", "Не удалось прочитать профиль: {}"),
    notify_tray => ("Ether keeps running in the tray.", "Ether продолжает работать в трее."),
    reconnecting_attempt => (
        "Reconnecting… (attempt {} of {})",
        "Переподключение… (попытка {} из {})"
    ),
    reconnect_gave_up => (
        "Couldn't reconnect. Check the log.",
        "Не удалось переподключиться. Смотрите журнал."
    ),
    diagnostics_copied => (
        "Diagnostics copied to the clipboard.",
        "Диагностика скопирована в буфер."
    ),
    diagnostics_failed => (
        "Couldn't copy the diagnostics to the clipboard.",
        "Не удалось скопировать диагностику в буфер."
    ),
    downloading_profile => (
        "Downloading the profile…",
        "Загрузка профиля…"
    ),
    autostart_failed => (
        "Couldn't change the autostart entry: {}",
        "Не удалось изменить автозапуск: {}"
    ),
    tray_connect_last => ("Connect to {}", "Подключиться к {}"),
    theme_save_failed => (
        "Couldn't save the theme: {}",
        "Не удалось сохранить тему: {}"
    ),
    language_save_failed => (
        "Couldn't save the language: {}",
        "Не удалось сохранить язык: {}"
    ),
    save_log_title => ("Save log", "Сохранить журнал"),
    import_title => ("Import OpenVPN profiles", "Импорт профилей OpenVPN"),
    import_filter => ("OpenVPN profile", "Профиль OpenVPN"),
    imported => ("Imported", "Импортировано"),
    skipped => ("Skipped", "Пропущено"),
    import_duplicate => ("already added", "уже добавлен"),
    import_not_ovpn => ("not an .ovpn file", "не .ovpn файл"),
    remove_failed => (
        "Could not remove the profile: {}",
        "Не удалось удалить профиль: {}"
    ),
    waiting_admin => (
        "Waiting for administrator authorization…",
        "Ожидание подтверждения администратора…"
    ),
    start_failed => (
        "Could not start OpenVPN: {}",
        "Не удалось запустить OpenVPN: {}"
    ),
    disconnecting => ("Disconnecting…", "Отключение…"),
    checking_code => ("Checking the code…", "Проверка кода…"),
    signing_in => ("Signing in…", "Вход…"),
    downloading => ("Downloading Ether {}…", "Загрузка Ether {}…"),
    keyring_save_failed => (
        "Couldn't save the password in your keyring: {}",
        "Не удалось сохранить пароль в связке ключей: {}"
    ),
    keyring_forget_failed => (
        "Couldn't forget the password in your keyring: {}",
        "Не удалось удалить пароль из связки ключей: {}"
    ),
    update_available => (
        "Ether {} is available (you have {}).",
        "Доступна версия Ether {} (у вас {})."
    ),
    update_password_note => (
        " You'll be asked for your password.",
        " Понадобится ваш пароль."
    ),
    up_to_date => ("You're up to date.", "У вас последняя версия."),
    update_check_failed => (
        "Couldn't check: {}",
        "Не удалось проверить: {}"
    ),
    installing => ("Installing the update…", "Установка обновления…"),
    update_done => (
        "The update is installed. Restart Ether to start using it.",
        "Обновление установлено. Перезапустите Ether."
    ),
    update_done_busy => (
        " Restarting will disconnect the VPN.",
        " Перезапуск отключит VPN."
    ),
    update_failed => (
        "The update failed: {}",
        "Обновление не удалось: {}"
    ),
    tray_connected_to => ("Connected to {}", "Подключено к {}"),
    tray_disconnected => ("Disconnected", "Отключено"),
    reconnecting => ("Reconnecting…", "Переподключение…"),
    connecting_to => ("Connecting… ({})", "Подключение… ({})"),
    signing_in_saved => (
        "Signing in with your saved password…",
        "Вход с сохранённым паролем…"
    ),
    saved_password_failed => (
        "Your saved password didn't work. Enter it again.",
        "Сохранённый пароль не подошёл. Введите его снова."
    ),
    auth_failed => (
        "That didn't work. Check your username and password and try again.",
        "Не получилось. Проверьте имя пользователя и пароль."
    ),
    auth_pending_timeout => (
        "Waiting for authentication… the server times out in {}s",
        "Ожидание аутентификации… сервер прервёт через {} с"
    ),
    auth_pending => (
        "Waiting for authentication…",
        "Ожидание аутентификации…"
    ),
    enter_code => ("Enter the 2FA code", "Введите код 2FA"),
    sign_in_vpn => ("Sign in to the VPN", "Вход в VPN"),
    enter_password => ("Enter the {} password", "Введите пароль {}"),
    waiting_credentials => (
        "Waiting for your credentials…",
        "Ожидание данных для входа…"
    ),
    waiting_code => ("Waiting for the 2FA code…", "Ожидание кода 2FA…"),
    saved_password_read_failed => (
        "Couldn't read your saved password: {}",
        "Не удалось прочитать сохранённый пароль: {}"
    ),

    // Tray menu
    tray_show => ("Show Ether", "Открыть Ether"),
    tray_connect => ("Connect", "Подключиться"),
    tray_update_to => ("Update to {}…", "Обновить до {}…"),
    tray_check_updates => (
        "Check for updates",
        "Проверить обновления"
    ),
    tray_quit => ("Quit", "Выход"),
}

/// "Imported: 3. Skipped: 2: already added, not an .ovpn file."
pub fn import_summary(lang: Lang, imported: usize, skipped: &[String]) -> String {
    let s = Strings::new(lang);
    if skipped.is_empty() {
        return format!("{}: {}.", s.imported, imported);
    }
    format!(
        "{}: {}. {}: {}: {}.",
        s.imported,
        imported,
        s.skipped,
        skipped.len(),
        skipped.join(", ")
    )
}

/// Fills the `{}` placeholders of a template in order.
///
/// `str::replace` would fill every placeholder with the same value, which breaks
/// messages like "Ether {} is available (you have {})".
pub fn fill(template: &str, values: &[&str]) -> String {
    let mut out = template.to_string();
    for value in values {
        match out.find("{}") {
            Some(at) => out.replace_range(at..at + 2, value),
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_fills_every_string() {
        for lang in [Lang::En, Lang::Ru] {
            let s = Strings::new(lang);
            // A missing translation would show up as an empty label in the UI.
            assert!(!s.connect.is_empty());
            assert!(!s.empty_hint.is_empty());
            assert!(!s.tray_quit.is_empty());
        }
    }

    #[test]
    fn languages_differ() {
        let en = Strings::new(Lang::En);
        let ru = Strings::new(Lang::Ru);
        assert_ne!(en.connect, ru.connect);
        assert_ne!(en.settings, ru.settings);
        assert_ne!(en.empty_title, ru.empty_title);
    }

    #[test]
    fn codes_round_trip() {
        for lang in [Lang::En, Lang::Ru] {
            assert_eq!(Lang::from_code(lang.code()), lang);
        }
        assert_eq!(Lang::from_code("de"), Lang::En);
    }

    #[test]
    fn summary_counts_what_was_skipped() {
        assert_eq!(import_summary(Lang::En, 3, &[]), "Imported: 3.");
        let skipped = vec!["already added".to_string(), "not an .ovpn file".to_string()];
        assert_eq!(
            import_summary(Lang::En, 1, &skipped),
            "Imported: 1. Skipped: 2: already added, not an .ovpn file."
        );
        assert_eq!(import_summary(Lang::Ru, 2, &[]), "Импортировано: 2.");
    }

    #[test]
    fn fill_replaces_placeholders_in_order() {
        assert_eq!(
            fill("Ether {} is available (you have {}).", &["0.2.0", "0.1.4"]),
            "Ether 0.2.0 is available (you have 0.1.4)."
        );
        assert_eq!(
            fill("Could not start OpenVPN: {}", &["denied"]),
            "Could not start OpenVPN: denied"
        );
        // Fewer values than placeholders leaves the rest alone instead of panicking.
        assert_eq!(fill("{} and {}", &["one"]), "one and {}");
        assert_eq!(fill("no placeholders", &["x"]), "no placeholders");
    }
}
