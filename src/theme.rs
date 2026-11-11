//! Asks the desktop whether it prefers a dark or a light interface.
//!
//! The answer comes from the XDG desktop portal, which every modern desktop
//! implements. When the portal isn't there, dark stays the default.

use zbus::zvariant::OwnedValue;

/// `org.freedesktop.appearance color-scheme`: 1 is dark, 2 is light, 0 is no
/// preference.
pub fn prefers_dark() -> bool {
    match color_scheme() {
        Some(2) => false,
        _ => true,
    }
}

fn color_scheme() -> Option<u32> {
    let connection = zbus::blocking::Connection::session().ok()?;
    let reply = connection
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.Settings"),
            "ReadOne",
            &("org.freedesktop.appearance", "color-scheme"),
        )
        .ok()?;
    let value: OwnedValue = reply.body().deserialize().ok()?;
    u32::try_from(value).ok()
}
