//! The system tray icon (StatusNotifierItem) and its menu.

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{StandardItem, SubMenu};
use ksni::{Icon, MenuItem, ToolTip};

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
        "vpndeck".into()
    }

    fn title(&self) -> String {
        "VPNdeck".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let svg = icon_svg(self.state.status);
        [22, 32, 48, 64].into_iter().filter_map(|size| render(&svg, size)).collect()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: "VPNdeck".into(), description: self.state.status_text.clone(), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        (self.send)(Command::ShowWindow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let s = &self.state;
        let mut menu = vec![
            StandardItem { label: s.status_text.clone(), enabled: false, ..Default::default() }.into(),
            MenuItem::Separator,
            item("Show VPNdeck", true, || Command::ShowWindow),
        ];
        if s.busy {
            menu.push(item("Disconnect", true, || Command::Disconnect));
        } else {
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
            menu.push(SubMenu { label: "Connect".into(), enabled: !s.profiles.is_empty(), submenu, ..Default::default() }.into());
        }
        menu.push(MenuItem::Separator);
        match &s.update {
            Some(version) => menu.push(item(format!("Update to {version}…"), true, || Command::ShowWindow)),
            None => menu.push(item("Check for updates", true, || Command::CheckUpdates)),
        }
        menu.push(item("Quit", true, || Command::Quit));
        menu
    }
}

/// The tray version of the app icon: the shield without its tile, plus a status dot.
fn icon_svg(status: Status) -> String {
    let (top, bottom) = match status {
        Status::Idle => ("#A3A8BA", "#646A80"),
        _ => ("#6D7CFF", "#3127A8"),
    };
    let badge = match status {
        Status::Idle => String::new(),
        Status::Connecting => badge("#F59E0B"),
        Status::Connected => badge("#22C55E"),
        Status::Error => badge("#EF4444"),
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{top}"/>
      <stop offset="1" stop-color="{bottom}"/>
    </linearGradient>
  </defs>
  <path d="M32 3 L57 12 V31 C57 46 47 56 32 61 C17 56 7 46 7 31 V12 Z" fill="url(#g)"/>
  <path d="M21 47 V33 A11 11 0 0 1 43 33 V47 Z" fill="#FFFFFF"/>
  {badge}
</svg>"##
    )
}

fn badge(color: &str) -> String {
    format!(r##"<circle cx="49" cy="49" r="12" fill="{color}" stroke="#FFFFFF" stroke-width="4"/>"##)
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
