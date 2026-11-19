//! Drives the real desktop UI in Glory's in-memory renderer.
//!
//! The harness mounts the same `app(ui)` as `main`, but nothing reaches the
//! network or a window. Requests meant for the SSH worker are captured, so a
//! test can check what an interaction asked the worker to do. Nodes are found
//! by their English label or CSS class, the way the WebView2 smoke tests find
//! them, and only nodes that are currently placed count as visible.
use std::cell::Cell;
use std::sync::mpsc::{self, Receiver};

use glory::App;
use glory::render::memory::Memory;
use glory::render::{
    Action as HostAction, Capabilities, Error as RenderError, Event, EventKind, Input, Mutation,
    NodeId, Props, Query, QueryToken, Receipt, Render, Role,
};

use super::*;

/// Glory's in-memory renderer, plus the focus a desktop host provides.
///
/// Dialogs move focus when they open and close. `Memory` rejects every
/// physical action, so this accepts focus requests on a valid target, records
/// the target and delegates everything else.
struct Headless {
    memory: Memory,
    focused: Rc<Cell<Option<NodeId>>>,
}

impl Render for Headless {
    fn listen(
        &mut self,
        node: NodeId,
        kind: EventKind,
        enabled: bool,
    ) -> std::result::Result<(), RenderError> {
        self.memory.listen(node, kind, enabled)
    }
    fn action(&mut self, action: HostAction) -> std::result::Result<(), RenderError> {
        match action {
            HostAction::Focus(node) => {
                action.validate_target(&self.memory.node(node)?.descriptor)?;
                self.focused.set(Some(node));
                Ok(())
            }
            other => self.memory.action(other),
        }
    }
    fn needs_observe(&self) -> bool {
        self.memory.needs_observe()
    }
    fn style_profile(&self) -> glory::render::style::Profile {
        self.memory.style_profile()
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            focus: true,
            ..self.memory.capabilities()
        }
    }
    fn apply(&mut self, mutations: &[Mutation]) -> std::result::Result<Receipt, RenderError> {
        self.memory.apply(mutations)
    }
    fn request(&mut self, query: Query, token: QueryToken) -> std::result::Result<(), RenderError> {
        self.memory.request(query, token)
    }
    fn drain(&mut self, inputs: &mut Vec<Input>) {
        self.memory.drain(inputs)
    }
}

struct Harness {
    window: App,
    ui: Ui,
    requests: Receiver<Request>,
    focused: Rc<Cell<Option<NodeId>>>,
    root: PathBuf,
}

impl Harness {
    fn new(servers: Vec<ServerProfile>) -> Self {
        let root = std::env::temp_dir().join(format!("arcnix-headless-{}", new_id()));
        std::fs::create_dir_all(&root).unwrap();
        let store = Rc::new(ConfigStore::at(root.join("config.json")));
        let config = AppConfig {
            servers,
            ..Default::default()
        };
        let focused = Rc::new(Cell::new(None));
        let host_focus = focused.clone();
        let window = App::new(move |runtime| Headless {
            memory: Memory::new(runtime),
            focused: host_focus,
        });
        let scope = window.scope().clone();
        i18n::init(&scope, config.settings.language);
        let state = scope.cage(initial_state(config, String::new(), &root));
        let (tx, requests) = mpsc::channel();
        let (update_controller, _) = updates_ui::start().unwrap();
        let (batch_events, _) = async_channel::bounded(1);
        let ui = Ui {
            wallpapers: Arc::default(),
            update_controller,
            batch_controller: Rc::default(),
            batch_events,
            logger: Rc::new(client_logs::Logger::new(store.path())),
            state,
            tx,
            store,
            writable: scope.cage(true),
            scope,
            shell_focus: window.scope().cage(None),
        };
        window.mount(app(ui.clone())).unwrap();
        Self {
            window,
            ui,
            requests,
            focused,
            root,
        }
    }

    /// Placed nodes in document order, starting under `from` (or the roots).
    fn visible(&self, from: Option<NodeId>) -> Vec<NodeId> {
        let tree = self.window.tree();
        let mut pending: Vec<NodeId> = match from {
            Some(node) => tree
                .node(node)
                .unwrap()
                .children
                .iter()
                .rev()
                .copied()
                .collect(),
            None => tree.roots().iter().rev().copied().collect(),
        };
        let mut found = Vec::new();
        while let Some(id) = pending.pop() {
            let node = tree.node(id).unwrap();
            if !node.placed || node.suspended || node.descriptor.view().hidden() {
                continue;
            }
            found.push(id);
            pending.extend(node.children.iter().rev().copied());
        }
        found
    }

    fn find(
        &self,
        within: Option<NodeId>,
        matches: impl Fn(glory::render::DescriptorView<'_>) -> bool,
    ) -> Vec<NodeId> {
        let tree = self.window.tree();
        self.visible(within)
            .into_iter()
            .filter(|id| matches(tree.node(*id).unwrap().descriptor.view()))
            .collect()
    }

    fn one(&self, what: &str, nodes: Vec<NodeId>) -> NodeId {
        assert_eq!(
            nodes.len(),
            1,
            "expected exactly one {what}, found {}",
            nodes.len()
        );
        nodes[0]
    }

    fn by_class(&self, within: Option<NodeId>, class: &str) -> NodeId {
        let nodes = self.find(within, |d| d.classes.iter().any(|c| c == class));
        self.one(&format!("node with class {class:?}"), nodes)
    }

    /// A button by its accessible name: an explicit name, else its label.
    fn button(&self, within: Option<NodeId>, name: &str) -> NodeId {
        let nodes = self.find(within, |d| match d.props {
            Props::Button(button) => d.name.as_deref().unwrap_or(&button.label) == name,
            _ => false,
        });
        self.one(&format!("button {name:?}"), nodes)
    }

    fn input(&self, within: Option<NodeId>, name: &str) -> NodeId {
        let nodes = self.find(within, |d| {
            matches!(d.props, Props::Input(_)) && d.name.as_deref() == Some(name)
        });
        self.one(&format!("input {name:?}"), nodes)
    }

    fn dialogs(&self, title: &str) -> Vec<NodeId> {
        self.find(None, |d| {
            d.role == Some(Role::Dialog) && d.name.as_deref() == Some(title)
        })
    }

    fn dialog(&self, title: &str) -> NodeId {
        let nodes = self.dialogs(title);
        self.one(&format!("dialog {title:?}"), nodes)
    }

    /// All visible text under `node`, one line per text node.
    fn text(&self, node: NodeId) -> String {
        let tree = self.window.tree();
        self.visible(Some(node))
            .into_iter()
            .filter_map(|id| match tree.node(id).unwrap().descriptor.view().props {
                Props::Text(text) => Some(text.content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn click(&self, node: NodeId) {
        assert!(
            self.window.dispatch(node, Event::Activate).unwrap(),
            "the click was not handled"
        );
    }

    fn type_into(&self, node: NodeId, value: &str) {
        let revision = match self
            .window
            .tree()
            .node(node)
            .unwrap()
            .descriptor
            .view()
            .props
        {
            Props::Input(input) => input.revision,
            other => panic!("not an input: {other:?}"),
        };
        let event = Event::ValueChanged {
            value: value.into(),
            composing: false,
            revision,
        };
        assert!(
            self.window.dispatch(node, event).unwrap(),
            "the input was not handled"
        );
    }

    /// Requests sent to the SSH worker since the last call.
    fn sent(&self) -> Vec<Action> {
        self.requests
            .try_iter()
            .map(|request| request.action)
            .collect()
    }

    /// Opens the server's workspace from the server list, then asks for the
    /// file-transfer workspace, which needs a connection.
    fn open_transfer_files(&self, server: &str) {
        self.click(self.by_class(None, "rail-servers"));
        let row = self.one(
            &format!("server row {server:?}"),
            self.find(None, |d| d.classes.iter().any(|c| c == "host-row"))
                .into_iter()
                .filter(|row| self.text(*row).lines().any(|line| line == server))
                .collect(),
        );
        self.click(self.by_class(Some(row), "host-select"));
        self.click(self.button(Some(self.by_class(None, "workbench-tabs")), "Home"));
        self.click(self.by_class(None, "overview-transfer"));
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn password_server(name: &str) -> ServerProfile {
    ServerProfile {
        name: name.into(),
        host: "192.0.2.10".into(),
        username: "deploy".into(),
        auth: AuthMethod::Password,
        ..Default::default()
    }
}

#[test]
fn a_password_login_asks_for_the_password_before_contacting_the_server() {
    let server = password_server("Staging");
    let id = server.id.clone();
    let app = Harness::new(vec![server]);

    app.open_transfer_files("Staging");
    let dialog = app.dialog("Connect to server");
    assert_eq!(app.focused.get(), Some(dialog), "the dialog takes focus");
    let text = app.text(dialog);
    assert!(text.contains("deploy@192.0.2.10:22"), "{text}");
    assert!(text.lines().any(|line| line == "Password"), "{text}");
    assert!(
        !app.sent()
            .iter()
            .any(|action| matches!(action, Action::Probe(..))),
        "nothing may reach the server before the password is entered"
    );

    app.type_into(
        app.input(Some(dialog), "Password or key passphrase"),
        "hunter2",
    );
    app.click(app.button(Some(dialog), "Connect"));

    assert!(app.dialogs("Connect to server").is_empty());
    let probes: Vec<_> = app
        .sent()
        .into_iter()
        .filter_map(|action| match action {
            Action::Probe(profile, _) => Some(profile),
            _ => None,
        })
        .collect();
    assert_eq!(probes.len(), 1);
    assert_eq!(probes[0].id, id);
    assert!(app.ui.state.with(|s| s.connecting && s.secret == "hunter2"));
}

#[test]
fn cancelling_the_connect_dialog_sends_nothing_to_the_server() {
    let app = Harness::new(vec![password_server("Staging")]);

    app.open_transfer_files("Staging");
    let dialog = app.dialog("Connect to server");
    app.sent();
    app.click(app.button(Some(dialog), "Cancel"));

    assert!(app.dialogs("Connect to server").is_empty());
    assert!(
        app.sent()
            .iter()
            .all(|action| !matches!(action, Action::Probe(..)))
    );
    assert!(app.ui.state.with(|s| !s.connecting && !s.connected));
}
