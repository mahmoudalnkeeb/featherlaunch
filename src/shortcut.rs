// SPDX-License-Identifier: GPL-3.0-only
//! Desktop shortcuts that start an instance.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::minecraft::instance::Instance;

fn file_name(name: &str) -> String {
    name.chars().map(|c| if "/\\:*?\"<>|".contains(c) { '-' } else { c }).collect()
}

/// Creates a shortcut on the desktop and returns where it is.
pub fn create(inst: &Instance) -> Result<PathBuf> {
    let dirs = directories::UserDirs::new().context("no home folder")?;
    let desktop = dirs.desktop_dir().context("no desktop folder")?;
    let exe = std::env::current_exe().context("finding the launcher's own path")?;
    write(desktop, &exe, inst)
}

/// Windows shortcuts are a binary format, so this asks Windows to write one. The values go through
/// the environment, so a name can't break out of the script.
#[cfg(windows)]
fn write(dir: &Path, exe: &Path, inst: &Instance) -> Result<PathBuf> {
    use std::os::windows::process::CommandExt;
    let path = dir.join(format!("{}.lnk", file_name(&inst.name)));
    let script = "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:FL_LINK); \
                  $s.TargetPath = $env:FL_EXE; $s.Arguments = $env:FL_ARGS; $s.Save()";
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("FL_LINK", &path)
        .env("FL_EXE", exe)
        .env("FL_ARGS", format!("--launch \"{}\"", inst.id))
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .status()
        .context("starting PowerShell")?;
    anyhow::ensure!(status.success(), "PowerShell could not create the shortcut");
    Ok(path)
}

#[cfg(not(windows))]
fn write(dir: &Path, exe: &Path, inst: &Instance) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(format!("{}.desktop", file_name(&inst.name)));
    std::fs::write(&path, desktop_entry(exe, inst))?;
    // File managers won't start an entry that isn't executable.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

#[cfg(any(not(windows), test))]
fn desktop_entry(exe: &Path, inst: &Instance) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec=\"{}\" --launch \"{}\"\nIcon=featherlaunch\nTerminal=false\nCategories=Game;\n",
        inst.name.replace('\n', " "),
        exe.display(),
        inst.id
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_for_files() {
        assert_eq!(file_name("Pack: 1/2"), "Pack- 1-2");
    }

    #[test]
    fn a_desktop_entry_launches_the_instance() {
        let inst = Instance { id: "my_pack".into(), name: "My Pack".into(), ..Default::default() };
        let entry = desktop_entry(Path::new("/opt/fl/featherlaunch"), &inst);
        assert!(entry.contains("Name=My Pack\n"));
        assert!(entry.contains("Exec=\"/opt/fl/featherlaunch\" --launch \"my_pack\"\n"));
    }

    #[test]
    fn creates_a_shortcut_file() {
        let dir = std::env::temp_dir().join(format!("featherlaunch-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let inst = Instance { id: "my_pack".into(), name: "My: Pack".into(), ..Default::default() };
        let path = write(&dir, Path::new("featherlaunch"), &inst).unwrap();
        assert!(path.starts_with(&dir) && path.exists(), "{}", path.display());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
