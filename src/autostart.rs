//! Starts Ether with the desktop session, through an XDG autostart entry.

use std::fs;
use std::io;
use std::path::PathBuf;

const FILE: &str = "ether-ovpn.desktop";

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("autostart").join(FILE))
}

pub fn enabled() -> bool {
    path().is_some_and(|path| path.exists())
}

/// Writes or removes the autostart entry. The entry points at this copy of the
/// program, so a tarball install and a package install each start themselves.
pub fn set(enabled: bool) -> io::Result<()> {
    let path = path().ok_or_else(|| io::Error::other("there is no config folder"))?;
    if !enabled {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
    }
    let exe = std::env::current_exe()?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // Desktop entries split Exec on spaces, so a path with one has to be quoted.
    let exec = if exe.to_string_lossy().contains(' ') {
        format!("\"{}\"", exe.display())
    } else {
        exe.display().to_string()
    };
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Ether\n\
         Comment=Connect to OpenVPN servers\n\
         Exec={exec}\n\
         Icon=ether-ovpn\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n"
    );
    fs::write(path, entry)
}
