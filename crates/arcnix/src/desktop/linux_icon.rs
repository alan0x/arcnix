use std::{fs, io::ErrorKind, path::Path};

use anyhow::{Context, Result};
use gtk::{gdk_pixbuf::Pixbuf, glib};

const ICON: &[u8] = include_bytes!("../../assets/arcnix.png");
const LAUNCHER: &str = "applications/arcnix.desktop";
const GROUP: &str = "Desktop Entry";

pub(crate) fn install() -> Result<()> {
    gtk::Window::set_default_icon(&Pixbuf::from_read(ICON)?);
    // Packaged installations can supply their own launcher. The portable
    // archive needs a user entry so GNOME can resolve the Wayland app_id.
    if glib::system_data_dirs()
        .iter()
        .any(|dir| dir.join(LAUNCHER).is_file())
    {
        return Ok(());
    }
    install_launcher(&glib::user_data_dir(), &std::env::current_exe()?)
}

fn install_launcher(data: &Path, executable: &Path) -> Result<()> {
    let launcher = data.join(LAUNCHER);
    let icon = data.join("arcnix/arcnix.png");
    let entry = glib::KeyFile::new();
    match fs::read_to_string(&launcher) {
        Ok(contents) => {
            entry.load_from_data(
                &contents,
                glib::KeyFileFlags::KEEP_COMMENTS | glib::KeyFileFlags::KEEP_TRANSLATIONS,
            )?;
            if !entry.boolean(GROUP, "X-Arcnix-Portable").unwrap_or(false) {
                return Ok(());
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            entry.set_string(GROUP, "Type", "Application");
            entry.set_string(GROUP, "Name", "Arcnix");
            entry.set_string(GROUP, "StartupWMClass", "Arcnix");
            entry.set_boolean(GROUP, "X-Arcnix-Portable", true);
        }
        Err(error) => return Err(error.into()),
    }
    let path = executable.to_str().context("Non-UTF-8 executable path")?;
    // Quote an Exec argument before KeyFile applies desktop-file escaping.
    let mut command = String::from("\"");
    for ch in path.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => command.push('\\'),
            '%' => command.push('%'),
            _ => {}
        }
        command.push(ch);
    }
    command.push('"');
    entry.set_string(GROUP, "Exec", &command);
    entry.set_string(GROUP, "Icon", icon.to_str().context("Non-UTF-8 icon path")?);
    write_changed(&icon, ICON)?;
    write_changed(&launcher, entry.to_data().as_bytes())
}

fn write_changed(path: &Path, bytes: &[u8]) -> Result<()> {
    if fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(());
    }
    fs::create_dir_all(path.parent().context("Missing desktop data directory")?)?;
    glib::file_set_contents(path, bytes)?;
    Ok(())
}
