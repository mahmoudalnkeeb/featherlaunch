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
    write(desktop, &exe, inst, icon(inst).as_deref())
}

/// The instance's icon as a file Windows can show. Without one, the shortcut shows the launcher's.
#[cfg(windows)]
fn icon(inst: &Instance) -> Option<PathBuf> {
    let ico = crate::icons::ico_from_image(&std::fs::read(inst.icon_path()).ok()?).ok()?;
    let path = inst.dir().join("shortcut.ico");
    std::fs::write(&path, ico).ok()?;
    Some(path)
}

#[cfg(not(windows))]
fn icon(inst: &Instance) -> Option<PathBuf> {
    Some(inst.icon_path()).filter(|path| path.is_file())
}

/// Windows shortcuts are a binary format, so this asks Windows to write one.
#[cfg(windows)]
fn write(dir: &Path, exe: &Path, inst: &Instance, icon: Option<&Path>) -> Result<PathBuf> {
    let path = dir.join(format!("{}.lnk", file_name(&inst.name)));
    shell_link::save(&path, exe, &format!("--launch \"{}\"", inst.id), icon)?;
    Ok(path)
}

/// The two COM interfaces a shortcut needs. `windows-sys` has no interface types, so the tables
/// below follow their order in shobjidl_core.h and objidl.h, and only name the calls used here.
#[cfg(windows)]
mod shell_link {
    use std::ffi::{OsStr, c_void};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::null_mut;

    use anyhow::{Result, ensure};
    use windows_sys::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
    };
    use windows_sys::Win32::UI::Shell::ShellLink;
    use windows_sys::core::{GUID, HRESULT};

    const IID_SHELL_LINK: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    const IID_PERSIST_FILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);

    #[repr(C)]
    struct Unknown {
        query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    #[repr(C)]
    struct ShellLinkCalls {
        unknown: Unknown,
        get_path_to_get_arguments: [usize; 8],
        set_arguments: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
        get_hotkey_to_get_icon_location: [usize; 5],
        set_icon_location: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> HRESULT,
        set_relative_path_to_resolve: [usize; 2],
        set_path: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    }

    #[repr(C)]
    struct PersistFileCalls {
        unknown: Unknown,
        get_class_id_to_load: [usize; 3],
        save: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> HRESULT,
    }

    /// Releases a COM object when it goes out of scope.
    struct Object(*mut c_void);

    impl Object {
        /// # Safety
        /// The object has to implement the interface `T` describes.
        unsafe fn calls<T>(&self) -> &T {
            unsafe { &**self.0.cast::<*const T>() }
        }
    }

    impl Drop for Object {
        fn drop(&mut self) {
            unsafe { (self.calls::<Unknown>().release)(self.0) };
        }
    }

    struct ComThread(bool);

    impl Drop for ComThread {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain([0]).collect()
    }

    fn check(result: HRESULT, what: &str) -> Result<()> {
        ensure!(result >= 0, "{what} failed with {:#010x}", result as u32);
        Ok(())
    }

    pub fn save(path: &Path, target: &Path, arguments: &str, icon: Option<&Path>) -> Result<()> {
        // Failing here means the thread already runs COM in another mode, which works as well.
        let _com = ComThread(unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) } >= 0);
        let mut link = null_mut();
        check(
            unsafe { CoCreateInstance(&ShellLink, null_mut(), CLSCTX_INPROC_SERVER, &IID_SHELL_LINK, &mut link) },
            "creating a shortcut",
        )?;
        let link = Object(link);
        unsafe {
            let calls = link.calls::<ShellLinkCalls>();
            check((calls.set_path)(link.0, wide(target.as_os_str()).as_ptr()), "setting the shortcut's target")?;
            check((calls.set_arguments)(link.0, wide(OsStr::new(arguments)).as_ptr()), "setting the shortcut's arguments")?;
            if let Some(icon) = icon {
                check((calls.set_icon_location)(link.0, wide(icon.as_os_str()).as_ptr(), 0), "setting the shortcut's icon")?;
            }
            let mut file = null_mut();
            check((calls.unknown.query_interface)(link.0, &IID_PERSIST_FILE, &mut file), "opening the shortcut's file")?;
            let file = Object(file);
            check((file.calls::<PersistFileCalls>().save)(file.0, wide(path.as_os_str()).as_ptr(), 1), "saving the shortcut")
        }
    }
}

#[cfg(not(windows))]
fn write(dir: &Path, exe: &Path, inst: &Instance, icon: Option<&Path>) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(format!("{}.desktop", file_name(&inst.name)));
    std::fs::write(&path, desktop_entry(exe, inst, icon))?;
    // File managers won't start an entry that isn't executable.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

#[cfg(any(not(windows), test))]
fn desktop_entry(exe: &Path, inst: &Instance, icon: Option<&Path>) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec=\"{}\" --launch \"{}\"\nIcon={}\nTerminal=false\nCategories=Game;\n",
        inst.name.replace('\n', " "),
        exe.display(),
        inst.id,
        icon.map_or("featherlaunch".to_string(), |path| path.display().to_string())
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
        let entry = desktop_entry(Path::new("/opt/fl/featherlaunch"), &inst, None);
        assert!(entry.contains("Name=My Pack\n"));
        assert!(entry.contains("Exec=\"/opt/fl/featherlaunch\" --launch \"my_pack\"\n"));
        assert!(entry.contains("Icon=featherlaunch\n"));
        let entry = desktop_entry(Path::new("/opt/fl/featherlaunch"), &inst, Some(Path::new("/data/my_pack/icon.png")));
        assert!(entry.contains("Icon=/data/my_pack/icon.png\n"));
    }

    #[test]
    fn creates_a_shortcut_file() {
        let dir = std::env::temp_dir().join(format!("featherlaunch-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let inst = Instance { id: "my_pack".into(), name: "My: Pack".into(), ..Default::default() };
        let icon = dir.join("shortcut.ico");
        let path = write(&dir, Path::new("featherlaunch"), &inst, Some(&icon)).unwrap();
        assert!(path.starts_with(&dir) && path.exists(), "{}", path.display());
        #[cfg(windows)]
        {
            // Shortcuts store their text as UTF-16.
            let saved = std::fs::read(&path).unwrap();
            for text in ["--launch \"my_pack\"", icon.to_str().unwrap()] {
                let wanted: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
                assert!(saved.windows(wanted.len()).any(|part| part == wanted), "{text} is in the shortcut");
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
