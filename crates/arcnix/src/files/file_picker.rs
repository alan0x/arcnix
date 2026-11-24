//! Native file selection; returns a path without opening the selected key.
use std::path::{Path, PathBuf};

pub(crate) fn field(ui: crate::Ui) -> impl glory::prelude::Widget {
    use glory::prelude::*;

    use crate::{AuthMethod, Dialog, i18n};
    let state = ui.state;
    col().class("field").fill((
        text()
            .class("field-label")
            .content(i18n::bind("私钥文件路径")),
        row().class("file-picker-field").fill((
            input()
                .name(i18n::bind("私钥文件路径"))
                .value(state.map(|s| {
                    s.editor
                        .private_key
                        .as_ref()
                        .map(|path| crate::local_display(path))
                        .unwrap_or_default()
                }))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        state.revise(|s| {
                            s.editor.private_key = if value.trim().is_empty() {
                                None
                            } else {
                                Some(value.into())
                            };
                            s.dialog_error.clear();
                        });
                    }
                }),
            sapid::button()
                .class("button browse-button")
                .label(i18n::bind("Browse…"))
                .on_press(move || {
                    let (current, id) =
                        state.with(|s| (s.editor.private_key.clone(), s.editor.id.clone()));
                    match private_key(
                        current.as_deref(),
                        &i18n::translate("Select SSH private key"),
                        &i18n::translate("Select file"),
                        &i18n::translate("All files"),
                    ) {
                        Ok(Some(path)) => state.revise(|s| {
                            if s.dialog == Some(Dialog::Server)
                                && s.editor.id == id
                                && s.editor.auth == AuthMethod::PrivateKey
                            {
                                s.editor.private_key = Some(path);
                                s.dialog_error.clear();
                            }
                        }),
                        Ok(None) => {}
                        Err(error) => state.revise(|s| {
                            s.dialog_error = format!("Could not select private key: {error:#}")
                        }),
                    }
                }),
        )),
    ))
}

#[cfg(windows)]
pub(crate) fn initial_directory(current: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = current {
        if path.is_dir() {
            return Some(path.to_path_buf());
        }
        if let Some(parent) = path.parent().filter(|parent| parent.is_dir()) {
            return Some(parent.to_path_buf());
        }
    }
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join(".ssh"))
        .filter(|path| path.is_dir())
}

#[cfg(windows)]
pub(crate) fn private_key(
    current: Option<&Path>,
    title: &str,
    choose: &str,
    all_files: &str,
) -> anyhow::Result<Option<PathBuf>> {
    pick_path(current, title, choose, all_files, false)
}
#[cfg(windows)]
fn pick_path(
    current: Option<&Path>,
    title: &str,
    choose: &str,
    all_files: &str,
    folder: bool,
) -> anyhow::Result<Option<PathBuf>> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use anyhow::{Context, ensure};
    use windows::Win32::Foundation::ERROR_CANCELLED;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor};
    use windows::core::{HRESULT, HSTRING};
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    // Called on the UI thread. The owner disables the application while Windows
    // runs its modal loop, so the result cannot land in a different server form.
    unsafe {
        let active = GetActiveWindow();
        ensure!(
            !active.is_invalid(),
            "Cannot find the application window for file selection"
        );
        let owner = GetAncestor(active, GA_ROOT);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .context("Cannot initialize the file dialog")?;
        let _apartment = Apartment;
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        configure_picker(&dialog, folder, all_files)?;
        dialog.SetTitle(&HSTRING::from(title))?;
        dialog.SetOkButtonLabel(&HSTRING::from(choose))?;
        let directory = if folder && current.is_none() {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .filter(|p| p.is_dir())
        } else {
            initial_directory(current)
        };
        if let Some(directory) = directory {
            let directory = HSTRING::from(directory.as_os_str());
            if let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(&directory, None) {
                let _ = dialog.SetFolder(&folder);
            }
        }
        match dialog.Show(Some(owner)) {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => {
                return Ok(None);
            }
            Err(error) => return Err(error).context("Cannot open the file selection window"),
        }
        let item = dialog.GetResult()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = PathBuf::from(OsString::from_wide(name.as_wide()));
        CoTaskMemFree(Some(name.0.cast()));
        ensure!(
            if folder {
                path.is_dir()
            } else {
                path.is_file()
            },
            "Please select an existing path"
        );
        Ok(Some(path))
    }
}

#[cfg(windows)]
fn configure_picker(
    dialog: &windows::Win32::UI::Shell::IFileOpenDialog,
    folder: bool,
    all_files: &str,
) -> anyhow::Result<()> {
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FOS_DONTADDTORECENT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR,
        FOS_PATHMUSTEXIST, FOS_PICKFOLDERS,
    };
    use windows::core::{HSTRING, PCWSTR, w};
    unsafe {
        dialog.SetOptions(
            dialog.GetOptions()?
                | if folder {
                    FOS_PICKFOLDERS
                } else {
                    FOS_FILEMUSTEXIST
                }
                | FOS_PATHMUSTEXIST
                | FOS_FORCEFILESYSTEM
                | FOS_DONTADDTORECENT
                | FOS_NOCHANGEDIR,
        )?;
        // Windows rejects SetFileTypes when FOS_PICKFOLDERS is active.
        if !folder {
            let name = HSTRING::from(all_files);
            dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
                pszName: PCWSTR(name.as_ptr()),
                pszSpec: w!("*"),
            }])?;
        }
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod native_tests {
    #[test]
    fn native_folder_and_file_options_are_accepted_by_windows() -> anyhow::Result<()> {
        use windows::Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        };
        use windows::Win32::UI::Shell::{
            FOS_FILEMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog,
        };
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _apartment = Apartment;
        for folder in [true, false] {
            let dialog: IFileOpenDialog =
                unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)? };
            super::configure_picker(&dialog, folder, "All files")?;
            let options = unsafe { dialog.GetOptions()? };
            assert_eq!(options.contains(FOS_PICKFOLDERS), folder);
            if !folder {
                assert!(options.contains(FOS_FILEMUSTEXIST));
            }
        }
        Ok(())
    }
}

pub(crate) fn folder(current: Option<&Path>, title: &str) -> anyhow::Result<Option<PathBuf>> {
    #[cfg(windows)]
    {
        pick_path(
            current,
            title,
            &crate::i18n::translate("Select folder"),
            "",
            true,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = (current, title);
        anyhow::bail!("Enter the folder path manually on this platform")
    }
}
#[cfg(windows)]
pub(crate) fn save(title: &str, name: &str) -> anyhow::Result<Option<PathBuf>> {
    save_in(title, name, None, None)
}

/// `folder` is where the dialog starts. `owner` is the window it belongs
/// to; without one it uses the active window.
#[cfg(windows)]
pub(crate) fn save_in(
    title: &str,
    name: &str,
    folder: Option<&Path>,
    owner: Option<isize>,
) -> anyhow::Result<Option<PathBuf>> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use windows::Win32::Foundation::ERROR_CANCELLED;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST,
        FileSaveDialog, IFileSaveDialog, IShellItem, SHCreateItemFromParsingName,
        SIGDN_FILESYSPATH,
    };
    use windows::core::{HRESULT, HSTRING};
    // The dialog owns its native modal loop; no shell command is constructed.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _apartment = Apartment;
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetTitle(&HSTRING::from(title))?;
        dialog.SetFileName(&HSTRING::from(name))?;
        if let Some(folder) = folder
            && let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                &HSTRING::from(folder.as_os_str()),
                None,
            )
        {
            let _ = dialog.SetFolder(&item);
        }
        dialog.SetOptions(
            dialog.GetOptions()?
                | FOS_FORCEFILESYSTEM
                | FOS_PATHMUSTEXIST
                | FOS_NOCHANGEDIR
                | FOS_OVERWRITEPROMPT,
        )?;
        let owner = owner.map_or_else(
            || GetActiveWindow(),
            |owner| windows::Win32::Foundation::HWND(owner as *mut _),
        );
        match dialog.Show(Some(owner)) {
            Ok(()) => {}
            Err(e) if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        let value = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = PathBuf::from(OsString::from_wide(value.as_wide()));
        CoTaskMemFree(Some(value.0.cast()));
        Ok(Some(path))
    }
}
#[cfg(not(windows))]
pub(crate) fn save(_title: &str, _name: &str) -> anyhow::Result<Option<PathBuf>> {
    anyhow::bail!("Native save dialogs are not available on this platform")
}

#[cfg(not(windows))]
pub(crate) fn private_key(
    _current: Option<&Path>,
    _title: &str,
    _choose: &str,
    _all_files: &str,
) -> anyhow::Result<Option<PathBuf>> {
    anyhow::bail!(
        "Native file selection is currently supported on Windows. Enter the path manually."
    )
}
