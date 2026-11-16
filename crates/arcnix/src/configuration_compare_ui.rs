//! Session-only current-file comparisons across independently selected servers.
use arcnix_core::configuration_compare as compare;
use arcnix_core::target_selection::{Filter, Group};

use super::*;
#[derive(Clone, Default)]
struct Credential {
    secret: String,
    sudo: String,
}
#[derive(Clone)]
struct Node {
    profile: ServerProfile,
    fingerprint: String,
    pending: bool,
    stale: bool,
    error: String,
    snapshot: Option<Arc<batch_worker::ComparedConfiguration>>,
}
impl Node {
    fn fresh(&self) -> bool {
        !self.pending
            && !self.stale
            && self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.snapshot.file.is_some())
    }
}
#[derive(Clone)]
pub(super) struct CompareState {
    pub visible: bool,
    serial: u64,
    running: Option<u64>,
    cancel: Option<Arc<AtomicBool>>,
    path: String,
    scope: Option<compare::Plan>,
    scope_admin: bool,
    administrator: bool,
    selected: HashSet<String>,
    filter: Filter,
    credentials: HashMap<String, Credential>,
    nodes: Vec<Node>,
    baseline: String,
    compared: String,
    diff: Option<compare::Comparison>,
    error: String,
}
impl Default for CompareState {
    fn default() -> Self {
        Self {
            visible: false,
            serial: 0,
            running: None,
            cancel: None,
            path: "/etc/nginx/nginx.conf".into(),
            scope: None,
            scope_admin: false,
            administrator: false,
            selected: HashSet::new(),
            filter: Filter::default(),
            credentials: HashMap::new(),
            nodes: vec![],
            baseline: String::new(),
            compared: String::new(),
            diff: None,
            error: String::new(),
        }
    }
}
pub(super) fn pending(s: &State) -> bool {
    s.configuration_compare.running.is_some()
}
pub(super) fn dismiss(s: &mut State) {
    if s.dialog == Some(Dialog::ConfigurationCompare) {
        s.configuration_compare.credentials.clear();
        s.configuration_compare.error.clear();
    }
}
fn stop(s: &mut State) {
    let v = &mut s.configuration_compare;
    if let Some(cancel) = &v.cancel {
        cancel.store(true, Ordering::Release);
    }
    v.serial += 1;
    v.credentials.clear();
    for node in &mut v.nodes {
        if node.pending {
            node.pending = false;
            node.stale = true;
            node.error = "Snapshot reading stopped".into();
        }
    }
    v.baseline.clear();
    v.compared.clear();
    v.diff = None;
}
pub(super) fn forget(s: &mut State) {
    stop(s);
    let v = &mut s.configuration_compare;
    v.nodes.clear();
    v.scope = None;
    v.error.clear();
    if s.dialog == Some(Dialog::ConfigurationCompare) {
        s.dialog = None;
    }
}
pub(super) fn hide(s: &mut State) {
    forget(s);
    s.configuration_compare.visible = false;
}
fn same(s: &State, node: &Node) -> bool {
    s.config
        .servers
        .iter()
        .find(|p| p.id == node.profile.id)
        .is_some_and(|p| {
            p.host == node.profile.host
                && p.port == node.profile.port
                && p.username == node.profile.username
                && s.config.trusted_fingerprint(&p.host, p.port) == Some(node.fingerprint.as_str())
        })
}
fn hidden(s: &State) -> usize {
    let v = &s.configuration_compare;
    v.selected.len()
        - v.filter
            .profiles(&s.config, &v.selected)
            .iter()
            .filter(|p| v.selected.contains(&p.id))
            .count()
}
fn selection(s: &mut State, id: &str) {
    let v = &s.configuration_compare;
    let mut next = v.selected.clone();
    if !next.remove(id) {
        next.insert(id.into());
    }
    match arcnix_core::target_selection::validate_profiles(&s.config, &next) {
        Ok(()) => {
            let v = &mut s.configuration_compare;
            v.selected = next;
            v.credentials.retain(|id, _| v.selected.contains(id));
            v.error.clear();
        }
        Err(e) => s.configuration_compare.error = e.to_string(),
    }
}
fn select_visible(s: &mut State) {
    let v = &s.configuration_compare;
    let mut next = v.selected.clone();
    next.extend(
        v.filter
            .profiles(&s.config, &v.selected)
            .into_iter()
            .filter(|p| s.config.trusted_fingerprint(&p.host, p.port).is_some())
            .map(|p| p.id.clone()),
    );
    match arcnix_core::target_selection::validate_profiles(&s.config, &next) {
        Ok(()) => {
            s.configuration_compare.selected = next;
            s.configuration_compare.error.clear();
        }
        Err(e) => s.configuration_compare.error = e.to_string(),
    }
}
impl Ui {
    fn draft_comparison_baseline(&self) {
        let draft = self.state.with(|s| -> anyhow::Result<_> {
            let v = &s.configuration_compare;
            anyhow::ensure!(
                !pending(s) && s.configurations.content.is_empty(),
                "Review the existing configuration draft before copying a baseline"
            );
            let node = v
                .nodes
                .iter()
                .find(|n| n.profile.id == v.baseline && n.fresh())
                .ok_or_else(|| anyhow::anyhow!("Choose an available baseline snapshot"))?;
            anyhow::ensure!(
                same(s, node),
                "Configuration server identity changed; read snapshots again"
            );
            let file = node
                .snapshot
                .as_ref()
                .unwrap()
                .snapshot
                .file
                .as_ref()
                .unwrap();
            anyhow::ensure!(
                !file.redacted && !file.shortened && !file.text.is_empty(),
                "A hidden, shortened or empty snapshot cannot become a publication draft"
            );
            Ok((v.scope.as_ref().unwrap().path.clone(), file.text.clone()))
        });
        match draft {
            Ok((path, content)) => self.state.revise(|s| {
                hide(s);
                s.configuration_records.visible = false;
                s.configurations.spec = arcnix_core::configurations::Spec {
                    path,
                    kind: arcnix_core::configurations::Kind::Custom,
                    ..Default::default()
                };
                s.configurations.content = content;
                s.configurations.parameterized = false;
                s.configurations.defaults.clear();
                s.configurations.seconds = "30".into();
                s.configurations.error.clear();
                s.configurations.editing = true;
            }),
            Err(e) => self
                .state
                .revise(|s| s.configuration_compare.error = e.to_string()),
        }
    }
    fn open_configuration_comparison(&self) {
        if self.state.with(pending) {
            return;
        }
        self.state.revise(|s| {
            s.configuration_compare.credentials.clear();
            s.configuration_compare.error.clear();
        });
        self.open_dialog(Dialog::ConfigurationCompare);
    }
    fn read_configuration_comparison(&self) {
        let prepared = self.state.with(|s| -> anyhow::Result<_> {
            let v = &s.configuration_compare;
            anyhow::ensure!(!pending(s), "Wait for the previous snapshot read");
            let plan = compare::Plan::new(v.path.clone())?;
            anyhow::ensure!(
                v.selected.len() >= 2,
                "Select at least two servers to compare"
            );
            arcnix_core::target_selection::validate_profiles(&s.config, &v.selected)?;
            anyhow::ensure!(
                hidden(s) == 0,
                "Review hidden selected servers before reading snapshots"
            );
            let mut targets = Vec::new();
            for p in s
                .config
                .servers
                .iter()
                .filter(|p| v.selected.contains(&p.id))
            {
                let credentials = v.credentials.get(&p.id).cloned().unwrap_or_default();
                for value in [&credentials.secret, &credentials.sudo] {
                    anyhow::ensure!(
                        value.len() <= 1024 && !value.contains(['\0', '\r', '\n']),
                        "Invalid credential"
                    );
                }
                let mut profile = p.clone();
                if profile.connection.is_none() {
                    profile.connection = Some(s.config.settings.connection);
                }
                targets.push(batch_worker::Target {
                    profile,
                    fingerprint: s
                        .config
                        .trusted_fingerprint(&p.host, p.port)
                        .unwrap()
                        .into(),
                    secret: zeroize::Zeroizing::new(credentials.secret),
                    sudo: zeroize::Zeroizing::new(credentials.sudo),
                });
            }
            Ok((plan, targets, v.administrator))
        });
        match prepared {
            Err(e) => self
                .state
                .revise(|s| s.configuration_compare.error = e.to_string()),
            Ok((plan, targets, admin)) => {
                let cancel = Arc::new(AtomicBool::new(false));
                let serial = self.state.revise(|s| {
                    let v = &mut s.configuration_compare;
                    v.serial += 1;
                    v.running = Some(v.serial);
                    v.cancel = Some(cancel.clone());
                    let retain = v.scope.as_ref().is_some_and(|p| p.path == plan.path)
                        && v.scope_admin == admin;
                    let mut old = std::mem::take(&mut v.nodes);
                    v.nodes = targets
                        .iter()
                        .map(|t| {
                            let snapshot = if retain {
                                old.iter_mut()
                                    .find(|n| {
                                        n.profile.id == t.profile.id
                                            && n.profile.host == t.profile.host
                                            && n.profile.port == t.profile.port
                                            && n.profile.username == t.profile.username
                                            && n.fingerprint == t.fingerprint
                                    })
                                    .and_then(|n| n.snapshot.take())
                            } else {
                                None
                            };
                            Node {
                                profile: t.profile.clone(),
                                fingerprint: t.fingerprint.clone(),
                                pending: true,
                                stale: snapshot.is_some(),
                                error: String::new(),
                                snapshot,
                            }
                        })
                        .collect();
                    v.scope = Some(plan.clone());
                    v.scope_admin = admin;
                    v.baseline.clear();
                    v.compared.clear();
                    v.diff = None;
                    v.error.clear();
                    v.credentials.clear();
                    s.dialog = None;
                    v.serial
                });
                batch_worker::compare_configurations(
                    serial,
                    targets,
                    plan,
                    admin,
                    cancel,
                    self.batch_events.clone(),
                );
            }
        }
    }
    fn refresh_configuration_diff(&self) {
        let pair = self.state.with(|s| {
            let v = &s.configuration_compare;
            let a = v
                .nodes
                .iter()
                .find(|n| n.profile.id == v.baseline && n.fresh())?
                .snapshot
                .clone()?;
            let b = v
                .nodes
                .iter()
                .find(|n| n.profile.id == v.compared && n.fresh())?
                .snapshot
                .clone()?;
            Some((a, b))
        });
        let diff = pair.map(|(a, b)| {
            compare::compare(
                a.snapshot.file.as_ref().unwrap(),
                b.snapshot.file.as_ref().unwrap(),
            )
        });
        self.state.revise(|s| s.configuration_compare.diff = diff);
    }
}
pub(super) fn event(ui: &Ui, event: batch_worker::Event) {
    match event {
        batch_worker::Event::ConfigurationComparisonComplete(serial) => {
            ui.state.revise(|s| {
                let v = &mut s.configuration_compare;
                if v.running == Some(serial) {
                    v.running = None;
                    v.cancel = None;
                }
                if v.serial == serial {
                    for node in &mut v.nodes {
                        if node.pending {
                            node.pending = false;
                            node.stale = true;
                            node.error = "Configuration snapshot unavailable".into();
                        }
                    }
                }
            });
        }
        batch_worker::Event::ConfigurationCompared { serial, id, result } => {
            ui.state.revise(|s| {
                if s.navigation != navigation::Page::Configurations
                    || !s.configuration_compare.visible
                    || s.configuration_compare.serial != serial
                {
                    return;
                }
                let valid = s
                    .configuration_compare
                    .nodes
                    .iter()
                    .find(|n| n.profile.id == id)
                    .is_some_and(|n| same(s, n));
                if let Some(node) = s
                    .configuration_compare
                    .nodes
                    .iter_mut()
                    .find(|n| n.profile.id == id)
                {
                    node.pending = false;
                    if !valid {
                        node.snapshot = None;
                        node.stale = true;
                        node.error =
                            "Configuration server identity changed; read snapshots again".into();
                    } else {
                        match result {
                            Ok(snapshot) => {
                                node.snapshot = Some(Arc::from(snapshot));
                                node.error.clear();
                                node.stale = false;
                            }
                            Err(error) => {
                                node.error = error;
                                node.stale = true;
                            }
                        }
                    }
                }
                let v = &mut s.configuration_compare;
                if !v.nodes.iter().any(|n| n.pending) && v.baseline.is_empty() {
                    v.baseline = v
                        .nodes
                        .iter()
                        .find(|n| n.fresh())
                        .map(|n| n.profile.id.clone())
                        .unwrap_or_default();
                    v.compared = v
                        .nodes
                        .iter()
                        .find(|n| n.fresh() && n.profile.id != v.baseline)
                        .map(|n| n.profile.id.clone())
                        .unwrap_or_default();
                }
            });
            ui.refresh_configuration_diff();
        }
        _ => {}
    }
}
fn node_status(node: &Node) -> String {
    if node.pending {
        return i18n::translate("Reading snapshot…");
    }
    if !node.error.is_empty() {
        return i18n::translate(if node.error.len() > 160 || node.error.contains('\n') {
            "Snapshot read failed"
        } else {
            &node.error
        });
    }
    if let Some(snapshot) = &node.snapshot {
        if let Some(issue) = snapshot.snapshot.issue {
            return i18n::translate(issue.label());
        }
        return i18n::translate("Snapshot read");
    }
    i18n::translate("No configuration snapshot")
}
fn metadata(node: &Node) -> String {
    node.snapshot
        .as_ref()
        .and_then(|s| s.snapshot.file.as_ref())
        .map(|f| {
            format!(
                "{} B · {}:{} · {:04o} · {}{}{}",
                f.bytes,
                f.uid,
                f.gid,
                f.mode,
                i18n::translate(f.endings),
                if f.bom { " · BOM" } else { "" },
                if f.symlink {
                    format!("\n{}", f.canonical)
                } else {
                    String::new()
                }
            )
        })
        .unwrap_or_default()
}
fn origin(node: &Node) -> String {
    let source = format!(
        "{}@{}:{}\n{}",
        node.profile.username, node.profile.host, node.profile.port, node.fingerprint
    );
    node.snapshot
        .as_ref()
        .and_then(|s| s.snapshot.file.as_ref())
        .map(|f| format!("{source}\n{}\nSHA-256: {}", f.canonical, f.sha256))
        .unwrap_or(source)
}
pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let read = ui.clone();
    let draft = ui.clone();
    col().class("configuration-comparison-page").fill((
        row().class("global-toolbar").fill((btn("Read configuration snapshots…",move||read.open_configuration_comparison()).class("button primary").enabled(state.map(|s|!pending(s))),btn("Use baseline as draft",move||draft.draft_comparison_baseline()).enabled(state.map(|s|!pending(s)&&s.configurations.content.is_empty()&&s.configuration_compare.nodes.iter().find(|n|n.profile.id==s.configuration_compare.baseline&&n.fresh()).and_then(|n|n.snapshot.as_ref()).and_then(|v|v.snapshot.file.as_ref()).is_some_and(|f|!f.redacted&&!f.shortened&&!f.text.is_empty()))),btn("Stop reading",move||state.revise(stop)).enabled(state.map(pending)),btn("Clear snapshots",move||state.revise(forget)))),
        row().class("help-heading").fill((text().class("section-title").content(i18n::bind("Compare current files")),crate::help_ui::group("Compare current files",&["Read one UTF-8 file on each selected server over independent SSH connections. Nothing is published or reloaded. Snapshots are taken separately and do not describe current configuration after later changes.","Configuration snapshots, diffs and read credentials stay in memory. Leaving Configurations clears comparison data. Explicit draft copies follow the editor's normal memory lifecycle. Known authentication values are hidden; other file secrets may remain visible.","Choose a successfully read baseline and a server to compare. Content equality uses full file checksums; ownership, permissions, line endings and BOM remain visible. Failed refreshes retain a marked previous snapshot that cannot be compared.","An unredacted baseline can become a custom publication draft when the current draft is empty. Copying only fills the editor; validation, reload, health checks, targets and server changes still require the normal review.","Reads allow ordinary files up to 128 KiB, including resolved symbolic paths. Missing files, permissions, changed files and unsupported content remain separate results. Queries are finite and never refresh automatically."]))),
        text().class("subtle configuration-comparison-path").content(state.map(|s|s.configuration_compare.scope.as_ref().map(|p|p.path.clone()).unwrap_or_default())),
        text().class(state.map(|s|if s.configuration_compare.error.is_empty()||s.dialog.is_some(){"hidden"}else{"form-error"}.to_owned())).content(i18n::bind(state.map(|s|s.configuration_compare.error.clone()))),
        text().class("subtle").content(state.map(|s|{let v=&s.configuration_compare;if v.nodes.is_empty(){i18n::translate(if pending(s){"Finishing previous snapshot read…"}else{"No configuration snapshots"})}else{i18n::translate(&format!("Available snapshots: {}/{} · Reading: {}",v.nodes.iter().filter(|n|n.fresh()).count(),v.nodes.len(),v.nodes.iter().filter(|n|n.pending).count()))}})),
        row().class("configuration-comparison-layout").fill((
        col().class("configuration-comparison-servers").fill(each(state.map(|s|s.configuration_compare.nodes.clone())).key(|n|n.profile.id.clone()).children({let ui=ui.clone();move|item|comparison_node(ui.clone(),item.value)})),
        col().class("configuration-comparison-result").fill((
            text().class("section-title").content(state.map(|s|{let v=&s.configuration_compare;let name=|id:&str|v.nodes.iter().find(|n|n.profile.id==id).map(|n|n.profile.name.clone()).unwrap_or_default();if v.diff.is_some(){format!("{} → {}",name(&v.baseline),name(&v.compared))}else if v.nodes.is_empty(){String::new()}else{i18n::translate("No comparison yet")}})),
            text().class("configuration-comparison-verdict").content(i18n::bind(state.map(|s|s.configuration_compare.diff.as_ref().map(|d|if d.identical{"File contents match"}else{"File contents differ"}.to_owned()).unwrap_or_default()))),
            text().class("observability-warning").content(i18n::bind(state.map(|s|s.configuration_compare.diff.as_ref().filter(|d|d.metadata_different).map(|_|"Ownership or permissions differ".to_owned()).unwrap_or_default()))),
            text().class("observability-warning").content(i18n::bind(state.map(|s|s.configuration_compare.diff.as_ref().filter(|d|d.redacted).map(|_|"Known credentials hidden; equality uses full checksums".to_owned()).unwrap_or_default()))),
            text().class("observability-warning").content(i18n::bind(state.map(|s|s.configuration_compare.diff.as_ref().filter(|d|d.shortened).map(|_|"Diff preview shortened".to_owned()).unwrap_or_default()))),
            text().class("script-code configuration-comparison-diff").selectable(true).content(state.map(|s|s.configuration_compare.diff.as_ref().map(|d|d.text.clone()).unwrap_or_default())),
        )),
        )),
    ))
}
fn comparison_node(ui: Ui, node: Cage<Node>) -> impl Widget {
    let state = ui.state;
    let id = node.get().profile.id;
    let b = id.clone();
    let c = id.clone();
    let active = id.clone();
    let enabled = id.clone();
    let base_ui = ui.clone();
    let compared_ui = ui.clone();
    col().class("configuration-comparison-server").fill((
        row().class("help-heading").fill((
            text()
                .class("section-title")
                .content(node.map(|n| n.profile.name.clone())),
            crate::help_ui::tip("Snapshot origin", node.map(origin)),
        )),
        text()
            .class("subtle configuration-comparison-endpoint")
            .content(node.map(|n| {
                format!(
                    "{}@{}:{}",
                    n.profile.username, n.profile.host, n.profile.port
                )
            })),
        row().class("help-heading").fill((
            text()
                .class("configuration-comparison-status")
                .content(node.map(node_status)),
            when(
                node.map(|n| n.error.len() > 160 || n.error.contains('\n')),
                move || crate::help_ui::tip("Snapshot read failure", node.map(|n| n.error.clone())),
            ),
        )),
        text().class("subtle").content(node.map(|n| {
            n.snapshot
                .as_ref()
                .map(|s| {
                    format!(
                        "{} {} · {} {}{}",
                        i18n::translate("Sampled"),
                        formatting::timestamp(Some(s.snapshot.sampled)),
                        i18n::translate("Received"),
                        formatting::timestamp(Some(s.received)),
                        if n.stale {
                            format!(
                                " · {}",
                                i18n::translate("Previous snapshot; excluded from comparison")
                            )
                        } else {
                            String::new()
                        }
                    )
                })
                .unwrap_or_default()
        })),
        text()
            .class("subtle configuration-comparison-metadata")
            .content(node.map(metadata)),
        row().class("script-actions").fill((
            btn("Use as baseline", move || {
                base_ui.state.revise(|s| {
                    let v = &mut s.configuration_compare;
                    v.baseline = b.clone();
                    if v.compared == b {
                        v.compared = v
                            .nodes
                            .iter()
                            .find(|n| n.fresh() && n.profile.id != b)
                            .map(|n| n.profile.id.clone())
                            .unwrap_or_default();
                    }
                });
                base_ui.refresh_configuration_diff();
            })
            .enabled(node.map(|n| n.fresh()))
            .class(state.map(move |s| {
                if s.configuration_compare.baseline == active {
                    "choice active comparison-baseline"
                } else {
                    "choice comparison-baseline"
                }
                .to_owned()
            })),
            btn("Compare with baseline", move || {
                compared_ui
                    .state
                    .revise(|s| s.configuration_compare.compared = c.clone());
                compared_ui.refresh_configuration_diff();
            })
            .enabled(state.map(move |s| {
                s.configuration_compare.baseline != enabled
                    && s.configuration_compare
                        .nodes
                        .iter()
                        .any(|n| n.profile.id == enabled && n.fresh())
                    && s.configuration_compare
                        .nodes
                        .iter()
                        .any(|n| n.profile.id == s.configuration_compare.baseline && n.fresh())
            }))
            .class("button comparison-target"),
        )),
        text()
            .class(node.map(|n| {
                if n.snapshot.as_ref().is_some_and(|s| !s.warning.is_empty()) {
                    "observability-warning"
                } else {
                    "hidden"
                }
                .to_owned()
            }))
            .content(node.map(|n| {
                n.snapshot
                    .as_ref()
                    .map(|s| s.warning.clone())
                    .unwrap_or_default()
            })),
    ))
}
fn credential_field(state: Cage<State>, id: String, administrator: bool) -> impl Widget {
    let label = if administrator {
        "Administrator password (optional)"
    } else {
        "SSH password or key passphrase"
    };
    let read = id.clone();
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .purpose(Purpose::Password)
            .name(i18n::bind(label))
            .value(state.map(move |s| {
                s.configuration_compare
                    .credentials
                    .get(&read)
                    .map(|c| {
                        if administrator {
                            c.sudo.clone()
                        } else {
                            c.secret.clone()
                        }
                    })
                    .unwrap_or_default()
            }))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    state.revise(|s| {
                        if s.dialog == Some(Dialog::ConfigurationCompare)
                            && s.configuration_compare.selected.contains(&id)
                        {
                            let entry = s
                                .configuration_compare
                                .credentials
                                .entry(id.clone())
                                .or_default();
                            if administrator {
                                entry.sudo = value;
                            } else {
                                entry.secret = value;
                            }
                        }
                    });
                }
            }),
    ))
}
fn comparison_target(state: Cage<State>, profile: ServerProfile) -> impl Widget {
    let id = profile.id.clone();
    let selected = id.clone();
    let toggle = id.clone();
    let visible = id.clone();
    let credentials = id.clone();
    let admin = id.clone();
    let host = profile.host.clone();
    let port = profile.port;
    let label = format!(
        "{} · {}@{}:{}",
        profile.name, profile.username, profile.host, profile.port
    );
    col().class("configuration-comparison-target").fill((
        btn(label, move || state.revise(|s| selection(s, &toggle)))
            .enabled(state.map(move |s| s.config.trusted_fingerprint(&host, port).is_some()))
            .class(state.map(move |s| {
                if s.configuration_compare.selected.contains(&selected) {
                    "target-choice active comparison-server-choice"
                } else {
                    "target-choice comparison-server-choice"
                }
                .to_owned()
            })),
        when(
            state.map(move |s| s.configuration_compare.selected.contains(&visible)),
            move || {
                let credentials = credentials.clone();
                let admin = admin.clone();
                col().class("form-stack").fill((
                    credential_field(state, credentials, false),
                    when(
                        state.map(|s| s.configuration_compare.administrator),
                        move || credential_field(state, admin.clone(), true),
                    ),
                ))
            },
        ),
    ))
}
pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::ConfigurationCompare)),
        move || {
            let submit = ui.clone();
            let close = ui.clone();
            modal(ui.clone(),"Read configuration snapshots",col().class("form-stack configuration-comparison-dialog").fill((
            col().class("field").fill((row().class("help-heading").fill((text().class("field-label").content(i18n::bind("Configuration file path")),crate::help_ui::tip("Read configuration snapshots","Read the same literal path on selected Linux servers with Python 3.8+. Administrator access is optional. Stop reading prevents queued reads and discards late results; an in-flight read may finish before disconnecting."))),input().name(i18n::bind("Configuration file path")).value(state.map(|s|s.configuration_compare.path.clone())).on(value_changed,move|e|if let Event::ValueChanged{value,..}=e{state.revise(|s|s.configuration_compare.path=value);}))),
            input().name(i18n::bind("Search servers")).value(state.map(|s|s.configuration_compare.filter.search.clone())).on(value_changed,move|e|if let Event::ValueChanged{value,..}=e{state.revise(|s|s.configuration_compare.filter.search=value);}),
            row().class("script-actions").fill(each(state.map(|s|{let mut groups=vec![(Group::All,i18n::translate("All groups")),(Group::Ungrouped,i18n::translate("Ungrouped"))];groups.extend(s.config.groups.iter().map(|g|(Group::Named(g.id.clone()),g.name.clone())));groups})).key(|(g,_)|format!("{g:?}")).children(move|item|{let (group,label)=item.value.get();let active=group.clone();btn(label,move||state.revise(|s|s.configuration_compare.filter.group=group.clone())).class(state.map(move|s|if s.configuration_compare.filter.group==active{"choice active"}else{"choice"}.to_owned()))})),
            server_labels_ui::filters(ui.clone(),|s|&s.configuration_compare.filter,|s|&mut s.configuration_compare.filter),
            row().class("script-actions").fill((btn("Select visible trusted servers",move||state.revise(select_visible)),btn("View selected servers",move||state.revise(|s|{s.configuration_compare.filter=Filter{selected_only:true,..Default::default()};})),btn("Show all servers",move||state.revise(|s|s.configuration_compare.filter=Filter::default())),btn("Clear selection",move||state.revise(|s|{s.configuration_compare.selected.clear();s.configuration_compare.credentials.clear();})))),
            text().class("subtle").content(state.map(|s|format!("{} · {}: {}",s.configuration_compare.selected.len(),i18n::translate("Hidden selected servers"),hidden(s)))),
            col().class("configuration-comparison-targets").fill(each(state.map(|s|s.configuration_compare.filter.profiles(&s.config,&s.configuration_compare.selected).into_iter().cloned().collect::<Vec<_>>())).key(|p|p.id.clone()).children(move|item|comparison_target(state,item.value.get()))),
            btn("Read as administrator",move||state.revise(|s|{s.configuration_compare.administrator= !s.configuration_compare.administrator;if !s.configuration_compare.administrator{for c in s.configuration_compare.credentials.values_mut(){c.sudo.clear();}}})).class(state.map(|s|if s.configuration_compare.administrator{"choice active"}else{"choice"}.to_owned())),
            text().class(state.map(|s|if s.configuration_compare.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(i18n::bind(state.map(|s|s.configuration_compare.error.clone()))),
        )),(btn("Cancel",move||close.close_dialog()),btn("Read snapshots",move||submit.read_configuration_comparison()).class("button primary")))
        },
    )
}
