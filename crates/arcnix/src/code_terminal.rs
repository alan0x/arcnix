//! Routes workbench terminal instances to the existing SSH PTY worker.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::terminal_worker::{Action as Pty, Event, OUTPUT_WINDOW, SESSION_LIMIT};
use crate::{Action, Ui};

#[derive(Clone, Default)]
pub(super) struct Sessions {
    ids: BTreeMap<u32, u64>,
    pub events: Vec<Value>,
}
impl Sessions {
    pub fn receive(&mut self, id: u64, event: Event) {
        let Some(token) = self
            .ids
            .iter()
            .find_map(|(&token, &worker)| (id == worker).then_some(token))
        else {
            return;
        };
        let payload = match event {
            Event::Started { .. } => json!({"token":token,"ready":true}),
            Event::Output { bytes, .. } => json!({"token":token,"output":STANDARD.encode(bytes)}),
            Event::Exited { code, .. } => {
                self.ids.remove(&token);
                json!({"token":token,"code":code})
            }
            Event::Failed { message, .. } => {
                self.ids.remove(&token);
                json!({"token":token,"error":message})
            }
        };
        self.events.push(payload);
    }
}

#[derive(Deserialize)]
struct Request {
    token: u32,
    #[serde(flatten)]
    action: Command,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
enum Command {
    Open { cols: u32, rows: u32 },
    Input { data: String },
    Resize { cols: u32, rows: u32 },
    Consumed { bytes: usize },
    Close,
}
pub(super) fn request(ui: &Ui, msg: Value) {
    let Ok(Request { token, action }) = serde_json::from_value(msg) else {
        return;
    };
    let action = ui.state.revise(|s| {
        if !s.connected {
            return None;
        }
        let sessions = &mut s.code.terminals;
        if let Command::Open { cols, rows } = action {
            if sessions.ids.contains_key(&token) {
                return None;
            }
            if sessions.ids.len() >= SESSION_LIMIT {
                let message = crate::i18n::translate(
                    "Close a terminal tab before opening another (8 maximum)",
                );
                sessions
                    .events
                    .push(json!({ "token": token, "error": message }));
                return None;
            }
            // Desktop terminal IDs count up from one. Never recycle an ID when
            // a WebView is recreated while its previous PTY is still closing.
            static NEXT: AtomicU64 = AtomicU64::new(1 << 63);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            sessions.ids.insert(token, id);
            return Some(Pty::OpenHere {
                id,
                columns: cols.clamp(2, 500),
                rows: rows.clamp(1, 300),
                directory: s.code.root.clone(),
            });
        }
        let id = *sessions.ids.get(&token)?;
        match action {
            Command::Input { data } if data.len() <= 16384_usize.div_ceil(3) * 4 => {
                Some(Pty::Input {
                    id,
                    bytes: STANDARD.decode(data).ok()?,
                })
            }
            Command::Resize { cols, rows } => Some(Pty::Resize {
                id,
                columns: cols.clamp(2, 500),
                rows: rows.clamp(1, 300),
            }),
            Command::Consumed { bytes } if bytes <= OUTPUT_WINDOW => {
                Some(Pty::Consumed { id, bytes })
            }
            Command::Close => {
                sessions.ids.remove(&token);
                Some(Pty::Close { id })
            }
            _ => None,
        }
    });
    if let Some(action) = action {
        ui.send(Action::Terminal(action));
    }
}
pub(super) fn close_all(ui: &Ui) {
    for id in ui
        .state
        .revise(|s| std::mem::take(&mut s.code.terminals.ids))
        .into_values()
    {
        ui.send(Action::Terminal(Pty::Close { id }));
    }
}
