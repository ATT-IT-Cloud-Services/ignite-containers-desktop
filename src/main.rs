mod api;
mod config;

use api::{
    merge_console_lines, tail_overlap, ApiRequest, ApiResponse, ApiWorker, Connection,
    ConsoleEvent, ConsoleMode, ConsoleWorker, Tag,
};
use config::SavedConfig;
use eframe::egui::{
    self, Align, Color32, FontId, Layout, RichText, ScrollArea, TextEdit, TextStyle, Vec2,
};
use ignite_containers_core::models::{
    Allocation, AuditEvent, Backup, BackupState, DatabaseHost, Node, NodeStatus, Server,
    ServerDatabase, ServerDetail, ServerState,
};
use ignite_containers_core::template::{Template, TemplateRecord};
use ignite_containers_proto::{FileContent, FileEntry, ServerStats};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const APP_TITLE: &str = "Ignite Containers Desktop";
const CONSOLE_LIMIT: usize = 5_000;
const DEFAULT_URL: &str = "http://127.0.0.1:8080";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Tab {
    Overview,
    Servers,
    Console,
    Files,
    Config,
    Schedules,
    Sftp,
    Environment,
    Jobs,
    Backups,
    Nodes,
    Templates,
    Databases,
    Notifications,
    ApiKeys,
    Audit,
    Proxy,
    Metrics,
    Raw,
}

impl Tab {
    const ALL: [Tab; 19] = [
        Tab::Overview,
        Tab::Servers,
        Tab::Console,
        Tab::Files,
        Tab::Config,
        Tab::Schedules,
        Tab::Sftp,
        Tab::Environment,
        Tab::Jobs,
        Tab::Backups,
        Tab::Nodes,
        Tab::Templates,
        Tab::Databases,
        Tab::Notifications,
        Tab::ApiKeys,
        Tab::Audit,
        Tab::Proxy,
        Tab::Metrics,
        Tab::Raw,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Servers => "Servers",
            Tab::Console => "Console",
            Tab::Files => "Files",
            Tab::Config => "Config",
            Tab::Schedules => "Schedules",
            Tab::Sftp => "SFTP",
            Tab::Environment => "Environment",
            Tab::Jobs => "Jobs",
            Tab::Backups => "Backups",
            Tab::Nodes => "Nodes",
            Tab::Templates => "Templates",
            Tab::Databases => "Databases",
            Tab::Notifications => "Notifications",
            Tab::ApiKeys => "API keys",
            Tab::Audit => "Audit",
            Tab::Proxy => "Proxy",
            Tab::Metrics => "Metrics",
            Tab::Raw => "Raw API",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Tab::Overview => "overview",
            Tab::Servers => "servers",
            Tab::Console => "console",
            Tab::Files => "files",
            Tab::Config => "config",
            Tab::Schedules => "schedules",
            Tab::Sftp => "sftp",
            Tab::Environment => "environment",
            Tab::Jobs => "jobs",
            Tab::Backups => "backups",
            Tab::Nodes => "nodes",
            Tab::Templates => "templates",
            Tab::Databases => "databases",
            Tab::Notifications => "notifications",
            Tab::ApiKeys => "api-keys",
            Tab::Audit => "audit",
            Tab::Proxy => "proxy",
            Tab::Metrics => "metrics",
            Tab::Raw => "raw",
        }
    }

    fn group(self) -> &'static str {
        match self {
            Tab::Overview
            | Tab::Servers
            | Tab::Console
            | Tab::Files
            | Tab::Config
            | Tab::Schedules
            | Tab::Sftp
            | Tab::Environment
            | Tab::Jobs
            | Tab::Backups => "Operate",
            Tab::Nodes | Tab::Templates | Tab::Databases | Tab::Proxy => "Platform",
            Tab::Notifications | Tab::ApiKeys | Tab::Audit | Tab::Metrics | Tab::Raw => {
                "Administration"
            }
        }
    }

    fn from_saved(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tab| tab.as_str() == value)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusKind {
    Info,
    Success,
    Error,
}

struct Toast {
    text: String,
    kind: StatusKind,
    at: Instant,
}

#[derive(Default)]
struct ConsoleLog {
    lines: Vec<String>,
    cursor: String,
    truncated: bool,
    last_error: Option<String>,
    last_update: Option<Instant>,
    /// Lines visible at the moment the user pressed Clear. The next tail chunk
    /// is deduplicated against this hint so old output does not immediately
    /// reappear.
    clear_hint: Option<Vec<String>>,
}

#[derive(Clone)]
struct FileEditor {
    path: String,
    content: String,
    encoding: String,
    size: u64,
    truncated: bool,
    dirty: bool,
}

#[derive(Default)]
struct FilesState {
    path: String,
    entries: Vec<FileEntry>,
    selected: Option<String>,
    editor: Option<FileEditor>,
    list_error: Option<String>,
    loading: bool,
    new_dir: String,
    rename_to: String,
    chmod_mode: String,
}

impl FilesState {
    fn fresh() -> Self {
        Self {
            path: "/".to_string(),
            chmod_mode: "0644".to_string(),
            ..Self::default()
        }
    }
}

#[derive(Default)]
struct CreateServerForm {
    show: bool,
    node_id: Option<String>,
    template_id: Option<String>,
    name: String,
    description: String,
    memory: String,
    disk: String,
    cpu: String,
    auto_allocations: String,
    variables: String,
    image: String,
    external_ref: String,
    idempotency_key: String,
    template_version: String,
    labels: String,
    protocol: String,
    error: Option<String>,
}

impl CreateServerForm {
    fn new() -> Self {
        Self {
            memory: "1024".to_string(),
            disk: "10240".to_string(),
            cpu: "1000".to_string(),
            auto_allocations: "1".to_string(),
            labels: "{}".to_string(),
            ..Self::default()
        }
    }
}

#[derive(Default)]
struct EditLimitsForm {
    server: Option<String>,
    memory: String,
    disk: String,
    cpu: String,
    swap: String,
    io_weight: String,
    pids_limit: String,
    cpuset: String,
    error: Option<String>,
}

impl EditLimitsForm {
    fn load(&mut self, server: &Server) {
        let id = server.id.to_string();
        self.server = Some(id);
        self.memory = server.limits.memory_mb.to_string();
        self.disk = server.limits.disk_mb.to_string();
        self.cpu = server.limits.cpu_millis.to_string();
        self.swap = server.limits.swap_mb.to_string();
        self.io_weight = server.limits.io_weight.to_string();
        self.pids_limit = server.limits.pids_limit.to_string();
        self.cpuset = server.limits.cpuset.clone().unwrap_or_default();
        self.error = None;
    }
}

#[derive(Default)]
struct ServerConfigForm {
    server: Option<String>,
    name: String,
    description: String,
    external_ref: String,
    image: String,
    template_version: String,
    labels: String,
    variables: String,
    error: Option<String>,
}

impl ServerConfigForm {
    fn load(&mut self, server: &Server) {
        self.server = Some(server.id.to_string());
        self.name = server.name.clone();
        self.description = server.description.clone();
        self.external_ref = server.external_ref.clone().unwrap_or_default();
        self.image = server.image.clone();
        self.template_version = server.template_version.clone();
        self.labels = serde_json::to_string_pretty(&server.labels).unwrap_or_else(|_| "{}".into());
        self.variables.clear();
        self.error = None;
    }
}

#[derive(Default)]
struct ScheduleForm {
    server: Option<String>,
    id: Option<String>,
    name: String,
    cron: String,
    actions: String,
    enabled: bool,
    error: Option<String>,
}

impl ScheduleForm {
    fn new() -> Self {
        Self {
            cron: "0 4 * * *".to_string(),
            actions: "[{\"type\":\"power\",\"signal\":\"restart\"}]".to_string(),
            enabled: true,
            ..Self::default()
        }
    }
}

#[derive(Default)]
struct ApiKeyForm {
    name: String,
    scopes: String,
    server_ids: String,
    days: String,
    error: Option<String>,
}

impl ApiKeyForm {
    fn new() -> Self {
        Self {
            scopes: "server:read".to_string(),
            ..Self::default()
        }
    }
}

#[derive(Default)]
struct NotificationForm {
    to: String,
    subject: String,
    body: String,
    error: Option<String>,
}

#[derive(Default)]
struct NodeForm {
    id: Option<String>,
    name: String,
    fqdn: String,
    endpoint: String,
    driver: String,
    maintenance: bool,
    labels: String,
    error: Option<String>,
}

impl NodeForm {
    fn load(&mut self, node: &Node) {
        self.id = Some(node.id.to_string());
        self.name = node.name.clone();
        self.fqdn = node.fqdn.clone();
        self.endpoint = node.endpoint.clone();
        self.driver = format!("{:?}", node.driver).to_lowercase();
        self.maintenance = node.maintenance;
        self.labels = serde_json::to_string_pretty(&node.labels).unwrap_or_else(|_| "{}".into());
        self.error = None;
    }
}

#[derive(Default)]
struct TemplatePublishForm {
    template_id: Option<String>,
    template: String,
    channel: String,
    notes: String,
    signature: String,
    public_key: String,
    error: Option<String>,
}

impl TemplatePublishForm {
    fn load(&mut self, record: &TemplateRecord) {
        self.template_id = Some(record.id.to_string());
        self.template = record
            .latest_version
            .as_ref()
            .and_then(|version| serde_json::to_string_pretty(&version.template).ok())
            .unwrap_or_else(|| "{}".to_string());
        self.channel = "stable".to_string();
        self.notes.clear();
        self.signature.clear();
        self.public_key.clear();
        self.error = None;
    }
}

#[derive(Default)]
struct TemplateFeedForm {
    feed: String,
    signature: String,
    public_key: String,
    create_missing: bool,
    error: Option<String>,
}

#[derive(Default)]
struct ProxyEditForm {
    id: Option<String>,
    domain: String,
    upstream: String,
    tls: String,
    enabled: bool,
    error: Option<String>,
}

impl ProxyEditForm {
    fn new() -> Self {
        Self {
            upstream: "http://127.0.0.1:8080".to_string(),
            tls: "auto".to_string(),
            enabled: true,
            ..Self::default()
        }
    }
}

#[derive(Default)]
struct CreateNodeForm {
    show: bool,
    name: String,
    fqdn: String,
    endpoint: String,
    driver: String,
    maintenance: bool,
    memory: String,
    disk: String,
    cpu: String,
    labels: String,
    agent_token: String,
    error: Option<String>,
}

impl CreateNodeForm {
    fn new() -> Self {
        Self {
            driver: "docker".to_string(),
            memory: "0".to_string(),
            disk: "0".to_string(),
            cpu: "0".to_string(),
            labels: "{}".to_string(),
            ..Self::default()
        }
    }
}

#[derive(Default)]
struct CreateAllocForm {
    ip: String,
    ports: String,
    protocol: String,
    label: String,
    error: Option<String>,
}

impl CreateAllocForm {
    fn new() -> Self {
        Self {
            ip: "0.0.0.0".to_string(),
            protocol: "tcp".to_string(),
            ..Self::default()
        }
    }
}

#[derive(Default)]
struct CreateDbHostForm {
    show: bool,
    name: String,
    host: String,
    port: String,
    admin_user: String,
    admin_password: String,
    max_databases: String,
    error: Option<String>,
}

impl CreateDbHostForm {
    fn new() -> Self {
        Self {
            port: "3306".to_string(),
            max_databases: "0".to_string(),
            ..Self::default()
        }
    }
}

#[derive(Default)]
struct CreateServerDbForm {
    host_id: Option<String>,
    remote: String,
    error: Option<String>,
}

struct CreatedDbInfo {
    database: String,
    username: String,
    password: String,
    host: String,
    port: u16,
    remote: String,
}

struct NodeSecret {
    node: String,
    token: String,
}

struct RawView {
    status: u16,
    content_type: Option<String>,
    body: String,
}

#[derive(Clone)]
enum ConfirmAction {
    DeleteServer {
        id: String,
        name: String,
    },
    DeleteFile {
        server: String,
        path: String,
    },
    DeleteAllocation {
        node: String,
        id: String,
        port: u16,
    },
    DeleteTemplate {
        id: String,
        name: String,
    },
    DeleteDbHost {
        id: String,
        name: String,
    },
    DeleteDb {
        server: String,
        id: String,
        name: String,
    },
    DeleteBackup {
        server: String,
        id: String,
        name: String,
    },
    RestoreBackup {
        server: String,
        id: String,
        name: String,
    },
    DeleteProxy {
        id: String,
        domain: String,
    },
    DeleteNode {
        id: String,
        name: String,
    },
    RevokeApiKey {
        id: String,
        name: String,
    },
    RevokeSftp {
        server: String,
        id: String,
        username: String,
    },
    DeleteSchedule {
        server: String,
        id: String,
        name: String,
    },
}

impl ConfirmAction {
    fn title(&self) -> &'static str {
        match self {
            ConfirmAction::DeleteServer { .. } => "Delete server?",
            ConfirmAction::DeleteFile { .. } => "Delete file?",
            ConfirmAction::DeleteAllocation { .. } => "Delete allocation?",
            ConfirmAction::DeleteTemplate { .. } => "Delete template?",
            ConfirmAction::DeleteDbHost { .. } => "Delete database host?",
            ConfirmAction::DeleteDb { .. } => "Delete server database?",
            ConfirmAction::DeleteBackup { .. } => "Delete backup?",
            ConfirmAction::RestoreBackup { .. } => "Restore backup?",
            ConfirmAction::DeleteProxy { .. } => "Delete proxy route?",
            ConfirmAction::DeleteNode { .. } => "Delete node?",
            ConfirmAction::RevokeApiKey { .. } => "Revoke API key?",
            ConfirmAction::RevokeSftp { .. } => "Revoke SFTP credential?",
            ConfirmAction::DeleteSchedule { .. } => "Delete schedule?",
        }
    }

    fn detail(&self) -> String {
        match self {
            ConfirmAction::DeleteServer { id, name } => {
                format!(
                    "Delete server '{name}' ({})? This cannot be undone.",
                    short_id(id)
                )
            }
            ConfirmAction::DeleteFile { path, .. } => {
                format!("Delete '{path}' from the server filesystem? This cannot be undone.")
            }
            ConfirmAction::DeleteAllocation { port, .. } => {
                format!("Delete allocation for port {port}? The port becomes available to other servers.")
            }
            ConfirmAction::DeleteTemplate { id, name } => {
                format!("Delete template '{name}' ({})?", short_id(id))
            }
            ConfirmAction::DeleteDbHost { name, .. } => {
                format!("Delete database host '{name}'? A host with databases cannot be deleted.")
            }
            ConfirmAction::DeleteDb { name, .. } => {
                format!("Drop database '{name}' and its user? This cannot be undone.")
            }
            ConfirmAction::DeleteBackup { name, .. } => {
                format!("Delete backup '{name}'? This cannot be undone.")
            }
            ConfirmAction::RestoreBackup { name, .. } => {
                format!("Restore backup '{name}'? Current server files will be replaced.")
            }
            ConfirmAction::DeleteProxy { domain, .. } => {
                format!("Delete proxy route for '{domain}'?")
            }
            ConfirmAction::DeleteNode { id, name } => {
                format!(
                    "Delete node '{name}' ({})? It must have no servers.",
                    short_id(id)
                )
            }
            ConfirmAction::RevokeApiKey { id, name } => {
                format!(
                    "Revoke API key '{name}' ({})? This cannot be undone.",
                    short_id(id)
                )
            }
            ConfirmAction::RevokeSftp { username, .. } => {
                format!("Revoke SFTP credential '{username}'?")
            }
            ConfirmAction::DeleteSchedule { name, .. } => {
                format!("Delete schedule '{name}'?")
            }
        }
    }
}

enum Action {
    SwitchTab(Tab),
    Connect,
    Disconnect,
    RefreshTab,
    RefreshAll,
    DismissToast,
    SelectServer(String),
    RefreshServers,
    RefreshDetail,
    LoadStats,
    Power {
        server: String,
        signal: &'static str,
    },
    Reinstall(String),
    SaveLimits,
    Suspend {
        server: String,
        resume: bool,
    },
    AskDeleteServer {
        id: String,
        name: String,
    },
    ToggleCreateServer,
    CreateServer,
    SelectNode(String),
    CreateNode,
    ToggleCreateNode,
    CreateAllocation,
    AskDeleteAllocation {
        node: String,
        id: String,
        port: u16,
    },
    SelectTemplate(String),
    ImportEgg,
    PreviewEgg,
    AskDeleteTemplate {
        id: String,
        name: String,
    },
    FilesGo,
    FilesUp,
    OpenEntry {
        path: String,
        is_directory: bool,
    },
    SaveFile,
    ReloadEditor,
    FileMkdir,
    FileRename,
    FileChmod,
    FileCompress,
    FileDecompress,
    AskDeleteFile(String),
    CreateDbHost,
    ToggleCreateDbHost,
    TestDbHost(String),
    AskDeleteDbHost {
        id: String,
        name: String,
    },
    ShowDbHost(String),
    CloseDbHost,
    CreateServerDb,
    AskDeleteDb {
        id: String,
        name: String,
    },
    CreateBackup,
    AskDeleteBackup {
        id: String,
        name: String,
    },
    AskRestoreBackup {
        id: String,
        name: String,
    },
    DownloadBackup(String),
    AskDeleteProxy {
        id: String,
        domain: String,
    },
    SendRaw,
    ConsoleClear,
    SendCommand,
    ConfirmYes,
    ConfirmNo,
    ClearNodeSecret,
    ClearCreatedDb,
    SaveServerConfig,
    ScheduleNew,
    ScheduleEdit(String),
    ScheduleSave,
    ScheduleDelete(String),
    ScheduleRun(String),
    SftpCreate,
    SftpRevoke(String),
    ApiKeyCreate,
    ApiKeyRevoke(String),
    NotificationSend,
    NotificationRetry(String),
    NodeSave,
    NodeRotate,
    NodeDelete,
    NodeUpdatesCheck,
    NodeUpdatesApply {
        reboot: bool,
    },
    NodeReboot,
    TemplatePublish,
    TemplateFeed,
    TemplateEditPublish,
    TemplateAutoUpdate,
    ProxyReconcile,
    ProxyEdit(String),
    ProxyNew,
    ProxySave,
    BulkPower,
    BulkCommand,
    ToggleBulk(String),
    AttachAllocation,
    DetachAllocation(String),
}

#[derive(Deserialize)]
struct MeInfo {
    key_id: Value,
    name: String,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    server_ids: Vec<String>,
    #[serde(default)]
    expires_at: Option<String>,
}

#[derive(Deserialize)]
struct ServersResponse {
    servers: Vec<Server>,
}

#[derive(Deserialize)]
struct NodesResponse {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct AllocationsResponse {
    allocations: Vec<Allocation>,
}

#[derive(Deserialize)]
struct TemplatesResponse {
    templates: Vec<TemplateRecord>,
}

#[derive(Deserialize)]
struct DbHostsResponse {
    hosts: Vec<DatabaseHost>,
}

#[derive(Deserialize)]
struct ServerDbsResponse {
    databases: Vec<ServerDatabase>,
}

#[derive(Deserialize)]
struct BackupsResponse {
    backups: Vec<Backup>,
}

#[derive(Deserialize)]
struct AuditResponse {
    events: Vec<AuditEvent>,
}

struct Startup {
    url: String,
    key: String,
    remember: bool,
    tab: Tab,
    server: Option<String>,
    config_note: Option<String>,
}

struct Args {
    api_url: Option<String>,
    api_key: Option<String>,
    help: bool,
    warnings: Vec<String>,
}

fn parse_args() -> Args {
    let mut args = Args {
        api_url: None,
        api_key: None,
        help: false,
        warnings: Vec::new(),
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        if let Some(value) = arg.strip_prefix("--api-url=") {
            args.api_url = Some(value.to_string());
        } else if arg == "--api-url" {
            match iter.next() {
                Some(value) => args.api_url = Some(value),
                None => args.warnings.push("--api-url requires a value".to_string()),
            }
        } else if let Some(value) = arg.strip_prefix("--api-key=") {
            args.api_key = Some(value.to_string());
        } else if arg == "--api-key" {
            match iter.next() {
                Some(value) => args.api_key = Some(value),
                None => args.warnings.push("--api-key requires a value".to_string()),
            }
        } else if arg == "--help" || arg == "-h" {
            args.help = true;
        } else if arg.starts_with("--") {
            args.warnings
                .push(format!("unknown argument ignored: {arg}"));
        }
    }
    args
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn main() -> eframe::Result<()> {
    let args = parse_args();
    if args.help {
        println!(
            "{APP_TITLE}\n\nUsage: ignite-containers-desktop [--api-url URL] [--api-key ic_...]\n\n\
             Environment: IGNITE_CONTAINERS_API_URL, IGNITE_CONTAINERS_API_KEY\n\
             Settings: $XDG_CONFIG_HOME/ignite-containers/desktop.json (or ~/.config/...)"
        );
        return Ok(());
    }

    let saved = config::load();
    let saved_path = config::config_path();
    let env_url = env_value("IGNITE_CONTAINERS_API_URL");
    let env_key = env_value("IGNITE_CONTAINERS_API_KEY");
    for warning in &args.warnings {
        eprintln!("{warning}");
    }

    let url = args
        .api_url
        .or(env_url)
        .or_else(|| saved.as_ref().and_then(|saved| saved.api_url.clone()))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_URL.to_string());
    let key = args
        .api_key
        .or(env_key)
        .or_else(|| saved.as_ref().and_then(|saved| saved.api_key.clone()))
        .unwrap_or_default();
    let remember = saved.as_ref().is_some_and(|saved| saved.remember);
    let tab = saved
        .as_ref()
        .and_then(|saved| saved.last_tab.as_deref())
        .and_then(Tab::from_saved)
        .unwrap_or(Tab::Overview);
    let server = saved.as_ref().and_then(|saved| saved.last_server.clone());
    let config_note = match (&saved_path, saved.as_ref()) {
        (Some(path), Some(_)) => Some(format!("Loaded settings from {}", path.display())),
        _ => None,
    };

    let startup = Startup {
        url,
        key,
        remember,
        tab,
        server,
        config_note,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_TITLE)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, startup)))),
    )
}

struct App {
    api: ApiWorker,
    console_worker: ConsoleWorker,

    url_input: String,
    key_input: String,
    show_key: bool,
    remember: bool,
    connecting: bool,
    pending_conn: Option<Connection>,
    conn: Option<Connection>,
    login_error: Option<String>,
    config_note: Option<String>,
    config_warning: Option<String>,

    tab: Tab,
    toast: Option<Toast>,
    inflight: usize,
    me: Option<MeInfo>,

    servers: Vec<Server>,
    server_filter: String,
    server_filter_state: String,
    server_filter_node: String,
    audit_limit: i64,
    servers_loaded: bool,
    servers_last_refresh: Instant,
    selected_server: Option<String>,
    pending_select_server: Option<String>,
    server_detail: Option<ServerDetail>,
    stats: Option<ServerStats>,
    stats_error: Option<String>,

    create_server: CreateServerForm,
    edit_limits: EditLimitsForm,
    server_config: ServerConfigForm,

    console_logs: HashMap<String, ConsoleLog>,
    console_input: String,
    console_stream: bool,
    console_watch: Option<(String, ConsoleMode)>,

    files: FilesState,
    schedule_form: ScheduleForm,
    sftp_ttl: String,
    api_key_form: ApiKeyForm,
    notification_form: NotificationForm,
    node_form: NodeForm,
    node_update_reboot: bool,
    publish_form: TemplatePublishForm,
    feed_form: TemplateFeedForm,
    template_auto_enabled: bool,
    template_auto_channel: String,
    proxy_edit: ProxyEditForm,
    feature_values: HashMap<String, Value>,
    feature_errors: HashMap<String, String>,
    bulk_selected: HashSet<String>,
    bulk_signal: String,
    bulk_command: String,
    attach_allocation: Option<String>,

    nodes: Vec<Node>,
    selected_node: Option<String>,
    node_allocations: HashMap<String, Vec<Allocation>>,
    create_node: CreateNodeForm,
    node_secret: Option<NodeSecret>,
    create_alloc: CreateAllocForm,

    templates: Vec<TemplateRecord>,
    selected_template: Option<String>,
    template_detail: Option<TemplateRecord>,
    egg_text: String,
    egg_version: String,
    egg_slug: String,
    egg_channel: String,
    egg_signature: String,
    egg_public_key: String,

    db_hosts: Vec<DatabaseHost>,
    db_host_view: Option<String>,
    server_dbs: Vec<ServerDatabase>,
    create_db_host: CreateDbHostForm,
    create_server_db: CreateServerDbForm,
    created_db: Option<CreatedDbInfo>,
    show_created_password: bool,

    backups: Vec<Backup>,
    backup_name: String,
    download_dir: String,

    audit: Vec<AuditEvent>,

    proxy_routes: Vec<Value>,
    proxy_error: Option<String>,

    raw_method: String,
    raw_path: String,
    raw_body: String,
    raw_view: Option<RawView>,
    raw_error: Option<String>,

    confirm: Option<ConfirmAction>,
    prefs_dirty: bool,
    last_prefs_save: Instant,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, startup: Startup) -> Self {
        install_style(&cc.egui_ctx);
        let api = ApiWorker::new(cc.egui_ctx.clone());
        let console_worker = ConsoleWorker::new(cc.egui_ctx.clone());
        let download_dir = std::env::var("HOME")
            .map(|home| format!("{home}/Downloads"))
            .unwrap_or_else(|_| ".".to_string());
        let mut app = Self {
            api,
            console_worker,
            url_input: startup.url,
            key_input: startup.key,
            show_key: false,
            remember: startup.remember,
            connecting: false,
            pending_conn: None,
            conn: None,
            login_error: None,
            config_note: startup.config_note,
            config_warning: None,
            tab: startup.tab,
            toast: None,
            inflight: 0,
            me: None,
            servers: Vec::new(),
            server_filter: String::new(),
            server_filter_state: String::new(),
            server_filter_node: String::new(),
            audit_limit: 200,
            servers_loaded: false,
            servers_last_refresh: Instant::now() - Duration::from_secs(60),
            selected_server: None,
            pending_select_server: startup.server,
            server_detail: None,
            stats: None,
            stats_error: None,
            create_server: CreateServerForm::new(),
            edit_limits: EditLimitsForm::default(),
            server_config: ServerConfigForm::default(),
            console_logs: HashMap::new(),
            console_input: String::new(),
            console_stream: true,
            console_watch: None,
            files: FilesState::fresh(),
            schedule_form: ScheduleForm::new(),
            sftp_ttl: String::new(),
            api_key_form: ApiKeyForm::new(),
            notification_form: NotificationForm::default(),
            node_form: NodeForm::default(),
            node_update_reboot: false,
            publish_form: TemplatePublishForm::default(),
            feed_form: TemplateFeedForm::default(),
            template_auto_enabled: false,
            template_auto_channel: "stable".to_string(),
            proxy_edit: ProxyEditForm::new(),
            feature_values: HashMap::new(),
            feature_errors: HashMap::new(),
            bulk_selected: HashSet::new(),
            bulk_signal: "restart".to_string(),
            bulk_command: String::new(),
            attach_allocation: None,
            nodes: Vec::new(),
            selected_node: None,
            node_allocations: HashMap::new(),
            create_node: CreateNodeForm::new(),
            node_secret: None,
            create_alloc: CreateAllocForm::new(),
            templates: Vec::new(),
            selected_template: None,
            template_detail: None,
            egg_text: String::new(),
            egg_version: String::new(),
            egg_slug: String::new(),
            egg_channel: "stable".to_string(),
            egg_signature: String::new(),
            egg_public_key: String::new(),
            db_hosts: Vec::new(),
            db_host_view: None,
            server_dbs: Vec::new(),
            create_db_host: CreateDbHostForm::new(),
            create_server_db: CreateServerDbForm::default(),
            created_db: None,
            show_created_password: false,
            backups: Vec::new(),
            backup_name: String::new(),
            download_dir,
            audit: Vec::new(),
            proxy_routes: Vec::new(),
            proxy_error: None,
            raw_method: "GET".to_string(),
            raw_path: "/api/v1/me".to_string(),
            raw_body: String::new(),
            raw_view: None,
            raw_error: None,
            confirm: None,
            prefs_dirty: false,
            last_prefs_save: Instant::now(),
        };

        if !app.key_input.trim().is_empty() {
            app.connect();
        }
        app
    }

    // ------------------------------------------------------------- transport

    fn connect(&mut self) {
        let url = self.url_input.trim().trim_end_matches('/').to_string();
        if url.is_empty() {
            self.toast_error("API URL is required");
            return;
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.toast_error("API URL must start with http:// or https://");
            return;
        }
        let key = self.key_input.trim().to_string();
        if key.is_empty() {
            self.toast_error("API key is required (ic_...)");
            return;
        }
        self.url_input = url.clone();
        let conn = Connection::new(url, key);
        self.pending_conn = Some(conn.clone());
        self.connecting = true;
        self.login_error = None;
        self.inflight += 1;
        self.api.send(ApiRequest {
            tag: Tag::Connect,
            conn,
            method: "GET".to_string(),
            path: "/api/v1/me".to_string(),
            body: None,
            download: None,
        });
    }

    fn send(&mut self, tag: Tag, method: &str, path: String, body: Option<Value>) {
        let Some(conn) = self.conn.clone() else {
            self.toast_error("Not connected");
            return;
        };
        self.inflight += 1;
        self.api.send(ApiRequest {
            tag,
            conn,
            method: method.to_string(),
            path,
            body,
            download: None,
        });
    }

    fn download(&mut self, tag: Tag, path: String, destination: PathBuf) {
        let Some(conn) = self.conn.clone() else {
            self.toast_error("Not connected");
            return;
        };
        self.inflight += 1;
        self.api.send(ApiRequest {
            tag,
            conn,
            method: "GET".to_string(),
            path,
            body: None,
            download: Some(destination),
        });
    }

    fn toast(&mut self, kind: StatusKind, text: impl Into<String>) {
        self.toast = Some(Toast {
            text: text.into(),
            kind,
            at: Instant::now(),
        });
    }

    fn toast_success(&mut self, text: impl Into<String>) {
        self.toast(StatusKind::Success, text);
    }

    fn toast_error(&mut self, text: impl Into<String>) {
        self.toast(StatusKind::Error, text);
    }

    fn toast_info(&mut self, text: impl Into<String>) {
        self.toast(StatusKind::Info, text);
    }

    fn ok(&mut self, outcome: Result<ApiResponse, String>) -> Option<ApiResponse> {
        match outcome {
            Ok(response) if response.is_success() => Some(response),
            Ok(response) => {
                let message = response.error_message();
                self.toast_error(message);
                None
            }
            Err(error) => {
                self.toast_error(error);
                None
            }
        }
    }

    // -------------------------------------------------------------- refresh

    fn refresh_all(&mut self) {
        self.refresh_me();
        self.refresh_servers();
        self.refresh_nodes();
        self.refresh_templates();
        self.refresh_feature("healthz", "/healthz");
        self.refresh_feature("readyz", "/readyz");
    }

    fn refresh_me(&mut self) {
        self.send(Tag::Me, "GET", "/api/v1/me".to_string(), None);
    }

    fn refresh_servers(&mut self) {
        self.servers_last_refresh = Instant::now();
        let mut path = "/api/v1/servers?limit=500".to_string();
        if !self.server_filter_state.is_empty() {
            path.push_str("&state=");
            path.push_str(&self.server_filter_state);
        }
        if !self.server_filter_node.is_empty() {
            path.push_str("&node_id=");
            path.push_str(&self.server_filter_node);
        }
        self.send(Tag::Servers, "GET", path, None);
    }

    fn refresh_nodes(&mut self) {
        self.send(Tag::Nodes, "GET", "/api/v1/nodes".to_string(), None);
    }

    fn refresh_templates(&mut self) {
        self.send(Tag::Templates, "GET", "/api/v1/templates".to_string(), None);
    }

    fn refresh_server_detail(&mut self, id: &str) {
        self.send(
            Tag::ServerDetail(id.to_string()),
            "GET",
            format!("/api/v1/servers/{id}"),
            None,
        );
    }

    fn refresh_stats(&mut self, id: &str) {
        self.send(
            Tag::ServerStats(id.to_string()),
            "GET",
            format!("/api/v1/servers/{id}/stats"),
            None,
        );
    }

    fn refresh_allocations(&mut self, node: &str) {
        self.send(
            Tag::Allocations(node.to_string()),
            "GET",
            format!("/api/v1/nodes/{node}/allocations"),
            None,
        );
    }

    fn refresh_files(&mut self) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        let path = self.files.path.clone();
        self.files.loading = true;
        self.files.list_error = None;
        self.send(
            Tag::FilesList {
                server: server.clone(),
                path: path.clone(),
            },
            "GET",
            format!("/api/v1/servers/{server}/files?path={}", encode_path(&path)),
            None,
        );
    }

    fn refresh_db_hosts(&mut self) {
        self.send(
            Tag::DbHosts,
            "GET",
            "/api/v1/database-hosts".to_string(),
            None,
        );
    }

    fn refresh_server_dbs(&mut self) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        self.send(
            Tag::ServerDbs(server.clone()),
            "GET",
            format!("/api/v1/servers/{server}/databases"),
            None,
        );
    }

    fn refresh_backups(&mut self) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        self.send(
            Tag::Backups(server.clone()),
            "GET",
            format!("/api/v1/servers/{server}/backups"),
            None,
        );
    }

    fn refresh_audit(&mut self) {
        self.send(
            Tag::Audit,
            "GET",
            format!("/api/v1/audit-events?limit={}", self.audit_limit.max(1)),
            None,
        );
    }

    fn refresh_proxy(&mut self) {
        self.send(Tag::Proxy, "GET", "/api/v1/proxy/routes".to_string(), None);
    }

    fn refresh_feature(&mut self, key: &str, path: &str) {
        self.send(Tag::Feature(key.to_string()), "GET", path.to_string(), None);
    }

    #[allow(clippy::too_many_arguments)]
    fn mutate_feature(
        &mut self,
        key: &str,
        refresh: &str,
        refresh_path: Option<&str>,
        method: &str,
        path: String,
        body: Option<Value>,
        success: &str,
        result_key: Option<&str>,
    ) {
        self.send(
            Tag::FeatureMutation {
                key: key.to_string(),
                refresh: refresh.to_string(),
                refresh_path: refresh_path.map(str::to_string),
                success: success.to_string(),
                result_key: result_key.map(str::to_string),
            },
            method,
            path,
            body,
        );
    }

    fn feature(&self, key: &str) -> Option<Value> {
        self.feature_values.get(key).cloned()
    }

    fn feature_error(&self, key: &str) -> Option<String> {
        self.feature_errors.get(key).cloned()
    }

    fn refresh_tab(&mut self) {
        match self.tab {
            Tab::Overview => self.refresh_all(),
            Tab::Servers | Tab::Console => self.refresh_servers(),
            Tab::Files => {
                if !self.servers_loaded {
                    self.refresh_servers();
                }
                if self.selected_server.is_some() {
                    self.refresh_files();
                }
            }
            Tab::Config => {
                if let Some(server) = self.selected_server.clone() {
                    self.refresh_server_detail(&server);
                }
            }
            Tab::Schedules => {
                if let Some(server) = self.selected_server.clone() {
                    self.refresh_feature(
                        &format!("schedules:{server}"),
                        &format!("/api/v1/servers/{server}/schedules"),
                    );
                }
            }
            Tab::Sftp => {
                if let Some(server) = self.selected_server.clone() {
                    self.refresh_feature(
                        &format!("sftp:{server}"),
                        &format!("/api/v1/servers/{server}/sftp"),
                    );
                }
            }
            Tab::Environment => {
                if let Some(server) = self.selected_server.clone() {
                    self.refresh_feature(
                        &format!("environment:{server}"),
                        &format!("/api/v1/servers/{server}/environment"),
                    );
                }
            }
            Tab::Jobs => {
                if let Some(server) = self.selected_server.clone() {
                    self.refresh_feature(
                        &format!("jobs:{server}"),
                        &format!("/api/v1/servers/{server}/jobs"),
                    );
                }
            }
            Tab::Nodes => {
                self.refresh_nodes();
                self.refresh_feature("allocations_all", "/api/v1/allocations");
                if let Some(node) = self.selected_node.clone() {
                    self.refresh_allocations(&node);
                    self.refresh_feature(
                        &format!("node_state:{node}"),
                        &format!("/api/v1/nodes/{node}/state"),
                    );
                    self.refresh_feature(
                        &format!("node_updates:{node}"),
                        &format!("/api/v1/nodes/{node}/updates"),
                    );
                }
            }
            Tab::Templates => {
                self.refresh_templates();
                if let Some(template) = self.selected_template.clone() {
                    self.refresh_feature(
                        &format!("template_versions:{template}"),
                        &format!("/api/v1/templates/{template}/versions"),
                    );
                }
            }
            Tab::Databases => {
                self.refresh_db_hosts();
                if self.selected_server.is_some() {
                    self.refresh_server_dbs();
                }
            }
            Tab::Notifications => {
                self.refresh_feature("notification_config", "/api/v1/notifications/config");
                self.refresh_feature("notifications", "/api/v1/notifications/emails?limit=200");
            }
            Tab::ApiKeys => self.refresh_feature("api_keys", "/api/v1/api-keys"),
            Tab::Backups => {
                if self.selected_server.is_some() {
                    self.refresh_backups();
                }
            }
            Tab::Audit => self.refresh_audit(),
            Tab::Proxy => {
                self.refresh_proxy();
                self.refresh_feature("proxy_status", "/api/v1/proxy/status");
            }
            Tab::Metrics => self.refresh_feature("metrics", "/metrics"),
            Tab::Raw => {}
        }
    }

    fn switch_tab(&mut self, tab: Tab) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        self.prefs_dirty = true;
        self.refresh_tab();
    }

    fn select_server(&mut self, id: String) {
        if self.selected_server.as_deref() == Some(id.as_str()) {
            return;
        }
        self.selected_server = Some(id.clone());
        self.server_detail = None;
        self.stats = None;
        self.stats_error = None;
        self.files = FilesState::fresh();
        self.server_dbs.clear();
        self.backups.clear();
        self.console_input.clear();
        self.schedule_form = ScheduleForm::new();
        self.schedule_form.server = Some(id.clone());
        self.sftp_ttl.clear();
        self.attach_allocation = None;
        self.feature_values.remove("sftp_created");
        self.feature_values.remove("server_free");
        self.prefs_dirty = true;
        self.refresh_server_detail(&id);
        self.refresh_stats(&id);
        match self.tab {
            Tab::Files => self.refresh_files(),
            Tab::Databases => self.refresh_server_dbs(),
            Tab::Backups => self.refresh_backups(),
            Tab::Schedules => self.refresh_feature(
                &format!("schedules:{id}"),
                &format!("/api/v1/servers/{id}/schedules"),
            ),
            Tab::Sftp => {
                self.refresh_feature(&format!("sftp:{id}"), &format!("/api/v1/servers/{id}/sftp"))
            }
            Tab::Environment => self.refresh_feature(
                &format!("environment:{id}"),
                &format!("/api/v1/servers/{id}/environment"),
            ),
            Tab::Jobs => {
                self.refresh_feature(&format!("jobs:{id}"), &format!("/api/v1/servers/{id}/jobs"))
            }
            _ => {}
        }
    }

    fn select_node(&mut self, id: String) {
        if self.selected_node.as_deref() == Some(id.as_str()) {
            return;
        }
        self.selected_node = Some(id.clone());
        self.refresh_allocations(&id);
        self.refresh_feature(
            &format!("node_state:{id}"),
            &format!("/api/v1/nodes/{id}/state"),
        );
        self.refresh_feature(
            &format!("node_updates:{id}"),
            &format!("/api/v1/nodes/{id}/updates"),
        );
        if let Some(node) = self.nodes.iter().find(|node| node.id.to_string() == id) {
            self.node_form.load(node);
        }
    }

    fn select_template(&mut self, id: String) {
        self.selected_template = Some(id.clone());
        self.template_detail = None;
        self.send(
            Tag::TemplateDetail(id.clone()),
            "GET",
            format!("/api/v1/templates/{id}"),
            None,
        );
        self.refresh_feature(
            &format!("template_versions:{id}"),
            &format!("/api/v1/templates/{id}/versions"),
        );
    }

    // -------------------------------------------------------------- results

    fn handle_api_results(&mut self) {
        let results = self.api.drain();
        for result in results {
            self.inflight = self.inflight.saturating_sub(1);
            self.handle_result(result.tag, result.outcome);
        }
    }

    fn handle_result(&mut self, tag: Tag, outcome: Result<ApiResponse, String>) {
        match tag {
            Tag::Connect => self.handle_connect(outcome),
            Tag::Me => {
                if let Some(response) = self.ok(outcome) {
                    match response.json::<MeInfo>() {
                        Ok(me) => self.me = Some(me),
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Tag::Servers => {
                if let Some(response) = self.ok(outcome) {
                    match response.json::<ServersResponse>() {
                        Ok(list) => {
                            self.servers = list.servers;
                            self.servers_loaded = true;
                            self.reconcile_server_selection();
                        }
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Tag::ServerDetail(id) => {
                if let Some(response) = self.ok(outcome) {
                    match response.json::<ServerDetail>() {
                        Ok(detail) => {
                            if self.selected_server.as_deref() == Some(id.as_str()) {
                                if self.server_config.server.as_deref() != Some(id.as_str()) {
                                    self.server_config.load(&detail.server);
                                }
                                let node_id = detail.server.node_id.to_string();
                                self.refresh_feature(
                                    &format!("server_free:{id}"),
                                    &format!("/api/v1/allocations?node_id={node_id}"),
                                );
                                self.server_detail = Some(detail);
                            }
                        }
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Tag::ServerStats(id) => match outcome {
                Ok(response) if response.is_success() => match response.json::<ServerStats>() {
                    Ok(stats) => {
                        if self.selected_server.as_deref() == Some(id.as_str()) {
                            self.stats = Some(stats);
                            self.stats_error = None;
                        }
                    }
                    Err(error) => self.toast_error(error),
                },
                Ok(response) => {
                    self.stats = None;
                    self.stats_error = Some(response.error_message());
                }
                Err(error) => {
                    self.stats = None;
                    self.stats_error = Some(error);
                }
            },
            Tag::Nodes => {
                if let Some(response) = self.ok(outcome) {
                    match response.json::<NodesResponse>() {
                        Ok(list) => {
                            self.nodes = list.nodes;
                            if self.selected_node.is_none() {
                                if let Some(first) = self.nodes.first() {
                                    let id = first.id.to_string();
                                    self.select_node(id);
                                }
                            } else if self.selected_node.as_deref().is_some_and(|selected| {
                                !self
                                    .nodes
                                    .iter()
                                    .any(|node| node.id.to_string() == selected)
                            }) {
                                self.selected_node = None;
                                if let Some(first) = self.nodes.first() {
                                    let id = first.id.to_string();
                                    self.select_node(id);
                                }
                            }
                        }
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Tag::Allocations(node) => {
                if let Some(response) = self.ok(outcome) {
                    match response.json::<AllocationsResponse>() {
                        Ok(list) => {
                            self.node_allocations.insert(node, list.allocations);
                        }
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Tag::Templates => {
                if let Some(response) = self.ok(outcome) {
                    match response.json::<TemplatesResponse>() {
                        Ok(list) => self.templates = list.templates,
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Tag::TemplateDetail(_) => {
                if let Some(response) = self.ok(outcome) {
                    match response.json::<TemplateRecord>() {
                        Ok(template) => {
                            self.template_auto_enabled = template.auto_update.enabled;
                            self.template_auto_channel =
                                format!("{:?}", template.auto_update.channel).to_lowercase();
                            self.template_detail = Some(template);
                        }
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Tag::FilesList { server, path } => {
                self.files.loading = false;
                if self.selected_server.as_deref() != Some(server.as_str()) {
                    return;
                }
                match outcome {
                    Ok(response) if response.is_success() => {
                        match response.json::<ignite_containers_proto::FileList>() {
                            Ok(list) => {
                                self.files.path = path;
                                self.files.entries = list.entries;
                                self.files.entries.sort_by(|left, right| {
                                    right
                                        .is_directory
                                        .cmp(&left.is_directory)
                                        .then_with(|| left.name.cmp(&right.name))
                                });
                                self.files.list_error = None;
                                if let Some(selected) = &self.files.selected {
                                    if !self
                                        .files
                                        .entries
                                        .iter()
                                        .any(|entry| &entry.path == selected)
                                    {
                                        self.files.selected = None;
                                    }
                                }
                            }
                            Err(error) => self.files.list_error = Some(error),
                        }
                    }
                    Ok(response) => self.files.list_error = Some(response.error_message()),
                    Err(error) => self.files.list_error = Some(error),
                }
            }
            Tag::FileRead { server, path } => {
                if self.selected_server.as_deref() != Some(server.as_str()) {
                    return;
                }
                if let Some(response) = self.ok(outcome) {
                    match response.json::<FileContent>() {
                        Ok(content) => {
                            self.files.selected = Some(path.clone());
                            self.files.editor = Some(FileEditor {
                                path: content.path,
                                content: content.content,
                                encoding: content.encoding,
                                size: content.size,
                                truncated: content.truncated,
                                dirty: false,
                            });
                        }
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Tag::DbHosts => {
                if let Some(response) = self.ok(outcome) {
                    match response.json::<DbHostsResponse>() {
                        Ok(list) => self.db_hosts = list.hosts,
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Tag::ServerDbs(server) => {
                if self.selected_server.as_deref() != Some(server.as_str()) {
                    return;
                }
                if let Some(response) = self.ok(outcome) {
                    match response.json::<ServerDbsResponse>() {
                        Ok(list) => self.server_dbs = list.databases,
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Tag::Backups(server) => {
                if self.selected_server.as_deref() != Some(server.as_str()) {
                    return;
                }
                if let Some(response) = self.ok(outcome) {
                    match response.json::<BackupsResponse>() {
                        Ok(list) => self.backups = list.backups,
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Tag::Audit => {
                if let Some(response) = self.ok(outcome) {
                    match response.json::<AuditResponse>() {
                        Ok(list) => self.audit = list.events,
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Tag::Proxy => match outcome {
                Ok(response) if response.is_success() => {
                    self.proxy_error = None;
                    self.proxy_routes = parse_proxy_routes(&response.body);
                }
                Ok(response) => {
                    self.proxy_routes.clear();
                    self.proxy_error = Some(proxy_error_message(response.status, &response.body));
                }
                Err(error) => {
                    self.proxy_routes.clear();
                    self.proxy_error = Some(error);
                }
            },
            Tag::Feature(key) => match outcome {
                Ok(response) if response.is_success() => {
                    let value = serde_json::from_str::<Value>(&response.body)
                        .unwrap_or_else(|_| Value::String(response.body.clone()));
                    self.feature_values.insert(key.clone(), value);
                    self.feature_errors.remove(&key);
                }
                Ok(response) => {
                    let message = response.error_message();
                    self.feature_errors.insert(key, message.clone());
                    self.toast_error(message);
                }
                Err(error) => {
                    self.feature_errors.insert(key, error.clone());
                    self.toast_error(error);
                }
            },
            Tag::FeatureMutation {
                key,
                refresh,
                refresh_path,
                success,
                result_key,
            } => match outcome {
                Ok(response) if response.is_success() => {
                    if let Some(result_key) = result_key {
                        if let Ok(value) = serde_json::from_str::<Value>(&response.body) {
                            self.feature_values.insert(result_key, value);
                        }
                    }
                    self.toast_success(success);
                    if !refresh.is_empty() {
                        if let Some(path) = refresh_path {
                            self.refresh_feature(&refresh, &path);
                        }
                    }
                }
                Ok(response) => {
                    let message = response.error_message();
                    self.feature_errors.insert(key, message.clone());
                    self.toast_error(message);
                }
                Err(error) => {
                    self.feature_errors.insert(key, error.clone());
                    self.toast_error(error);
                }
            },
            Tag::NodeUpdate { id } => {
                if self.ok(outcome).is_some() {
                    self.toast_success(format!("Node {} updated", short_id(&id)));
                    self.refresh_nodes();
                    self.refresh_feature(
                        &format!("node_state:{id}"),
                        &format!("/api/v1/nodes/{id}/state"),
                    );
                }
            }
            Tag::NodeRotate { id } => {
                if let Some(response) = self.ok(outcome) {
                    if let Ok(value) = serde_json::from_str::<Value>(&response.body) {
                        self.feature_values.insert("node_token".to_string(), value);
                    }
                    self.toast_success(format!("Token rotated for {}", short_id(&id)));
                    self.refresh_nodes();
                }
            }
            Tag::NodeDelete { id } => {
                if self.ok(outcome).is_some() {
                    self.toast_success(format!("Node {} deleted", short_id(&id)));
                    if self.selected_node.as_deref() == Some(id.as_str()) {
                        self.selected_node = None;
                    }
                    self.refresh_nodes();
                }
            }
            Tag::NodeUpdatesCheck { id } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Update check queued");
                    self.refresh_feature(
                        &format!("node_updates:{id}"),
                        &format!("/api/v1/nodes/{id}/updates"),
                    );
                    self.refresh_nodes();
                }
            }
            Tag::NodeUpdatesApply { id, reboot } => {
                if self.ok(outcome).is_some() {
                    self.toast_success(if reboot {
                        "Updates queued (reboot requested)"
                    } else {
                        "Updates queued"
                    });
                    self.refresh_feature(
                        &format!("node_updates:{id}"),
                        &format!("/api/v1/nodes/{id}/updates"),
                    );
                }
            }
            Tag::NodeReboot { id } => {
                if self.ok(outcome).is_some() {
                    self.toast_success(format!("Reboot queued for {}", short_id(&id)));
                }
            }
            Tag::TemplatePublish { id } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Template version published");
                    self.refresh_templates();
                    self.send(
                        Tag::TemplateDetail(id.clone()),
                        "GET",
                        format!("/api/v1/templates/{id}"),
                        None,
                    );
                    self.refresh_feature(
                        &format!("template_versions:{id}"),
                        &format!("/api/v1/templates/{id}/versions"),
                    );
                }
            }
            Tag::TemplateFeed => {
                if let Some(response) = self.ok(outcome) {
                    let published = serde_json::from_str::<Value>(&response.body)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("published")
                                .and_then(Value::as_array)
                                .map(Vec::len)
                        })
                        .unwrap_or(0);
                    self.toast_success(format!("Template feed synced ({published} published)"));
                    self.refresh_templates();
                }
            }
            Tag::TemplateAutoUpdate { id } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Template auto-update updated");
                    self.refresh_templates();
                    self.send(
                        Tag::TemplateDetail(id.clone()),
                        "GET",
                        format!("/api/v1/templates/{id}"),
                        None,
                    );
                }
            }
            Tag::ProxyUpdate { id } => {
                if self.ok(outcome).is_some() {
                    self.toast_success(format!("Proxy route {} updated", short_id(&id)));
                    self.refresh_proxy();
                    self.refresh_feature("proxy_status", "/api/v1/proxy/status");
                }
            }
            Tag::ProxyReconcile => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Caddy routes reconciled");
                    self.refresh_proxy();
                    self.refresh_feature("proxy_status", "/api/v1/proxy/status");
                }
            }
            Tag::BulkPower => {
                if let Some(response) = self.ok(outcome) {
                    let count = serde_json::from_str::<Value>(&response.body)
                        .ok()
                        .and_then(|value| {
                            value.get("results").and_then(Value::as_array).map(Vec::len)
                        })
                        .unwrap_or(0);
                    self.toast_success(format!("Bulk power queued for {count} server(s)"));
                    self.refresh_servers();
                }
            }
            Tag::AllocationAttach { server, .. } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Allocation attached");
                    self.refresh_server_detail(&server);
                    self.attach_allocation = None;
                }
            }
            Tag::AllocationDetach { server, .. } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Allocation detached");
                    self.refresh_server_detail(&server);
                }
            }
            Tag::BulkCommand => {
                if let Some(response) = self.ok(outcome) {
                    let count = serde_json::from_str::<Value>(&response.body)
                        .ok()
                        .and_then(|value| {
                            value.get("results").and_then(Value::as_array).map(Vec::len)
                        })
                        .unwrap_or(0);
                    self.toast_success(format!("Bulk command queued for {count} server(s)"));
                }
            }
            Tag::Raw => match outcome {
                Ok(response) => {
                    self.raw_view = Some(RawView {
                        status: response.status,
                        content_type: response.content_type,
                        body: pretty_body(&response.body),
                    });
                    self.raw_error = None;
                }
                Err(error) => {
                    self.raw_error = Some(error);
                    self.raw_view = None;
                }
            },
            Tag::ServerCreate => self.handle_server_create(outcome),
            Tag::ServerUpdate { server } => {
                if self.ok(outcome).is_some() {
                    self.toast_success(format!("Server {} updated", short_id(&server)));
                    self.refresh_servers();
                    self.refresh_server_detail(&server);
                    self.refresh_stats(&server);
                }
            }
            Tag::ServerPower { server, action } => {
                if self.ok(outcome).is_some() {
                    self.toast_success(format!("{action} requested for {}", short_id(&server)));
                    self.refresh_servers();
                    self.refresh_server_detail(&server);
                    self.refresh_stats(&server);
                }
            }
            Tag::ConsoleCommand { .. } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Console command sent");
                }
            }
            Tag::ServerReinstall { server } => {
                if self.ok(outcome).is_some() {
                    self.toast_success(format!("Reinstall requested for {}", short_id(&server)));
                    self.refresh_servers();
                    self.refresh_server_detail(&server);
                }
            }
            Tag::ServerSuspend { server, resume } => {
                if self.ok(outcome).is_some() {
                    self.toast_success(if resume {
                        "Server resumed".to_string()
                    } else {
                        "Server suspended".to_string()
                    });
                    self.refresh_servers();
                    self.refresh_server_detail(&server);
                }
            }
            Tag::ServerDelete { server } => {
                if self.ok(outcome).is_some() {
                    self.toast_success(format!("Server {} deleted", short_id(&server)));
                    if self.selected_server.as_deref() == Some(server.as_str()) {
                        self.selected_server = None;
                        self.server_detail = None;
                        self.stats = None;
                    }
                    self.refresh_servers();
                }
            }
            Tag::NodeCreate => {
                if let Some(response) = self.ok(outcome) {
                    match serde_json::from_str::<Value>(&response.body) {
                        Ok(value) => {
                            let node = value
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("node")
                                .to_string();
                            if let Some(token) = value.get("agent_token").and_then(Value::as_str) {
                                self.node_secret = Some(NodeSecret {
                                    node: node.clone(),
                                    token: token.to_string(),
                                });
                            }
                            self.toast_success(format!("Node '{node}' created"));
                            self.refresh_nodes();
                        }
                        Err(error) => self.toast_error(format!(
                            "Node created but the response was unreadable: {error}"
                        )),
                    }
                }
            }
            Tag::AllocationCreate { node } => {
                if let Some(response) = self.ok(outcome) {
                    let message = response
                        .json::<Value>()
                        .ok()
                        .and_then(|value| {
                            let created = value.get("created")?.as_u64()?;
                            let skipped = value.get("skipped").and_then(Value::as_u64).unwrap_or(0);
                            Some(format!(
                                "Created {created} allocation(s), {skipped} skipped"
                            ))
                        })
                        .unwrap_or_else(|| "Allocations created".to_string());
                    self.toast_success(message);
                    self.refresh_allocations(&node);
                }
            }
            Tag::AllocationDelete { node, .. } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Allocation deleted");
                    self.refresh_allocations(&node);
                    self.refresh_feature("allocations_all", "/api/v1/allocations");
                }
            }
            Tag::TemplateImport => {
                if let Some(response) = self.ok(outcome) {
                    match serde_json::from_str::<Value>(&response.body) {
                        Ok(value) => {
                            let name = value
                                .pointer("/template/meta/name")
                                .and_then(Value::as_str)
                                .or_else(|| value.get("slug").and_then(Value::as_str))
                                .unwrap_or("template")
                                .to_string();
                            let saved = value.get("saved").and_then(Value::as_bool).unwrap_or(true);
                            self.toast_success(if saved {
                                format!("Imported template '{name}'")
                            } else {
                                format!("Translated template '{name}' (not saved)")
                            });
                            if saved {
                                self.egg_text.clear();
                                self.refresh_templates();
                            }
                        }
                        Err(error) => self.toast_error(format!(
                            "Template imported but the response was unreadable: {error}"
                        )),
                    }
                }
            }
            Tag::TemplateDelete { id } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Template deleted");
                    if self.selected_template.as_deref() == Some(id.as_str()) {
                        self.selected_template = None;
                        self.template_detail = None;
                    }
                    self.refresh_templates();
                }
            }
            Tag::FileSave { server } => {
                if self.ok(outcome).is_some() {
                    if let Some(editor) = &mut self.files.editor {
                        editor.dirty = false;
                    }
                    self.toast_success("File saved");
                    if self.selected_server.as_deref() == Some(server.as_str()) {
                        self.refresh_files();
                    }
                }
            }
            Tag::FileMkdir { server } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Directory created");
                    self.files.new_dir.clear();
                    if self.selected_server.as_deref() == Some(server.as_str()) {
                        self.refresh_files();
                    }
                }
            }
            Tag::FileRename { server } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("File renamed");
                    self.files.rename_to.clear();
                    if self.selected_server.as_deref() == Some(server.as_str()) {
                        self.refresh_files();
                    }
                }
            }
            Tag::FileChmod { server } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Permissions updated");
                    if self.selected_server.as_deref() == Some(server.as_str()) {
                        self.refresh_files();
                    }
                }
            }
            Tag::FileDelete { server } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("File deleted");
                    self.files.selected = None;
                    self.files.editor = None;
                    if self.selected_server.as_deref() == Some(server.as_str()) {
                        self.refresh_files();
                    }
                }
            }
            Tag::FileCompress { server } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Archive created");
                    if self.selected_server.as_deref() == Some(server.as_str()) {
                        self.refresh_files();
                    }
                }
            }
            Tag::FileDecompress { server } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Archive extracted");
                    if self.selected_server.as_deref() == Some(server.as_str()) {
                        self.refresh_files();
                    }
                }
            }
            Tag::DbHostCreate => {
                if let Some(response) = self.ok(outcome) {
                    let name = response
                        .json::<DatabaseHost>()
                        .map(|host| host.name)
                        .unwrap_or_else(|_| "database host".to_string());
                    self.toast_success(format!("Database host '{name}' created"));
                    self.create_db_host.show = false;
                    self.create_db_host.admin_password.clear();
                    self.refresh_db_hosts();
                }
            }
            Tag::DbHostDelete { .. } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Database host deleted");
                    self.refresh_db_hosts();
                }
            }
            Tag::DbHostTest { id } => match outcome {
                Ok(response) if response.is_success() => {
                    let version = serde_json::from_str::<Value>(&response.body)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("version")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .unwrap_or_else(|| "connected".to_string());
                    self.toast_success(format!(
                        "Database host {}:{id} reachable ({version})",
                        short_id(&id)
                    ));
                }
                Ok(response) => self.toast_error(response.error_message()),
                Err(error) => self.toast_error(error),
            },
            Tag::DbCreate { server } => {
                if let Some(response) = self.ok(outcome) {
                    match serde_json::from_str::<Value>(&response.body) {
                        Ok(value) => {
                            let database = value
                                .get("database_name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            self.created_db = Some(CreatedDbInfo {
                                database,
                                username: value
                                    .get("username")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                                password: value
                                    .get("password")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                                host: value
                                    .get("host")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                                port: value
                                    .get("port")
                                    .and_then(Value::as_u64)
                                    .and_then(|port| u16::try_from(port).ok())
                                    .unwrap_or(3306),
                                remote: value
                                    .get("remote")
                                    .and_then(Value::as_str)
                                    .unwrap_or("%")
                                    .to_string(),
                            });
                            self.show_created_password = false;
                            self.toast_success("Database created; credentials shown below");
                            if self.selected_server.as_deref() == Some(server.as_str()) {
                                self.refresh_server_dbs();
                            }
                        }
                        Err(error) => self.toast_error(format!(
                            "Database created but the response was unreadable: {error}"
                        )),
                    }
                }
            }
            Tag::DbDelete { server, .. } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Database deleted");
                    if self.selected_server.as_deref() == Some(server.as_str()) {
                        self.refresh_server_dbs();
                    }
                }
            }
            Tag::BackupCreate { server } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Backup started");
                    self.backup_name.clear();
                    if self.selected_server.as_deref() == Some(server.as_str()) {
                        self.refresh_backups();
                    }
                }
            }
            Tag::BackupDelete { server, .. } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Backup deleted");
                    if self.selected_server.as_deref() == Some(server.as_str()) {
                        self.refresh_backups();
                    }
                }
            }
            Tag::BackupRestore { server, .. } => {
                if self.ok(outcome).is_some() {
                    self.toast_success("Restore started");
                    if self.selected_server.as_deref() == Some(server.as_str()) {
                        self.refresh_backups();
                    }
                }
            }
            Tag::BackupDownload { .. } => {
                if let Some(response) = self.ok(outcome) {
                    self.toast_success(response.body);
                }
            }
            Tag::ProxyCreate => match outcome {
                Ok(response) if response.is_success() => {
                    self.proxy_error = None;
                    self.toast_success("Proxy route created");
                    self.refresh_proxy();
                    self.refresh_feature("proxy_status", "/api/v1/proxy/status");
                }
                Ok(response) => {
                    let message = proxy_error_message(response.status, &response.body);
                    self.proxy_error = Some(message.clone());
                    self.toast_error(message);
                }
                Err(error) => {
                    self.proxy_error = Some(error.clone());
                    self.toast_error(error);
                }
            },
            Tag::ProxyDelete { .. } => match outcome {
                Ok(response) if response.is_success() => {
                    self.toast_success("Proxy route deleted");
                    self.refresh_proxy();
                    self.refresh_feature("proxy_status", "/api/v1/proxy/status");
                }
                Ok(response) => {
                    let message = proxy_error_message(response.status, &response.body);
                    self.toast_error(message);
                }
                Err(error) => self.toast_error(error),
            },
        }
    }

    fn handle_connect(&mut self, outcome: Result<ApiResponse, String>) {
        self.connecting = false;
        match outcome {
            Ok(response) if response.is_success() => {
                self.conn = self.pending_conn.take();
                self.login_error = None;
                match response.json::<MeInfo>() {
                    Ok(me) => {
                        let name = me.name.clone();
                        self.me = Some(me);
                        self.toast_success(format!("Connected as {name}"));
                    }
                    Err(error) => self.toast_error(error),
                }
                self.save_prefs();
                self.refresh_all();
            }
            Ok(response) => {
                self.pending_conn = None;
                self.conn = None;
                let message = response.error_message();
                self.login_error = Some(message.clone());
                self.toast_error(message);
            }
            Err(error) => {
                self.pending_conn = None;
                self.conn = None;
                self.login_error = Some(error.clone());
                self.toast_error(error);
            }
        }
    }

    fn handle_server_create(&mut self, outcome: Result<ApiResponse, String>) {
        if let Some(response) = self.ok(outcome) {
            match response.json::<ServerDetail>() {
                Ok(detail) => {
                    let id = detail.server.id.to_string();
                    let name = detail.server.name.clone();
                    self.selected_server = Some(id.clone());
                    self.server_detail = Some(detail);
                    self.stats = None;
                    self.toast_success(format!("Server '{name}' created"));
                    self.create_server.name.clear();
                    self.create_server.show = false;
                    self.create_server.error = None;
                    self.refresh_servers();
                    self.refresh_stats(&id);
                }
                Err(error) => self.toast_error(error),
            }
        }
    }

    fn reconcile_server_selection(&mut self) {
        if let Some(pending) = self.pending_select_server.take() {
            if self
                .servers
                .iter()
                .any(|server| server.id.to_string() == pending)
            {
                self.select_server(pending);
                return;
            }
        }
        if let Some(selected) = &self.selected_server {
            if !self
                .servers
                .iter()
                .any(|server| server.id.to_string() == *selected)
            {
                self.selected_server = None;
                self.server_detail = None;
                self.stats = None;
            }
        }
        if self.selected_server.is_none() {
            if let Some(first) = self.servers.first() {
                let id = first.id.to_string();
                self.select_server(id);
            }
        }
    }

    // ------------------------------------------------------------- console

    fn handle_console_events(&mut self) {
        for event in self.console_worker.drain() {
            match event {
                ConsoleEvent::Lines {
                    server_id,
                    lines,
                    cursor,
                    truncated,
                } => {
                    let log = self.console_logs.entry(server_id).or_default();
                    if let Some(hint) = log.clear_hint.take() {
                        let overlap = tail_overlap(&hint, &lines);
                        if overlap < lines.len() {
                            log.lines.extend(lines[overlap..].iter().cloned());
                        }
                    } else {
                        merge_console_lines(&mut log.lines, &lines, CONSOLE_LIMIT);
                    }
                    let excess = log.lines.len().saturating_sub(CONSOLE_LIMIT);
                    if excess > 0 {
                        log.lines.drain(0..excess);
                    }
                    log.cursor = cursor;
                    log.truncated = truncated;
                    log.last_error = None;
                    log.last_update = Some(Instant::now());
                }
                ConsoleEvent::Error { server_id, message } => {
                    let log = self.console_logs.entry(server_id).or_default();
                    log.last_error = Some(message);
                    log.last_update = Some(Instant::now());
                }
            }
        }
    }

    fn sync_console_poller(&mut self) {
        let mode = if self.console_stream {
            ConsoleMode::Stream
        } else {
            ConsoleMode::Poll
        };
        let desired = if self.tab == Tab::Console && self.conn.is_some() {
            self.selected_server
                .clone()
                .filter(|id| self.server_state(id) == Some(ServerState::Running))
                .map(|id| (id, mode))
        } else {
            None
        };
        if desired == self.console_watch {
            return;
        }
        match &desired {
            Some((id, mode)) => {
                if let Some(conn) = &self.conn {
                    self.console_worker.watch(id, conn, *mode);
                }
            }
            None => self.console_worker.pause(),
        }
        self.console_watch = desired;
    }

    fn server_state(&self, id: &str) -> Option<ServerState> {
        self.servers
            .iter()
            .find(|server| server.id.to_string() == id)
            .map(|server| server.state)
    }

    fn send_console_command(&mut self) {
        let Some(server) = self.selected_server.clone() else {
            self.toast_error("Select a server first");
            return;
        };
        let command = self.console_input.trim().to_string();
        if command.is_empty() {
            return;
        }
        self.send(
            Tag::ConsoleCommand {
                server: server.clone(),
            },
            "POST",
            format!("/api/v1/servers/{server}/command"),
            Some(json!({ "command": command })),
        );
        self.console_input.clear();
    }

    // --------------------------------------------------------------- actions

    fn apply_actions(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::SwitchTab(tab) => self.switch_tab(tab),
                Action::Connect => self.connect(),
                Action::Disconnect => {
                    self.console_worker.pause();
                    self.console_watch = None;
                    self.conn = None;
                    self.me = None;
                    self.pending_conn = None;
                    self.connecting = false;
                    self.login_error = None;
                    self.toast_info("Disconnected");
                }
                Action::RefreshTab => self.refresh_tab(),
                Action::RefreshAll => self.refresh_all(),
                Action::DismissToast => self.toast = None,
                Action::SelectServer(id) => self.select_server(id),
                Action::RefreshServers => self.refresh_servers(),
                Action::RefreshDetail => {
                    if let Some(server) = self.selected_server.clone() {
                        self.refresh_server_detail(&server);
                    }
                }
                Action::LoadStats => {
                    if let Some(server) = self.selected_server.clone() {
                        self.refresh_stats(&server);
                    }
                }
                Action::Power { server, signal } => {
                    self.send(
                        Tag::ServerPower {
                            server: server.clone(),
                            action: signal,
                        },
                        "POST",
                        format!("/api/v1/servers/{server}/power"),
                        Some(json!({ "signal": signal })),
                    );
                }
                Action::Reinstall(server) => {
                    self.send(
                        Tag::ServerReinstall {
                            server: server.clone(),
                        },
                        "POST",
                        format!("/api/v1/servers/{server}/reinstall"),
                        Some(json!({})),
                    );
                }
                Action::SaveLimits => self.save_limits(),
                Action::Suspend { server, resume } => {
                    let endpoint = if resume { "resume" } else { "suspend" };
                    self.send(
                        Tag::ServerSuspend {
                            server: server.clone(),
                            resume,
                        },
                        "POST",
                        format!("/api/v1/servers/{server}/{endpoint}"),
                        Some(json!({})),
                    );
                }
                Action::AskDeleteServer { id, name } => {
                    self.confirm = Some(ConfirmAction::DeleteServer { id, name });
                }
                Action::ToggleCreateServer => {
                    self.create_server.show = !self.create_server.show;
                }
                Action::CreateServer => self.create_server(),
                Action::SelectNode(id) => self.select_node(id),
                Action::CreateNode => self.create_node(),
                Action::ToggleCreateNode => {
                    self.create_node.show = !self.create_node.show;
                }
                Action::CreateAllocation => self.create_allocation(),
                Action::AskDeleteAllocation { node, id, port } => {
                    self.confirm = Some(ConfirmAction::DeleteAllocation { node, id, port });
                }
                Action::SelectTemplate(id) => self.select_template(id),
                Action::ImportEgg => self.import_egg(true),
                Action::PreviewEgg => self.import_egg(false),
                Action::AskDeleteTemplate { id, name } => {
                    self.confirm = Some(ConfirmAction::DeleteTemplate { id, name });
                }
                Action::FilesGo => {
                    self.files.path = normalize_path(&self.files.path);
                    self.refresh_files();
                }
                Action::FilesUp => {
                    self.files.path = parent_path(&self.files.path);
                    self.files.selected = None;
                    self.refresh_files();
                }
                Action::OpenEntry { path, is_directory } => {
                    if is_directory {
                        self.files.path = path;
                        self.files.selected = None;
                        self.refresh_files();
                    } else if let Some(server) = self.selected_server.clone() {
                        self.files.selected = Some(path.clone());
                        self.send(
                            Tag::FileRead {
                                server: server.clone(),
                                path: path.clone(),
                            },
                            "GET",
                            format!(
                                "/api/v1/servers/{server}/files/content?path={}",
                                encode_path(&path)
                            ),
                            None,
                        );
                    }
                }
                Action::SaveFile => self.save_file(),
                Action::ReloadEditor => {
                    if let Some(editor) = &self.files.editor {
                        let path = editor.path.clone();
                        self.open_file(path);
                    }
                }
                Action::FileMkdir => self.file_mkdir(),
                Action::FileRename => self.file_rename(),
                Action::FileChmod => self.file_chmod(),
                Action::FileCompress => self.file_compress(false),
                Action::FileDecompress => self.file_compress(true),
                Action::AskDeleteFile(path) => {
                    if let Some(server) = self.selected_server.clone() {
                        self.confirm = Some(ConfirmAction::DeleteFile { server, path });
                    }
                }
                Action::CreateDbHost => self.create_db_host(),
                Action::ToggleCreateDbHost => {
                    self.create_db_host.show = !self.create_db_host.show;
                }
                Action::ShowDbHost(id) => self.show_db_host(&id),
                Action::CloseDbHost => self.db_host_view = None,
                Action::TestDbHost(id) => {
                    self.send(
                        Tag::DbHostTest { id: id.clone() },
                        "POST",
                        format!("/api/v1/database-hosts/{id}/test"),
                        Some(json!({})),
                    );
                }
                Action::AskDeleteDbHost { id, name } => {
                    self.confirm = Some(ConfirmAction::DeleteDbHost { id, name });
                }
                Action::CreateServerDb => self.create_server_db(),
                Action::AskDeleteDb { id, name } => {
                    if let Some(server) = self.selected_server.clone() {
                        self.confirm = Some(ConfirmAction::DeleteDb { server, id, name });
                    }
                }
                Action::CreateBackup => {
                    if let Some(server) = self.selected_server.clone() {
                        let mut body = Map::new();
                        if !self.backup_name.trim().is_empty() {
                            body.insert("name".to_string(), json!(self.backup_name.trim()));
                        }
                        self.send(
                            Tag::BackupCreate {
                                server: server.clone(),
                            },
                            "POST",
                            format!("/api/v1/servers/{server}/backups"),
                            Some(Value::Object(body)),
                        );
                    }
                }
                Action::AskDeleteBackup { id, name } => {
                    if let Some(server) = self.selected_server.clone() {
                        self.confirm = Some(ConfirmAction::DeleteBackup { server, id, name });
                    }
                }
                Action::AskRestoreBackup { id, name } => {
                    if let Some(server) = self.selected_server.clone() {
                        self.confirm = Some(ConfirmAction::RestoreBackup { server, id, name });
                    }
                }
                Action::DownloadBackup(id) => self.download_backup(&id),
                Action::AskDeleteProxy { id, domain } => {
                    self.confirm = Some(ConfirmAction::DeleteProxy { id, domain });
                }
                Action::SendRaw => self.send_raw(),
                Action::ConsoleClear => self.clear_console(),
                Action::SendCommand => self.send_console_command(),
                Action::ConfirmYes => self.confirm_action(),
                Action::ConfirmNo => self.confirm = None,
                Action::ClearNodeSecret => self.node_secret = None,
                Action::ClearCreatedDb => self.created_db = None,
                Action::SaveServerConfig => self.save_server_config(),
                Action::ScheduleNew => self.schedule_new(),
                Action::ScheduleEdit(id) => self.schedule_edit(&id),
                Action::ScheduleSave => self.schedule_save(),
                Action::ScheduleDelete(id) => {
                    let server = self.selected_server.clone().unwrap_or_default();
                    let name = self
                        .feature(&format!("schedules:{server}"))
                        .and_then(|value| value.get("schedules").and_then(Value::as_array).cloned())
                        .and_then(|items| {
                            items.into_iter().find(|item| {
                                item.get("id").and_then(Value::as_str) == Some(id.as_str())
                            })
                        })
                        .and_then(|item| {
                            item.get("name").and_then(Value::as_str).map(str::to_string)
                        })
                        .unwrap_or_else(|| short_id(&id).to_string());
                    self.confirm = Some(ConfirmAction::DeleteSchedule { server, id, name });
                }
                Action::ScheduleRun(id) => self.schedule_run(&id),
                Action::SftpCreate => self.create_sftp(),
                Action::SftpRevoke(id) => {
                    let server = self.selected_server.clone().unwrap_or_default();
                    let username = self
                        .feature(&format!("sftp:{server}"))
                        .and_then(|value| {
                            value.get("credentials").and_then(Value::as_array).cloned()
                        })
                        .and_then(|items| {
                            items.into_iter().find(|item| {
                                item.get("id").and_then(Value::as_str) == Some(id.as_str())
                            })
                        })
                        .and_then(|item| {
                            item.get("username")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .unwrap_or_else(|| short_id(&id).to_string());
                    self.confirm = Some(ConfirmAction::RevokeSftp {
                        server,
                        id,
                        username,
                    });
                }
                Action::ApiKeyCreate => self.create_api_key(),
                Action::ApiKeyRevoke(id) => {
                    let name = self
                        .feature("api_keys")
                        .and_then(|value| value.get("keys").and_then(Value::as_array).cloned())
                        .and_then(|keys| {
                            keys.into_iter().find(|key| {
                                key.get("id").and_then(Value::as_str) == Some(id.as_str())
                            })
                        })
                        .and_then(|key| key.get("name").and_then(Value::as_str).map(str::to_string))
                        .unwrap_or_else(|| short_id(&id).to_string());
                    self.confirm = Some(ConfirmAction::RevokeApiKey { id, name });
                }
                Action::NotificationSend => self.send_notification(),
                Action::NotificationRetry(id) => self.retry_notification(&id),
                Action::NodeSave => self.save_node(),
                Action::NodeRotate => self.rotate_node(),
                Action::NodeDelete => self.ask_delete_node(),
                Action::NodeUpdatesCheck => self.check_node_updates(),
                Action::NodeUpdatesApply { reboot } => self.apply_node_updates(reboot),
                Action::NodeReboot => self.reboot_node(),
                Action::TemplatePublish => self.publish_template(),
                Action::TemplateFeed => self.sync_template_feed(),
                Action::TemplateEditPublish => self.edit_publish_template(),
                Action::TemplateAutoUpdate => self.update_template_auto(),
                Action::ProxyReconcile => self.reconcile_proxy(),
                Action::ProxyEdit(id) => self.proxy_edit_route(&id),
                Action::ProxyNew => self.proxy_new(),
                Action::ProxySave => self.save_proxy_route(),
                Action::BulkPower => self.bulk_power(),
                Action::BulkCommand => self.bulk_command(),
                Action::AttachAllocation => self.attach_allocation(),
                Action::DetachAllocation(allocation) => self.detach_allocation(&allocation),
                Action::ToggleBulk(id) => {
                    if !self.bulk_selected.insert(id.clone()) {
                        self.bulk_selected.remove(&id);
                    }
                }
            }
        }
    }

    fn confirm_action(&mut self) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        match confirm {
            ConfirmAction::DeleteServer { id, .. } => {
                self.send(
                    Tag::ServerDelete { server: id.clone() },
                    "DELETE",
                    format!("/api/v1/servers/{id}"),
                    None,
                );
            }
            ConfirmAction::DeleteFile { server, path } => {
                self.send(
                    Tag::FileDelete {
                        server: server.clone(),
                    },
                    "DELETE",
                    format!("/api/v1/servers/{server}/files?path={}", encode_path(&path)),
                    None,
                );
            }
            ConfirmAction::DeleteAllocation { node, id, .. } => {
                self.send(
                    Tag::AllocationDelete {
                        node: node.clone(),
                        id: id.clone(),
                    },
                    "DELETE",
                    format!("/api/v1/allocations/{id}"),
                    None,
                );
            }
            ConfirmAction::DeleteTemplate { id, .. } => {
                self.send(
                    Tag::TemplateDelete { id: id.clone() },
                    "DELETE",
                    format!("/api/v1/templates/{id}"),
                    None,
                );
            }
            ConfirmAction::DeleteDbHost { id, .. } => {
                self.send(
                    Tag::DbHostDelete { id: id.clone() },
                    "DELETE",
                    format!("/api/v1/database-hosts/{id}"),
                    None,
                );
            }
            ConfirmAction::DeleteDb { server, id, .. } => {
                self.send(
                    Tag::DbDelete {
                        server: server.clone(),
                        id: id.clone(),
                    },
                    "DELETE",
                    format!("/api/v1/servers/{server}/databases/{id}"),
                    None,
                );
            }
            ConfirmAction::DeleteBackup { server, id, .. } => {
                self.send(
                    Tag::BackupDelete {
                        server: server.clone(),
                        id: id.clone(),
                    },
                    "DELETE",
                    format!("/api/v1/servers/{server}/backups/{id}"),
                    None,
                );
            }
            ConfirmAction::RestoreBackup { server, id, .. } => {
                self.send(
                    Tag::BackupRestore {
                        server: server.clone(),
                        id: id.clone(),
                    },
                    "POST",
                    format!("/api/v1/servers/{server}/backups/{id}/restore"),
                    Some(json!({})),
                );
            }
            ConfirmAction::DeleteProxy { id, .. } => {
                self.send(
                    Tag::ProxyDelete { id: id.clone() },
                    "DELETE",
                    format!("/api/v1/proxy/routes/{id}"),
                    None,
                );
            }
            ConfirmAction::DeleteNode { id, .. } => {
                self.send(
                    Tag::NodeDelete { id: id.clone() },
                    "DELETE",
                    format!("/api/v1/nodes/{id}"),
                    None,
                );
            }
            ConfirmAction::RevokeApiKey { id, .. } => {
                self.mutate_feature(
                    "api_key_revoke",
                    "api_keys",
                    Some("/api/v1/api-keys"),
                    "DELETE",
                    format!("/api/v1/api-keys/{id}"),
                    None,
                    "API key revoked",
                    None,
                );
            }
            ConfirmAction::RevokeSftp { server, id, .. } => {
                let key = format!("sftp:{server}");
                self.mutate_feature(
                    "sftp_revoke",
                    &key,
                    Some(&key),
                    "DELETE",
                    format!("/api/v1/servers/{server}/sftp/{id}"),
                    None,
                    "SFTP credential revoked",
                    None,
                );
            }
            ConfirmAction::DeleteSchedule { server, id, .. } => {
                let key = format!("schedules:{server}");
                self.mutate_feature(
                    "schedule_delete",
                    &key,
                    Some(&key),
                    "DELETE",
                    format!("/api/v1/schedules/{id}"),
                    None,
                    "Schedule deleted",
                    None,
                );
            }
        }
    }

    fn save_file(&mut self) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        let Some(editor) = &self.files.editor else {
            return;
        };
        if editor.encoding != "utf8" {
            self.toast_error("This file is not UTF-8; editing is disabled");
            return;
        }
        let path = editor.path.clone();
        let body = json!({
            "path": path,
            "content": editor.content,
            "encoding": "utf8",
        });
        self.send(
            Tag::FileSave {
                server: server.clone(),
            },
            "PUT",
            format!("/api/v1/servers/{server}/files/content"),
            Some(body),
        );
    }

    fn open_file(&mut self, path: String) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        self.send(
            Tag::FileRead {
                server: server.clone(),
                path: path.clone(),
            },
            "GET",
            format!(
                "/api/v1/servers/{server}/files/content?path={}",
                encode_path(&path)
            ),
            None,
        );
    }

    fn file_mkdir(&mut self) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        let name = self.files.new_dir.trim();
        if name.is_empty() {
            return;
        }
        let path = resolve_path(&self.files.path, name);
        self.send(
            Tag::FileMkdir {
                server: server.clone(),
            },
            "POST",
            format!("/api/v1/servers/{server}/files/mkdir"),
            Some(json!({ "path": path })),
        );
    }

    fn file_rename(&mut self) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        let Some(from) = self.files.selected.clone() else {
            self.toast_error("Select a file or directory first");
            return;
        };
        let target = self.files.rename_to.trim();
        if target.is_empty() {
            return;
        }
        let to = resolve_path(&self.files.path, target);
        self.send(
            Tag::FileRename {
                server: server.clone(),
            },
            "POST",
            format!("/api/v1/servers/{server}/files/rename"),
            Some(json!({ "from": from, "to": to })),
        );
    }

    fn file_chmod(&mut self) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        let Some(path) = self.files.selected.clone() else {
            self.toast_error("Select a file or directory first");
            return;
        };
        let mode = self.files.chmod_mode.trim();
        if mode.is_empty() {
            return;
        }
        self.send(
            Tag::FileChmod {
                server: server.clone(),
            },
            "POST",
            format!("/api/v1/servers/{server}/files/chmod"),
            Some(json!({ "path": path, "mode": mode })),
        );
    }

    fn file_compress(&mut self, decompress: bool) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        let Some(path) = self.files.selected.clone() else {
            self.toast_error("Select a file or directory first");
            return;
        };
        let action = if decompress { "decompress" } else { "compress" };
        let tag = if decompress {
            Tag::FileDecompress {
                server: server.clone(),
            }
        } else {
            Tag::FileCompress {
                server: server.clone(),
            }
        };
        self.send(
            tag,
            "POST",
            format!("/api/v1/servers/{server}/files/{action}"),
            Some(json!({ "path": path })),
        );
    }

    fn create_server(&mut self) {
        self.create_server.error = None;
        let Some(node_id) = self.create_server.node_id.clone() else {
            self.create_server.error = Some("Select a node".to_string());
            return;
        };
        let Some(template_id) = self.create_server.template_id.clone() else {
            self.create_server.error = Some("Select a template".to_string());
            return;
        };
        let name = self.create_server.name.trim().to_string();
        if name.is_empty() {
            self.create_server.error = Some("Name is required".to_string());
            return;
        }
        let memory = match parse_u64(&self.create_server.memory, "memory MB") {
            Ok(value) => value,
            Err(error) => {
                self.create_server.error = Some(error);
                return;
            }
        };
        let disk = match parse_u64(&self.create_server.disk, "disk MB") {
            Ok(value) => value,
            Err(error) => {
                self.create_server.error = Some(error);
                return;
            }
        };
        let cpu = match parse_u64(&self.create_server.cpu, "CPU millis") {
            Ok(value) => value,
            Err(error) => {
                self.create_server.error = Some(error);
                return;
            }
        };
        let auto_allocations = match self.create_server.auto_allocations.trim().parse::<u16>() {
            Ok(value) if value > 0 => value,
            _ => {
                self.create_server.error = Some("Auto allocations must be at least 1".to_string());
                return;
            }
        };
        let variables = match parse_variables(&self.create_server.variables) {
            Ok(variables) => variables,
            Err(error) => {
                self.create_server.error = Some(error);
                return;
            }
        };

        let mut body = json!({
            "node_id": node_id,
            "template_id": template_id,
            "name": name,
            "limits": {
                "memory_mb": memory,
                "disk_mb": disk,
                "cpu_millis": cpu,
                "swap_mb": 0,
                "io_weight": 500,
                "pids_limit": 512,
            },
            "auto_allocations": auto_allocations,
            "variables": variables,
        });
        if !self.create_server.description.trim().is_empty() {
            body["description"] = json!(self.create_server.description.trim());
        }
        if !self.create_server.external_ref.trim().is_empty() {
            body["external_ref"] = json!(self.create_server.external_ref.trim());
        }
        if !self.create_server.idempotency_key.trim().is_empty() {
            body["idempotency_key"] = json!(self.create_server.idempotency_key.trim());
        }
        if !self.create_server.template_version.trim().is_empty() {
            body["template_version"] = json!(self.create_server.template_version.trim());
        }
        if !self.create_server.protocol.trim().is_empty() {
            body["protocol"] = json!(self.create_server.protocol.trim());
        }
        if !self.create_server.image.trim().is_empty() {
            body["image"] = json!(self.create_server.image.trim());
        }
        let labels = match parse_json_object(&self.create_server.labels, "Labels") {
            Ok(value) => clean_object(value),
            Err(error) => {
                self.create_server.error = Some(error);
                return;
            }
        };
        if labels
            .as_object()
            .map(|object| !object.is_empty())
            .unwrap_or(false)
        {
            body["labels"] = labels;
        }
        self.send(
            Tag::ServerCreate,
            "POST",
            "/api/v1/servers".to_string(),
            Some(body),
        );
    }

    fn save_limits(&mut self) {
        self.edit_limits.error = None;
        let Some(server) = self.selected_server.clone() else {
            self.edit_limits.error = Some("Select a server first".to_string());
            return;
        };
        let memory = match parse_u64(&self.edit_limits.memory, "memory MB") {
            Ok(value) => value,
            Err(error) => {
                self.edit_limits.error = Some(error);
                return;
            }
        };
        let disk = match parse_u64(&self.edit_limits.disk, "disk MB") {
            Ok(value) => value,
            Err(error) => {
                self.edit_limits.error = Some(error);
                return;
            }
        };
        let cpu = match parse_u64(&self.edit_limits.cpu, "CPU millis") {
            Ok(value) => value,
            Err(error) => {
                self.edit_limits.error = Some(error);
                return;
            }
        };
        let swap = match parse_u64(&self.edit_limits.swap, "swap MB") {
            Ok(value) => value,
            Err(error) => {
                self.edit_limits.error = Some(error);
                return;
            }
        };
        let io_weight = match self.edit_limits.io_weight.trim().parse::<u16>() {
            Ok(value) if value == 0 || (10..=1000).contains(&value) => value,
            _ => {
                self.edit_limits.error =
                    Some("I/O weight must be 0 (unlimited) or between 10 and 1000".to_string());
                return;
            }
        };
        let pids_limit = match self.edit_limits.pids_limit.trim().parse::<u32>() {
            Ok(value) => value,
            Err(_) => {
                self.edit_limits.error = Some("PIDs limit must be a number".to_string());
                return;
            }
        };
        let cpuset = self.edit_limits.cpuset.trim();
        let cpuset = (!cpuset.is_empty()).then(|| cpuset.to_string());
        let mut limits = json!({
            "memory_mb": memory,
            "disk_mb": disk,
            "cpu_millis": cpu,
            "swap_mb": swap,
            "io_weight": io_weight,
            "pids_limit": pids_limit,
        });
        if let Some(cpuset) = cpuset {
            limits["cpuset"] = json!(cpuset);
        }
        self.send(
            Tag::ServerUpdate {
                server: server.clone(),
            },
            "PATCH",
            format!("/api/v1/servers/{server}"),
            Some(json!({ "limits": limits })),
        );
    }

    fn save_server_config(&mut self) {
        self.server_config.error = None;
        let Some(server) = self.selected_server.clone() else {
            self.server_config.error = Some("Select a server first".to_string());
            return;
        };
        let labels = match parse_json_object(&self.server_config.labels, "Labels") {
            Ok(value) => clean_object(value),
            Err(error) => {
                self.server_config.error = Some(error);
                return;
            }
        };
        let mut body = json!({
            "name": self.server_config.name.trim(),
            "description": self.server_config.description.trim(),
            "external_ref": self.server_config.external_ref.trim(),
            "labels": labels,
        });
        if !self.server_config.image.trim().is_empty() {
            body["image"] = json!(self.server_config.image.trim());
        }
        if !self.server_config.template_version.trim().is_empty() {
            body["template_version"] = json!(self.server_config.template_version.trim());
        }
        let variables = match parse_json_object(&self.server_config.variables, "Variables") {
            Ok(value) => clean_object(value),
            Err(error) => {
                self.server_config.error = Some(error);
                return;
            }
        };
        if variables.as_object().is_some_and(|map| !map.is_empty()) {
            body["variables"] = variables;
        }
        self.send(
            Tag::ServerUpdate {
                server: server.clone(),
            },
            "PATCH",
            format!("/api/v1/servers/{server}"),
            Some(body),
        );
    }

    fn schedule_push(&mut self, value: Value) {
        let mut actions =
            serde_json::from_str::<Vec<Value>>(&self.schedule_form.actions).unwrap_or_default();
        actions.push(value);
        self.schedule_form.actions = serde_json::to_string_pretty(&actions)
            .unwrap_or_else(|_| self.schedule_form.actions.clone());
    }

    fn schedule_new(&mut self) {
        let server = self.selected_server.clone();
        self.schedule_form = ScheduleForm::new();
        self.schedule_form.server = server;
    }

    fn schedule_edit(&mut self, id: &str) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        let Some(value) = self.feature(&format!("schedules:{server}")) else {
            return;
        };
        let Some(schedule) = value
            .get("schedules")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
            })
        else {
            return;
        };
        self.schedule_form.server = Some(server);
        self.schedule_form.id = Some(id.to_string());
        self.schedule_form.name = schedule
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.schedule_form.cron = schedule
            .get("cron")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.schedule_form.actions = schedule
            .get("actions")
            .map(|value| serde_json::to_string_pretty(value).unwrap_or_else(|_| "[]".into()))
            .unwrap_or_else(|| "[]".into());
        self.schedule_form.enabled = schedule
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        self.schedule_form.error = None;
    }

    fn schedule_save(&mut self) {
        self.schedule_form.error = None;
        let Some(server) = self.selected_server.clone() else {
            self.schedule_form.error = Some("Select a server first".to_string());
            return;
        };
        let actions = match serde_json::from_str::<Value>(&self.schedule_form.actions) {
            Ok(value) if value.is_array() => value,
            Ok(_) => {
                self.schedule_form.error = Some("Actions must be a JSON array".to_string());
                return;
            }
            Err(error) => {
                self.schedule_form.error = Some(format!("Actions JSON: {error}"));
                return;
            }
        };
        let body = json!({
            "name": self.schedule_form.name.trim(),
            "cron": self.schedule_form.cron.trim(),
            "actions": actions,
            "enabled": self.schedule_form.enabled,
        });
        let key = format!("schedules:{server}");
        let path = format!("/api/v1/servers/{server}/schedules");
        if let Some(id) = self.schedule_form.id.clone() {
            self.mutate_feature(
                "schedule",
                &key,
                Some(&key),
                "PATCH",
                format!("/api/v1/schedules/{id}"),
                Some(body),
                "Schedule updated",
                None,
            );
        } else {
            self.mutate_feature(
                "schedule",
                &key,
                Some(&key),
                "POST",
                path,
                Some(body),
                "Schedule created",
                None,
            );
        }
        self.schedule_form = ScheduleForm::new();
        self.schedule_form.server = Some(server);
    }

    fn schedule_run(&mut self, id: &str) {
        self.mutate_feature(
            "schedule",
            "",
            None,
            "POST",
            format!("/api/v1/schedules/{id}/run"),
            Some(json!({})),
            "Schedule queued",
            None,
        );
    }

    fn create_sftp(&mut self) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        let mut body = Map::new();
        if let Ok(ttl) = self.sftp_ttl.trim().parse::<i64>() {
            body.insert("ttl_seconds".to_string(), json!(ttl));
        }
        let key = format!("sftp:{server}");
        self.mutate_feature(
            "sftp_create",
            &key,
            Some(&key),
            "POST",
            format!("/api/v1/servers/{server}/sftp"),
            Some(Value::Object(body)),
            "SFTP credential created",
            Some("sftp_created"),
        );
    }

    fn create_api_key(&mut self) {
        self.api_key_form.error = None;
        let name = self.api_key_form.name.trim().to_string();
        if name.is_empty() {
            self.api_key_form.error = Some("Name is required".to_string());
            return;
        }
        let scopes: Vec<String> = self
            .api_key_form
            .scopes
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect();
        if scopes.is_empty() {
            self.api_key_form.error = Some("At least one scope is required".to_string());
            return;
        }
        let server_ids: Vec<String> = self
            .api_key_form
            .server_ids
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect();
        let mut body = json!({ "name": name, "scopes": scopes, "server_ids": server_ids });
        if let Ok(days) = self.api_key_form.days.trim().parse::<i64>() {
            if days > 0 {
                body["expires_at"] =
                    json!((chrono::Utc::now() + chrono::Duration::days(days)).to_rfc3339());
            }
        }
        self.mutate_feature(
            "api_key_create",
            "api_keys",
            Some("/api/v1/api-keys"),
            "POST",
            "/api/v1/api-keys".to_string(),
            Some(body),
            "API key created",
            Some("api_key_created"),
        );
        self.api_key_form.name.clear();
        self.api_key_form.server_ids.clear();
        self.api_key_form.days.clear();
    }

    fn send_notification(&mut self) {
        self.notification_form.error = None;
        let to: Vec<String> = self
            .notification_form
            .to
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect();
        let body = json!({ "to": to, "subject": self.notification_form.subject.trim(), "body": self.notification_form.body });
        self.mutate_feature(
            "notification_send",
            "notifications",
            Some("/api/v1/notifications/emails?limit=200"),
            "POST",
            "/api/v1/notifications/email".to_string(),
            Some(body),
            "Email queued",
            None,
        );
        self.notification_form.subject.clear();
        self.notification_form.body.clear();
    }

    fn retry_notification(&mut self, id: &str) {
        self.mutate_feature(
            "notification_retry",
            "notifications",
            Some("/api/v1/notifications/emails?limit=200"),
            "POST",
            format!("/api/v1/notifications/emails/{id}/retry"),
            Some(json!({})),
            "Email retry queued",
            None,
        );
    }

    fn save_node(&mut self) {
        self.node_form.error = None;
        let Some(id) = self.node_form.id.clone() else {
            self.node_form.error = Some("Select a node first".to_string());
            return;
        };
        let labels = match parse_json_object(&self.node_form.labels, "Labels") {
            Ok(value) => clean_object(value),
            Err(error) => {
                self.node_form.error = Some(error);
                return;
            }
        };
        let body = json!({
            "name": self.node_form.name.trim(),
            "fqdn": self.node_form.fqdn.trim(),
            "endpoint": self.node_form.endpoint.trim(),
            "driver": self.node_form.driver,
            "maintenance": self.node_form.maintenance,
            "labels": labels,
        });
        self.send(
            Tag::NodeUpdate { id: id.clone() },
            "PATCH",
            format!("/api/v1/nodes/{id}"),
            Some(body),
        );
    }

    fn rotate_node(&mut self) {
        let Some(node) = self.selected_node.clone() else {
            return;
        };
        self.send(
            Tag::NodeRotate { id: node.clone() },
            "POST",
            format!("/api/v1/nodes/{node}/rotate-token"),
            Some(json!({})),
        );
    }

    fn ask_delete_node(&mut self) {
        let Some(node) = self.selected_node.clone() else {
            return;
        };
        let Some(record) = self.nodes.iter().find(|item| item.id.to_string() == node) else {
            return;
        };
        self.confirm = Some(ConfirmAction::DeleteNode {
            id: node,
            name: record.name.clone(),
        });
    }

    fn check_node_updates(&mut self) {
        let Some(node) = self.selected_node.clone() else {
            return;
        };
        self.send(
            Tag::NodeUpdatesCheck { id: node.clone() },
            "POST",
            format!("/api/v1/nodes/{node}/updates/check"),
            Some(json!({})),
        );
    }

    fn apply_node_updates(&mut self, reboot: bool) {
        let Some(node) = self.selected_node.clone() else {
            return;
        };
        self.send(
            Tag::NodeUpdatesApply {
                id: node.clone(),
                reboot,
            },
            "POST",
            format!("/api/v1/nodes/{node}/updates/apply"),
            Some(json!({ "reboot": reboot })),
        );
    }

    fn reboot_node(&mut self) {
        let Some(node) = self.selected_node.clone() else {
            return;
        };
        self.send(
            Tag::NodeReboot { id: node.clone() },
            "POST",
            format!("/api/v1/nodes/{node}/reboot"),
            Some(json!({})),
        );
    }

    fn edit_publish_template(&mut self) {
        if let Some(record) = &self.template_detail {
            self.publish_form.load(record);
        }
    }

    fn publish_template(&mut self) {
        self.publish_form.error = None;
        let Some(id) = self.publish_form.template_id.clone() else {
            self.publish_form.error = Some("Select a template first".to_string());
            return;
        };
        let template = match serde_json::from_str::<Value>(&self.publish_form.template) {
            Ok(value) => value,
            Err(error) => {
                self.publish_form.error = Some(format!("Template JSON: {error}"));
                return;
            }
        };
        let mut body = json!({
            "template": template,
            "channel": self.publish_form.channel,
            "notes": self.publish_form.notes.trim(),
        });
        if !self.publish_form.signature.trim().is_empty() {
            body["signature"] = json!(self.publish_form.signature.trim());
        }
        if !self.publish_form.public_key.trim().is_empty() {
            body["public_key"] = json!(self.publish_form.public_key.trim());
        }
        self.send(
            Tag::TemplatePublish { id: id.clone() },
            "POST",
            format!("/api/v1/templates/{id}/versions"),
            Some(body),
        );
    }

    fn update_template_auto(&mut self) {
        let Some(id) = self.selected_template.clone() else {
            return;
        };
        self.send(
            Tag::TemplateAutoUpdate { id: id.clone() },
            "PATCH",
            format!("/api/v1/templates/{id}"),
            Some(json!({ "auto_update": { "enabled": self.template_auto_enabled, "channel": self.template_auto_channel } })),
        );
    }

    fn sync_template_feed(&mut self) {
        self.feed_form.error = None;
        let feed = match serde_json::from_str::<Value>(&self.feed_form.feed) {
            Ok(value) => value,
            Err(error) => {
                self.feed_form.error = Some(format!("Feed JSON: {error}"));
                return;
            }
        };
        let body = json!({
            "feed": feed,
            "signature": self.feed_form.signature.trim(),
            "public_key": self.feed_form.public_key.trim(),
            "create_missing": self.feed_form.create_missing,
        });
        self.send(
            Tag::TemplateFeed,
            "POST",
            "/api/v1/template-feeds".to_string(),
            Some(body),
        );
    }

    fn proxy_new(&mut self) {
        self.proxy_edit = ProxyEditForm::new();
    }

    fn proxy_edit_route(&mut self, id: &str) {
        let Some(route) = self
            .proxy_routes
            .iter()
            .find(|route| route_field(route, "id").and_then(Value::as_str) == Some(id))
        else {
            return;
        };
        let mut form = ProxyEditForm::new();
        form.id = Some(id.to_string());
        form.domain = route_field(route, "domain")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        form.upstream = route_field(route, "upstream")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        form.tls = route_field(route, "tls")
            .map(|value| value.as_str().unwrap_or("auto").to_string())
            .unwrap_or_else(|| "auto".to_string());
        form.enabled = route_field(route, "enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        self.proxy_edit = form;
    }

    fn save_proxy_route(&mut self) {
        let domain = self.proxy_edit.domain.trim().to_string();
        let upstream = self.proxy_edit.upstream.trim().to_string();
        if domain.is_empty() || upstream.is_empty() {
            self.proxy_edit.error = Some("Domain and upstream are required".to_string());
            return;
        }
        let body = json!({
            "domain": domain,
            "upstream": upstream,
            "tls": self.proxy_edit.tls,
            "enabled": self.proxy_edit.enabled,
        });
        if let Some(id) = self.proxy_edit.id.clone() {
            self.send(
                Tag::ProxyUpdate { id },
                "PATCH",
                format!(
                    "/api/v1/proxy/routes/{}",
                    self.proxy_edit.id.clone().unwrap_or_default()
                ),
                Some(body),
            );
        } else {
            self.send(
                Tag::ProxyCreate,
                "POST",
                "/api/v1/proxy/routes".to_string(),
                Some(body),
            );
        }
        self.proxy_edit = ProxyEditForm::new();
    }

    fn reconcile_proxy(&mut self) {
        self.send(
            Tag::ProxyReconcile,
            "POST",
            "/api/v1/proxy/reconcile".to_string(),
            None,
        );
    }

    fn attach_allocation(&mut self) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        let Some(allocation) = self.attach_allocation.clone() else {
            self.toast_error("Select a free allocation first");
            return;
        };
        self.send(
            Tag::AllocationAttach {
                server: server.clone(),
                allocation: allocation.clone(),
            },
            "POST",
            format!("/api/v1/servers/{server}/allocations"),
            Some(json!({ "allocation_id": allocation })),
        );
    }

    fn detach_allocation(&mut self, allocation: &str) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        self.send(
            Tag::AllocationDetach {
                server: server.clone(),
                allocation: allocation.to_string(),
            },
            "DELETE",
            format!("/api/v1/servers/{server}/allocations/{allocation}"),
            None,
        );
    }

    fn bulk_power(&mut self) {
        let ids: Vec<String> = self.bulk_selected.iter().cloned().collect();
        if ids.is_empty() {
            self.toast_error("Select at least one server");
            return;
        }
        self.send(
            Tag::BulkPower,
            "POST",
            "/api/v1/servers/bulk/power".to_string(),
            Some(json!({ "server_ids": ids, "signal": self.bulk_signal })),
        );
    }

    fn bulk_command(&mut self) {
        let ids: Vec<String> = self.bulk_selected.iter().cloned().collect();
        if ids.is_empty() {
            self.toast_error("Select at least one server");
            return;
        }
        let command = self.bulk_command.trim().to_string();
        if command.is_empty() {
            self.toast_error("Enter a command");
            return;
        }
        self.send(
            Tag::BulkCommand,
            "POST",
            "/api/v1/servers/bulk/command".to_string(),
            Some(json!({ "server_ids": ids, "command": command })),
        );
        self.bulk_command.clear();
    }

    fn create_node(&mut self) {
        self.create_node.error = None;
        let name = self.create_node.name.trim().to_string();
        let fqdn = self.create_node.fqdn.trim().to_string();
        let endpoint = self.create_node.endpoint.trim().to_string();
        if name.is_empty() || fqdn.is_empty() || endpoint.is_empty() {
            self.create_node.error = Some("Name, FQDN and endpoint are all required".to_string());
            return;
        }
        let memory = match parse_u64(&self.create_node.memory, "memory MB") {
            Ok(value) => value,
            Err(error) => {
                self.create_node.error = Some(error);
                return;
            }
        };
        let disk = match parse_u64(&self.create_node.disk, "disk MB") {
            Ok(value) => value,
            Err(error) => {
                self.create_node.error = Some(error);
                return;
            }
        };
        let cpu = match parse_u64(&self.create_node.cpu, "CPU millis") {
            Ok(value) => value,
            Err(error) => {
                self.create_node.error = Some(error);
                return;
            }
        };
        let labels = match parse_json_object(&self.create_node.labels, "Labels") {
            Ok(value) => clean_object(value),
            Err(error) => {
                self.create_node.error = Some(error);
                return;
            }
        };
        let mut body = json!({
            "name": name,
            "fqdn": fqdn,
            "endpoint": endpoint,
            "driver": self.create_node.driver,
            "maintenance": self.create_node.maintenance,
            "labels": labels,
            "capacity": {
                "memory_mb": memory,
                "disk_mb": disk,
                "cpu_millis": cpu,
            },
        });
        if !self.create_node.agent_token.trim().is_empty() {
            body["agent_token"] = json!(self.create_node.agent_token.trim());
        }
        self.send(
            Tag::NodeCreate,
            "POST",
            "/api/v1/nodes".to_string(),
            Some(body),
        );
    }

    fn create_allocation(&mut self) {
        self.create_alloc.error = None;
        let Some(node) = self.selected_node.clone() else {
            self.create_alloc.error = Some("Select a node first".to_string());
            return;
        };
        if self.create_alloc.ports.trim().is_empty() {
            self.create_alloc.error = Some("Enter a port range".to_string());
            return;
        }
        let mut body = Map::new();
        if !self.create_alloc.ip.trim().is_empty() {
            body.insert("ip".to_string(), json!(self.create_alloc.ip.trim()));
        }
        if !self.create_alloc.ports.trim().is_empty() {
            body.insert("ports".to_string(), json!(self.create_alloc.ports.trim()));
        }
        body.insert("protocol".to_string(), json!(self.create_alloc.protocol));
        if !self.create_alloc.label.trim().is_empty() {
            body.insert("label".to_string(), json!(self.create_alloc.label.trim()));
        }
        self.send(
            Tag::AllocationCreate { node: node.clone() },
            "POST",
            format!("/api/v1/nodes/{node}/allocations"),
            Some(Value::Object(body)),
        );
    }

    fn import_egg(&mut self, save: bool) {
        if self.egg_text.trim().is_empty() {
            self.toast_error("Paste a Pterodactyl egg JSON document first");
            return;
        }
        let egg: Value = match serde_json::from_str(&self.egg_text) {
            Ok(value) => value,
            Err(error) => {
                self.toast_error(format!("Invalid egg JSON: {error}"));
                return;
            }
        };
        let version = self.egg_version.trim().to_string();
        let version = if version.is_empty() {
            egg.pointer("/meta/version")
                .and_then(Value::as_str)
                .filter(|value| looks_like_semver(value))
                .unwrap_or_default()
                .to_string()
        } else {
            version
        };
        if version.is_empty() {
            self.toast_error(
                "Enter a template version, or set a semver meta.version in the template JSON",
            );
            return;
        }
        let mut body =
            json!({ "egg": egg, "version": version, "save": save, "channel": self.egg_channel });
        if !self.egg_slug.trim().is_empty() {
            body["slug"] = json!(self.egg_slug.trim());
        }
        if !self.egg_signature.trim().is_empty() {
            body["signature"] = json!(self.egg_signature.trim());
        }
        if !self.egg_public_key.trim().is_empty() {
            body["public_key"] = json!(self.egg_public_key.trim());
        }
        self.send(
            Tag::TemplateImport,
            "POST",
            "/api/v1/templates".to_string(),
            Some(body),
        );
    }

    fn show_db_host(&mut self, id: &str) {
        self.db_host_view = Some(id.to_string());
        self.refresh_feature(
            &format!("db_host:{id}"),
            &format!("/api/v1/database-hosts/{id}"),
        );
    }

    fn create_db_host(&mut self) {
        self.create_db_host.error = None;
        let name = self.create_db_host.name.trim().to_string();
        let host = self.create_db_host.host.trim().to_string();
        let admin_user = self.create_db_host.admin_user.trim().to_string();
        let admin_password = self.create_db_host.admin_password.clone();
        if name.is_empty() || host.is_empty() || admin_user.is_empty() {
            self.create_db_host.error = Some("Name, host and admin user are required".to_string());
            return;
        }
        if admin_password.is_empty() {
            self.create_db_host.error = Some("Admin password is required".to_string());
            return;
        }
        let port = match self.create_db_host.port.trim().parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => {
                self.create_db_host.error = Some("Port must be 1-65535".to_string());
                return;
            }
        };
        let max_databases = self
            .create_db_host
            .max_databases
            .trim()
            .parse::<u32>()
            .unwrap_or(0);
        let body = json!({
            "name": name,
            "host": host,
            "port": port,
            "admin_user": admin_user,
            "admin_password": admin_password,
            "max_databases": max_databases,
        });
        self.send(
            Tag::DbHostCreate,
            "POST",
            "/api/v1/database-hosts".to_string(),
            Some(body),
        );
    }

    fn create_server_db(&mut self) {
        self.create_server_db.error = None;
        let Some(server) = self.selected_server.clone() else {
            self.create_server_db.error = Some("Select a server first".to_string());
            return;
        };
        let mut body = Map::new();
        if let Some(host_id) = &self.create_server_db.host_id {
            body.insert("host_id".to_string(), json!(host_id));
        }
        if !self.create_server_db.remote.trim().is_empty() {
            body.insert(
                "remote".to_string(),
                json!(self.create_server_db.remote.trim()),
            );
        }
        self.send(
            Tag::DbCreate {
                server: server.clone(),
            },
            "POST",
            format!("/api/v1/servers/{server}/databases"),
            Some(Value::Object(body)),
        );
    }

    fn download_backup(&mut self, id: &str) {
        let Some(server) = self.selected_server.clone() else {
            return;
        };
        let name = self
            .backups
            .iter()
            .find(|backup| backup.id.to_string() == id)
            .map(|backup| sanitize_filename(&backup.name))
            .unwrap_or_else(|| "backup".to_string());
        let directory = PathBuf::from(self.download_dir.trim());
        let destination = directory.join(format!("{}-{}.tar.gz", name, short_id(id)));
        self.download(
            Tag::BackupDownload {
                server: server.clone(),
                id: id.to_string(),
            },
            format!("/api/v1/servers/{server}/backups/{id}/download"),
            destination,
        );
    }

    fn send_raw(&mut self) {
        self.raw_error = None;
        let path = self.raw_path.trim().to_string();
        if !path.starts_with("/api/v1") {
            self.raw_error = Some("Path must start with /api/v1".to_string());
            return;
        }
        let body = if self.raw_body.trim().is_empty() {
            None
        } else {
            match serde_json::from_str::<Value>(&self.raw_body) {
                Ok(value) => Some(value),
                Err(error) => {
                    self.raw_error = Some(format!("Body is not valid JSON: {error}"));
                    return;
                }
            }
        };
        let method = self.raw_method.clone();
        self.send(Tag::Raw, &method, path, body);
    }

    fn clear_console(&mut self) {
        if let Some(server) = self.selected_server.clone() {
            let log = self.console_logs.entry(server).or_default();
            log.clear_hint = Some(log.lines.clone());
            log.lines.clear();
        }
    }

    fn save_prefs(&mut self) {
        let saved = SavedConfig {
            api_url: Some(self.url_input.trim().to_string()),
            api_key: if self.remember {
                Some(self.key_input.clone())
            } else {
                None
            },
            remember: self.remember,
            last_tab: Some(self.tab.as_str().to_string()),
            last_server: self.selected_server.clone(),
        };
        if let Err(error) = config::save(&saved) {
            if self.config_warning.is_none() {
                self.config_warning = Some(format!("Could not save settings: {error}"));
            }
        }
        self.prefs_dirty = false;
        self.last_prefs_save = Instant::now();
    }

    fn maybe_save_prefs(&mut self) {
        if self.prefs_dirty && self.last_prefs_save.elapsed() > Duration::from_secs(2) {
            self.save_prefs();
        }
    }

    // ------------------------------------------------------------------ ui

    fn update_ui(&mut self, ctx: &egui::Context) {
        self.handle_api_results();
        self.handle_console_events();

        let mut actions = Vec::new();
        self.render_top_bar(ctx, &mut actions);
        self.render_status_bar(ctx, &mut actions);

        if self.conn.is_some() {
            self.render_nav(ctx, &mut actions);
            egui::CentralPanel::default().show(ctx, |ui| {
                if self.tab == Tab::Console {
                    self.page_console(ui, &mut actions);
                } else {
                    ScrollArea::vertical()
                        .id_salt(("page_scroll", self.tab))
                        .auto_shrink([false, false])
                        .show(ui, |ui| match self.tab {
                            Tab::Overview => self.page_overview(ui, &mut actions),
                            Tab::Servers => self.page_servers(ui, &mut actions),
                            Tab::Files => self.page_files(ui, &mut actions),
                            Tab::Config => self.page_config(ui, &mut actions),
                            Tab::Schedules => self.page_schedules(ui, &mut actions),
                            Tab::Sftp => self.page_sftp(ui, &mut actions),
                            Tab::Environment => self.page_environment(ui, &mut actions),
                            Tab::Jobs => self.page_jobs(ui, &mut actions),
                            Tab::Nodes => self.page_nodes(ui, &mut actions),
                            Tab::Templates => self.page_templates(ui, &mut actions),
                            Tab::Databases => self.page_databases(ui, &mut actions),
                            Tab::Notifications => self.page_notifications(ui, &mut actions),
                            Tab::ApiKeys => self.page_api_keys(ui, &mut actions),
                            Tab::Backups => self.page_backups(ui, &mut actions),
                            Tab::Audit => self.page_audit(ui, &mut actions),
                            Tab::Proxy => self.page_proxy(ui, &mut actions),
                            Tab::Metrics => self.page_metrics(ui, &mut actions),
                            Tab::Raw => self.page_raw(ui, &mut actions),
                            Tab::Console => {}
                        });
                }
            });
        } else {
            self.render_connection(ctx, &mut actions);
        }

        self.render_confirm(ctx, &mut actions);
        self.apply_actions(actions);
        self.sync_console_poller();

        if self.tab == Tab::Console
            && self.conn.is_some()
            && self.servers_last_refresh.elapsed() > Duration::from_secs(5)
        {
            self.refresh_servers();
        }
        if self.inflight > 0 || self.connecting {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.toast.is_some() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        self.maybe_save_prefs();
    }

    fn render_top_bar(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading(RichText::new("Ignite Containers").strong());
                ui.separator();
                if self.connecting {
                    ui.spinner();
                    ui.label("Connecting...");
                } else if let Some(conn) = &self.conn {
                    let color = if self.me.is_some() {
                        Color32::from_rgb(95, 200, 120)
                    } else {
                        Color32::from_rgb(220, 180, 80)
                    };
                    ui.colored_label(color, "Connected");
                    truncate_label(ui, &conn.base_url, 340.0);
                } else {
                    ui.colored_label(Color32::from_rgb(220, 100, 100), "Not connected");
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.conn.is_some() {
                        if ui.button("Disconnect").clicked() {
                            actions.push(Action::Disconnect);
                        }
                        if ui.button("Refresh").clicked() {
                            actions.push(Action::RefreshAll);
                        }
                    }
                    if self.inflight > 0 {
                        ui.label(format!("{} in flight", self.inflight));
                    }
                });
            });
            ui.add_space(2.0);
        });
    }

    fn render_status_bar(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| toast.at.elapsed() > Duration::from_secs(30))
        {
            self.toast = None;
        }
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(toast) = &self.toast {
                    let (color, icon) = match toast.kind {
                        StatusKind::Info => (Color32::from_rgb(130, 170, 220), "info"),
                        StatusKind::Success => (Color32::from_rgb(110, 210, 130), "ok"),
                        StatusKind::Error => (Color32::from_rgb(235, 120, 110), "error"),
                    };
                    ui.colored_label(color, icon);
                    ui.add(egui::Label::new(RichText::new(&toast.text).color(color)).wrap());
                    if ui.small_button("Dismiss").clicked() {
                        actions.push(Action::DismissToast);
                    }
                } else if let Some(warning) = &self.config_warning {
                    ui.colored_label(Color32::from_rgb(230, 180, 90), warning);
                } else {
                    ui.label(
                        RichText::new("Ready")
                            .small()
                            .color(Color32::from_rgb(140, 145, 155)),
                    );
                }
            });
        });
    }

    fn render_nav(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        egui::SidePanel::left("nav")
            .exact_width(178.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                let mut group = "";
                for tab in Tab::ALL {
                    if tab.group() != group {
                        group = tab.group();
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(group.to_uppercase())
                                .small()
                                .color(Color32::from_rgb(110, 120, 135)),
                        );
                        ui.add_space(2.0);
                    }
                    let selected = self.tab == tab;
                    if ui
                        .add_sized(
                            [ui.available_width(), 28.0],
                            egui::SelectableLabel::new(selected, tab.label()),
                        )
                        .clicked()
                    {
                        actions.push(Action::SwitchTab(tab));
                    }
                }
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.add_space(8.0);
                    if let Some(me) = &self.me {
                        ui.label(
                            RichText::new(format!("Key: {}", me.name))
                                .small()
                                .color(Color32::from_rgb(140, 145, 155)),
                        );
                    }
                    ui.label(
                        RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                            .small()
                            .color(Color32::from_rgb(110, 115, 125)),
                    );
                });
            });
    }

    fn render_connection(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ScrollArea::vertical()
                .id_salt("connection_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(48.0);
                        ui.heading(RichText::new(APP_TITLE).size(26.0).strong());
                        ui.label(
                            RichText::new(
                                "Connect to an Ignite Containers control plane to manage nodes, servers and the live console.",
                            )
                            .color(Color32::from_rgb(170, 176, 188)),
                        );
                        ui.add_space(22.0);
                    });
                    ui.horizontal(|ui| {
                        let available = ui.available_width();
                        let margin = ((available - 560.0) / 2.0).max(0.0);
                        ui.add_space(margin);
                        ui.vertical(|ui| {
                            ui.set_max_width(560.0);
                            egui::Frame::group(ui.style())
                                .inner_margin(egui::Margin::same(18))
                                .show(ui, |ui| {
                                    egui::Grid::new("connection_grid")
                                        .num_columns(2)
                                        .spacing([14.0, 10.0])
                                        .show(ui, |ui| {
                                            ui.label("API URL");
                                            ui.add(
                                                TextEdit::singleline(&mut self.url_input)
                                                    .hint_text(DEFAULT_URL)
                                                    .desired_width(400.0),
                                            );
                                            ui.end_row();

                                            ui.label("API key");
                                            ui.horizontal(|ui| {
                                                ui.add(
                                                    TextEdit::singleline(&mut self.key_input)
                                                        .password(!self.show_key)
                                                        .hint_text("ic_...")
                                                        .desired_width(330.0),
                                                );
                                                ui.checkbox(&mut self.show_key, "Show");
                                            });
                                            ui.end_row();
                                        });
                                    ui.add_space(4.0);
                                    ui.checkbox(
                                        &mut self.remember,
                                        "Remember settings (API key stored in plain text, mode 0600)",
                                    );
                                    ui.add_space(8.0);
                                    ui.horizontal(|ui| {
                                        let button = egui::Button::new(
                                            RichText::new("Connect").strong(),
                                        )
                                        .min_size(Vec2::new(110.0, 30.0));
                                        if ui
                                            .add_enabled(!self.connecting, button)
                                            .clicked()
                                        {
                                            actions.push(Action::Connect);
                                        }
                                        if self.connecting {
                                            ui.spinner();
                                            ui.label("Checking /api/v1/me ...");
                                        }
                                    });
                                    if let Some(error) = &self.login_error {
                                        ui.add_space(6.0);
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(error)
                                                    .color(Color32::from_rgb(235, 120, 110)),
                                            )
                                            .wrap(),
                                        );
                                    }
                                    if let Some(note) = &self.config_note {
                                        ui.add_space(6.0);
                                        ui.label(
                                            RichText::new(note)
                                                .small()
                                                .color(Color32::from_rgb(130, 136, 148)),
                                        );
                                    }
                                    if let Some(warning) = &self.config_warning {
                                        ui.add_space(6.0);
                                        ui.label(
                                            RichText::new(warning).small().color(
                                                Color32::from_rgb(230, 180, 90),
                                            ),
                                        );
                                    }
                                });
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new(
                                    "Environment: IGNITE_CONTAINERS_API_URL / IGNITE_CONTAINERS_API_KEY\n\
                                     CLI: --api-url http://127.0.0.1:8080 --api-key ic_...",
                                )
                                .small()
                                .color(Color32::from_rgb(120, 126, 138)),
                            );
                        });
                    });
                });
        });
    }

    fn render_confirm(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(confirm) = self.confirm.clone() else {
            return;
        };
        let response = egui::Modal::new(egui::Id::new("confirm_modal")).show(ctx, |ui| {
            ui.set_min_width(400.0);
            ui.heading(confirm.title());
            ui.add_space(6.0);
            ui.add(egui::Label::new(confirm.detail()).wrap());
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    actions.push(Action::ConfirmNo);
                }
                let label = if matches!(confirm, ConfirmAction::RestoreBackup { .. }) {
                    "Restore"
                } else {
                    "Delete"
                };
                let color = if matches!(confirm, ConfirmAction::RestoreBackup { .. }) {
                    Color32::from_rgb(230, 190, 90)
                } else {
                    Color32::from_rgb(235, 120, 110)
                };
                if ui
                    .button(RichText::new(label).color(color).strong())
                    .clicked()
                {
                    actions.push(Action::ConfirmYes);
                }
            });
        });
        if response.should_close() {
            actions.push(Action::ConfirmNo);
        }
    }

    // --------------------------------------------------------------- pages

    fn page_overview(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Overview");
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshAll);
            }
        });
        if let Some(conn) = &self.conn {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Control plane: {}", conn.base_url))
                        .small()
                        .color(Color32::from_rgb(150, 156, 168)),
                );
                if ui.small_button("API docs").clicked() {
                    ui.ctx()
                        .open_url(egui::OpenUrl::new_tab(format!("{}/docs", conn.base_url)));
                }
                if ui.small_button("OpenAPI").clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(format!(
                        "{}/openapi.json",
                        conn.base_url
                    )));
                }
            });
        }
        ui.horizontal(|ui| {
            match self.feature("healthz") {
                Some(value) => {
                    let version = value
                        .get("version")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    ui.colored_label(
                        Color32::from_rgb(95, 200, 120),
                        format!(
                            "healthz ok{}",
                            if version.is_empty() {
                                String::new()
                            } else {
                                format!(" · {version}")
                            }
                        ),
                    );
                }
                None => {
                    ui.label(RichText::new("healthz …").small());
                }
            }
            match self.feature("readyz") {
                Some(value) => {
                    let ready = value.get("status").and_then(Value::as_str) == Some("ready");
                    let database = value
                        .get("database")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    ui.colored_label(
                        if ready {
                            Color32::from_rgb(95, 200, 120)
                        } else {
                            Color32::from_rgb(230, 180, 90)
                        },
                        format!("readyz · database {database}"),
                    );
                }
                None => {
                    ui.label(RichText::new("readyz …").small());
                }
            }
        });
        ui.add_space(10.0);

        ui.columns(3, |columns| {
            overview_card(
                &mut columns[0],
                "Nodes",
                self.nodes.len(),
                format!(
                    "{} online",
                    self.nodes
                        .iter()
                        .filter(|node| node.status == NodeStatus::Online)
                        .count()
                ),
                Tab::Nodes,
                actions,
            );
            overview_card(
                &mut columns[1],
                "Servers",
                self.servers.len(),
                format!(
                    "{} running",
                    self.servers
                        .iter()
                        .filter(|server| server.state == ServerState::Running)
                        .count()
                ),
                Tab::Servers,
                actions,
            );
            overview_card(
                &mut columns[2],
                "Templates",
                self.templates.len(),
                format!(
                    "{} auto-update",
                    self.templates
                        .iter()
                        .filter(|template| template.auto_update.enabled)
                        .count()
                ),
                Tab::Templates,
                actions,
            );
        });

        ui.add_space(12.0);
        ui.group(|ui| {
            ui.heading("Identity");
            if let Some(me) = &self.me {
                egui::Grid::new("me_grid")
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Key");
                        ui.label(&me.name);
                        ui.end_row();
                        ui.label("Key ID");
                        ui.label(value_text(&me.key_id));
                        ui.end_row();
                        ui.label("Scopes");
                        ui.add(
                            egui::Label::new(if me.scopes.is_empty() {
                                "none".to_string()
                            } else {
                                me.scopes.join(", ")
                            })
                            .wrap(),
                        );
                        ui.end_row();
                        ui.label("Server allow-list");
                        ui.label(if me.server_ids.is_empty() {
                            "all servers".to_string()
                        } else {
                            format!("{} server(s)", me.server_ids.len())
                        });
                        ui.end_row();
                        ui.label("Expires");
                        ui.label(me.expires_at.as_deref().unwrap_or("never"));
                        ui.end_row();
                    });
            } else {
                ui.label("Loading /api/v1/me ...");
            }
        });

        ui.add_space(12.0);
        ui.group(|ui| {
            ui.heading("Server states");
            if self.servers.is_empty() {
                ui.label(
                    RichText::new("No servers yet. Create one from the Servers tab.")
                        .color(Color32::from_rgb(150, 156, 168)),
                );
            } else {
                let mut counts: HashMap<&'static str, usize> = HashMap::new();
                for server in &self.servers {
                    *counts.entry(state_label(server.state)).or_default() += 1;
                }
                ui.horizontal_wrapped(|ui| {
                    for state in [
                        ServerState::Running,
                        ServerState::Starting,
                        ServerState::Stopping,
                        ServerState::Offline,
                        ServerState::Installing,
                        ServerState::Suspended,
                        ServerState::Pending,
                        ServerState::Unknown,
                    ] {
                        let count = counts.get(state_label(state)).copied().unwrap_or(0);
                        if count > 0 {
                            ui.label(
                                RichText::new(format!("{}: {}", state_label(state), count))
                                    .color(state_color(state)),
                            );
                        }
                    }
                });
            }
        });
    }

    // The remaining pages are implemented below in this file.

    fn page_servers(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Servers");
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshServers);
            }
            if ui
                .button(if self.create_server.show {
                    "Hide create form"
                } else {
                    "Create server"
                })
                .clicked()
            {
                actions.push(Action::ToggleCreateServer);
            }
            ui.label(
                RichText::new(format!("{} server(s)", self.servers.len()))
                    .small()
                    .color(Color32::from_rgb(150, 156, 168)),
            );
            ui.label("Name");
            ui.add(
                TextEdit::singleline(&mut self.server_filter)
                    .hint_text("contains…")
                    .desired_width(150.0),
            );
            let mut state_filter = self.server_filter_state.clone();
            egui::ComboBox::from_id_salt("server_filter_state")
                .selected_text(if state_filter.is_empty() {
                    "any state".to_string()
                } else {
                    state_filter.clone()
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut state_filter, String::new(), "any state");
                    for state in [
                        "running",
                        "starting",
                        "stopping",
                        "offline",
                        "failed",
                        "suspended",
                        "installing",
                        "unknown",
                    ] {
                        ui.selectable_value(&mut state_filter, state.to_string(), state);
                    }
                });
            if state_filter != self.server_filter_state {
                self.server_filter_state = state_filter;
                actions.push(Action::RefreshServers);
            }
            let mut node_filter = self.server_filter_node.clone();
            egui::ComboBox::from_id_salt("server_filter_node")
                .selected_text(
                    self.nodes
                        .iter()
                        .find(|node| node.id.to_string() == node_filter)
                        .map_or_else(|| "any node".to_string(), |node| node.name.clone()),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut node_filter, String::new(), "any node");
                    for node in &self.nodes {
                        ui.selectable_value(&mut node_filter, node.id.to_string(), &node.name);
                    }
                });
            if node_filter != self.server_filter_node {
                self.server_filter_node = node_filter;
                actions.push(Action::RefreshServers);
            }
        });

        if self.create_server.show {
            self.render_create_server_form(ui, actions);
        }

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.strong("Server list");
                ui.label(
                    RichText::new("Select a row to manage it")
                        .small()
                        .color(Color32::from_rgb(150, 156, 168)),
                );
            });
            if self.servers.is_empty() {
                ui.label(
                    RichText::new(if self.servers_loaded {
                        "No servers yet. Use Create server to provision one."
                    } else {
                        "Loading servers..."
                    })
                    .color(Color32::from_rgb(150, 156, 168)),
                );
                return;
            }

            let selected_server = self.selected_server.clone();
            let node_names: HashMap<String, String> = self
                .nodes
                .iter()
                .map(|node| (node.id.to_string(), node.name.clone()))
                .collect();
            let primary_port = self.server_detail.as_ref().and_then(|detail| {
                let primary = detail.server.primary_allocation_id?;
                detail
                    .allocations
                    .iter()
                    .find(|allocation| allocation.id == primary)
                    .map(|allocation| allocation.port)
            });
            ui.horizontal(|ui| {
                if ui.button("Select all").clicked() {
                    self.bulk_selected = self
                        .servers
                        .iter()
                        .map(|server| server.id.to_string())
                        .collect();
                }
                if ui.button("Clear selection").clicked() {
                    self.bulk_selected.clear();
                }
                ui.label(format!("{} selected", self.bulk_selected.len()));
                egui::ComboBox::from_id_salt("bulk_signal")
                    .selected_text(&self.bulk_signal)
                    .show_ui(ui, |ui| {
                        for signal in ["start", "stop", "restart", "kill"] {
                            ui.selectable_value(&mut self.bulk_signal, signal.to_string(), signal);
                        }
                    });
                if ui.button("Bulk power").clicked() {
                    actions.push(Action::BulkPower);
                }
                ui.add(
                    TextEdit::singleline(&mut self.bulk_command)
                        .hint_text("bulk console command")
                        .desired_width(220.0),
                );
                if ui.button("Bulk command").clicked() {
                    actions.push(Action::BulkCommand);
                }
            });
            let filter = self.server_filter.trim().to_lowercase();
            ScrollArea::vertical()
                .id_salt("servers_list")
                .max_height(260.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("servers_grid")
                        .num_columns(6)
                        .striped(true)
                        .spacing([14.0, 6.0])
                        .show(ui, |ui| {
                            ui.strong("");
                            ui.strong("Name");
                            ui.strong("State");
                            ui.strong("Node");
                            ui.strong("Primary alloc / port");
                            ui.strong("Memory / CPU");
                            ui.end_row();
                            for server in self.servers.iter().filter(|server| {
                                filter.is_empty() || server.name.to_lowercase().contains(&filter)
                            }) {
                                let id = server.id.to_string();
                                let mut checked = self.bulk_selected.contains(&id);
                                if ui.checkbox(&mut checked, "").changed() {
                                    actions.push(Action::ToggleBulk(id.clone()));
                                }
                                let is_selected = selected_server.as_deref() == Some(id.as_str());
                                if ui
                                    .selectable_label(is_selected, &server.name)
                                    .on_hover_text(format!(
                                        "id: {}\nimage: {}\ntemplate: {} {}",
                                        id,
                                        server.image,
                                        server.template_id,
                                        server.template_version
                                    ))
                                    .clicked()
                                {
                                    actions.push(Action::SelectServer(id.clone()));
                                }
                                ui.colored_label(
                                    state_color(server.state),
                                    state_label(server.state),
                                );
                                let node = node_names
                                    .get(&server.node_id.to_string())
                                    .cloned()
                                    .unwrap_or_else(|| short_id(&server.node_id.to_string()));
                                truncate_label(ui, &node, 150.0);
                                let port = if is_selected {
                                    primary_port
                                        .map(|port| port.to_string())
                                        .unwrap_or_else(|| "-".to_string())
                                } else {
                                    server
                                        .primary_allocation_id
                                        .map(|allocation| short_id(&allocation.to_string()))
                                        .unwrap_or_else(|| "-".to_string())
                                };
                                ui.label(port);
                                ui.label(format!(
                                    "{} MB / {}",
                                    server.limits.memory_mb,
                                    format_cpu(server.limits.cpu_millis)
                                ));
                                ui.end_row();
                            }
                        });
                });
        });

        ui.add_space(10.0);
        self.render_selected_server(ui, actions);
    }

    fn render_create_server_form(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.group(|ui| {
            ui.heading("Create server");
            ui.label(
                RichText::new(
                    "POST /api/v1/servers. Limits are absolute values; CPU is milli-cores (1000 = one core).",
                )
                .small()
                .color(Color32::from_rgb(150, 156, 168)),
            );
            let mut node_id = self.create_server.node_id.clone();
            let mut template_id = self.create_server.template_id.clone();
            egui::Grid::new("create_server_form")
                .num_columns(2)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Node");
                    egui::ComboBox::from_id_salt("create_server_node")
                        .selected_text(
                            node_id
                                .as_ref()
                                .and_then(|id| {
                                    self.nodes
                                        .iter()
                                        .find(|node| node.id.to_string() == *id)
                                        .map(|node| format!("{} ({})", node.name, short_id(id)))
                                })
                                .unwrap_or_else(|| "Select node".to_string()),
                        )
                        .show_ui(ui, |ui| {
                            for node in &self.nodes {
                                let id = node.id.to_string();
                                ui.selectable_value(
                                    &mut node_id,
                                    Some(id.clone()),
                                    format!("{} ({})", node.name, short_id(&id)),
                                );
                            }
                        });
                    ui.end_row();

                    ui.label("Template");
                    egui::ComboBox::from_id_salt("create_server_template")
                        .selected_text(
                            template_id
                                .as_ref()
                                .and_then(|id| {
                                    self.templates
                                        .iter()
                                        .find(|template| template.id.to_string() == *id)
                                        .map(|template| {
                                            format!("{} ({})", template.name, template.slug)
                                        })
                                })
                                .unwrap_or_else(|| "Select template".to_string()),
                        )
                        .show_ui(ui, |ui| {
                            for template in &self.templates {
                                let id = template.id.to_string();
                                ui.selectable_value(
                                    &mut template_id,
                                    Some(id.clone()),
                                    format!("{} ({})", template.name, template.slug),
                                );
                            }
                        });
                    ui.end_row();

                    ui.label("Name");
                    ui.add(
                        TextEdit::singleline(&mut self.create_server.name)
                            .hint_text("survival-1")
                            .desired_width(320.0),
                    );
                    ui.end_row();

                    ui.label("Description");
                    ui.add(
                        TextEdit::singleline(&mut self.create_server.description)
                            .hint_text("optional")
                            .desired_width(420.0),
                    );
                    ui.end_row();

                    ui.label("Memory (MB, 0=unlimited)");
                    ui.add(
                        TextEdit::singleline(&mut self.create_server.memory)
                            .desired_width(120.0),
                    );
                    ui.end_row();

                    ui.label("Disk (MB, 0=unlimited)");
                    ui.add(
                        TextEdit::singleline(&mut self.create_server.disk)
                            .desired_width(120.0),
                    );
                    ui.end_row();

                    ui.label("CPU (milli-cores, 0=unlimited)");
                    ui.add(
                        TextEdit::singleline(&mut self.create_server.cpu)
                            .desired_width(120.0),
                    );
                    ui.end_row();

                    ui.label("Auto allocations");
                    ui.add(
                        TextEdit::singleline(&mut self.create_server.auto_allocations)
                            .desired_width(80.0),
                    );
                    ui.end_row();

                    ui.label("External ref");
                    ui.add(
                        TextEdit::singleline(&mut self.create_server.external_ref)
                            .hint_text("optional")
                            .desired_width(360.0),
                    );
                    ui.end_row();

                    ui.label("Idempotency key");
                    ui.add(
                        TextEdit::singleline(&mut self.create_server.idempotency_key)
                            .hint_text("optional 8-128 chars")
                            .desired_width(360.0),
                    );
                    ui.end_row();

                    ui.label("Template version");
                    ui.add(
                        TextEdit::singleline(&mut self.create_server.template_version)
                            .hint_text("optional, default latest")
                            .desired_width(180.0),
                    );
                    ui.end_row();

                    ui.label("Protocol");
                    egui::ComboBox::from_id_salt("create_server_protocol")
                        .selected_text(if self.create_server.protocol.is_empty() { "template default".to_string() } else { self.create_server.protocol.clone() })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.create_server.protocol, String::new(), "template default");
                            ui.selectable_value(&mut self.create_server.protocol, "tcp".to_string(), "tcp");
                            ui.selectable_value(&mut self.create_server.protocol, "udp".to_string(), "udp");
                            ui.selectable_value(&mut self.create_server.protocol, "both".to_string(), "both");
                        });
                    ui.end_row();

                    ui.label("Labels");
                    ui.vertical(|ui| {
                        kv_editor(ui, "create_server_labels", "", &mut self.create_server.labels);
                    });
                    ui.end_row();

                    ui.label("Image label");
                    ui.add(
                        TextEdit::singleline(&mut self.create_server.image)
                            .hint_text("optional, e.g. java17")
                            .desired_width(220.0),
                    );
                    ui.end_row();

                    ui.label("Variables");
                    ui.add(
                        TextEdit::multiline(&mut self.create_server.variables)
                            .hint_text("KEY=VALUE, one per line")
                            .desired_rows(3)
                            .desired_width(420.0),
                    );
                    ui.end_row();
                });
            let template_changed = self.create_server.template_id != template_id;
            self.create_server.node_id = node_id;
            self.create_server.template_id = template_id;
            if template_changed {
                if let Some(template) = self
                    .create_server
                    .template_id
                    .as_ref()
                    .and_then(|id| self.templates.iter().find(|template| template.id.to_string() == *id))
                    .and_then(|record| record.latest_version.as_ref())
                {
                    self.create_server.variables = template_variables_text(&template.template);
                }
            }
            if self.nodes.is_empty() {
                ui.label(
                    RichText::new("No nodes available; create a node first.")
                        .small()
                        .color(Color32::from_rgb(230, 180, 90)),
                );
            }
            if self.templates.is_empty() {
                ui.label(
                    RichText::new("No templates available; import an egg first.")
                        .small()
                        .color(Color32::from_rgb(230, 180, 90)),
                );
            }
            if let Some(error) = &self.create_server.error {
                ui.colored_label(Color32::from_rgb(235, 120, 110), error);
            }
            if ui
                .add_enabled(
                    !self.nodes.is_empty() && !self.templates.is_empty(),
                    egui::Button::new(RichText::new("Create server").strong()),
                )
                .clicked()
            {
                actions.push(Action::CreateServer);
            }
        });
    }

    fn render_selected_server(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let Some(selected) = self.selected_server.clone() else {
            ui.label(RichText::new("No server selected.").color(Color32::from_rgb(150, 156, 168)));
            return;
        };
        let Some(detail) = self.server_detail.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Loading server {} ...", short_id(&selected)));
            });
            return;
        };
        let server = detail.server.clone();
        let suspended = server.suspended || server.state == ServerState::Suspended;
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.heading(&server.name);
                ui.colored_label(state_color(server.state), state_label(server.state));
                if server.suspended {
                    ui.colored_label(Color32::from_rgb(230, 160, 80), "suspended");
                }
            });
            egui::Grid::new("server_detail_grid")
                .num_columns(4)
                .spacing([18.0, 6.0])
                .show(ui, |ui| {
                    ui.label("ID");
                    ui.label(server.id.to_string());
                    ui.label("Node");
                    ui.label(self.node_display_name(&server.node_id.to_string()));
                    ui.end_row();

                    ui.label("Template");
                    ui.label(format!(
                        "{} {}",
                        self.template_display_name(&server.template_id.to_string()),
                        server.template_version
                    ));
                    ui.label("Image");
                    truncate_label(ui, &server.image, 260.0);
                    ui.end_row();

                    ui.label("Container");
                    ui.label(server.container_id.as_deref().unwrap_or("-"));
                    ui.label("Generation");
                    ui.label(server.generation.to_string());
                    ui.end_row();

                    ui.label("Desired power");
                    ui.label(if server.desired_running {
                        "running"
                    } else {
                        "stopped"
                    });
                    ui.label("Created");
                    ui.label(server.created_at.to_string());
                    ui.end_row();

                    ui.label("Startup");
                    ui.add(
                        egui::Label::new(RichText::new(&server.startup).monospace().size(12.0))
                            .truncate(),
                    )
                    .on_hover_text(&server.startup);
                    ui.label("Primary allocation");
                    ui.label(
                        server
                            .primary_allocation_id
                            .map(|id| short_id(&id.to_string()))
                            .unwrap_or_else(|| "-".to_string()),
                    );
                    ui.end_row();
                });
            if let Some(error) = &server.last_error {
                ui.colored_label(
                    Color32::from_rgb(235, 120, 110),
                    format!("Last error: {error}"),
                );
            }

            ui.add_space(8.0);
            if self.edit_limits.server.as_deref() != Some(server.id.to_string().as_str()) {
                self.edit_limits.load(&server);
            }
            ui.strong("Resource limits");
            ui.label(
                RichText::new(
                    "PATCH /api/v1/servers/{id} — memory, disk and CPU are absolute values; CPU is milli-cores (1000 = one core).",
                )
                .small()
                .color(Color32::from_rgb(150, 156, 168)),
            );
            egui::Grid::new("edit_limits_grid")
                .num_columns(4)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Memory (MB, 0=unlimited)");
                    ui.add(
                        TextEdit::singleline(&mut self.edit_limits.memory).desired_width(110.0),
                    );
                    ui.label("Disk (MB, 0=unlimited)");
                    ui.add(TextEdit::singleline(&mut self.edit_limits.disk).desired_width(110.0));
                    ui.end_row();

                    ui.label("CPU (milli-cores, 0=unlimited)");
                    ui.add(TextEdit::singleline(&mut self.edit_limits.cpu).desired_width(110.0));
                    ui.label("Swap (MB)");
                    ui.add(TextEdit::singleline(&mut self.edit_limits.swap).desired_width(110.0));
                    ui.end_row();

                    ui.label("I/O weight (0=unlimited)");
                    ui.add(
                        TextEdit::singleline(&mut self.edit_limits.io_weight).desired_width(110.0),
                    );
                    ui.label("PIDs limit (0=unlimited)");
                    ui.add(
                        TextEdit::singleline(&mut self.edit_limits.pids_limit)
                            .desired_width(110.0),
                    );
                    ui.end_row();

                    ui.label("CPU set");
                    ui.add(
                        TextEdit::singleline(&mut self.edit_limits.cpuset)
                            .hint_text("optional, e.g. 0-3")
                            .desired_width(110.0),
                    );
                    ui.label("");
                    ui.label("");
                    ui.end_row();
                });
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Button::new(RichText::new("Save limits").strong()))
                    .clicked()
                {
                    actions.push(Action::SaveLimits);
                }
                ui.label(
                    RichText::new(
                        "Applies to the running container after the agent reconciles; data is kept.",
                    )
                    .small()
                    .color(Color32::from_rgb(150, 156, 168)),
                );
            });
            if let Some(error) = &self.edit_limits.error {
                ui.colored_label(Color32::from_rgb(235, 120, 110), error);
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.strong("Allocations");
                if ui.small_button("Refresh").clicked() {
                    actions.push(Action::RefreshDetail);
                }
            });
            if detail.allocations.is_empty() {
                ui.label("No allocations attached.");
            } else {
                for allocation in &detail.allocations {
                    let primary =
                        server.primary_allocation_id == Some(allocation.id) || allocation.primary;
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "{}{}:{}/{}",
                            if primary { "* " } else { "  " },
                            allocation.ip,
                            allocation.port,
                            allocation.protocol.as_str()
                        ));
                        if !primary && ui.small_button("Detach").clicked() {
                            actions.push(Action::DetachAllocation(allocation.id.to_string()));
                        }
                    });
                }
            }
            let free = self.feature(&format!("server_free:{}", server.id))
                .and_then(|value| value.get("allocations").and_then(Value::as_array).cloned())
                .unwrap_or_default()
                .into_iter()
                .filter(|allocation| allocation.get("server_id").map(Value::is_null).unwrap_or(true))
                .collect::<Vec<_>>();
            if !free.is_empty() {
                ui.horizontal(|ui| {
                    ui.label("Attach free allocation");
                    egui::ComboBox::from_id_salt("attach_free_allocation")
                        .selected_text(self.attach_allocation.as_deref().unwrap_or("Select"))
                        .show_ui(ui, |ui| {
                            for allocation in &free {
                                let id = allocation.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                                let label = format!(
                                    "{}:{}/{}",
                                    allocation.get("ip").and_then(Value::as_str).unwrap_or("-"),
                                    allocation.get("port").and_then(Value::as_u64).unwrap_or(0),
                                    allocation.get("protocol").and_then(Value::as_str).unwrap_or("-")
                                );
                                ui.selectable_value(&mut self.attach_allocation, Some(id), label);
                            }
                        });
                    if ui.button("Attach").clicked() {
                        actions.push(Action::AttachAllocation);
                    }
                });
            }

            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for (label, signal) in [
                    ("Start", "start"),
                    ("Stop", "stop"),
                    ("Restart", "restart"),
                    ("Kill", "kill"),
                ] {
                    if ui
                        .add_enabled(!suspended, egui::Button::new(label))
                        .clicked()
                    {
                        actions.push(Action::Power {
                            server: server.id.to_string(),
                            signal,
                        });
                    }
                }
                ui.separator();
                if ui
                    .add_enabled(!suspended, egui::Button::new("Reinstall"))
                    .on_hover_text("Recreate the container and rerun the install script")
                    .clicked()
                {
                    actions.push(Action::Reinstall(server.id.to_string()));
                }
                if ui
                    .button(if server.suspended {
                        "Resume"
                    } else {
                        "Suspend"
                    })
                    .clicked()
                {
                    actions.push(Action::Suspend {
                        server: server.id.to_string(),
                        resume: server.suspended,
                    });
                }
                if ui.button("Delete").clicked() {
                    actions.push(Action::AskDeleteServer {
                        id: server.id.to_string(),
                        name: server.name.clone(),
                    });
                }
                ui.separator();
                if ui.button("Refresh detail").clicked() {
                    actions.push(Action::RefreshDetail);
                }
                if ui.button("Load stats").clicked() {
                    actions.push(Action::LoadStats);
                }
            });

            ui.add_space(6.0);
            if let Some(stats) = self.stats.clone() {
                ui.strong("Live stats");
                egui::Grid::new("stats_grid")
                    .num_columns(4)
                    .spacing([16.0, 5.0])
                    .show(ui, |ui| {
                        ui.label("CPU");
                        ui.label(format!("{:.1}%", stats.cpu_percent));
                        ui.label("Memory");
                        ui.label(format!(
                            "{} / {}",
                            format_bytes(stats.memory_bytes),
                            format_bytes(stats.memory_limit_bytes)
                        ));
                        ui.end_row();
                        ui.label("Disk");
                        ui.label(format!(
                            "{} / {}",
                            format_bytes(stats.disk_bytes),
                            format_bytes(stats.disk_limit_bytes)
                        ));
                        ui.label("Network");
                        ui.label(format!(
                            "rx {} / tx {}",
                            format_bytes(stats.network_rx_bytes),
                            format_bytes(stats.network_tx_bytes)
                        ));
                        ui.end_row();
                        ui.label("PIDs");
                        ui.label(stats.pids.to_string());
                        ui.label("Uptime");
                        ui.label(format_uptime(stats.uptime_seconds));
                        ui.end_row();
                        ui.label("Sampled");
                        ui.label(stats.sampled_at.to_string());
                        ui.label("State");
                        ui.label(state_label(stats.state));
                        ui.end_row();
                    });
            } else if let Some(error) = &self.stats_error {
                ui.label(
                    RichText::new(format!("Stats unavailable: {error}"))
                        .small()
                        .color(Color32::from_rgb(230, 180, 90)),
                );
            }
        });
    }

    fn page_console(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Console");
            let mut selected = self.selected_server.clone();
            egui::ComboBox::from_id_salt("console_server")
                .selected_text(self.server_combo_text(selected.as_deref()))
                .show_ui(ui, |ui| {
                    for server in &self.servers {
                        let id = server.id.to_string();
                        ui.selectable_value(
                            &mut selected,
                            Some(id.clone()),
                            format!("{} ({})", server.name, short_id(&id)),
                        );
                    }
                });
            if selected != self.selected_server {
                if let Some(id) = selected {
                    actions.push(Action::SelectServer(id));
                }
            }
            if ui.button("Clear").clicked() {
                actions.push(Action::ConsoleClear);
            }
            if ui.button("Refresh servers").clicked() {
                actions.push(Action::RefreshServers);
            }
            ui.checkbox(&mut self.console_stream, "Live SSE stream");
            ui.label(
                RichText::new(if self.console_stream {
                    "GET /logs/stream"
                } else {
                    "GET /logs poll"
                })
                .small()
                .color(Color32::from_rgb(150, 156, 168)),
            );
        });

        let Some(server_id) = self.selected_server.clone() else {
            ui.add_space(20.0);
            ui.label(
                RichText::new("Select a server to open its console.")
                    .color(Color32::from_rgb(150, 156, 168)),
            );
            return;
        };
        let state = self.server_state(&server_id);
        let running = state == Some(ServerState::Running);
        if running {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(95, 200, 120), "running");
                ui.label(
                    RichText::new(if self.console_stream {
                        "Streaming GET /logs/stream (SSE) in the console worker"
                    } else {
                        "Polling GET /logs?lines=500 every 1.5s in the background worker"
                    })
                    .small()
                    .color(Color32::from_rgb(150, 156, 168)),
                );
            });
        } else {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(230, 180, 90), "paused");
                ui.label(
                    RichText::new(format!(
                        "Server is {}; polling is paused until it is running.",
                        state.map_or("not loaded", state_label)
                    ))
                    .small()
                    .color(Color32::from_rgb(150, 156, 168)),
                );
            });
        }

        let (line_count, last_error, last_update, truncated) = {
            let log = self.console_logs.entry(server_id.clone()).or_default();
            (
                log.lines.len(),
                log.last_error.clone(),
                log.last_update,
                log.truncated,
            )
        };
        if let Some(error) = &last_error {
            ui.colored_label(
                Color32::from_rgb(235, 120, 110),
                format!("Console error: {error}"),
            );
        }
        if truncated {
            ui.label(
                RichText::new("The agent reported that older log output was truncated.")
                    .small()
                    .color(Color32::from_rgb(230, 180, 90)),
            );
        }

        let row_height = ui.text_style_height(&TextStyle::Monospace) + 2.0;
        let console_height = (ui.available_height() - 92.0).max(180.0);
        egui::Frame::new()
            .fill(Color32::from_rgb(8, 10, 13))
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                let log = self.console_logs.entry(server_id.clone()).or_default();
                if log.lines.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(if running {
                            "Waiting for output..."
                        } else {
                            "No buffered output. Start the server to poll logs."
                        })
                        .monospace()
                        .color(Color32::from_rgb(120, 128, 140)),
                    );
                }
                let output = ScrollArea::vertical()
                    .id_salt(("console_output", &server_id))
                    .stick_to_bottom(true)
                    .max_height(console_height)
                    .auto_shrink([false, false])
                    .show_rows(ui, row_height, log.lines.len(), |ui, range| {
                        for index in range {
                            let line = &log.lines[index];
                            let response = ui.add(
                                egui::Label::new(
                                    RichText::new(line)
                                        .monospace()
                                        .size(13.0)
                                        .color(Color32::from_rgb(205, 214, 225)),
                                )
                                .truncate(),
                            );
                            if line.chars().count() > 150 {
                                response.on_hover_text(line);
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{line_count} line(s) buffered"))
                            .small()
                            .color(Color32::from_rgb(120, 128, 140)),
                    );
                    if let Some(update) = last_update {
                        ui.label(
                            RichText::new(format!("last poll {}s ago", update.elapsed().as_secs()))
                                .small()
                                .color(Color32::from_rgb(120, 128, 140)),
                        );
                    }
                    let at_bottom = output.state.offset.y + output.inner_rect.height()
                        >= output.content_size.y - 8.0;
                    ui.label(
                        RichText::new(if at_bottom {
                            "following output"
                        } else {
                            "scrolled up (auto-follow paused)"
                        })
                        .small()
                        .color(if at_bottom {
                            Color32::from_rgb(110, 190, 130)
                        } else {
                            Color32::from_rgb(230, 180, 90)
                        }),
                    );
                });
            });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let edit = ui.add(
                TextEdit::singleline(&mut self.console_input)
                    .font(TextStyle::Monospace)
                    .hint_text("console command, e.g. say hello")
                    .desired_width(f32::INFINITY),
            );
            let enter = edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let send = ui
                .add_enabled(
                    !self.console_input.trim().is_empty(),
                    egui::Button::new(RichText::new("Send").strong()),
                )
                .clicked();
            if enter || send {
                actions.push(Action::SendCommand);
            }
            if enter {
                edit.request_focus();
            }
        });
        ui.label(
            RichText::new("Enter sends. Commands are POSTed to /api/v1/servers/{id}/command.")
                .small()
                .color(Color32::from_rgb(130, 136, 148)),
        );
    }

    fn page_files(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Files");
            let mut selected = self.selected_server.clone();
            egui::ComboBox::from_id_salt("files_server")
                .selected_text(self.server_combo_text(selected.as_deref()))
                .show_ui(ui, |ui| {
                    for server in &self.servers {
                        let id = server.id.to_string();
                        ui.selectable_value(
                            &mut selected,
                            Some(id.clone()),
                            format!("{} ({})", server.name, short_id(&id)),
                        );
                    }
                });
            if selected != self.selected_server {
                if let Some(id) = selected {
                    actions.push(Action::SelectServer(id));
                }
            }
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshTab);
            }
        });

        if self.selected_server.is_none() {
            ui.add_space(20.0);
            ui.label(
                RichText::new("Select a server to browse its filesystem.")
                    .color(Color32::from_rgb(150, 156, 168)),
            );
            return;
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Up").clicked() {
                actions.push(Action::FilesUp);
            }
            ui.add(
                TextEdit::singleline(&mut self.files.path)
                    .font(TextStyle::Monospace)
                    .desired_width(420.0),
            );
            if ui.button("Go").clicked() {
                actions.push(Action::FilesGo);
            }
            if self.files.loading {
                ui.spinner();
            }
        });

        if let Some(error) = &self.files.list_error {
            ui.colored_label(
                Color32::from_rgb(235, 120, 110),
                format!("Could not list directory: {error}"),
            );
        }

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("Directory: {}", self.files.path));
                if ui.small_button("Reload").clicked() {
                    actions.push(Action::RefreshTab);
                }
            });
            let entries = self.files.entries.clone();
            let selected_entry = self.files.selected.clone();
            if entries.is_empty() {
                ui.label(
                    RichText::new("Empty directory (or still loading).")
                        .color(Color32::from_rgb(150, 156, 168)),
                );
            } else {
                ScrollArea::vertical()
                    .id_salt("files_list")
                    .max_height(240.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("files_grid")
                            .num_columns(4)
                            .striped(true)
                            .spacing([14.0, 5.0])
                            .show(ui, |ui| {
                                ui.strong("Name");
                                ui.strong("Size");
                                ui.strong("Mode");
                                ui.strong("Modified");
                                ui.end_row();
                                for entry in &entries {
                                    let path = entry.path.clone();
                                    let is_selected =
                                        selected_entry.as_deref() == Some(path.as_str());
                                    let icon = if entry.is_directory {
                                        "[dir]"
                                    } else {
                                        "[file]"
                                    };
                                    let label = format!("{icon} {}", entry.name);
                                    if ui
                                        .selectable_label(is_selected, label)
                                        .on_hover_text(&path)
                                        .clicked()
                                    {
                                        actions.push(Action::OpenEntry {
                                            path: path.clone(),
                                            is_directory: entry.is_directory,
                                        });
                                    }
                                    ui.label(if entry.is_directory {
                                        "-".to_string()
                                    } else {
                                        format_bytes(entry.size)
                                    });
                                    ui.label(
                                        entry
                                            .mode
                                            .map(|mode| format!("{mode:04o}"))
                                            .unwrap_or_else(|| "-".to_string()),
                                    );
                                    ui.label(
                                        entry
                                            .modified_at
                                            .map(|time| time.to_string())
                                            .unwrap_or_else(|| "-".to_string()),
                                    );
                                    ui.end_row();
                                }
                            });
                    });
            }
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.strong("File actions");
            if let Some(selected) = &self.files.selected {
                ui.label(
                    RichText::new(format!("Selected: {selected}"))
                        .monospace()
                        .small(),
                );
            } else {
                ui.label(
                    RichText::new(
                        "Click a file name to select and open it, or a directory to enter it.",
                    )
                    .small()
                    .color(Color32::from_rgb(150, 156, 168)),
                );
            }
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut self.files.rename_to)
                        .hint_text("new name or /absolute/path")
                        .desired_width(260.0),
                );
                if ui
                    .add_enabled(
                        self.files.selected.is_some() && !self.files.rename_to.trim().is_empty(),
                        egui::Button::new("Rename"),
                    )
                    .clicked()
                {
                    actions.push(Action::FileRename);
                }
                ui.separator();
                ui.add(
                    TextEdit::singleline(&mut self.files.chmod_mode)
                        .hint_text("0644")
                        .desired_width(80.0),
                );
                if ui
                    .add_enabled(self.files.selected.is_some(), egui::Button::new("Chmod"))
                    .clicked()
                {
                    actions.push(Action::FileChmod);
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut self.files.new_dir)
                        .hint_text("new directory")
                        .desired_width(260.0),
                );
                if ui
                    .add_enabled(
                        !self.files.new_dir.trim().is_empty(),
                        egui::Button::new("Create directory"),
                    )
                    .clicked()
                {
                    actions.push(Action::FileMkdir);
                }
                ui.separator();
                if ui
                    .add_enabled(self.files.selected.is_some(), egui::Button::new("Compress"))
                    .clicked()
                {
                    actions.push(Action::FileCompress);
                }
                if ui
                    .add_enabled(
                        self.files.selected.is_some(),
                        egui::Button::new("Decompress"),
                    )
                    .clicked()
                {
                    actions.push(Action::FileDecompress);
                }
                if ui
                    .add_enabled(
                        self.files.selected.is_some(),
                        egui::Button::new(
                            RichText::new("Delete").color(Color32::from_rgb(235, 120, 110)),
                        ),
                    )
                    .clicked()
                {
                    if let Some(path) = self.files.selected.clone() {
                        actions.push(Action::AskDeleteFile(path));
                    }
                }
            });
        });

        ui.add_space(8.0);
        if let Some(editor) = &mut self.files.editor {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.strong(format!("Editor: {}", editor.path));
                    ui.label(
                        RichText::new(format!(
                            "{} · {} bytes{}",
                            editor.encoding,
                            editor.size,
                            if editor.truncated {
                                " · truncated by server"
                            } else {
                                ""
                            }
                        ))
                        .small()
                        .color(Color32::from_rgb(150, 156, 168)),
                    );
                    if editor.dirty {
                        ui.colored_label(Color32::from_rgb(230, 180, 90), "unsaved");
                    }
                });
                if editor.encoding != "utf8" {
                    ui.colored_label(
                        Color32::from_rgb(230, 180, 90),
                        "Non-UTF-8 content is shown read-only (base64 encoded by the agent).",
                    );
                }
                let editable = editor.encoding == "utf8" && !editor.truncated;
                let response = ui.add_enabled_ui(editable, |ui| {
                    ui.add(
                        TextEdit::multiline(&mut editor.content)
                            .code_editor()
                            .desired_rows(14)
                            .desired_width(f32::INFINITY),
                    )
                });
                if response.inner.changed() {
                    editor.dirty = true;
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(editor.dirty && editable, egui::Button::new("Save"))
                        .clicked()
                    {
                        actions.push(Action::SaveFile);
                    }
                    if ui.button("Reload").clicked() {
                        actions.push(Action::ReloadEditor);
                    }
                    if editor.truncated {
                        ui.label(
                            RichText::new("Refusing to save a truncated file.")
                                .small()
                                .color(Color32::from_rgb(230, 180, 90)),
                        );
                    }
                });
            });
        }
    }

    fn page_config(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.heading("Server configuration");
        let Some(server_id) = self.selected_server.clone() else {
            ui.label("Select a server in Servers or Console first.");
            return;
        };
        if self.server_config.server.as_deref() != Some(server_id.as_str()) {
            if let Some(detail) = &self.server_detail {
                let server = detail.server.clone();
                self.server_config.load(&server);
            }
        }
        if self.server_config.server.is_none() {
            ui.label("Loading server configuration…");
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshDetail);
            }
            return;
        }
        ui.label(
            RichText::new("Template variables are write-only here; inspect the rendered Environment tab for current values.")
                .small()
                .color(Color32::from_rgb(150, 156, 168)),
        );
        egui::Grid::new("server_config_grid")
            .num_columns(2)
            .spacing([14.0, 7.0])
            .show(ui, |ui| {
                ui.label("Name");
                ui.add(TextEdit::singleline(&mut self.server_config.name).desired_width(360.0));
                ui.end_row();
                ui.label("Description");
                ui.add(
                    TextEdit::singleline(&mut self.server_config.description).desired_width(500.0),
                );
                ui.end_row();
                ui.label("External ref");
                ui.add(
                    TextEdit::singleline(&mut self.server_config.external_ref).desired_width(360.0),
                );
                ui.end_row();
                ui.label("Image");
                ui.add(TextEdit::singleline(&mut self.server_config.image).desired_width(500.0));
                ui.end_row();
                ui.label("Template version");
                ui.add(
                    TextEdit::singleline(&mut self.server_config.template_version)
                        .desired_width(180.0),
                );
                ui.end_row();
            });
        ui.add_space(6.0);
        kv_editor(
            ui,
            "server_config_labels",
            "Labels",
            &mut self.server_config.labels,
        );
        kv_editor(
            ui,
            "server_config_variables",
            "Template variables (leave empty to keep current values)",
            &mut self.server_config.variables,
        );
        if let Some(error) = &self.server_config.error {
            ui.colored_label(Color32::from_rgb(235, 120, 110), error);
        }
        ui.horizontal(|ui| {
            if ui.button("Save configuration").clicked() {
                actions.push(Action::SaveServerConfig);
            }
            if ui.button("Reload").clicked() {
                actions.push(Action::RefreshDetail);
            }
        });
    }

    fn page_schedules(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.heading("Schedules");
        let Some(server) = self.selected_server.clone() else {
            ui.label("Select a server first.");
            return;
        };
        let key = format!("schedules:{server}");
        let value = self.feature(&key);
        if let Some(error) = self.feature_error(&key) {
            ui.colored_label(Color32::from_rgb(235, 120, 110), error);
        }
        let schedules = value
            .as_ref()
            .and_then(|value| value.get("schedules"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        ui.horizontal(|ui| {
            if ui.button("New schedule").clicked() {
                actions.push(Action::ScheduleNew);
            }
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshTab);
            }
        });
        ui.separator();
        egui::Grid::new("schedule_form_grid")
            .num_columns(2)
            .spacing([14.0, 6.0])
            .show(ui, |ui| {
                ui.label("Name");
                ui.add(TextEdit::singleline(&mut self.schedule_form.name).desired_width(280.0));
                ui.end_row();
                ui.label("Cron (UTC)");
                ui.add(TextEdit::singleline(&mut self.schedule_form.cron).desired_width(180.0));
                ui.end_row();
            });
        ui.label("Actions (JSON array)");
        ui.add(
            TextEdit::multiline(&mut self.schedule_form.actions)
                .desired_rows(6)
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            if ui.small_button("+ restart").clicked() {
                self.schedule_push(json!({"type":"power","signal":"restart"}));
            }
            if ui.small_button("+ stop").clicked() {
                self.schedule_push(json!({"type":"power","signal":"stop"}));
            }
            if ui.small_button("Format JSON").clicked() {
                if let Ok(value) = serde_json::from_str::<Value>(&self.schedule_form.actions) {
                    self.schedule_form.actions = serde_json::to_string_pretty(&value)
                        .unwrap_or_else(|_| self.schedule_form.actions.clone());
                }
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.schedule_form.enabled, "Enabled");
            if ui
                .button(if self.schedule_form.id.is_some() {
                    "Save changes"
                } else {
                    "Create schedule"
                })
                .clicked()
            {
                actions.push(Action::ScheduleSave);
            }
            if self.schedule_form.id.is_some() && ui.button("Cancel edit").clicked() {
                actions.push(Action::ScheduleNew);
            }
        });
        if let Some(error) = &self.schedule_form.error {
            ui.colored_label(Color32::from_rgb(235, 120, 110), error);
        }
        ui.separator();
        if schedules.is_empty() {
            ui.label("No schedules.");
        } else {
            egui::Grid::new("schedule_list_grid")
                .num_columns(5)
                .striped(true)
                .spacing([12.0, 5.0])
                .show(ui, |ui| {
                    ui.strong("Name");
                    ui.strong("Cron");
                    ui.strong("Enabled");
                    ui.strong("Next run");
                    ui.strong("");
                    ui.end_row();
                    for schedule in schedules {
                        let id = schedule
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        if id.is_empty() {
                            continue;
                        }
                        ui.label(schedule.get("name").and_then(Value::as_str).unwrap_or("-"));
                        ui.monospace(schedule.get("cron").and_then(Value::as_str).unwrap_or("-"));
                        ui.label(
                            if schedule
                                .get("enabled")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                            {
                                "yes"
                            } else {
                                "no"
                            },
                        );
                        ui.label(
                            schedule
                                .get("next_run_at")
                                .and_then(Value::as_str)
                                .unwrap_or("-"),
                        );
                        ui.horizontal(|ui| {
                            if ui.small_button("Edit").clicked() {
                                actions.push(Action::ScheduleEdit(id.clone()));
                            }
                            if ui.small_button("Run").clicked() {
                                actions.push(Action::ScheduleRun(id.clone()));
                            }
                            if ui.small_button("Delete").clicked() {
                                actions.push(Action::ScheduleDelete(id.clone()));
                            }
                        });
                        ui.end_row();
                    }
                });
        }
    }

    fn page_sftp(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.heading("SFTP credentials");
        let Some(server) = self.selected_server.clone() else {
            ui.label("Select a server first.");
            return;
        };
        let key = format!("sftp:{server}");
        let value = self.feature(&key);
        if let Some(error) = self.feature_error(&key) {
            ui.colored_label(Color32::from_rgb(235, 120, 110), error);
        }
        ui.label("SFTP requires a native agent with the hostctl/SFTP feature; the containerized agent reports it as unsupported.");
        if let Some(created) = self.feature("sftp_created") {
            ui.group(|ui| {
                ui.strong("Created credential (shown once)");
                if let Some(password) = created.get("password").and_then(Value::as_str) {
                    ui.monospace(format!(
                        "host={} port={} username={} password={}",
                        created.get("host").and_then(Value::as_str).unwrap_or("-"),
                        created.get("port").and_then(Value::as_u64).unwrap_or(0),
                        created
                            .get("username")
                            .and_then(Value::as_str)
                            .unwrap_or("-"),
                        password
                    ));
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label("TTL seconds (optional)");
            ui.add(TextEdit::singleline(&mut self.sftp_ttl).desired_width(120.0));
            if ui.button("Create credential").clicked() {
                actions.push(Action::SftpCreate);
            }
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshTab);
            }
        });
        ui.separator();
        let credentials = value
            .as_ref()
            .and_then(|value| value.get("credentials"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if credentials.is_empty() {
            ui.label("No SFTP credentials.");
        } else {
            egui::Grid::new("sftp_list_grid")
                .num_columns(4)
                .striped(true)
                .spacing([12.0, 5.0])
                .show(ui, |ui| {
                    ui.strong("Username");
                    ui.strong("Expires");
                    ui.strong("State");
                    ui.strong("");
                    ui.end_row();
                    for credential in credentials {
                        let id = credential
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        if id.is_empty() {
                            continue;
                        }
                        ui.monospace(
                            credential
                                .get("username")
                                .and_then(Value::as_str)
                                .unwrap_or("-"),
                        );
                        ui.label(
                            credential
                                .get("expires_at")
                                .and_then(Value::as_str)
                                .unwrap_or("-"),
                        );
                        ui.label(if credential.get("revoked_at").is_some() {
                            "revoked"
                        } else {
                            "active"
                        });
                        if ui.small_button("Revoke").clicked() {
                            actions.push(Action::SftpRevoke(id));
                        }
                        ui.end_row();
                    }
                });
        }
    }

    fn page_environment(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.heading("Rendered environment");
        let Some(server) = self.selected_server.clone() else {
            ui.label("Select a server first.");
            return;
        };
        let key = format!("environment:{server}");
        if let Some(error) = self.feature_error(&key) {
            ui.colored_label(Color32::from_rgb(235, 120, 110), error);
        }
        ui.label("Requires server:secrets or admin. Change template variables in Config.");
        if ui.button("Refresh").clicked() {
            actions.push(Action::RefreshTab);
        }
        if let Some(value) = self.feature(&key) {
            if let Some(environment) = value.get("environment").and_then(Value::as_object) {
                egui::Grid::new("environment_grid")
                    .num_columns(2)
                    .striped(true)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        ui.strong("Key");
                        ui.strong("Value");
                        ui.end_row();
                        for (key, value) in environment {
                            ui.monospace(key);
                            ui.monospace(value.as_str().unwrap_or("—"));
                            ui.end_row();
                        }
                    });
            }
        }
    }

    fn page_jobs(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.heading("Jobs");
        let Some(server) = self.selected_server.clone() else {
            ui.label("Select a server first.");
            return;
        };
        let key = format!("jobs:{server}");
        if let Some(error) = self.feature_error(&key) {
            ui.colored_label(Color32::from_rgb(235, 120, 110), error);
        }
        if ui.button("Refresh").clicked() {
            actions.push(Action::RefreshTab);
        }
        let jobs = self
            .feature(&key)
            .and_then(|value| value.get("jobs").and_then(Value::as_array).cloned())
            .unwrap_or_default();
        if jobs.is_empty() {
            ui.label("No jobs.");
        } else {
            egui::Grid::new("jobs_grid")
                .num_columns(5)
                .striped(true)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.strong("ID");
                    ui.strong("Kind");
                    ui.strong("Status");
                    ui.strong("Created");
                    ui.strong("Error");
                    ui.end_row();
                    for job in jobs {
                        ui.monospace(short_id(
                            job.get("id").and_then(Value::as_str).unwrap_or("-"),
                        ));
                        ui.label(job.get("kind").and_then(Value::as_str).unwrap_or("-"));
                        ui.label(job.get("status").and_then(Value::as_str).unwrap_or("-"));
                        ui.label(job.get("created_at").and_then(Value::as_str).unwrap_or("-"));
                        ui.label(job.get("last_error").and_then(Value::as_str).unwrap_or("—"));
                        ui.end_row();
                    }
                });
        }
    }

    fn page_notifications(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Notifications");
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshTab);
            }
        });
        if let Some(error) = self.feature_error("notification_config") {
            ui.colored_label(Color32::from_rgb(235, 120, 110), error);
        }
        if let Some(config) = self.feature("notification_config") {
            ui.group(|ui| {
                ui.strong("SMTP");
                ui.label(format!(
                    "enabled={} from={} recipients={} tls={}",
                    config
                        .get("enabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    config.get("from").and_then(Value::as_str).unwrap_or("—"),
                    config
                        .get("recipients")
                        .and_then(Value::as_array)
                        .map(|items| items
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", "))
                        .unwrap_or_default(),
                    config.get("tls").and_then(Value::as_str).unwrap_or("—"),
                ));
            });
        }
        egui::Grid::new("notification_form_grid")
            .num_columns(2)
            .spacing([14.0, 6.0])
            .show(ui, |ui| {
                ui.label("To (comma separated, blank = default)");
                ui.add(TextEdit::singleline(&mut self.notification_form.to).desired_width(500.0));
                ui.end_row();
                ui.label("Subject");
                ui.add(
                    TextEdit::singleline(&mut self.notification_form.subject).desired_width(500.0),
                );
                ui.end_row();
            });
        ui.label("Body");
        ui.add(
            TextEdit::multiline(&mut self.notification_form.body)
                .desired_rows(6)
                .desired_width(f32::INFINITY),
        );
        if let Some(error) = &self.notification_form.error {
            ui.colored_label(Color32::from_rgb(235, 120, 110), error);
        }
        if ui.button("Queue email").clicked() {
            actions.push(Action::NotificationSend);
        }
        ui.separator();
        let emails = self
            .feature("notifications")
            .and_then(|value| value.get("emails").and_then(Value::as_array).cloned())
            .unwrap_or_default();
        if emails.is_empty() {
            ui.label("No emails.");
        } else {
            egui::Grid::new("notification_list_grid")
                .num_columns(5)
                .striped(true)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.strong("Subject");
                    ui.strong("To");
                    ui.strong("Status");
                    ui.strong("Created");
                    ui.strong("");
                    ui.end_row();
                    for email in emails {
                        let id = email
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        ui.label(email.get("subject").and_then(Value::as_str).unwrap_or("-"));
                        ui.label(
                            email
                                .get("recipients")
                                .and_then(Value::as_array)
                                .map(|items| {
                                    items
                                        .iter()
                                        .filter_map(Value::as_str)
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                })
                                .unwrap_or_default(),
                        );
                        ui.label(email.get("status").and_then(Value::as_str).unwrap_or("-"));
                        ui.label(
                            email
                                .get("created_at")
                                .and_then(Value::as_str)
                                .unwrap_or("-"),
                        );
                        if !id.is_empty() && ui.small_button("Retry").clicked() {
                            actions.push(Action::NotificationRetry(id));
                        }
                        ui.end_row();
                    }
                });
        }
    }

    fn page_api_keys(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("API keys");
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshTab);
            }
        });
        if let Some(error) = self.feature_error("api_keys") {
            ui.colored_label(Color32::from_rgb(235, 120, 110), error);
        }
        if let Some(created) = self.feature("api_key_created") {
            if let Some(token) = created.get("token").and_then(Value::as_str) {
                ui.group(|ui| {
                    ui.strong("Created key (shown once)");
                    ui.monospace(token);
                });
            }
        }
        egui::Grid::new("api_key_form_grid")
            .num_columns(2)
            .spacing([14.0, 6.0])
            .show(ui, |ui| {
                ui.label("Name");
                ui.add(TextEdit::singleline(&mut self.api_key_form.name).desired_width(280.0));
                ui.end_row();
                ui.label("Scopes (comma separated)");
                ui.add(TextEdit::singleline(&mut self.api_key_form.scopes).desired_width(600.0));
                ui.end_row();
                ui.label("Server allow-list (comma separated UUIDs)");
                ui.add(
                    TextEdit::singleline(&mut self.api_key_form.server_ids).desired_width(600.0),
                );
                ui.end_row();
                ui.label("Expires in days");
                ui.add(TextEdit::singleline(&mut self.api_key_form.days).desired_width(100.0));
                ui.end_row();
            });
        if let Some(error) = &self.api_key_form.error {
            ui.colored_label(Color32::from_rgb(235, 120, 110), error);
        }
        if ui.button("Create key").clicked() {
            actions.push(Action::ApiKeyCreate);
        }
        ui.separator();
        let keys = self
            .feature("api_keys")
            .and_then(|value| value.get("keys").and_then(Value::as_array).cloned())
            .unwrap_or_default();
        if keys.is_empty() {
            ui.label("No API keys.");
        } else {
            egui::Grid::new("api_key_list_grid")
                .num_columns(6)
                .striped(true)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.strong("Name");
                    ui.strong("Prefix");
                    ui.strong("Scopes");
                    ui.strong("Status");
                    ui.strong("Last used");
                    ui.strong("");
                    ui.end_row();
                    for key in keys {
                        let id = key
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        ui.label(key.get("name").and_then(Value::as_str).unwrap_or("-"));
                        ui.monospace(key.get("prefix").and_then(Value::as_str).unwrap_or("-"));
                        ui.label(
                            key.get("scopes")
                                .and_then(Value::as_array)
                                .map(|items| {
                                    items
                                        .iter()
                                        .filter_map(Value::as_str)
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                })
                                .unwrap_or_default(),
                        );
                        ui.label(if key.get("revoked_at").is_some() {
                            "revoked"
                        } else {
                            "active"
                        });
                        ui.label(
                            key.get("last_used_at")
                                .and_then(Value::as_str)
                                .unwrap_or("—"),
                        );
                        if !id.is_empty() && ui.small_button("Revoke").clicked() {
                            actions.push(Action::ApiKeyRevoke(id));
                        }
                        ui.end_row();
                    }
                });
        }
    }

    fn page_metrics(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.heading("Metrics");
        if let Some(error) = self.feature_error("metrics") {
            ui.colored_label(Color32::from_rgb(235, 120, 110), error);
        }
        if ui.button("Refresh").clicked() {
            actions.push(Action::RefreshTab);
        }
        if let Some(value) = self.feature("metrics") {
            let text = value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string());
            ScrollArea::vertical().max_height(620.0).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(text).monospace().size(11.0)).wrap());
            });
        }
    }

    fn page_nodes(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Nodes");
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshTab);
            }
            if ui
                .button(if self.create_node.show {
                    "Hide create form"
                } else {
                    "Create node"
                })
                .clicked()
            {
                actions.push(Action::ToggleCreateNode);
            }
            ui.label(
                RichText::new(format!("{} node(s)", self.nodes.len()))
                    .small()
                    .color(Color32::from_rgb(150, 156, 168)),
            );
        });

        if self.create_node.show {
            self.render_create_node_form(ui, actions);
        }

        if let Some(secret) = &self.node_secret {
            ui.group(|ui| {
                ui.colored_label(
                    Color32::from_rgb(230, 180, 90),
                    format!("Agent token for '{}' (shown once)", secret.node),
                );
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&secret.token)
                                .monospace()
                                .size(12.0)
                                .color(Color32::from_rgb(220, 226, 235)),
                        )
                        .wrap(),
                    );
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(secret.token.clone());
                    }
                    if ui.button("Dismiss").clicked() {
                        actions.push(Action::ClearNodeSecret);
                    }
                });
            });
        }

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.strong("Node list");
            if self.nodes.is_empty() {
                ui.label(
                    RichText::new("No nodes registered yet.")
                        .color(Color32::from_rgb(150, 156, 168)),
                );
                return;
            }
            let nodes = self.nodes.clone();
            let selected = self.selected_node.clone();
            ScrollArea::vertical()
                .id_salt("nodes_list")
                .max_height(230.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("nodes_grid")
                        .num_columns(5)
                        .striped(true)
                        .spacing([14.0, 6.0])
                        .show(ui, |ui| {
                            ui.strong("Name");
                            ui.strong("Status");
                            ui.strong("Endpoint");
                            ui.strong("Capacity");
                            ui.strong("Heartbeat");
                            ui.end_row();
                            for node in &nodes {
                                let id = node.id.to_string();
                                if ui
                                    .selectable_label(
                                        selected.as_deref() == Some(id.as_str()),
                                        &node.name,
                                    )
                                    .on_hover_text(format!(
                                        "id: {}\nfqdn: {}\ndriver: {}",
                                        id,
                                        node.fqdn,
                                        node.driver.as_str()
                                    ))
                                    .clicked()
                                {
                                    actions.push(Action::SelectNode(id.clone()));
                                }
                                ui.colored_label(
                                    node_status_color(node.status),
                                    node_status_label(node.status),
                                );
                                truncate_label(ui, &node.endpoint, 240.0);
                                ui.label(format!(
                                    "{} MB / {} / {} MB",
                                    node.capacity.memory_mb,
                                    format_cpu(node.capacity.cpu_millis),
                                    node.capacity.disk_mb
                                ));
                                ui.label(
                                    node.last_heartbeat_at
                                        .map(|time| time.to_string())
                                        .unwrap_or_else(|| "never".to_string()),
                                );
                                ui.end_row();
                            }
                        });
                });
        });

        ui.add_space(8.0);
        self.render_node_detail(ui, actions);

        ui.add_space(8.0);
        ui.collapsing("All allocations", |ui| {
            if let Some(error) = self.feature_error("allocations_all") {
                ui.colored_label(Color32::from_rgb(235, 120, 110), error);
            }
            let allocations = self
                .feature("allocations_all")
                .and_then(|value| value.get("allocations").and_then(Value::as_array).cloned())
                .unwrap_or_default();
            if allocations.is_empty() {
                ui.label("No allocations.");
            } else {
                ScrollArea::vertical()
                    .id_salt("all_allocations")
                    .max_height(240.0)
                    .show(ui, |ui| {
                        egui::Grid::new("all_allocations_grid")
                            .num_columns(5)
                            .striped(true)
                            .spacing([14.0, 4.0])
                            .show(ui, |ui| {
                                ui.strong("Node");
                                ui.strong("Address");
                                ui.strong("Server");
                                ui.strong("Label");
                                ui.strong("");
                                ui.end_row();
                                for allocation in allocations {
                                    let id = allocation
                                        .get("id")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string();
                                    let node = allocation
                                        .get("node_id")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string();
                                    if id.is_empty() {
                                        continue;
                                    }
                                    ui.label(short_id(&node));
                                    ui.label(format!(
                                        "{}:{}/{}",
                                        allocation.get("ip").and_then(Value::as_str).unwrap_or("-"),
                                        allocation.get("port").and_then(Value::as_u64).unwrap_or(0),
                                        allocation
                                            .get("protocol")
                                            .and_then(Value::as_str)
                                            .unwrap_or("-")
                                    ));
                                    ui.label(
                                        allocation
                                            .get("server_id")
                                            .and_then(Value::as_str)
                                            .map(short_id)
                                            .unwrap_or_else(|| "free".to_string()),
                                    );
                                    ui.label(
                                        allocation
                                            .get("label")
                                            .and_then(Value::as_str)
                                            .unwrap_or("-"),
                                    );
                                    if allocation
                                        .get("server_id")
                                        .map(Value::is_null)
                                        .unwrap_or(true)
                                        && ui.small_button("Delete").clicked()
                                    {
                                        actions.push(Action::AskDeleteAllocation {
                                            node: node.clone(),
                                            id,
                                            port: allocation
                                                .get("port")
                                                .and_then(Value::as_u64)
                                                .and_then(|port| u16::try_from(port).ok())
                                                .unwrap_or(0),
                                        });
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            }
        });
    }

    fn render_create_node_form(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.group(|ui| {
            ui.heading("Create node");
            ui.label(
                RichText::new(
                    "POST /api/v1/nodes. Capacity may be zero to let the agent report it on its first heartbeat.",
                )
                .small()
                .color(Color32::from_rgb(150, 156, 168)),
            );
            egui::Grid::new("create_node_form")
                .num_columns(2)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.add(
                        TextEdit::singleline(&mut self.create_node.name)
                            .hint_text("node-1")
                            .desired_width(300.0),
                    );
                    ui.end_row();
                    ui.label("FQDN");
                    ui.add(
                        TextEdit::singleline(&mut self.create_node.fqdn)
                            .hint_text("node1.example.com")
                            .desired_width(300.0),
                    );
                    ui.end_row();
                    ui.label("Endpoint");
                    ui.add(
                        TextEdit::singleline(&mut self.create_node.endpoint)
                            .hint_text("https://node1.example.com:8443")
                            .desired_width(340.0),
                    );
                    ui.end_row();
                    ui.label("Driver");
                    egui::ComboBox::from_id_salt("create_node_driver")
                        .selected_text(&self.create_node.driver)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.create_node.driver,
                                "docker".to_string(),
                                "docker",
                            );
                            ui.selectable_value(
                                &mut self.create_node.driver,
                                "kubernetes".to_string(),
                                "kubernetes",
                            );
                        });
                    ui.end_row();
                    ui.label("Maintenance");
                    ui.checkbox(&mut self.create_node.maintenance, "enable");
                    ui.end_row();
                    ui.label("Memory (MB)");
                    ui.add(
                        TextEdit::singleline(&mut self.create_node.memory)
                            .desired_width(120.0),
                    );
                    ui.end_row();
                    ui.label("Disk (MB)");
                    ui.add(
                        TextEdit::singleline(&mut self.create_node.disk).desired_width(120.0),
                    );
                    ui.end_row();
                    ui.label("CPU (milli-cores)");
                    ui.add(
                        TextEdit::singleline(&mut self.create_node.cpu).desired_width(120.0),
                    );
                    ui.end_row();
                    ui.label("Labels");
                    ui.vertical(|ui| {
                        kv_editor(ui, "create_node_labels", "", &mut self.create_node.labels);
                    });
                    ui.end_row();
                    ui.label("Agent token (optional)");
                    ui.add(TextEdit::singleline(&mut self.create_node.agent_token).desired_width(500.0));
                    ui.end_row();
                });
            if let Some(error) = &self.create_node.error {
                ui.colored_label(Color32::from_rgb(235, 120, 110), error);
            }
            if ui.button("Create node").clicked() {
                actions.push(Action::CreateNode);
            }
        });
    }

    fn render_node_detail(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let Some(node_id) = self.selected_node.clone() else {
            return;
        };
        let Some(node) = self
            .nodes
            .iter()
            .find(|node| node.id.to_string() == node_id)
            .cloned()
        else {
            return;
        };
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.heading(&node.name);
                ui.colored_label(
                    node_status_color(node.status),
                    node_status_label(node.status),
                );
                if node.maintenance {
                    ui.colored_label(Color32::from_rgb(230, 160, 80), "maintenance");
                }
            });
            egui::Grid::new("node_detail_grid")
                .num_columns(4)
                .spacing([18.0, 6.0])
                .show(ui, |ui| {
                    ui.label("ID");
                    ui.label(node.id.to_string());
                    ui.label("Driver");
                    ui.label(node.driver.as_str());
                    ui.end_row();
                    ui.label("FQDN");
                    ui.label(&node.fqdn);
                    ui.label("Endpoint");
                    ui.label(&node.endpoint);
                    ui.end_row();
                    ui.label("Agent version");
                    ui.label(node.agent_version.as_deref().unwrap_or("-"));
                    ui.label("Updates available");
                    ui.label(node.updates_available.to_string());
                    ui.end_row();
                    ui.label("Capacity");
                    ui.label(format!(
                        "{} MB RAM / {} / {} MB disk",
                        node.capacity.memory_mb,
                        format_cpu(node.capacity.cpu_millis),
                        node.capacity.disk_mb
                    ));
                    ui.label("Last heartbeat");
                    ui.label(
                        node.last_heartbeat_at
                            .map(|time| time.to_string())
                            .unwrap_or_else(|| "never".to_string()),
                    );
                    ui.end_row();
                });

            if self.node_form.id.as_deref() != Some(node_id.as_str()) {
                self.node_form.load(&node);
            }
            ui.add_space(10.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("Node actions");
                if ui.button("Rotate token").clicked() {
                    actions.push(Action::NodeRotate);
                }
                if ui.button("Check updates").clicked() {
                    actions.push(Action::NodeUpdatesCheck);
                }
                if ui.button("Apply updates").clicked() {
                    actions.push(Action::NodeUpdatesApply {
                        reboot: self.node_update_reboot,
                    });
                }
                ui.checkbox(&mut self.node_update_reboot, "reboot if required");
                if ui
                    .button(RichText::new("Reboot host").color(Color32::from_rgb(235, 170, 90)))
                    .clicked()
                {
                    actions.push(Action::NodeReboot);
                }
                if ui
                    .button(RichText::new("Delete node").color(Color32::from_rgb(235, 120, 110)))
                    .clicked()
                {
                    actions.push(Action::NodeDelete);
                }
            });
            if let Some(token) = self.feature("node_token") {
                if let Some(token) = token.get("agent_token").and_then(Value::as_str) {
                    ui.group(|ui| {
                        ui.strong("Rotated agent token (shown once)");
                        ui.monospace(token);
                    });
                }
            }
            ui.add_space(8.0);
            ui.collapsing("Edit node", |ui| {
                egui::Grid::new("node_edit_grid")
                    .num_columns(2)
                    .spacing([14.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.add(TextEdit::singleline(&mut self.node_form.name).desired_width(300.0));
                        ui.end_row();
                        ui.label("FQDN");
                        ui.add(TextEdit::singleline(&mut self.node_form.fqdn).desired_width(300.0));
                        ui.end_row();
                        ui.label("Endpoint");
                        ui.add(
                            TextEdit::singleline(&mut self.node_form.endpoint).desired_width(360.0),
                        );
                        ui.end_row();
                        ui.label("Driver");
                        egui::ComboBox::from_id_salt("node_edit_driver")
                            .selected_text(&self.node_form.driver)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.node_form.driver,
                                    "docker".to_string(),
                                    "docker",
                                );
                                ui.selectable_value(
                                    &mut self.node_form.driver,
                                    "kubernetes".to_string(),
                                    "kubernetes",
                                );
                            });
                        ui.end_row();
                        ui.label("Maintenance");
                        ui.checkbox(&mut self.node_form.maintenance, "enabled");
                        ui.end_row();
                        ui.label("Labels");
                        ui.vertical(|ui| {
                            kv_editor(ui, "node_form_labels", "", &mut self.node_form.labels);
                        });
                        ui.end_row();
                    });
                if let Some(error) = &self.node_form.error {
                    ui.colored_label(Color32::from_rgb(235, 120, 110), error);
                }
                if ui.button("Save node").clicked() {
                    actions.push(Action::NodeSave);
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.strong("Reported state");
                if ui.small_button("Refresh").clicked() {
                    actions.push(Action::RefreshTab);
                }
            });
            if let Some(error) = self.feature_error(&format!("node_state:{node_id}")) {
                ui.colored_label(Color32::from_rgb(235, 120, 110), error);
            }
            if let Some(state) = self.feature(&format!("node_state:{node_id}")) {
                let text = serde_json::to_string_pretty(&state).unwrap_or_else(|_| "{}".into());
                ScrollArea::vertical()
                    .id_salt("node_state_json")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(text).monospace().size(11.0)).wrap());
                    });
            }
            ui.add_space(6.0);
            ui.strong("Host updates");
            if let Some(updates) = self.feature(&format!("node_updates:{node_id}")) {
                ui.label(format!(
                    "supported={} available={} reboot_required={} checked={} applied={} error={}",
                    updates
                        .get("supported")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    updates
                        .get("updates_available")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    updates
                        .get("reboot_required")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    updates
                        .get("checked_at")
                        .and_then(Value::as_str)
                        .unwrap_or("—"),
                    updates
                        .get("applied_at")
                        .and_then(Value::as_str)
                        .unwrap_or("—"),
                    updates
                        .get("last_error")
                        .and_then(Value::as_str)
                        .unwrap_or("—"),
                ));
            } else if let Some(error) = self.feature_error(&format!("node_updates:{node_id}")) {
                ui.colored_label(Color32::from_rgb(235, 120, 110), error);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.strong(format!(
                    "Allocations ({})",
                    self.node_allocations.get(&node_id).map_or(0, Vec::len)
                ));
                if ui.small_button("Refresh").clicked() {
                    self.refresh_allocations(&node_id);
                }
            });
            let allocations = self
                .node_allocations
                .get(&node_id)
                .cloned()
                .unwrap_or_default();
            if allocations.is_empty() {
                ui.label("No allocations on this node.");
            } else {
                ScrollArea::vertical()
                    .id_salt("allocations_list")
                    .max_height(200.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("allocations_grid")
                            .num_columns(5)
                            .striped(true)
                            .spacing([14.0, 5.0])
                            .show(ui, |ui| {
                                ui.strong("Address");
                                ui.strong("Protocol");
                                ui.strong("Server");
                                ui.strong("Label");
                                ui.strong("");
                                ui.end_row();
                                for allocation in &allocations {
                                    ui.label(format!("{}:{}", allocation.ip, allocation.port));
                                    ui.label(allocation.protocol.as_str());
                                    ui.label(
                                        allocation
                                            .server_id
                                            .map(|id| short_id(&id.to_string()))
                                            .unwrap_or_else(|| "free".to_string()),
                                    );
                                    ui.label(allocation.label.as_deref().unwrap_or("-"));
                                    if ui.small_button("Delete").clicked() {
                                        actions.push(Action::AskDeleteAllocation {
                                            node: node_id.clone(),
                                            id: allocation.id.to_string(),
                                            port: allocation.port,
                                        });
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Ports");
                ui.add(
                    TextEdit::singleline(&mut self.create_alloc.ports)
                        .hint_text("25565-25575,27015")
                        .desired_width(200.0),
                );
                ui.label("IP");
                ui.add(TextEdit::singleline(&mut self.create_alloc.ip).desired_width(120.0));
                ui.label("Protocol");
                egui::ComboBox::from_id_salt("alloc_protocol")
                    .selected_text(&self.create_alloc.protocol)
                    .show_ui(ui, |ui| {
                        for protocol in ["tcp", "udp", "both"] {
                            ui.selectable_value(
                                &mut self.create_alloc.protocol,
                                protocol.to_string(),
                                protocol,
                            );
                        }
                    });
                ui.label("Label");
                ui.add(TextEdit::singleline(&mut self.create_alloc.label).desired_width(120.0));
                if ui.button("Create allocations").clicked() {
                    actions.push(Action::CreateAllocation);
                }
            });
            if let Some(error) = &self.create_alloc.error {
                ui.colored_label(Color32::from_rgb(235, 120, 110), error);
            }
        });
    }

    fn page_templates(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Templates");
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshTab);
            }
            ui.label(
                RichText::new(format!("{} template(s)", self.templates.len()))
                    .small()
                    .color(Color32::from_rgb(150, 156, 168)),
            );
        });

        ui.group(|ui| {
            ui.strong("Template list");
            if self.templates.is_empty() {
                ui.label(
                    RichText::new("No templates imported yet.")
                        .color(Color32::from_rgb(150, 156, 168)),
                );
            } else {
                let templates = self.templates.clone();
                let selected = self.selected_template.clone();
                ScrollArea::vertical()
                    .id_salt("templates_list")
                    .max_height(230.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("templates_grid")
                            .num_columns(5)
                            .striped(true)
                            .spacing([14.0, 6.0])
                            .show(ui, |ui| {
                                ui.strong("Name");
                                ui.strong("Slug");
                                ui.strong("Version");
                                ui.strong("Channel");
                                ui.strong("Auto update");
                                ui.end_row();
                                for template in &templates {
                                    let id = template.id.to_string();
                                    if ui
                                        .selectable_label(
                                            selected.as_deref() == Some(id.as_str()),
                                            &template.name,
                                        )
                                        .on_hover_text(format!(
                                            "id: {}\n{}",
                                            id, template.description
                                        ))
                                        .clicked()
                                    {
                                        actions.push(Action::SelectTemplate(id.clone()));
                                    }
                                    ui.label(&template.slug);
                                    ui.label(
                                        template
                                            .latest_version
                                            .as_ref()
                                            .map(|version| version.version.clone())
                                            .unwrap_or_else(|| "unpublished".to_string()),
                                    );
                                    ui.label(
                                        template
                                            .latest_version
                                            .as_ref()
                                            .map(|version| format!("{:?}", version.channel))
                                            .unwrap_or_else(|| "-".to_string()),
                                    );
                                    ui.label(if template.auto_update.enabled {
                                        format!("{:?}", template.auto_update.channel)
                                    } else {
                                        "off".to_string()
                                    });
                                    ui.end_row();
                                }
                            });
                    });
            }
        });

        ui.add_space(8.0);
        if let Some(template) = self.template_detail.clone() {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.heading(&template.name);
                    if ui
                        .button(RichText::new("Delete").color(Color32::from_rgb(235, 120, 110)))
                        .clicked()
                    {
                        actions.push(Action::AskDeleteTemplate {
                            id: template.id.to_string(),
                            name: template.name.clone(),
                        });
                    }
                });
                egui::Grid::new("template_detail_grid")
                    .num_columns(4)
                    .spacing([18.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Slug");
                        ui.label(&template.slug);
                        ui.label("ID");
                        ui.label(template.id.to_string());
                        ui.end_row();
                        ui.label("Description");
                        ui.add(egui::Label::new(&template.description).wrap());
                        ui.label("Auto update");
                        ui.label(if template.auto_update.enabled {
                            format!("{:?}", template.auto_update.channel)
                        } else {
                            "disabled".to_string()
                        });
                        ui.end_row();
                        if let Some(version) = &template.latest_version {
                            ui.label("Latest version");
                            ui.label(format!("{} ({:?})", version.version, version.channel));
                            ui.label("Digest");
                            truncate_label(ui, &version.digest, 260.0);
                            ui.end_row();
                            ui.label("Published");
                            ui.label(version.created_at.to_string());
                            ui.label("Signature");
                            ui.label(version.signature.as_deref().unwrap_or("unsigned"));
                            ui.end_row();
                            ui.label("Image(s)");
                            ui.label(
                                version
                                    .template
                                    .runtime
                                    .images
                                    .iter()
                                    .map(|(label, image)| format!("{label}={image}"))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            );
                            ui.label("Startup");
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&version.template.runtime.startup)
                                        .monospace()
                                        .size(12.0),
                                )
                                .truncate(),
                            );
                            ui.end_row();
                        }
                    });
                egui::CollapsingHeader::new("Full template JSON")
                    .id_salt("template_json")
                    .show(ui, |ui| {
                        let pretty = serde_json::to_string_pretty(&template)
                            .unwrap_or_else(|_| "{}".to_string());
                        ui.add(
                            egui::Label::new(RichText::new(pretty).monospace().size(11.0)).wrap(),
                        );
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.strong("Auto-update");
                    ui.checkbox(&mut self.template_auto_enabled, "enabled");
                    egui::ComboBox::from_id_salt("template_auto_channel")
                        .selected_text(&self.template_auto_channel)
                        .show_ui(ui, |ui| {
                            for channel in ["stable", "beta", "dev"] {
                                ui.selectable_value(
                                    &mut self.template_auto_channel,
                                    channel.to_string(),
                                    channel,
                                );
                            }
                        });
                    if ui.button("Save auto-update").clicked() {
                        actions.push(Action::TemplateAutoUpdate);
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.strong("Versions");
                    if ui.small_button("Refresh").clicked() {
                        actions.push(Action::RefreshTab);
                    }
                    if ui.button("Prepare publish form").clicked() {
                        actions.push(Action::TemplateEditPublish);
                    }
                });
                if let Some(error) =
                    self.feature_error(&format!("template_versions:{}", template.id))
                {
                    ui.colored_label(Color32::from_rgb(235, 120, 110), error);
                }
                if let Some(value) = self.feature(&format!("template_versions:{}", template.id)) {
                    let versions = value
                        .get("versions")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    if versions.is_empty() {
                        ui.label("No versions.");
                    } else {
                        egui::Grid::new("template_versions_grid")
                            .num_columns(4)
                            .striped(true)
                            .spacing([12.0, 4.0])
                            .show(ui, |ui| {
                                ui.strong("Version");
                                ui.strong("Channel");
                                ui.strong("Created");
                                ui.strong("Digest");
                                ui.end_row();
                                for version in versions {
                                    ui.monospace(
                                        version
                                            .get("version")
                                            .and_then(Value::as_str)
                                            .unwrap_or("-"),
                                    );
                                    ui.label(
                                        version
                                            .get("channel")
                                            .and_then(Value::as_str)
                                            .unwrap_or("-"),
                                    );
                                    ui.label(
                                        version
                                            .get("created_at")
                                            .and_then(Value::as_str)
                                            .unwrap_or("-"),
                                    );
                                    truncate_label(
                                        ui,
                                        version
                                            .get("digest")
                                            .and_then(Value::as_str)
                                            .unwrap_or("-"),
                                        220.0,
                                    );
                                    ui.end_row();
                                }
                            });
                    }
                }
                ui.collapsing("Publish version", |ui| {
                    ui.label("Template JSON");
                    ui.add(
                        TextEdit::multiline(&mut self.publish_form.template)
                            .code_editor()
                            .desired_rows(12)
                            .desired_width(f32::INFINITY),
                    );
                    egui::Grid::new("publish_grid")
                        .num_columns(2)
                        .spacing([14.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("Channel");
                            egui::ComboBox::from_id_salt("publish_channel")
                                .selected_text(&self.publish_form.channel)
                                .show_ui(ui, |ui| {
                                    for channel in ["stable", "beta", "dev"] {
                                        ui.selectable_value(
                                            &mut self.publish_form.channel,
                                            channel.to_string(),
                                            channel,
                                        );
                                    }
                                });
                            ui.end_row();
                            ui.label("Notes");
                            ui.add(
                                TextEdit::singleline(&mut self.publish_form.notes)
                                    .desired_width(420.0),
                            );
                            ui.end_row();
                            ui.label("Signature (optional)");
                            ui.add(
                                TextEdit::singleline(&mut self.publish_form.signature)
                                    .desired_width(520.0),
                            );
                            ui.end_row();
                            ui.label("Public key hex (optional)");
                            ui.add(
                                TextEdit::singleline(&mut self.publish_form.public_key)
                                    .desired_width(520.0),
                            );
                            ui.end_row();
                        });
                    if let Some(error) = &self.publish_form.error {
                        ui.colored_label(Color32::from_rgb(235, 120, 110), error);
                    }
                    if ui.button("Publish version").clicked() {
                        actions.push(Action::TemplatePublish);
                    }
                });
            });
        }

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.heading("Import Pterodactyl egg");
            ui.label(
                RichText::new(
                    "POST /api/v1/templates with {\"egg\": <egg or full template>, \"version\": <optional>}",
                )
                .small()
                .color(Color32::from_rgb(150, 156, 168)),
            );
            egui::Grid::new("egg_meta_grid").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                ui.label("Version (blank = template meta.version)");
                ui.add(TextEdit::singleline(&mut self.egg_version).desired_width(120.0));
                ui.end_row();
                ui.label("Slug (optional)");
                ui.add(TextEdit::singleline(&mut self.egg_slug).desired_width(180.0));
                ui.end_row();
                ui.label("Channel");
                egui::ComboBox::from_id_salt("egg_channel").selected_text(&self.egg_channel).show_ui(ui, |ui| {
                    for channel in ["stable", "beta", "dev"] {
                        ui.selectable_value(&mut self.egg_channel, channel.to_string(), channel);
                    }
                });
                ui.end_row();
                ui.label("Signature (optional)");
                ui.add(TextEdit::singleline(&mut self.egg_signature).desired_width(520.0));
                ui.end_row();
                ui.label("Public key hex (optional)");
                ui.add(TextEdit::singleline(&mut self.egg_public_key).desired_width(520.0));
                ui.end_row();
            });
            ui.add(
                TextEdit::multiline(&mut self.egg_text)
                    .hint_text("Paste the egg JSON here")
                    .code_editor()
                    .desired_rows(10)
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.egg_text.trim().is_empty(),
                        egui::Button::new(RichText::new("Preview").strong()),
                    )
                    .clicked()
                {
                    actions.push(Action::PreviewEgg);
                }
                if ui
                    .add_enabled(
                        !self.egg_text.trim().is_empty(),
                        egui::Button::new(RichText::new("Import egg").strong()),
                    )
                    .clicked()
                {
                    actions.push(Action::ImportEgg);
                }
            });
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.heading("Signed template feed");
            ui.label(
                RichText::new("POST /api/v1/template-feeds with feed, signature and public_key.")
                    .small()
                    .color(Color32::from_rgb(150, 156, 168)),
            );
            ui.label("Feed JSON");
            ui.add(
                TextEdit::multiline(&mut self.feed_form.feed)
                    .code_editor()
                    .desired_rows(10)
                    .desired_width(f32::INFINITY),
            );
            egui::Grid::new("feed_grid")
                .num_columns(2)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Signature");
                    ui.add(
                        TextEdit::singleline(&mut self.feed_form.signature).desired_width(520.0),
                    );
                    ui.end_row();
                    ui.label("Public key hex");
                    ui.add(
                        TextEdit::singleline(&mut self.feed_form.public_key).desired_width(520.0),
                    );
                    ui.end_row();
                    ui.label("Create missing");
                    ui.checkbox(&mut self.feed_form.create_missing, "yes");
                    ui.end_row();
                });
            if let Some(error) = &self.feed_form.error {
                ui.colored_label(Color32::from_rgb(235, 120, 110), error);
            }
            if ui.button("Sync feed").clicked() {
                actions.push(Action::TemplateFeed);
            }
        });
    }

    fn page_databases(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Databases");
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshTab);
            }
            if ui
                .button(if self.create_db_host.show {
                    "Hide host form"
                } else {
                    "Add database host"
                })
                .clicked()
            {
                actions.push(Action::ToggleCreateDbHost);
            }
        });

        if self.create_db_host.show {
            ui.group(|ui| {
                ui.strong("Create database host");
                egui::Grid::new("create_db_host_grid")
                    .num_columns(2)
                    .spacing([14.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.add(
                            TextEdit::singleline(&mut self.create_db_host.name)
                                .desired_width(260.0),
                        );
                        ui.end_row();
                        ui.label("Host");
                        ui.add(
                            TextEdit::singleline(&mut self.create_db_host.host)
                                .hint_text("db.internal")
                                .desired_width(260.0),
                        );
                        ui.end_row();
                        ui.label("Port");
                        ui.add(
                            TextEdit::singleline(&mut self.create_db_host.port).desired_width(80.0),
                        );
                        ui.end_row();
                        ui.label("Admin user");
                        ui.add(
                            TextEdit::singleline(&mut self.create_db_host.admin_user)
                                .desired_width(220.0),
                        );
                        ui.end_row();
                        ui.label("Admin password");
                        ui.add(
                            TextEdit::singleline(&mut self.create_db_host.admin_password)
                                .password(true)
                                .desired_width(260.0),
                        );
                        ui.end_row();
                        ui.label("Max databases (0 = unlimited)");
                        ui.add(
                            TextEdit::singleline(&mut self.create_db_host.max_databases)
                                .desired_width(100.0),
                        );
                        ui.end_row();
                    });
                if let Some(error) = &self.create_db_host.error {
                    ui.colored_label(Color32::from_rgb(235, 120, 110), error);
                }
                if ui.button("Create host").clicked() {
                    actions.push(Action::CreateDbHost);
                }
            });
        }

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.strong("Database hosts");
            if self.db_hosts.is_empty() {
                ui.label(
                    RichText::new("No database hosts configured.")
                        .color(Color32::from_rgb(150, 156, 168)),
                );
            } else {
                let hosts = self.db_hosts.clone();
                ScrollArea::vertical()
                    .id_salt("db_hosts_list")
                    .max_height(220.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("db_hosts_grid")
                            .num_columns(5)
                            .striped(true)
                            .spacing([14.0, 5.0])
                            .show(ui, |ui| {
                                ui.strong("Name");
                                ui.strong("Address");
                                ui.strong("Admin");
                                ui.strong("Max DBs");
                                ui.strong("");
                                ui.end_row();
                                for host in &hosts {
                                    let host_id = host.id.to_string();
                                    if ui
                                        .selectable_label(
                                            self.db_host_view.as_deref() == Some(host_id.as_str()),
                                            &host.name,
                                        )
                                        .clicked()
                                    {
                                        actions.push(Action::ShowDbHost(host_id.clone()));
                                    }
                                    ui.label(format!("{}:{}", host.host, host.port));
                                    ui.label(&host.admin_user);
                                    ui.label(if host.max_databases == 0 {
                                        "unlimited".to_string()
                                    } else {
                                        host.max_databases.to_string()
                                    });
                                    ui.horizontal(|ui| {
                                        if ui.small_button("Test").clicked() {
                                            actions.push(Action::TestDbHost(host.id.to_string()));
                                        }
                                        if ui.small_button("Delete").clicked() {
                                            actions.push(Action::AskDeleteDbHost {
                                                id: host.id.to_string(),
                                                name: host.name.clone(),
                                            });
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                    });
            }
        });

        if let Some(id) = self.db_host_view.clone() {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.strong("Database host detail");
                    if ui.small_button("Close").clicked() {
                        actions.push(Action::CloseDbHost);
                    }
                });
                if let Some(error) = self.feature_error(&format!("db_host:{id}")) {
                    ui.colored_label(Color32::from_rgb(235, 120, 110), error);
                }
                if let Some(value) = self.feature(&format!("db_host:{id}")) {
                    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into());
                    ui.add(egui::Label::new(RichText::new(text).monospace().size(11.0)).wrap());
                }
            });
        }

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.strong("Server databases");
            let Some(server) = self.selected_server.clone() else {
                ui.label(
                    RichText::new("Select a server in Servers or Console first.")
                        .color(Color32::from_rgb(150, 156, 168)),
                );
                return;
            };
            ui.label(
                RichText::new(format!(
                    "GET /api/v1/servers/{}/databases",
                    short_id(&server)
                ))
                .small()
                .color(Color32::from_rgb(150, 156, 168)),
            );
            if self.server_dbs.is_empty() {
                ui.label("No databases for this server.");
            } else {
                for database in &self.server_dbs {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} (user {}, remote {})",
                                database.database_name, database.username, database.remote
                            ))
                            .monospace(),
                        );
                        if ui.small_button("Delete").clicked() {
                            actions.push(Action::AskDeleteDb {
                                id: database.id.to_string(),
                                name: database.database_name.clone(),
                            });
                        }
                    });
                }
            }
            ui.horizontal(|ui| {
                let mut host_id = self.create_server_db.host_id.clone();
                egui::ComboBox::from_id_salt("create_db_host_select")
                    .selected_text(
                        host_id
                            .as_ref()
                            .and_then(|selected| {
                                self.db_hosts
                                    .iter()
                                    .find(|host| host.id.to_string() == *selected)
                                    .map(|host| host.name.clone())
                            })
                            .unwrap_or_else(|| "auto-select host".to_string()),
                    )
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut host_id, None, "auto-select host");
                        for host in &self.db_hosts {
                            let id = host.id.to_string();
                            ui.selectable_value(
                                &mut host_id,
                                Some(id.clone()),
                                format!("{} ({})", host.name, host.host),
                            );
                        }
                    });
                self.create_server_db.host_id = host_id;
                ui.label("Remote");
                ui.add(
                    TextEdit::singleline(&mut self.create_server_db.remote)
                        .hint_text("%")
                        .desired_width(100.0),
                );
                if ui.button("Create database").clicked() {
                    actions.push(Action::CreateServerDb);
                }
            });
            if let Some(error) = &self.create_server_db.error {
                ui.colored_label(Color32::from_rgb(235, 120, 110), error);
            }
            if let Some(db) = &self.created_db {
                ui.add_space(6.0);
                ui.group(|ui| {
                    ui.colored_label(
                        Color32::from_rgb(95, 200, 120),
                        format!(
                            "Created database '{}' with credentials (shown once)",
                            db.database
                        ),
                    );
                    egui::Grid::new("created_db_grid")
                        .num_columns(2)
                        .spacing([14.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Host");
                            ui.label(format!("{}:{}", db.host, db.port));
                            ui.end_row();
                            ui.label("Database");
                            ui.label(&db.database);
                            ui.end_row();
                            ui.label("Username");
                            ui.label(&db.username);
                            ui.end_row();
                            ui.label("Remote");
                            ui.label(&db.remote);
                            ui.end_row();
                            ui.label("Password");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(if self.show_created_password {
                                            db.password.as_str()
                                        } else {
                                            "••••••••"
                                        })
                                        .monospace(),
                                    )
                                    .wrap(),
                                );
                                ui.checkbox(&mut self.show_created_password, "Show");
                                if ui.small_button("Copy").clicked() {
                                    ui.ctx().copy_text(db.password.clone());
                                }
                            });
                            ui.end_row();
                        });
                    if ui.small_button("Dismiss").clicked() {
                        actions.push(Action::ClearCreatedDb);
                    }
                });
            }
        });
    }

    fn page_backups(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Backups");
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshTab);
            }
        });
        let Some(server) = self.selected_server.clone() else {
            ui.add_space(20.0);
            ui.label(
                RichText::new("Select a server to list its backups.")
                    .color(Color32::from_rgb(150, 156, 168)),
            );
            return;
        };
        ui.label(
            RichText::new(format!(
                "Server {} ({})",
                self.server_combo_text(Some(&server)),
                short_id(&server)
            ))
            .small()
            .color(Color32::from_rgb(150, 156, 168)),
        );

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut self.backup_name)
                    .hint_text("backup name (optional)")
                    .desired_width(260.0),
            );
            if ui.button("Create backup").clicked() {
                actions.push(Action::CreateBackup);
            }
            ui.separator();
            ui.label("Download directory");
            ui.add(
                TextEdit::singleline(&mut self.download_dir)
                    .desired_width(280.0)
                    .font(TextStyle::Monospace),
            );
        });
        ui.label(
            RichText::new(
                "Backups are created asynchronously; refresh to watch state changes. Downloads are written by the API worker.",
            )
            .small()
            .color(Color32::from_rgb(150, 156, 168)),
        );

        ui.add_space(8.0);
        if self.backups.is_empty() {
            ui.label(
                RichText::new("No backups for this server.")
                    .color(Color32::from_rgb(150, 156, 168)),
            );
        } else {
            let backups = self.backups.clone();
            ScrollArea::vertical()
                .id_salt("backups_list")
                .max_height(300.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("backups_grid")
                        .num_columns(6)
                        .striped(true)
                        .spacing([14.0, 6.0])
                        .show(ui, |ui| {
                            ui.strong("Name");
                            ui.strong("State");
                            ui.strong("Size");
                            ui.strong("Created");
                            ui.strong("Completed");
                            ui.strong("Actions");
                            ui.end_row();
                            for backup in &backups {
                                ui.label(&backup.name)
                                    .on_hover_text(format!("id: {}", backup.id));
                                ui.colored_label(
                                    backup_color(backup.state),
                                    backup_label(backup.state),
                                );
                                ui.label(format_bytes(backup.bytes));
                                ui.label(backup.created_at.to_string());
                                ui.label(
                                    backup
                                        .completed_at
                                        .map(|time| time.to_string())
                                        .unwrap_or_else(|| "-".to_string()),
                                );
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_enabled(
                                            backup.state == BackupState::Completed,
                                            egui::Button::new("Download").small(),
                                        )
                                        .clicked()
                                    {
                                        actions.push(Action::DownloadBackup(backup.id.to_string()));
                                    }
                                    if ui
                                        .add_enabled(
                                            backup.state == BackupState::Completed,
                                            egui::Button::new("Restore").small(),
                                        )
                                        .clicked()
                                    {
                                        actions.push(Action::AskRestoreBackup {
                                            id: backup.id.to_string(),
                                            name: backup.name.clone(),
                                        });
                                    }
                                    if ui.small_button("Delete").clicked() {
                                        actions.push(Action::AskDeleteBackup {
                                            id: backup.id.to_string(),
                                            name: backup.name.clone(),
                                        });
                                    }
                                });
                                ui.end_row();
                            }
                        });
                });
        }
    }

    fn page_audit(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Audit");
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshTab);
            }
            ui.label("Limit");
            ui.add(egui::DragValue::new(&mut self.audit_limit).range(1..=1000));
            if ui.button("Reload").clicked() {
                actions.push(Action::RefreshTab);
            }
            ui.label(
                RichText::new(format!("{} event(s), newest first", self.audit.len()))
                    .small()
                    .color(Color32::from_rgb(150, 156, 168)),
            );
        });
        ui.add_space(8.0);
        if self.audit.is_empty() {
            ui.label(
                RichText::new("No audit events visible to this key.")
                    .color(Color32::from_rgb(150, 156, 168)),
            );
            return;
        }
        let events = self.audit.clone();
        ScrollArea::vertical()
            .id_salt("audit_list")
            .max_height(560.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("audit_grid")
                    .num_columns(6)
                    .striped(true)
                    .spacing([14.0, 5.0])
                    .show(ui, |ui| {
                        ui.strong("Time");
                        ui.strong("Action");
                        ui.strong("Resource");
                        ui.strong("Resource ID");
                        ui.strong("Key");
                        ui.strong("Metadata");
                        ui.end_row();
                        for event in &events {
                            ui.label(event.created_at.to_string());
                            ui.label(audit_action_label(event.action));
                            ui.label(&event.resource);
                            ui.label(event.resource_id.as_deref().unwrap_or("-"));
                            ui.label(
                                event
                                    .api_key_id
                                    .map(|id| short_id(&id.to_string()))
                                    .unwrap_or_else(|| "-".to_string()),
                            );
                            let metadata = serde_json::to_string(&event.metadata)
                                .unwrap_or_else(|_| "{}".to_string());
                            truncate_label(ui, &metadata, 240.0);
                            ui.end_row();
                        }
                    });
            });
    }

    fn page_proxy(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Reverse proxy");
            if ui.button("Refresh").clicked() {
                actions.push(Action::RefreshTab);
            }
            if ui.button("Apply all routes").clicked() {
                actions.push(Action::ProxyReconcile);
            }
        });
        if let Some(status) = self.feature("proxy_status") {
            ui.horizontal(|ui| {
                if status
                    .get("configured")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    if status
                        .get("reachable")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        ui.colored_label(Color32::from_rgb(95, 200, 120), "Caddy reachable");
                    } else {
                        ui.colored_label(Color32::from_rgb(235, 120, 110), "Caddy unreachable");
                    }
                } else {
                    ui.colored_label(Color32::from_rgb(230, 180, 90), "Caddy not configured");
                }
                ui.label(
                    status
                        .get("admin_url")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                );
                if let Some(error) = status.get("error").and_then(Value::as_str) {
                    ui.colored_label(Color32::from_rgb(235, 120, 110), error);
                }
            });
        } else if let Some(error) = self.feature_error("proxy_status") {
            ui.colored_label(Color32::from_rgb(235, 120, 110), error);
        }
        if let Some(error) = &self.proxy_error {
            ui.add_space(6.0);
            ui.colored_label(Color32::from_rgb(230, 180, 90), error);
        }
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.strong(if self.proxy_edit.id.is_some() {
                    "Edit route"
                } else {
                    "Create route"
                });
                if self.proxy_edit.id.is_some() && ui.button("New route").clicked() {
                    actions.push(Action::ProxyNew);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Domain");
                ui.add(
                    TextEdit::singleline(&mut self.proxy_edit.domain)
                        .hint_text("game.example.com")
                        .desired_width(220.0),
                );
                ui.label("Upstream");
                ui.add(
                    TextEdit::singleline(&mut self.proxy_edit.upstream)
                        .desired_width(250.0)
                        .font(TextStyle::Monospace),
                );
                ui.label("TLS");
                egui::ComboBox::from_id_salt("proxy_edit_tls")
                    .selected_text(&self.proxy_edit.tls)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.proxy_edit.tls, "auto".to_string(), "auto");
                        ui.selectable_value(
                            &mut self.proxy_edit.tls,
                            "internal".to_string(),
                            "internal",
                        );
                    });
                ui.checkbox(&mut self.proxy_edit.enabled, "enabled");
            });
            if let Some(error) = &self.proxy_edit.error {
                ui.colored_label(Color32::from_rgb(235, 120, 110), error);
            }
            if ui
                .button(if self.proxy_edit.id.is_some() {
                    "Save route"
                } else {
                    "Create route"
                })
                .clicked()
            {
                actions.push(Action::ProxySave);
            }
        });
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.strong("Routes");
            if self.proxy_routes.is_empty() {
                ui.label(
                    RichText::new("No proxy routes returned.")
                        .color(Color32::from_rgb(150, 156, 168)),
                );
            } else {
                let routes = self.proxy_routes.clone();
                ScrollArea::vertical()
                    .id_salt("proxy_routes_list")
                    .max_height(320.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("proxy_routes_grid")
                            .num_columns(6)
                            .striped(true)
                            .spacing([14.0, 5.0])
                            .show(ui, |ui| {
                                ui.strong("ID");
                                ui.strong("Domain");
                                ui.strong("Upstream");
                                ui.strong("TLS");
                                ui.strong("State");
                                ui.strong("");
                                ui.end_row();
                                for route in &routes {
                                    let id = route_field(route, "id")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string();
                                    let domain = route_field(route, "domain")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string();
                                    let upstream = route_field(route, "upstream")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string();
                                    let tls = route_field(route, "tls")
                                        .map(|value| value.as_str().unwrap_or("auto").to_string())
                                        .unwrap_or_else(|| "-".to_string());
                                    let enabled = route_field(route, "enabled")
                                        .and_then(Value::as_bool)
                                        .unwrap_or(true);
                                    ui.label(if id.is_empty() {
                                        "-".to_string()
                                    } else {
                                        short_id(&id)
                                    });
                                    ui.label(if domain.is_empty() { "-" } else { &domain });
                                    ui.label(if upstream.is_empty() { "-" } else { &upstream });
                                    ui.label(tls);
                                    ui.label(if enabled { "enabled" } else { "disabled" });
                                    ui.horizontal(|ui| {
                                        if !id.is_empty() && ui.small_button("Edit").clicked() {
                                            actions.push(Action::ProxyEdit(id.clone()));
                                        }
                                        if !id.is_empty() && ui.small_button("Delete").clicked() {
                                            actions.push(Action::AskDeleteProxy {
                                                id: id.clone(),
                                                domain: domain.clone(),
                                            });
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                    });
            }
            egui::CollapsingHeader::new("Raw route JSON")
                .id_salt("proxy_raw_json")
                .show(ui, |ui| {
                    let pretty = serde_json::to_string_pretty(&self.proxy_routes)
                        .unwrap_or_else(|_| "[]".to_string());
                    ui.add(egui::Label::new(RichText::new(pretty).monospace().size(11.0)).wrap());
                });
        });
    }

    fn page_raw(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.heading("Raw API");
        ui.label(
            RichText::new(
                "Escape hatch for endpoints not surfaced elsewhere. Ask for any path under /api/v1.",
            )
            .small()
            .color(Color32::from_rgb(150, 156, 168)),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("raw_method")
                .selected_text(&self.raw_method)
                .width(100.0)
                .show_ui(ui, |ui| {
                    for method in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
                        ui.selectable_value(&mut self.raw_method, method.to_string(), method);
                    }
                });
            ui.add(
                TextEdit::singleline(&mut self.raw_path)
                    .font(TextStyle::Monospace)
                    .hint_text("/api/v1/me")
                    .desired_width(520.0),
            );
            if ui.button(RichText::new("Send").strong()).clicked() {
                actions.push(Action::SendRaw);
            }
        });
        ui.label("JSON body (optional)");
        ui.add(
            TextEdit::multiline(&mut self.raw_body)
                .code_editor()
                .desired_rows(8)
                .desired_width(f32::INFINITY)
                .hint_text("{\n  \"signal\": \"start\"\n}"),
        );
        if let Some(error) = &self.raw_error {
            ui.colored_label(Color32::from_rgb(235, 120, 110), error);
        }
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.strong("Response");
                if let Some(view) = &self.raw_view {
                    let color = if (200..300).contains(&view.status) {
                        Color32::from_rgb(95, 200, 120)
                    } else {
                        Color32::from_rgb(235, 120, 110)
                    };
                    ui.colored_label(color, format!("HTTP {}", view.status));
                    if let Some(content_type) = &view.content_type {
                        ui.label(
                            RichText::new(content_type)
                                .small()
                                .color(Color32::from_rgb(150, 156, 168)),
                        );
                    }
                    if ui.small_button("Copy body").clicked() {
                        ui.ctx().copy_text(view.body.clone());
                    }
                }
            });
            if let Some(view) = &self.raw_view {
                ScrollArea::vertical()
                    .id_salt("raw_response")
                    .max_height(460.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&view.body).monospace().size(12.0))
                                .wrap(),
                        );
                    });
            } else {
                ui.label(RichText::new("No response yet.").color(Color32::from_rgb(150, 156, 168)));
            }
        });
    }

    fn server_combo_text(&self, id: Option<&str>) -> String {
        id.and_then(|id| {
            self.servers
                .iter()
                .find(|server| server.id.to_string() == id)
                .map(|server| format!("{} ({})", server.name, short_id(id)))
        })
        .unwrap_or_else(|| "Select server".to_string())
    }

    fn node_display_name(&self, id: &str) -> String {
        self.nodes
            .iter()
            .find(|node| node.id.to_string() == id)
            .map(|node| node.name.clone())
            .unwrap_or_else(|| short_id(id))
    }

    fn template_display_name(&self, id: &str) -> String {
        self.templates
            .iter()
            .find(|template| template.id.to_string() == id)
            .map(|template| template.name.clone())
            .unwrap_or_else(|| short_id(id))
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_ui(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.console_worker.pause();
        self.save_prefs();
    }
}

// ------------------------------------------------------------------ helpers

fn install_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(18, 20, 25);
    visuals.window_fill = Color32::from_rgb(24, 27, 33);
    visuals.extreme_bg_color = Color32::from_rgb(8, 10, 13);
    visuals.faint_bg_color = Color32::from_rgb(28, 31, 38);
    visuals.selection.bg_fill = Color32::from_rgb(44, 96, 150);
    visuals.hyperlink_color = Color32::from_rgb(105, 170, 240);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(24, 27, 33);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(38, 42, 50);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(50, 56, 66);
    visuals.widgets.active.bg_fill = Color32::from_rgb(58, 66, 80);
    style.visuals = visuals;
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(8.0, 4.0);
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(13.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(14.0));
    ctx.set_style(style);
}

fn overview_card(
    ui: &mut egui::Ui,
    title: &str,
    count: usize,
    subtitle: String,
    tab: Tab,
    actions: &mut Vec<Action>,
) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_min_width(180.0);
            ui.label(
                RichText::new(title)
                    .small()
                    .color(Color32::from_rgb(150, 156, 168)),
            );
            ui.label(RichText::new(count.to_string()).size(30.0).strong());
            ui.label(
                RichText::new(subtitle)
                    .small()
                    .color(Color32::from_rgb(140, 146, 158)),
            );
            if ui
                .small_button(format!("Open {}", title.to_lowercase()))
                .clicked()
            {
                actions.push(Action::SwitchTab(tab));
            }
        });
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn state_label(state: ServerState) -> &'static str {
    match state {
        ServerState::Installing => "installing",
        ServerState::Offline => "offline",
        ServerState::Starting => "starting",
        ServerState::Running => "running",
        ServerState::Stopping => "stopping",
        ServerState::Suspended => "suspended",
        ServerState::Pending => "pending",
        ServerState::Unknown => "unknown",
    }
}

fn state_color(state: ServerState) -> Color32 {
    match state {
        ServerState::Running => Color32::from_rgb(95, 200, 120),
        ServerState::Starting | ServerState::Installing => Color32::from_rgb(230, 180, 90),
        ServerState::Stopping => Color32::from_rgb(230, 150, 80),
        ServerState::Suspended => Color32::from_rgb(220, 120, 160),
        ServerState::Pending => Color32::from_rgb(110, 170, 240),
        ServerState::Offline | ServerState::Unknown => Color32::from_rgb(150, 156, 168),
    }
}

fn node_status_label(status: NodeStatus) -> &'static str {
    match status {
        NodeStatus::Pending => "pending",
        NodeStatus::Online => "online",
        NodeStatus::Degraded => "degraded",
        NodeStatus::Offline => "offline",
        NodeStatus::Maintenance => "maintenance",
    }
}

fn node_status_color(status: NodeStatus) -> Color32 {
    match status {
        NodeStatus::Online => Color32::from_rgb(95, 200, 120),
        NodeStatus::Degraded => Color32::from_rgb(230, 180, 90),
        NodeStatus::Maintenance => Color32::from_rgb(220, 140, 90),
        NodeStatus::Pending => Color32::from_rgb(110, 170, 240),
        NodeStatus::Offline => Color32::from_rgb(220, 105, 105),
    }
}

fn backup_label(state: BackupState) -> &'static str {
    match state {
        BackupState::Creating => "creating",
        BackupState::Completed => "completed",
        BackupState::Failed => "failed",
        BackupState::Restoring => "restoring",
    }
}

fn backup_color(state: BackupState) -> Color32 {
    match state {
        BackupState::Completed => Color32::from_rgb(95, 200, 120),
        BackupState::Creating | BackupState::Restoring => Color32::from_rgb(230, 180, 90),
        BackupState::Failed => Color32::from_rgb(235, 120, 110),
    }
}

fn audit_action_label(action: ignite_containers_core::models::AuditAction) -> String {
    serde_json::to_value(action)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{action:?}").to_lowercase())
}

fn truncate_label(ui: &mut egui::Ui, text: &str, width: f32) {
    ui.add_sized(
        [width, ui.text_style_height(&TextStyle::Body)],
        egui::Label::new(text).truncate(),
    )
    .on_hover_text(text);
}

fn format_cpu(millis: u64) -> String {
    if millis.is_multiple_of(1000) {
        format!("{} core(s)", millis / 1000)
    } else {
        format!("{:.2} cores", millis as f64 / 1000.0)
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.1} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m {}s", seconds % 60)
    }
}

fn clean_object(mut value: Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        map.retain(|key, _| !key.trim().is_empty());
    }
    value
}

fn kv_editor(ui: &mut egui::Ui, id: &str, title: &str, json_text: &mut String) {
    if !title.is_empty() {
        ui.label(title);
    }
    let mut pairs: Vec<(String, String)> = serde_json::from_str::<Value>(json_text)
        .ok()
        .and_then(|value| match value {
            Value::Object(map) => Some(
                map.into_iter()
                    .map(|(key, value)| {
                        let value = value
                            .as_str()
                            .map_or_else(|| value.to_string(), str::to_string);
                        (key, value)
                    })
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    let mut remove = None;
    egui::Grid::new(id)
        .num_columns(3)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (index, (key, value)) in pairs.iter_mut().enumerate() {
                ui.add(
                    TextEdit::singleline(key)
                        .hint_text("key")
                        .desired_width(180.0),
                );
                ui.add(
                    TextEdit::singleline(value)
                        .hint_text("value")
                        .desired_width(320.0),
                );
                if ui.small_button("×").clicked() {
                    remove = Some(index);
                }
                ui.end_row();
            }
        });
    if let Some(index) = remove {
        pairs.remove(index);
    }
    if ui.small_button("+ Add row").clicked() {
        pairs.push((String::new(), String::new()));
    }
    let map: Map<String, Value> = pairs
        .into_iter()
        .map(|(key, value)| (key, json!(value)))
        .collect();
    if let Ok(text) = serde_json::to_string_pretty(&map) {
        *json_text = text;
    }
}

fn parse_json_object(value: &str, label: &str) -> Result<Value, String> {
    if value.trim().is_empty() {
        return Ok(json!({}));
    }
    match serde_json::from_str::<Value>(value) {
        Ok(value) if value.is_object() => Ok(value),
        Ok(_) => Err(format!("{label} must be a JSON object")),
        Err(error) => Err(format!("{label} JSON: {error}")),
    }
}

fn parse_u64(value: &str, label: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("{label} must be a non-negative integer"))
}

fn looks_like_semver(value: &str) -> bool {
    let core = value.trim();
    if core.is_empty() {
        return false;
    }
    let core = core.split(['-', '+']).next().unwrap_or(core);
    let mut parts = core.split('.');
    let (Some(major), Some(minor), Some(patch), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    [major, minor, patch]
        .iter()
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

fn template_variables_text(template: &Template) -> String {
    template
        .variables
        .iter()
        .map(|variable| format!("{}={}", variable.key, variable.default))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_variables(text: &str) -> Result<Map<String, Value>, String> {
    let mut variables = Map::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("Variable line is missing '=': {line}"));
        };
        if key.trim().is_empty() {
            return Err(format!("Variable name is empty: {line}"));
        }
        variables.insert(key.trim().to_string(), json!(value.trim()));
    }
    Ok(variables)
}

fn normalize_path(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn parent_path(path: &str) -> String {
    let normalized = normalize_path(path);
    let trimmed = normalized.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }
    match trimmed.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(index) => trimmed[..index].to_string(),
    }
}

fn resolve_path(base: &str, value: &str) -> String {
    if value.starts_with('/') {
        value.to_string()
    } else {
        let base = normalize_path(base);
        if base == "/" {
            format!("/{value}")
        } else {
            format!("{}/{}", base.trim_end_matches('/'), value)
        }
    }
}

fn encode_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "backup".to_string()
    } else {
        cleaned
    }
}

fn pretty_body(body: &str) -> String {
    match serde_json::from_str::<Value>(body) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| body.to_string()),
        Err(_) => body.to_string(),
    }
}

fn route_field<'a>(route: &'a Value, key: &str) -> Option<&'a Value> {
    route
        .get(key)
        .or_else(|| route.pointer(&format!("/route/{key}")))
}

fn parse_proxy_routes(body: &str) -> Vec<Value> {
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Array(routes)) => routes,
        Ok(Value::Object(object)) => {
            if let Some(Value::Array(routes)) = object.get("routes") {
                routes.clone()
            } else if let Some(Value::Array(routes)) = object.get("data") {
                routes.clone()
            } else {
                vec![Value::Object(object)]
            }
        }
        _ => Vec::new(),
    }
}

fn proxy_error_message(status: u16, body: &str) -> String {
    if status == 404 {
        "Proxy routes are not available on this control plane yet (HTTP 404). \
         The backend proxy integration is being added in parallel."
            .to_string()
    } else {
        api::api_error_message(status, body)
    }
}

#[cfg(test)]
mod tests {
    use super::{encode_path, normalize_path, parent_path, resolve_path};

    #[test]
    fn paths_normalize_and_resolve() {
        assert_eq!(normalize_path(""), "/");
        assert_eq!(normalize_path("plugins/"), "/plugins");
        assert_eq!(normalize_path("/plugins/"), "/plugins");
        assert_eq!(parent_path("/plugins/config"), "/plugins");
        assert_eq!(parent_path("/plugins"), "/");
        assert_eq!(parent_path("/"), "/");
        assert_eq!(
            resolve_path("/plugins", "config.yml"),
            "/plugins/config.yml"
        );
        assert_eq!(resolve_path("/", "config.yml"), "/config.yml");
        assert_eq!(resolve_path("/plugins", "/tmp/x"), "/tmp/x");
    }

    #[test]
    fn paths_are_percent_encoded_but_slashes_are_kept() {
        assert_eq!(
            encode_path("/plugins/my config.yml"),
            "/plugins/my%20config.yml"
        );
    }
}
