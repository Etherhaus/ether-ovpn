//! The system tray icon (StatusNotifierItem) and its menu.

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{StandardItem, SubMenu};
use ksni::{Icon, MenuItem, ToolTip};

use crate::i18n::{fill, Lang, Strings};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Connecting,
    Connected,
    Error,
}

pub enum Command {
    ShowWindow,
    Connect(usize),
    Disconnect,
    CheckUpdates,
    Quit,
}

/// Everything the tray shows. The app pushes a new copy whenever it changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub status: Status,
    pub status_text: String,
    pub profiles: Vec<String>,
    pub busy: bool,
    pub update: Option<String>,
    pub lang: Lang,
    /// The profile connected to most recently, offered as a one-click entry.
    pub last: Option<(usize, String)>,
}

pub struct Tray {
    state: State,
    send: Box<dyn Fn(Command) + Send>,
}

/// Starts the tray icon, or returns `None` when the desktop has no tray to show it in.
pub fn spawn(send: impl Fn(Command) + Send + 'static) -> Option<Handle<Tray>> {
    // A sandboxed app can't own the StatusNotifierItem-<pid> bus name.
    Tray { state: State::default(), send: Box::new(send) }.disable_dbus_name(crate::sandboxed()).spawn().ok()
}

pub fn set_state(handle: &Handle<Tray>, state: State) {
    handle.update(|tray| tray.state = state);
}

fn item(label: impl Into<String>, enabled: bool, command: fn() -> Command) -> MenuItem<Tray> {
    StandardItem {
        label: label.into(),
        enabled,
        activate: Box::new(move |tray: &mut Tray| (tray.send)(command())),
        ..Default::default()
    }
    .into()
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "ether-ovpn".into()
    }

    fn title(&self) -> String {
        "Ether".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let svg = icon_svg(self.state.status);
        [22, 32, 48, 64].into_iter().filter_map(|size| render(&svg, size)).collect()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: "Ether".into(), description: self.state.status_text.clone(), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        (self.send)(Command::ShowWindow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let s = &self.state;
        let t = Strings::new(s.lang);
        let mut menu = vec![
            StandardItem { label: s.status_text.clone(), enabled: false, ..Default::default() }.into(),
            MenuItem::Separator,
            item(t.tray_show, true, || Command::ShowWindow),
        ];
        if s.busy {
            menu.push(item(t.disconnect, true, || Command::Disconnect));
        } else {
            // The profile used last is one click away, above the full list.
            if let Some((index, name)) = &s.last {
                let index = *index;
                menu.push(
                    StandardItem {
                        label: fill(t.tray_connect_last, &[name]),
                        activate: Box::new(move |tray: &mut Tray| (tray.send)(Command::Connect(index))),
                        ..Default::default()
                    }
                    .into(),
                );
            }
            let submenu = s
                .profiles
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    StandardItem {
                        label: name.clone(),
                        activate: Box::new(move |tray: &mut Tray| (tray.send)(Command::Connect(i))),
                        ..Default::default()
                    }
                    .into()
                })
                .collect();
            menu.push(SubMenu { label: t.tray_connect.into(), enabled: !s.profiles.is_empty(), submenu, ..Default::default() }.into());
        }
        menu.push(MenuItem::Separator);
        match &s.update {
            Some(version) => menu.push(item(t.tray_update_to.replace("{}", version), true, || Command::ShowWindow)),
            None => menu.push(item(t.tray_check_updates, true, || Command::CheckUpdates)),
        }
        menu.push(item(t.tray_quit, true, || Command::Quit));
        menu
    }
}

/// The tray version of the app icon: the shield mark, tinted by status.
///
/// The tray has no room for the tile, so the mark stands alone. The colours are
/// the ones the window uses for the same states — coral when connected, never
/// green.
fn icon_svg(status: Status) -> String {
    let color = match status {
        Status::Idle => "#8b8b93",
        Status::Connecting => "#f8e45c",
        Status::Connected => "#fb8762",
        Status::Error => "#f66151",
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">
  <g transform="translate(32 31) scale(1.18) translate(-32 -31)">
    <path fill-rule="evenodd" fill="{color}" d="M32 11.5 C 37.75 15.25 43.25 17.25 49.5 17.75 C 49.5 29.5 47 41.25 32 50.5 C 17 41.25 14.5 29.5 14.5 17.75 C 20.75 17.25 26.25 15.25 32 11.5 Z M32 25.25 a 5.25 5.25 0 0 1 5.25 5.25 c 0 2.125 -1.25 3.875 -3 4.75 l 2 8 q 0.25 1.25 -1 1.25 h -6.5 q -1.25 0 -1 -1.25 l 2 -8 c -1.75 -0.875 -3 -2.625 -3 -4.75 a 5.25 5.25 0 0 1 5.25 -5.25 Z"/>
  </g>
</svg>"##
    )
}

fn render(svg: &str, size: u32) -> Option<Icon> {
    let tree = resvg::usvg::Tree::from_str(svg, &Default::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
    let scale = size as f32 / tree.size().width();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // StatusNotifierItem wants straight-alpha ARGB32 in network byte order.
    let data = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.alpha(), c.red(), c.green(), c.blue()]
        })
        .collect();
    Some(Icon { width: size as i32, height: size as i32, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_every_status_icon() {
        for status in [Status::Idle, Status::Connecting, Status::Connected, Status::Error] {
            let icon = render(&icon_svg(status), 22).expect("icon renders");
            assert_eq!(icon.data.len(), 22 * 22 * 4);
            assert!(icon.data.chunks(4).any(|px| px[0] == 255), "icon is not blank");
        }
    }
}
