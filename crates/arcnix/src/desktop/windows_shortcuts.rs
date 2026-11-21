//! Route the Windows equivalents of desktop actions while this HWND owns input.
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::task::Waker;
use std::thread::JoinHandle;

use arcnix_core::desktop_shortcuts::{Action, Preset, Scope};
use glory::wry::shell::{NativeWindow, NativeWindowExtWindows};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Input::{RAWINPUTDEVICE, RIDEV_REMOVE, RegisterRawInputDevices};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::desktop_shortcuts_ui::{can_run, desktop_active, keys, run};
use crate::{State, Ui, WorkspacePage, desktop_customization_ui, server_desktop};

#[derive(Clone, Copy)]
enum Command {
    Binding(Action),
    ShowDesktop,
    CloseWindow,
    CloseTab,
}

#[derive(Clone, Copy, PartialEq)]
enum Modifier {
    Alt,
    AltShift,
    Win,
    Ctrl,
}

const SHORTCUTS: [(VIRTUAL_KEY, Modifier, Command); 8] = [
    (VK_TAB, Modifier::Alt, Command::Binding(Action::NextApp)),
    (
        VK_TAB,
        Modifier::AltShift,
        Command::Binding(Action::PreviousApp),
    ),
    (VK_E, Modifier::Win, Command::Binding(Action::OpenFiles)),
    (VK_UP, Modifier::Win, Command::Binding(Action::Maximize)),
    (VK_DOWN, Modifier::Win, Command::Binding(Action::Minimize)),
    (VK_D, Modifier::Win, Command::ShowDesktop),
    (VK_F4, Modifier::Alt, Command::CloseWindow),
    (VK_F4, Modifier::Ctrl, Command::CloseTab),
];

pub(super) fn use_window_keyboard_input() -> windows::core::Result<()> {
    // Tao registers a raw keyboard sink, but Glory consumes only window input.
    // That unused registration prevents foreground low-level shortcut delivery
    // on Windows 11 (tauri-apps/wry#1761). Keep the pointer registration intact.
    let keyboard = RAWINPUTDEVICE {
        usUsagePage: 1,
        usUsage: 6,
        dwFlags: RIDEV_REMOVE,
        ..Default::default()
    };
    unsafe { RegisterRawInputDevices(&[keyboard], size_of::<RAWINPUTDEVICE>() as u32) }
}

struct Routing {
    window: HWND,
    enabled: Arc<AtomicU8>,
    held: [bool; SHORTCUTS.len()],
    pending: SyncSender<Command>,
    wake: Waker,
}

thread_local! {
    static ROUTING: RefCell<Option<Routing>> = const { RefCell::new(None) };
}

fn down(key: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(i32::from(key.0)) < 0 }
}

impl Routing {
    fn intercept(&mut self, event: &KBDLLHOOKSTRUCT) -> bool {
        let matches_key = |key: VIRTUAL_KEY| u32::from(key.0) == event.vkCode;
        if !SHORTCUTS.iter().any(|&(key, _, _)| matches_key(key)) {
            return false;
        }
        if let Some(index) = SHORTCUTS
            .iter()
            .enumerate()
            .position(|(index, &(key, _, _))| self.held[index] && matches_key(key))
        {
            if event.flags.contains(LLKHF_UP) {
                self.held[index] = false;
            }
            return true;
        }
        if event.flags.contains(LLKHF_UP) {
            return false;
        }
        // Read the other, already-held modifiers. Windows updates the current
        // key's asynchronous state only after the low-level callback returns.
        let win = down(VK_LWIN) || down(VK_RWIN);
        let alt = event.flags.contains(LLKHF_ALTDOWN);
        let shift = down(VK_SHIFT);
        let modifier = match (down(VK_CONTROL), win, alt, shift) {
            (true, false, false, false) => Modifier::Ctrl,
            (false, true, false, false) => Modifier::Win,
            (false, false, true, false) => Modifier::Alt,
            (false, false, true, true) => Modifier::AltShift,
            _ => return false,
        };
        let Some((index, &(_, _, command))) = SHORTCUTS
            .iter()
            .enumerate()
            .find(|&(_, &(key, mods, _))| matches_key(key) && mods == modifier)
        else {
            return false;
        };
        if self.enabled.load(Ordering::Relaxed) & (1 << index) == 0 {
            return false;
        }
        // Even if dispatch is backed up, never forward an internal Alt+F4
        // to the host as a request to quit the entire workspace.
        if self.pending.try_send(command).is_err() && !matches!(command, Command::CloseWindow) {
            return false;
        }
        self.held[index] = true;
        // A swallowed chord otherwise looks like a bare Win/Alt tap to the shell.
        // Use the unassigned key-up used by PowerToys' shortcut dispatcher.
        let mask_modifier = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0xff),
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        };
        if modifier != Modifier::Ctrl {
            unsafe {
                SendInput(&[mask_modifier], size_of::<INPUT>() as i32);
            }
        }
        self.wake.wake_by_ref();
        true
    }
}

unsafe extern "system" fn shortcut(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32
        && ROUTING.with(|slot| {
            let Ok(mut slot) = slot.try_borrow_mut() else {
                return false;
            };
            slot.as_mut().is_some_and(|route| {
                // Check again at delivery, even if the shell hasn't polled focus loss yet.
                unsafe {
                    GetForegroundWindow() == route.window
                        && route.intercept(&*(data.0 as *const KBDLLHOOKSTRUCT))
                }
            })
        })
    {
        return LRESULT(1);
    }
    unsafe { CallNextHookEx(None, code, message, data) }
}

struct Listener {
    thread: JoinHandle<()>,
    thread_id: u32,
    epoch: u64,
    enabled: Arc<AtomicU8>,
    pending: Receiver<Command>,
}

impl Listener {
    fn start(window: isize, epoch: u64, mask: u8, wake: Waker) -> anyhow::Result<Self> {
        let (commands, pending) = sync_channel(16);
        let (ready, started) = sync_channel(1);
        let enabled = Arc::new(AtomicU8::new(mask));
        let allowed = enabled.clone();
        // The WebView UI thread can be busy rendering. Windows needs a message
        // pump that can acknowledge a low-level input callback immediately.
        let thread = std::thread::Builder::new()
            .name("desktop-shortcuts".into())
            .spawn(move || unsafe {
                let mut msg = MSG::default();
                let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                let hook = GetModuleHandleW(None).and_then(|module| {
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(shortcut), Some(module.into()), 0)
                });
                let hook = match hook {
                    Ok(hook) => hook,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                ROUTING.with(|slot| {
                    slot.replace(Some(Routing {
                        window: HWND(window as *mut _),
                        enabled: allowed,
                        held: [false; SHORTCUTS.len()],
                        pending: commands,
                        wake,
                    }))
                });
                if ready.send(Ok(GetCurrentThreadId())).is_ok() {
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {}
                }
                let _ = UnhookWindowsHookEx(hook);
                ROUTING.with(|slot| slot.replace(None));
            })?;
        let thread_id = started.recv()??;
        Ok(Self {
            thread,
            thread_id,
            epoch,
            enabled,
            pending,
        })
    }

    fn stop(self) {
        self.enabled.store(0, Ordering::Relaxed);
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        let _ = self.thread.join();
    }
}

#[derive(Default)]
pub(super) struct Capture {
    listener: Option<Listener>,
    failed: bool,
}

fn can_dispatch(s: &State) -> bool {
    desktop_active(s)
        && s.file_menu.is_none()
        && s.terminal.menu.is_none()
        && !desktop_customization_ui::menu_open(s)
}

fn enabled(s: &State, command: Command) -> bool {
    match command {
        // Keep Alt+F4 inside the desktop even when a modal or an empty desktop
        // leaves nothing to close. The dispatcher rechecks the active surface.
        Command::CloseWindow => keys(s)
            .chords(Action::CloseWindow)
            .any(|key| key == "Alt+F4"),
        _ if !can_dispatch(s) => false,
        Command::CloseTab => {
            let keys = keys(s);
            let scope = match s.workspace_page {
                WorkspacePage::Files(_) => Scope::Files,
                WorkspacePage::Terminal => Scope::Terminal,
                WorkspacePage::Code => Scope::Code,
                _ => Scope::Desktop,
            };
            keys.action(Scope::Desktop, "Ctrl+F4") == Some(Action::CloseTab)
                && (scope == Scope::Desktop || keys.action(scope, "Ctrl+F4").is_none())
        }
        Command::Binding(action) => {
            let keys = keys(s);
            !keys.overridden(action) && keys.binding(action).is_some() && can_run(s, action)
        }
        Command::ShowDesktop => keys(s)
            .chords(Action::ShowDesktop)
            .any(|key| key == "Meta+D"),
    }
}

impl Capture {
    pub(super) fn poll(&mut self, ui: &Ui, window: &NativeWindow, wake: Option<&Waker>) {
        let hwnd = window.hwnd();
        let active = unsafe { GetForegroundWindow() }.0 as isize == hwnd
            && ui.state.with(|s| {
                server_desktop::visible(s)
                    && s.config.settings.capture_desktop_shortcuts
                    && keys(s).preset == Preset::Windows
            });
        let epoch = ui.state.with(|s| s.epoch);
        if self
            .listener
            .as_ref()
            .is_some_and(|listener| !active || listener.epoch != epoch)
        {
            self.release();
        }
        if !active {
            self.failed = false;
            return;
        }
        let mask = ui.state.with(|s| {
            SHORTCUTS
                .iter()
                .enumerate()
                .fold(0, |mask, (bit, &(_, _, command))| {
                    mask | (u8::from(enabled(s, command)) << bit)
                })
        });
        if self.listener.is_none()
            && !self.failed
            && let Some(wake) = wake
        {
            match Listener::start(hwnd, epoch, mask, wake.clone()) {
                Ok(listener) => self.listener = Some(listener),
                Err(error) => {
                    self.failed = true;
                    ui.log(format!("Windows shortcut capture: {error}"));
                }
            }
        }
        let Some(listener) = &self.listener else {
            return;
        };
        listener.enabled.store(mask, Ordering::Relaxed);
        for command in listener.pending.try_iter() {
            if ui.state.with(|s| can_dispatch(s) && enabled(s, command)) {
                if ui.state.with(|s| {
                    s.workspace_page == WorkspacePage::Settings && s.server_apps.recording_shortcut
                }) {
                    ui.state.revise(|s| {
                        s.server_apps.shortcut_error =
                            "This Windows shortcut is already mapped.".into()
                    });
                } else {
                    match command {
                        Command::ShowDesktop => ui.toggle_show_desktop(),
                        Command::CloseWindow | Command::CloseTab => {
                            ui.close_active_desktop(matches!(command, Command::CloseTab));
                        }
                        Command::Binding(Action::Maximize)
                            if ui
                                .state
                                .with(|s| s.workbench.window(s.workspace_page).maximized) => {}
                        Command::Binding(action) => run(ui, action),
                    }
                }
            }
        }
    }

    fn release(&mut self) {
        if let Some(listener) = self.listener.take() {
            listener.stop();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.release();
    }
}
