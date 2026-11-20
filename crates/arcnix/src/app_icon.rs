//! Native window presentation for the server desktop.
use std::collections::HashMap;
use std::task::Waker;

use glory::wry::shell::{Fullscreen, NativeLifecycle, NativeWindow, ShellError};

use super::{Ui, WorkspacePage, code_native, desktop_customization_ui::wallpaper_asset};

pub(super) struct DesktopWindow {
    ui: Ui,
    fullscreen: bool,
    wake: Option<Waker>,
    epoch: Option<u64>,
    code: HashMap<u64, code_native::Host>,
    #[cfg(windows)]
    browsers: HashMap<u64, super::web_browser_native::Host>,
    #[cfg(windows)]
    video: super::mpv_native::Host,
    #[cfg(windows)]
    exiting: bool,
}

impl DesktopWindow {
    pub(super) fn new(ui: Ui) -> Self {
        Self {
            #[cfg(windows)]
            video: super::mpv_native::Host::new(ui.clone()),
            ui,
            fullscreen: false,
            wake: None,
            epoch: None,
            code: HashMap::new(),
            #[cfg(windows)]
            browsers: HashMap::new(),
            #[cfg(windows)]
            exiting: false,
        }
    }
}

impl NativeLifecycle for DesktopWindow {
    fn poll(&mut self, window: &NativeWindow) -> Result<bool, ShellError> {
        let state = self.ui.state;
        let fullscreen = state.with(super::server_desktop::fullscreen);
        self.ui.wallpapers.show(state.with(wallpaper_asset));
        self.ui.player.sync_surface(&self.ui);
        if self.fullscreen != fullscreen {
            window.set_fullscreen(
                fullscreen.then(|| Fullscreen::Borderless(window.current_monitor())),
            );
            self.fullscreen = fullscreen;
        }
        let (epoch, live, code_open) = state.with(|s| {
            (
                s.connected.then_some(s.epoch),
                s.connected_epochs(),
                s.workbench.apps.contains(&WorkspacePage::Code),
            )
        });
        if self.epoch != epoch {
            if let Some(old) = self.epoch {
                if let Some(code) = self.code.get_mut(&old) {
                    code.hide();
                }
                #[cfg(windows)]
                if let Some(browser) = self.browsers.get_mut(&old) {
                    browser.hide();
                }
            }
            self.epoch = epoch;
        }
        self.code.retain(|epoch, _| live.contains(epoch));
        if let Some(epoch) = epoch {
            if code_open {
                self.code
                    .entry(epoch)
                    .or_insert_with(|| {
                        let mut code = code_native::Host::new(self.ui.clone());
                        code.wake = self.wake.clone();
                        code
                    })
                    .poll(window);
            } else if let Some(mut code) = self.code.remove(&epoch) {
                code.poll(window);
            }
        }
        #[cfg(windows)]
        {
            self.video.poll(window);
            if self.exiting {
                return Ok(self
                    .browsers
                    .values_mut()
                    .all(|browser| browser.exit_ready()));
            }
            self.browsers.retain(|epoch, _| live.contains(epoch));
            if let Some(epoch) = epoch {
                if state.with(|s| s.workbench.apps.contains(&WorkspacePage::Browser)) {
                    self.browsers.entry(epoch).or_insert_with(|| {
                        let mut browser = super::web_browser_native::Host::new(self.ui.clone());
                        if let Some(wake) = &self.wake {
                            browser.set_wake(wake.clone());
                        }
                        browser
                    });
                }
                if let Some(browser) = self.browsers.get_mut(&epoch) {
                    browser.poll(window);
                }
            }
        }
        Ok(false)
    }

    #[cfg(windows)]
    fn close_requested(&mut self, _window: &NativeWindow) -> Result<bool, ShellError> {
        self.exiting = true;
        let mut ready = true;
        for browser in self.browsers.values_mut() {
            ready &= browser.close_requested();
        }
        Ok(ready)
    }

    fn closed(&mut self, _window: &NativeWindow) {
        self.ui.wallpapers.show(None);
        #[cfg(windows)]
        for browser in self.browsers.values_mut() {
            browser.close();
        }
    }

    #[cfg(not(windows))]
    fn opened(
        &mut self,
        _window: &NativeWindow,
        wake: &std::task::Waker,
    ) -> Result<(), ShellError> {
        self.wake = Some(wake.clone());
        Ok(())
    }

    #[cfg(windows)]
    fn opened(&mut self, window: &NativeWindow, wake: &std::task::Waker) -> Result<(), ShellError> {
        self.wake = Some(wake.clone());
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
