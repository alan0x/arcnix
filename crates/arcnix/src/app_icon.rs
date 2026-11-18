//! Native window presentation for the server desktop.
use std::sync::Arc;

use glory::prelude::Cage;
use glory::wry::shell::{Fullscreen, NativeLifecycle, NativeWindow, ShellError};

use super::desktop_customization_ui::{WallpaperAssets, wallpaper_asset};

pub(super) struct DesktopWindow {
    pub state: Cage<super::State>,
    pub fullscreen: bool,
    pub wallpapers: Arc<WallpaperAssets>,
    #[cfg(windows)]
    pub browser: super::web_browser_native::Host,
}

impl NativeLifecycle for DesktopWindow {
    fn poll(&mut self, window: &NativeWindow) -> Result<bool, ShellError> {
        let fullscreen = self.state.with(super::server_desktop::fullscreen);
        self.wallpapers.show(self.state.with(wallpaper_asset));
        if self.fullscreen != fullscreen {
            window.set_fullscreen(
                fullscreen.then(|| Fullscreen::Borderless(window.current_monitor())),
            );
            self.fullscreen = fullscreen;
        }
        #[cfg(windows)]
        {
            self.browser.poll(window);
            if self.browser.exit_ready() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[cfg(windows)]
    fn close_requested(&mut self, _window: &NativeWindow) -> Result<bool, ShellError> {
        Ok(self.browser.close_requested())
    }

    fn closed(&mut self, _window: &NativeWindow) {
        self.wallpapers.show(None);
        #[cfg(windows)]
        self.browser.close();
    }

    #[cfg(windows)]
    fn opened(
        &mut self,
        window: &NativeWindow,
        wake: &std::task::Waker,
    ) -> Result<(), ShellError> {
        self.browser.set_wake(wake.clone());
        use glory::wry::shell::NativeWindowExtWindows;
        use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_SHARED, LoadImageW, SM_CXICON,
            SM_CXSMICON, SM_CYICON, SM_CYSMICON, SendMessageW, WM_SETICON,
        };
        use windows::core::PCWSTR;

        // LR_SHARED gives the module ownership of these resource-backed handles.
        unsafe {
            let instance = GetModuleHandleW(None)
                .map_err(|error| glory::wry::shell::ShellError(error.to_string()))?;
            for (kind, width, height) in [
                (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
                (ICON_BIG, SM_CXICON, SM_CYICON),
            ] {
                let icon = LoadImageW(
                    Some(HINSTANCE(instance.0)),
                    PCWSTR(std::ptr::without_provenance(1)),
                    IMAGE_ICON,
                    GetSystemMetrics(width),
                    GetSystemMetrics(height),
                    LR_SHARED,
                )
                .map_err(|error| glory::wry::shell::ShellError(error.to_string()))?;
                SendMessageW(
                    HWND(window.hwnd() as *mut _),
                    WM_SETICON,
                    Some(WPARAM(kind as usize)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
        Ok(())
    }
}
