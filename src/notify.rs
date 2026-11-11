//! Desktop notifications through `org.freedesktop.Notifications`.
//!
//! The window lives in the tray most of the time, so a connection that drops
//! while it is hidden would otherwise go unnoticed.

use std::collections::HashMap;

use zbus::zvariant::Value;

const APP: &str = "Ether";
const ICON: &str = "ether-ovpn";

/// Shows a notification. Never blocks the caller and stays silent when the
/// desktop has no notification service: the D-Bus round trip runs on its own
/// thread and every failure is dropped.
pub fn send(summary: impl Into<String>, body: impl Into<String>) {
    let (summary, body) = (summary.into(), body.into());
    std::thread::spawn(move || {
        let Ok(connection) = zbus::blocking::Connection::session() else { return };
        let hints: HashMap<String, Value> = HashMap::new();
        let _ = connection.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(APP, 0u32, ICON, summary, body, Vec::<String>::new(), hints, 5000i32),
        );
    });
}
