use crate::route_manager::{validate_routes, RouteManager, RouteOperation, RouteState};
use crate::{
    api::{RouteSnapshot, Session},
    config::{self, Active},
    keys, *,
};
use std::{
    ffi::OsString,
    fs,
    path::Path,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader},
    process::{Child, Command},
    sync::{Mutex, RwLock},
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub data_dir: PathBuf,
    pub tinc_binary: PathBuf,
    pub interface: String,
    pub owner: String,
    #[serde(default = "default_prefix")]
    pub interface_prefix: u8,
}
fn default_prefix() -> u8 {
    24
}

const ADAPTER_STATE_VERSION: u8 = 2;
const ADAPTER_STATE_FILE: &str = "adapter-state.json";
const CONNECTIVITY_PROBE_INTERVAL: Duration = Duration::from_secs(5);
const CONNECTIVITY_GRACE_PERIOD: Duration = Duration::from_secs(15);
const CONNECTIVITY_FAILURE_THRESHOLD: u8 = 3;
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);
const MAX_MANAGED_ROUTES: usize = 1025;
const MAX_LOG_ENTRIES: usize = 200;
const MAX_TINC_LOG_CHARS: usize = 512;
const MAX_TINC_OUTPUT_BYTES: u64 = 256 * 1024;

async fn append_log(logs: &Arc<Mutex<Vec<LogEntry>>>, message: String) {
    let mut logs = logs.lock().await;
    if logs.len() >= MAX_LOG_ENTRIES {
        logs.remove(0);
    }
    logs.push(LogEntry {
        time: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        message,
    });
}

fn sanitize_tinc_output(line: &str) -> Option<String> {
    let cleaned: String = line
        .chars()
        .filter(|character| !character.is_control() || *character == '\t')
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return None;
    }
    let lowercase = cleaned.to_ascii_lowercase();
    if ["private key", "password", "token", "authorization"]
        .iter()
        .any(|marker| lowercase.contains(marker))
    {
        return Some("已省略可能包含敏感字段的 Tinc 输出".into());
    }
    let mut characters = cleaned.chars();
    let mut visible: String = characters.by_ref().take(MAX_TINC_LOG_CHARS).collect();
    if characters.next().is_some() {
        visible.push('…');
    }
    Some(visible)
}

async fn capture_tinc_output<R>(reader: R, stream: &'static str, logs: Arc<Mutex<Vec<LogEntry>>>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut lines = BufReader::new(reader.take(MAX_TINC_OUTPUT_BYTES)).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(line) = sanitize_tinc_output(&line) {
            append_log(&logs, format!("[Tinc {stream}] {line}")).await;
        }
    }
}

fn tinc_start_args(config_dir: &Path) -> Vec<OsString> {
    vec![
        "-D".into(),
        "--debug=1".into(),
        "-c".into(),
        config_dir.as_os_str().to_owned(),
    ]
}

fn background_keepalive_due(
    child_running: bool,
    process_running: bool,
    last_keepalive: Option<tokio::time::Instant>,
    request_in_flight: bool,
) -> bool {
    child_running
        && process_running
        && !request_in_flight
        && last_keepalive.is_none_or(|last| last.elapsed() >= KEEPALIVE_INTERVAL)
}

fn session_authorization_lost(code: &str) -> bool {
    matches!(code, "auth_expired" | "assignment_missing")
}

fn keepalive_error_requires_immediate_failure(code: &str) -> bool {
    session_authorization_lost(code) || code == "node_revoked"
}

fn keepalive_failure_label(code: &str) -> &'static str {
    match code {
        "network_timeout" => "网络请求超时",
        "network" => "网络连接失败",
        "session_unavailable" => "会话服务暂不可用",
        "keepalive_failed" => "心跳服务暂不可用",
        "route_snapshot_unavailable" => "授权路由快照暂不可用",
        "gateway_unavailable" => "网关服务暂不可用",
        "http" | "server_error" => "服务端暂时异常",
        "protocol" => "续期响应契约无效",
        _ => "未知临时错误",
    }
}

fn route_lease_remaining_seconds(
    deadline: Option<std::time::Instant>,
    now: std::time::Instant,
) -> u64 {
    let remaining = deadline
        .map(|deadline| deadline.saturating_duration_since(now))
        .unwrap_or_default();
    remaining
        .as_secs()
        .saturating_add(u64::from(remaining.subsec_nanos() > 0))
}

fn keepalive_recovery_message(
    failure_streak: u32,
    deadline: Option<std::time::Instant>,
    now: std::time::Instant,
) -> Option<String> {
    (failure_streak > 0).then(|| {
        format!(
            "路由授权续期已恢复（此前连续失败 {failure_streak} 次）；新租约剩余约 {} 秒",
            route_lease_remaining_seconds(deadline, now)
        )
    })
}

fn next_probe_connection(
    result: Option<bool>,
    within_grace: bool,
    current: &str,
    consecutive_failures: u8,
) -> (u8, &'static str) {
    match result {
        Some(true) => (0, "connected"),
        Some(false) => {
            let failures = consecutive_failures.saturating_add(1);
            if within_grace && current != "connected" {
                (failures, "unknown")
            } else if failures < CONNECTIVITY_FAILURE_THRESHOLD {
                (
                    failures,
                    if current == "connected" {
                        "connected"
                    } else {
                        "unknown"
                    },
                )
            } else {
                (failures, "unreachable")
            }
        }
        None => (0, "unknown"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeepaliveTicket {
    session_epoch: u64,
    vpn_epoch: u64,
    request_id: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterState {
    version: u8,
    interface: String,
    address: String,
    prefix: u8,
    generation: String,
    profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    node_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    network_id: Option<u64>,
    #[serde(default)]
    route_revision: Option<String>,
    #[serde(default)]
    desired_routes: Vec<RouteState>,
    #[serde(default, alias = "routes")]
    owned_routes: Vec<RouteState>,
    #[serde(default)]
    pending_operations: Vec<RouteOperation>,
    #[cfg(unix)]
    #[serde(default)]
    link_touched: Option<bool>,
}

impl AdapterState {
    fn current(
        settings: &Settings,
        active: &Active,
        route_manager: &RouteManager,
        route_revision: Option<&str>,
        _link_touched: bool,
    ) -> Self {
        Self {
            version: ADAPTER_STATE_VERSION,
            interface: settings.interface.clone(),
            address: active.identity.node_ip.clone(),
            prefix: settings.interface_prefix,
            generation: active.generation.clone(),
            profile: active.profile.clone(),
            node_id: active.identity.node_id,
            network_id: active.identity.network_id,
            route_revision: route_revision.map(str::to_owned),
            desired_routes: route_manager.desired().to_vec(),
            owned_routes: route_manager.owned().to_vec(),
            pending_operations: route_manager.pending().to_vec(),
            #[cfg(unix)]
            link_touched: Some(_link_touched),
        }
    }

    fn validate(&self, settings: &Settings, active: Option<&Active>) -> Result<()> {
        let active = active.ok_or_else(|| adapter_state_error("有效配置不存在"))?;
        let supported_version = matches!(self.version, 1 | ADAPTER_STATE_VERSION);
        let stable_identity_matches = self.version == 1
            || (self.node_id == active.identity.node_id
                && self.network_id == active.identity.network_id);
        #[cfg(unix)]
        let link_ownership_valid = self.version == 1 || self.link_touched.is_some();
        #[cfg(not(unix))]
        let link_ownership_valid = true;
        let matches = supported_version
            && self.interface == settings.interface
            && self.address == active.identity.node_ip
            && self.prefix == settings.interface_prefix
            && self.generation == active.generation
            && self.profile == active.profile
            && stable_identity_matches
            && link_ownership_valid;
        let revision_valid = self.route_revision.as_ref().is_none_or(|revision| {
            !revision.is_empty()
                && revision.len() <= 256
                && revision.trim() == revision
                && revision.bytes().all(|byte| byte.is_ascii_graphic())
        });
        let address = self.address.parse::<std::net::Ipv4Addr>();
        if !matches || !revision_valid || address.is_err() {
            return Err(adapter_state_error("记录与当前管理员设置或有效配置不一致"));
        }
        let address = address.unwrap();
        let desired_routes = if self.version == 1 {
            &self.owned_routes
        } else {
            &self.desired_routes
        };
        let required_gateway = if desired_routes.is_empty() {
            None
        } else {
            let target = config::server_probe_target(&settings.data_dir, active)?;
            Some(target)
        };
        validate_route_states(desired_routes, address, required_gateway)?;
        validate_route_states(&self.owned_routes, address, None)?;
        if self.pending_operations.len() > MAX_MANAGED_ROUTES.saturating_mul(2) {
            return Err(adapter_state_error("记录中的待处理路由操作过多"));
        }
        for operation in &self.pending_operations {
            let route = match operation {
                RouteOperation::Add(route) | RouteOperation::Remove(route) => route,
            };
            validate_route_states(std::slice::from_ref(route), address, None)?;
            let consistent = match operation {
                RouteOperation::Add(route) => {
                    desired_routes.contains(route) && !self.owned_routes.contains(route)
                }
                RouteOperation::Remove(route) => {
                    !desired_routes.contains(route) && self.owned_routes.contains(route)
                }
            };
            if !consistent {
                return Err(adapter_state_error("待处理路由操作与期望或归属状态不一致"));
            }
        }
        Ok(())
    }

    fn route_manager(&self) -> RouteManager {
        RouteManager::restore(
            if self.version == 1 {
                self.owned_routes.clone()
            } else {
                self.desired_routes.clone()
            },
            self.owned_routes.clone(),
            self.pending_operations.clone(),
        )
    }

    #[cfg(unix)]
    fn owns_link_state(&self) -> bool {
        self.link_touched.unwrap_or(false)
    }
}

fn validate_route_states(
    routes: &[RouteState],
    own_ip: std::net::Ipv4Addr,
    required_gateway: Option<std::net::Ipv4Addr>,
) -> Result<()> {
    if !validate_routes(routes, own_ip, MAX_MANAGED_ROUTES) {
        return Err(adapter_state_error("记录包含非预期路由"));
    }
    if required_gateway.is_some_and(|gateway| {
        !routes
            .iter()
            .any(|route| route.destination == gateway.to_string())
    }) {
        return Err(adapter_state_error("记录缺少当前服务端节点路由"));
    }
    Ok(())
}

fn managed_routes(
    gateway: std::net::Ipv4Addr,
    allowed: &[AllowedRoute],
) -> Result<Vec<RouteState>> {
    let mut routes = vec![RouteState {
        destination: gateway.to_string(),
        prefix: 32,
    }];
    for route in allowed {
        if route.prefix != 32 {
            return Err(Error::protocol("服务端路由快照仅支持 IPv4 /32"));
        }
        if route.destination != gateway {
            routes.push(RouteState {
                destination: route.destination.to_string(),
                prefix: route.prefix,
            });
        }
    }
    if routes.len() > MAX_MANAGED_ROUTES {
        return Err(Error::protocol("服务端路由快照超过客户端安全上限"));
    }
    Ok(routes)
}

fn adapter_state_error(reason: &str) -> Error {
    Error::new(
        "adapter_state_invalid",
        format!("网卡恢复记录无效（{reason}）；已拒绝自动清理，请由管理员核查服务数据目录"),
        false,
    )
}

fn route_lease_missing() -> Error {
    Error::new(
        "route_lease_missing",
        "服务端未返回完整的路由版本与授权租约，已拒绝启动或继续 VPN",
        false,
    )
}

fn running_route_lease_expired(
    child_exists: bool,
    process_running: bool,
    deadline: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    child_exists && process_running && deadline.is_none_or(|deadline| now >= deadline)
}

fn route_lease_is_valid(deadline: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    deadline.is_some_and(|deadline| now < deadline)
}

fn route_snapshot_needs_reconcile(
    current_revision: Option<&str>,
    current_routes: &[RouteState],
    has_pending_operations: bool,
    next_revision: &str,
    next_routes: &[RouteState],
) -> Result<bool> {
    if current_revision == Some(next_revision) && current_routes != next_routes {
        return Err(Error::protocol(
            "服务端在相同 route_revision 下返回了不同路由内容，已拒绝更新",
        ));
    }
    Ok(has_pending_operations
        || current_revision != Some(next_revision)
        || current_routes != next_routes)
}

fn keepalive_ticket_is_current(
    ticket: KeepaliveTicket,
    session_epoch: u64,
    vpn_epoch: u64,
    active_request: Option<u64>,
) -> bool {
    ticket.session_epoch == session_epoch
        && ticket.vpn_epoch == vpn_epoch
        && active_request == Some(ticket.request_id)
}

fn read_adapter_state(
    root: &Path,
    settings: &Settings,
    active: Option<&Active>,
) -> Result<Option<AdapterState>> {
    let path = root.join(ADAPTER_STATE_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let state: AdapterState = serde_json::from_slice(&fs::read(path).map_err(Error::io)?)
        .map_err(|_| adapter_state_error("JSON 损坏或包含未知字段"))?;
    state.validate(settings, active)?;
    Ok(Some(state))
}

pub struct Engine {
    pub status: Arc<RwLock<Status>>,
    pub logs: Arc<Mutex<Vec<LogEntry>>>,
    settings: Settings,
    session: Option<Session>,
    session_epoch: u64,
    vpn_epoch: u64,
    next_keepalive_request_id: u64,
    active_keepalive_request: Option<u64>,
    active: Option<Active>,
    child: Option<Child>,
    adapter_address_configured: bool,
    route_manager: RouteManager,
    probe_target: Option<std::net::Ipv4Addr>,
    last_probe: Option<tokio::time::Instant>,
    probe_started_at: Option<tokio::time::Instant>,
    consecutive_probe_failures: u8,
    last_keepalive: Option<tokio::time::Instant>,
    keepalive_failure_streak: u32,
    route_revision: Option<String>,
    route_lease_deadline: Option<std::time::Instant>,
    #[cfg(unix)]
    adapter_link_touched: bool,
    #[cfg(windows)]
    job: platform_windows::ProcessJob,
}
impl Engine {
    pub fn new(settings: Settings) -> Result<Self> {
        if !settings.data_dir.is_absolute() || !settings.tinc_binary.is_absolute() {
            return Err(Error::protocol("服务路径必须为管理员设置的绝对路径"));
        }
        if !(1..=32).contains(&settings.interface_prefix) {
            return Err(Error::protocol("网卡前缀必须在 1–32 之间"));
        }
        keys::private_dir(&settings.data_dir)?;
        let active = config::read_active(&settings.data_dir)?;
        let adapter_state = read_adapter_state(&settings.data_dir, &settings, active.as_ref())?;
        let mut status = Status::default();
        if let Some(a) = &active {
            status.configured = Some(a.identity.clone());
            if a.revoked {
                status.phase = Phase::Failed;
                status.stage = "已恢复被撤销的本地配置；需要管理员重新分配节点".into();
                status.last_error = Some(Error::new(
                    "node_revoked",
                    "该节点已被后台撤销；本机私钥已保留但不能继续连接",
                    false,
                ));
            } else if keys::candidate_exists(&settings.data_dir, &a.profile) {
                status.phase = Phase::Failed;
                status.stage = "检测到未完成的密钥轮换；请重新登录并更新配置".into();
                status.last_error = Some(Error::new(
                    "key_rotation_incomplete",
                    "上次密钥轮换未完成；已阻止连接，请重新登录并更新配置完成恢复",
                    false,
                ));
            } else {
                status.phase = Phase::Stopped;
                status.stage = "已恢复本地配置，VPN 未启动；请重新登录后连接".into();
            }
        }
        Ok(Self {
            status: Arc::new(RwLock::new(status)),
            logs: Arc::new(Mutex::new(Vec::new())),
            settings,
            session: None,
            session_epoch: 0,
            vpn_epoch: 0,
            next_keepalive_request_id: 0,
            active_keepalive_request: None,
            active,
            child: None,
            adapter_address_configured: adapter_state.is_some(),
            route_manager: adapter_state
                .as_ref()
                .map(AdapterState::route_manager)
                .unwrap_or_default(),
            probe_target: None,
            last_probe: None,
            probe_started_at: None,
            consecutive_probe_failures: 0,
            last_keepalive: None,
            keepalive_failure_streak: 0,
            route_revision: adapter_state
                .as_ref()
                .and_then(|state| state.route_revision.clone()),
            route_lease_deadline: None,
            #[cfg(unix)]
            adapter_link_touched: adapter_state
                .as_ref()
                .is_some_and(AdapterState::owns_link_state),
            #[cfg(windows)]
            job: platform_windows::ProcessJob::new().map_err(Error::io)?,
        })
    }
    async fn stage(&self, phase: Phase, message: &str) {
        {
            let mut s = self.status.write().await;
            s.phase = phase;
            s.stage = message.into();
            s.last_error = None;
        }
        self.log(message).await;
    }
    async fn log(&self, message: &str) {
        append_log(&self.logs, message.into()).await;
    }
    pub async fn fail(&mut self, error: Error) -> Reply {
        if session_authorization_lost(&error.code) {
            if self.child.is_some() || self.adapter_cleanup_pending() {
                if let Err(cleanup_error) = self.stop().await {
                    self.log(&cleanup_error.message).await;
                }
            }
            self.session = None;
            self.session_epoch = self.session_epoch.wrapping_add(1);
            self.active_keepalive_request = None;
            self.status.write().await.session = None;
        } else if error.code == "node_revoked" {
            // Revocation is a node-lifecycle event, not merely a failed HTTP request.
            // Persist it locally so a service restart cannot revive the old profile.
            if let Err(cleanup_error) = self.stop().await {
                self.log(&cleanup_error.message).await;
            }
            if let Some(active) = self.active.as_mut() {
                active.revoked = true;
                if let Err(write_error) = config::write_active(&self.settings.data_dir, active) {
                    self.log(&write_error.message).await;
                }
            }
            self.session = None;
            self.session_epoch = self.session_epoch.wrapping_add(1);
            self.active_keepalive_request = None;
            let mut status = self.status.write().await;
            status.session = None;
            status.phase = Phase::Failed;
            status.stage = error.message.clone();
        } else if error.code == "route_lease_expired"
            && (self.child.is_some() || self.adapter_cleanup_pending())
        {
            if let Err(cleanup_error) = self.stop().await {
                self.log(&cleanup_error.message).await;
            }
        }
        let mut status = self.status.write().await;
        if !status.process_running {
            status.phase = Phase::Failed;
        }
        status.last_error = Some(error.clone());
        drop(status);
        self.log(&error.message).await;
        Reply::Error(error)
    }
    pub async fn handle(&mut self, request: Request) -> Reply {
        let result = self.execute(request).await;
        match result {
            Ok(()) => Reply::Status(Box::new(self.status.read().await.clone())),
            Err(e) => self.fail(e).await,
        }
    }
    async fn execute(&mut self, request: Request) -> Result<()> {
        self.poll().await?;
        match request {
            Request::Login {
                server,
                access_code,
                sid,
                password,
                allow_http,
            } => {
                let session = Session::login(
                    &server,
                    access_code.as_deref(),
                    sid.as_deref(),
                    &password,
                    allow_http,
                )
                .await?;
                if self.child.is_some() {
                    let active = self.active.as_ref().ok_or_else(|| {
                        Error::new(
                            "profile_mismatch",
                            "VPN 正在运行，但活动配置不存在；请先停止 VPN",
                            false,
                        )
                    })?;
                    if keys::profile_id(&session.origin, &session.identity) != active.profile
                        || session.identity != active.identity
                    {
                        return Err(Error::new(
                            "profile_mismatch",
                            "VPN 正在运行，不能切换到其他节点；请先停止 VPN",
                            false,
                        ));
                    }
                }
                {
                    let mut s = self.status.write().await;
                    s.session = Some(session.identity.clone());
                    s.last_error = None;
                    if !s.process_running {
                        s.phase = if self.active.is_some() {
                            Phase::Stopped
                        } else {
                            Phase::Unconfigured
                        };
                    }
                }
                self.session = Some(session);
                self.session_epoch = self.session_epoch.wrapping_add(1);
                self.active_keepalive_request = None;
                self.keepalive_failure_streak = 0;
                self.log("登录成功，已取得后台分配的网络与节点地址").await;
            }
            Request::Configure => self.configure().await?,
            Request::RotateKey => self.rotate_key().await?,
            Request::StartVpn => self.start().await?,
            Request::StopVpn => self.stop().await?,
            Request::Logout => {
                if self.child.is_some() || self.adapter_cleanup_pending() {
                    self.stop().await?;
                }
                self.session = None;
                self.session_epoch = self.session_epoch.wrapping_add(1);
                self.active_keepalive_request = None;
                self.status.write().await.session = None;
                self.last_keepalive = None;
                self.log("VPN 已停止，本地会话已清除；服务端未提供撤销接口")
                    .await;
            }
            Request::GetStatus | Request::GetRecentLogs => (),
        }
        Ok(())
    }
    async fn configure(&mut self) -> Result<()> {
        if self.child.is_some() || self.adapter_cleanup_pending() {
            return Err(Error::new(
                "busy",
                "请先停止 VPN 并完成专用网卡清理，再重新配置",
                true,
            ));
        }
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| Error::new("auth_required", "请先登录", false))?;
        let identity = session.identity.clone();
        let profile = keys::profile_id(&session.origin, &identity);
        let legacy_profile = keys::legacy_profile_id(&session.origin, &identity);
        keys::migrate_legacy_key(&self.settings.data_dir, &legacy_profile, &profile)?;
        self.stage(Phase::Configuring, "正在下载并校验配置").await;
        // Preserve the observed old-client protocol: download precedes key exchange.
        let archive = session.download().await?;
        let validated = config::validate_zip(&archive, &identity, &self.settings.interface)?;
        self.stage(Phase::Configuring, "正在生成或复用本机 RSA 密钥")
            .await;
        let mut worker = Command::new(std::env::current_exe().map_err(Error::io)?);
        worker
            .arg("--key-worker")
            .arg(&self.settings.data_dir)
            .arg(&profile)
            .arg(&identity.sid)
            .arg(&identity.net_name)
            .arg(&identity.node_ip)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        hide(&mut worker);
        let output = tokio::time::timeout(Duration::from_secs(120), worker.output())
            .await
            .map_err(|_| Error::new("key_timeout", "密钥生成超过 120 秒，已终止；可重试", true))?
            .map_err(|_| Error::new("key_generation", "无法启动本机密钥生成子进程", true))?;
        if !output.status.success() {
            return Err(Error::new(
                "key_generation",
                "密钥生成或读取失败；已保留原密钥，请检查服务存储权限",
                true,
            ));
        }
        let public =
            String::from_utf8(output.stdout).map_err(|_| Error::protocol("本机公钥编码失败"))?;
        config::client_host(&public, &identity.node_ip)?;
        self.stage(Phase::Configuring, "正在上传公钥并校验服务端公钥")
            .await;
        let master = session.upload(&public).await?;
        config::server_host(&master)?;
        self.stage(Phase::Configuring, "正在应用经过校验的配置")
            .await;
        let active = config::apply(
            &self.settings.data_dir,
            validated,
            &identity,
            &profile,
            &public,
            &master,
        )?;
        self.active = Some(active);
        keys::discard_candidate(&self.settings.data_dir, &profile)?;
        {
            let mut s = self.status.write().await;
            s.configured = Some(identity);
            // 下载与公钥交换只有在后端网络 READY 时才会成功，无需额外审批。
            s.confirmation = "ready".into();
        }
        self.stage(Phase::Stopped, "配置已应用；服务端网络已确认就绪")
            .await;
        Ok(())
    }

    async fn rotate_key(&mut self) -> Result<()> {
        if self.child.is_some() || self.adapter_cleanup_pending() {
            return Err(Error::new("busy", "请先停止 VPN，再执行安全换钥", true));
        }
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| Error::new("auth_required", "请先重新登录当前节点", false))?;
        let identity = session.identity.clone();
        let profile = keys::profile_id(&session.origin, &identity);
        let active = self
            .active
            .clone()
            .ok_or_else(|| Error::new("not_configured", "请先完成配置", false))?;
        if active.profile != profile || active.identity != identity || active.revoked {
            return Err(Error::new(
                "profile_mismatch",
                "当前登录节点与活动配置不一致，不能换钥",
                false,
            ));
        }
        if keys::candidate_exists(&self.settings.data_dir, &profile) {
            return Err(Error::new(
                "key_rotation_incomplete",
                "检测到未完成的密钥轮换；请先更新配置完成恢复",
                false,
            ));
        }

        self.stage(Phase::Configuring, "正在校验换钥配置").await;
        let archive = session.download().await?;
        let validated = config::validate_zip(&archive, &identity, &self.settings.interface)?;
        let old_public = keys::ensure_key(&self.settings.data_dir, &profile, &identity)?;
        let candidate = keys::generate_candidate(&identity)?;
        keys::store_candidate(
            &self.settings.data_dir,
            &profile,
            candidate.private_pem.as_bytes(),
        )?;
        self.stage(Phase::Configuring, "正在上传候选公钥").await;
        let master = match session.upload(&candidate.public_host).await {
            Ok(master) => master,
            Err(error) => {
                let _ = keys::discard_candidate(&self.settings.data_dir, &profile);
                return Err(error);
            }
        };
        if let Err(error) = config::server_host(&master) {
            let _ = keys::discard_candidate(&self.settings.data_dir, &profile);
            if session.upload(&old_public).await.is_err() {
                return Err(Error::new(
                    "key_rotation_rollback_failed",
                    "换钥响应校验失败，且服务端旧公钥回滚失败；已保留旧私钥，请联系管理员",
                    false,
                ));
            }
            return Err(error);
        }
        let old_private = match keys::activate_candidate(&self.settings.data_dir, &profile) {
            Ok(key) => key,
            Err(error) => {
                let _ = keys::discard_candidate(&self.settings.data_dir, &profile);
                if session.upload(&old_public).await.is_err() {
                    return Err(Error::new(
                        "key_rotation_rollback_failed",
                        "候选私钥激活失败，且服务端旧公钥回滚失败；请联系管理员",
                        false,
                    ));
                }
                return Err(error);
            }
        };
        let applied = config::apply(
            &self.settings.data_dir,
            validated,
            &identity,
            &profile,
            &candidate.public_host,
            &master,
        );
        let new_active = match applied {
            Ok(active) => active,
            Err(error) => {
                let local_rollback =
                    keys::replace_private_key(&self.settings.data_dir, &profile, &old_private);
                let server_rollback = session.upload(&old_public).await;
                if local_rollback.is_err() || server_rollback.is_err() {
                    return Err(Error::new(
                        "key_rotation_rollback_failed",
                        "换钥本地提交失败，且旧密钥状态未能完整回滚；请勿重试连接并联系管理员",
                        false,
                    ));
                }
                keys::discard_candidate(&self.settings.data_dir, &profile)?;
                return Err(error);
            }
        };
        self.active = Some(new_active);
        keys::discard_candidate(&self.settings.data_dir, &profile)?;
        self.status.write().await.configured = Some(identity);
        self.stage(Phase::Stopped, "密钥已安全轮换；服务端网络已确认就绪")
            .await;
        Ok(())
    }
    pub async fn poll(&mut self) -> Result<()> {
        if running_route_lease_expired(
            self.child.is_some(),
            self.status.read().await.process_running,
            self.route_lease_deadline,
            std::time::Instant::now(),
        ) {
            return Err(Error::new(
                "route_lease_expired",
                "路由授权租约已到期，VPN 将停止并清理自有路由",
                false,
            ));
        }
        if let Some(child) = &mut self.child {
            if let Some(code) = child.try_wait().map_err(Error::io)? {
                self.child = None;
                self.vpn_epoch = self.vpn_epoch.wrapping_add(1);
                self.active_keepalive_request = None;
                let mut s = self.status.write().await;
                s.process_running = false;
                s.connection = "unknown".into();
                s.phase = Phase::Failed;
                let error = Error::new(
                    "process_exit",
                    format!(
                        "Tinc 进程已退出（代码 {:?}），请检查驱动、网卡及配置",
                        code.code()
                    ),
                    true,
                );
                s.last_error = Some(error.clone());
                drop(s);
                self.log(&error.message).await;
                let adapter_ip = self
                    .active
                    .as_ref()
                    .map(|active| active.identity.node_ip.clone());
                if let Some(ip) = adapter_ip {
                    self.clear_adapter(&ip).await?;
                }
                self.probe_target = None;
                self.last_probe = None;
                self.probe_started_at = None;
                self.consecutive_probe_failures = 0;
                self.route_revision = None;
                self.route_lease_deadline = None;
            }
        }
        if self.child.is_some()
            && self.status.read().await.process_running
            && self
                .last_probe
                .is_none_or(|last| last.elapsed() >= CONNECTIVITY_PROBE_INTERVAL)
        {
            self.probe_connection().await;
        }
        Ok(())
    }

    /// Starts a periodic keepalive without retaining the Agent's Engine lock
    /// while the HTTP request is in flight.
    pub async fn begin_background_keepalive(&mut self) -> Option<(Session, KeepaliveTicket)> {
        let process_running = self.status.read().await.process_running;
        if background_keepalive_due(
            self.child.is_some(),
            process_running,
            self.last_keepalive,
            self.active_keepalive_request.is_some(),
        ) {
            let session = self.session.clone()?;
            self.last_keepalive = Some(tokio::time::Instant::now());
            self.next_keepalive_request_id = self.next_keepalive_request_id.wrapping_add(1);
            let request_id = self.next_keepalive_request_id;
            self.active_keepalive_request = Some(request_id);
            Some((
                session,
                KeepaliveTicket {
                    session_epoch: self.session_epoch,
                    vpn_epoch: self.vpn_epoch,
                    request_id,
                },
            ))
        } else {
            None
        }
    }

    pub async fn finish_background_keepalive(
        &mut self,
        ticket: KeepaliveTicket,
        keepalive: Result<Option<RouteSnapshot>>,
    ) -> Result<()> {
        if !keepalive_ticket_is_current(
            ticket,
            self.session_epoch,
            self.vpn_epoch,
            self.active_keepalive_request,
        ) {
            return Ok(());
        }
        self.active_keepalive_request = None;
        if let Err(error) = &keepalive {
            if keepalive_error_requires_immediate_failure(&error.code) {
                return Err(error.clone());
            }
            self.keepalive_failure_streak = self.keepalive_failure_streak.saturating_add(1);
            let remaining =
                route_lease_remaining_seconds(self.route_lease_deadline, std::time::Instant::now());
            self.log(&format!(
                "路由授权续期暂时失败（{}，连续 {} 次）；将继续重试，当前租约剩余约 {} 秒",
                keepalive_failure_label(&error.code),
                self.keepalive_failure_streak,
                remaining
            ))
            .await;
            // A transient or malformed response does not revoke an otherwise
            // valid lease. It becomes fail-closed only at the saved deadline.
            return self.ensure_route_lease_valid();
        }
        if self.child.is_none() || !self.status.read().await.process_running {
            return Ok(());
        }
        let Some(snapshot) = keepalive.ok().flatten() else {
            self.keepalive_failure_streak = self.keepalive_failure_streak.saturating_add(1);
            let remaining =
                route_lease_remaining_seconds(self.route_lease_deadline, std::time::Instant::now());
            self.log(&format!(
                "路由授权续期响应缺少有效快照（连续 {} 次）；将继续重试，当前租约剩余约 {} 秒",
                self.keepalive_failure_streak, remaining
            ))
            .await;
            // A successful but legacy/malformed heartbeat must not erase an
            // existing authorization lease. Keep using the last snapshot only
            // until its original deadline, then fail closed in poll().
            return self.ensure_route_lease_valid();
        };
        // Once the previous local lease has expired, even a late successful
        // response from a request started earlier cannot resurrect that VPN.
        self.ensure_route_lease_valid()?;
        let previous_failures = self.keepalive_failure_streak;
        self.apply_route_snapshot(&snapshot).await?;
        if let Some(message) = keepalive_recovery_message(
            previous_failures,
            self.route_lease_deadline,
            std::time::Instant::now(),
        ) {
            self.log(&message).await;
        }
        self.keepalive_failure_streak = 0;
        Ok(())
    }

    fn ensure_route_lease_valid(&self) -> Result<()> {
        if route_lease_is_valid(self.route_lease_deadline, std::time::Instant::now()) {
            Ok(())
        } else {
            Err(Error::new(
                "route_lease_expired",
                "无法续期路由授权，租约已到期；VPN 将停止并清理自有路由",
                false,
            ))
        }
    }

    async fn apply_route_snapshot(&mut self, snapshot: &RouteSnapshot) -> Result<()> {
        let gateway = self
            .probe_target
            .ok_or_else(|| Error::new("route_sync", "VPN 运行状态缺少网关路由目标", true))?;
        let revision = snapshot.revision().ok_or_else(route_lease_missing)?;
        let deadline = snapshot.lease_deadline().ok_or_else(route_lease_missing)?;
        if deadline <= std::time::Instant::now() {
            return Err(Error::new(
                "route_lease_expired",
                "服务端返回的路由授权租约已到期",
                false,
            ));
        }
        let desired = managed_routes(gateway, snapshot.routes())?;
        let needs_reconcile = route_snapshot_needs_reconcile(
            self.route_revision.as_deref(),
            self.route_manager.desired(),
            !self.route_manager.pending().is_empty(),
            revision,
            &desired,
        )?;
        self.route_lease_deadline = Some(deadline);
        if needs_reconcile {
            self.reconcile_routes(&desired).await?;
            self.route_revision = Some(revision.to_owned());
            let ip = self
                .active
                .as_ref()
                .ok_or_else(|| Error::new("route_sync", "有效配置不存在，无法保存路由版本", false))?
                .identity
                .node_ip
                .clone();
            self.write_adapter_state(&ip)?;
        }
        Ok(())
    }
    async fn start(&mut self) -> Result<()> {
        if self.child.is_some() {
            return Ok(());
        }
        if self.adapter_cleanup_pending() {
            return Err(Error::new(
                "adapter_cleanup",
                "检测到尚未清理的专用网卡状态；请先重试停止",
                true,
            ));
        }
        self.vpn_epoch = self.vpn_epoch.wrapping_add(1);
        self.active_keepalive_request = None;
        let active = self
            .active
            .clone()
            .ok_or_else(|| Error::new("not_configured", "请先完成配置", false))?;
        if active.revoked {
            return Err(Error::new(
                "node_revoked",
                "该节点已被后台撤销，不能重新连接",
                false,
            ));
        }
        if keys::candidate_exists(&self.settings.data_dir, &active.profile) {
            return Err(Error::new(
                "key_rotation_incomplete",
                "检测到未完成的密钥轮换；请先重新登录并更新配置",
                false,
            ));
        }
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| Error::new("auth_required", "启动前请重新登录当前节点", false))?;
        if keys::profile_id(&session.origin, &session.identity) != active.profile
            || session.identity != active.identity
        {
            return Err(Error::new(
                "profile_mismatch",
                "当前登录节点与活动配置不一致，请先应用对应配置",
                false,
            ));
        }
        let refreshed_snapshot = session.keepalive().await?.ok_or_else(route_lease_missing)?;
        let route_revision = refreshed_snapshot
            .revision()
            .ok_or_else(route_lease_missing)?
            .to_owned();
        let route_lease_deadline = refreshed_snapshot
            .lease_deadline()
            .ok_or_else(route_lease_missing)?;
        if route_lease_deadline <= std::time::Instant::now() {
            return Err(Error::new(
                "route_lease_expired",
                "服务端返回的路由授权租约已到期",
                false,
            ));
        }
        if !self.settings.tinc_binary.is_file() {
            return Err(Error::new(
                "tinc_missing",
                "未安装管理员指定的 Tinc 核心程序",
                false,
            ));
        }
        let probe_target = config::server_probe_target(&self.settings.data_dir, &active)?;
        let routes = managed_routes(probe_target, refreshed_snapshot.routes())?;
        let mut version = Command::new(&self.settings.tinc_binary);
        version.arg("--version").kill_on_drop(true);
        hide(&mut version);
        let output = tokio::time::timeout(Duration::from_secs(5), version.output())
            .await
            .map_err(|_| Error::new("tinc_timeout", "Tinc 版本检测超时", true))?
            .map_err(|_| Error::new("tinc_version", "无法检测 Tinc 版本", false))?;
        let v = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() || !v.contains("1.0.") {
            return Err(Error::new(
                "tinc_version",
                "当前实现要求 tinc 1.0.x（RSA）；检测到的核心版本不匹配或未知",
                false,
            ));
        }
        self.stage(Phase::Starting, "正在启动 Tinc 并设置虚拟网卡")
            .await;
        let mut command = Command::new(&self.settings.tinc_binary);
        command
            .args(tinc_start_args(&config::generation_path(
                &self.settings.data_dir,
                &active,
            )))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        hide(&mut command);
        let mut child = command.spawn().map_err(|_| {
            Error::new(
                "start_failed",
                "Tinc 进程无法启动，请检查安装、权限和驱动",
                true,
            )
        })?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        self.child = Some(child);
        if let Some(stdout) = stdout {
            tokio::spawn(capture_tinc_output(stdout, "stdout", self.logs.clone()));
        }
        if let Some(stderr) = stderr {
            tokio::spawn(capture_tinc_output(stderr, "stderr", self.logs.clone()));
        }
        #[cfg(windows)]
        if let Some(handle) = self.child.as_ref().and_then(Child::raw_handle) {
            if unsafe { self.job.assign(handle) }.is_err() {
                self.stop().await?;
                return Err(Error::new(
                    "process_ownership",
                    "无法建立 Tinc 进程所有权保护，已停止启动",
                    false,
                ));
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        self.poll().await?;
        if self.child.is_none() {
            return Err(Error::new(
                "start_failed",
                "Tinc 在启动阶段退出，请检查驱动及配置",
                true,
            ));
        }
        self.route_revision = Some(route_revision);
        self.route_lease_deadline = Some(route_lease_deadline);
        self.keepalive_failure_streak = 0;
        if let Err(error) = self.set_adapter(&active.identity.node_ip, &routes).await {
            self.stop().await?;
            return Err(error);
        }
        self.poll().await?;
        if self.child.is_none() {
            self.stop().await?;
            return Err(Error::new("start_failed", "Tinc 在网卡设置期间退出", true));
        }
        {
            let mut s = self.status.write().await;
            s.process_running = true;
            s.connection = "unknown".into();
        }
        self.probe_target = Some(probe_target);
        self.last_probe = None;
        self.probe_started_at = Some(tokio::time::Instant::now());
        self.consecutive_probe_failures = 0;
        self.stage(Phase::Running, "Tinc 进程运行中；正在建立安全隧道")
            .await;
        self.probe_connection().await;
        Ok(())
    }
    async fn set_adapter(&mut self, ip: &str, routes: &[RouteState]) -> Result<()> {
        ip.parse::<std::net::Ipv4Addr>()
            .map_err(|_| Error::protocol("无效节点地址"))?;
        #[cfg(windows)]
        {
            let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            let mut c = Command::new(PathBuf::from(system.clone()).join("System32/netsh.exe"));
            c.args(windows_set_adapter_args(
                &self.settings.interface,
                ip,
                self.settings.interface_prefix,
            ));
            run_adapter(c).await?;
            self.adapter_address_configured = true;
            self.write_adapter_state(ip)?;
        }
        #[cfg(unix)]
        {
            let mut c = Command::new("/usr/sbin/ip");
            c.args(linux_set_address_args(
                &self.settings.interface,
                ip,
                self.settings.interface_prefix,
            ));
            run_adapter(c).await?;
            self.adapter_address_configured = true;
            self.write_adapter_state(ip)?;
            let mut c = Command::new("/usr/sbin/ip");
            c.args(linux_link_args(&self.settings.interface, "up"));
            self.adapter_link_touched = true;
            // Persist the cleanup intent before changing link state. Lowering an
            // already-down dedicated interface during recovery is harmless.
            self.write_adapter_state(ip)?;
            run_adapter(c).await?;
        }
        self.reconcile_routes(routes).await
    }

    async fn install_managed_route(&mut self, ip: &str, route: &RouteState) -> Result<()> {
        if self.route_manager.owned().contains(route) {
            self.route_manager
                .complete(&RouteOperation::Add(route.clone()));
            return self.write_adapter_state(ip);
        }
        let target = route
            .destination
            .parse::<std::net::Ipv4Addr>()
            .map_err(|_| Error::protocol("服务端路由快照包含无效地址"))?;
        if route.prefix != 32 {
            return Err(Error::protocol("服务端路由快照仅支持 IPv4 /32"));
        }
        #[cfg(windows)]
        {
            let tinc_index =
                platform_windows::interface_index(&self.settings.interface).map_err(|_| {
                    Error::new(
                        "route_check",
                        "无法确认 Tinc 虚拟网卡，已取消添加路由；请检查网卡安装状态",
                        true,
                    )
                })?;
            if platform_windows::exact_ipv4_route(tinc_index, target, route.prefix).map_err(
                |_| {
                    Error::new(
                        "route_check",
                        "无法核对 Tinc 虚拟网卡上的精确路由，已取消添加路由",
                        true,
                    )
                },
            )? {
                self.route_manager
                    .complete(&RouteOperation::Add(route.clone()));
                return self.write_adapter_state(ip);
            }
            let best = platform_windows::best_ipv4_route(target).map_err(|_| {
                Error::new(
                    "route_check",
                    "无法检查目标地址的现有系统路由，已取消添加路由",
                    true,
                )
            })?;
            if windows_route_conflicts(best, tinc_index) {
                return Err(Error::new(
                    "route_conflict",
                    format!(
                        "目标地址 {target}/32 与本机现有网络路由冲突，未修改系统路由；请断开冲突网络或联系管理员调整 VPN 地址"
                    ),
                    false,
                ));
            }
            let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            let mut c = Command::new(PathBuf::from(system).join("System32/netsh.exe"));
            c.args(windows_add_route_args(&self.settings.interface, target));
            run_adapter(c).await?;
        }
        #[cfg(unix)]
        {
            if self.linux_route_exists(route).await? {
                self.route_manager
                    .complete(&RouteOperation::Add(route.clone()));
                return self.write_adapter_state(ip);
            }
            if self.linux_route_conflicts(target).await? {
                return Err(Error::new(
                    "route_conflict",
                    format!(
                        "目标地址 {target}/32 与本机现有网络路由冲突，未修改系统路由；请断开冲突网络或联系管理员调整 VPN 地址"
                    ),
                    false,
                ));
            }
            let mut c = Command::new("/usr/sbin/ip");
            c.args(linux_add_route_args(&self.settings.interface, target));
            run_adapter(c).await?;
        }
        let operation = RouteOperation::Add(route.clone());
        self.route_manager.complete(&operation);
        self.write_adapter_state(ip)
    }

    async fn remove_managed_route(&mut self, ip: &str, route: &RouteState) -> Result<()> {
        if !self.route_manager.owned().contains(route) {
            self.route_manager
                .complete(&RouteOperation::Remove(route.clone()));
            return self.write_adapter_state(ip);
        }
        #[cfg(windows)]
        {
            let target = route
                .destination
                .parse::<std::net::Ipv4Addr>()
                .map_err(|_| Error::protocol("恢复记录包含无效地址"))?;
            let tinc_index =
                platform_windows::interface_index(&self.settings.interface).map_err(|_| {
                    Error::new(
                        "route_check",
                        "无法确认 Tinc 虚拟网卡，已取消删除路由",
                        true,
                    )
                })?;
            if !platform_windows::exact_ipv4_route(tinc_index, target, route.prefix).map_err(
                |_| {
                    Error::new(
                        "route_check",
                        "无法核对 Tinc 虚拟网卡上的精确路由，已取消删除路由",
                        true,
                    )
                },
            )? {
                self.route_manager
                    .complete(&RouteOperation::Remove(route.clone()));
                return self.write_adapter_state(ip);
            }
            let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            let mut c = Command::new(PathBuf::from(system).join("System32/netsh.exe"));
            c.args(windows_delete_route_args(
                &self.settings.interface,
                &route.destination,
                route.prefix,
            ));
            run_adapter(c).await?;
        }
        #[cfg(unix)]
        {
            if !self.linux_route_exists(route).await? {
                self.route_manager
                    .complete(&RouteOperation::Remove(route.clone()));
                return self.write_adapter_state(ip);
            }
            let mut c = Command::new("/usr/sbin/ip");
            c.args(linux_delete_route_args(
                &self.settings.interface,
                &route.destination,
                route.prefix,
            ));
            run_adapter(c).await?;
        }
        self.route_manager
            .complete(&RouteOperation::Remove(route.clone()));
        self.write_adapter_state(ip)
    }

    #[cfg(unix)]
    async fn linux_route_exists(&self, route: &RouteState) -> Result<bool> {
        let mut command = Command::new("/usr/sbin/ip");
        command.args(linux_inspect_route_args(
            &self.settings.interface,
            &route.destination,
            route.prefix,
        ));
        command.kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(5), command.output())
            .await
            .map_err(|_| Error::new("route_check", "检查 Linux 精确路由超时", true))?
            .map_err(|_| Error::new("route_check", "无法检查 Linux 精确路由", true))?;
        if !output.status.success() {
            return Err(Error::new(
                "route_check",
                "Linux 拒绝检查 Tinc 接口上的精确路由",
                true,
            ));
        }
        Ok(!output.stdout.is_empty())
    }

    #[cfg(unix)]
    async fn linux_route_conflicts(&self, target: std::net::Ipv4Addr) -> Result<bool> {
        let mut command = Command::new("/usr/sbin/ip");
        command.args(linux_matching_route_args(target));
        command.kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(5), command.output())
            .await
            .map_err(|_| Error::new("route_check", "检查 Linux 目标路由超时", true))?
            .map_err(|_| Error::new("route_check", "无法检查 Linux 目标路由", true))?;
        if !output.status.success() {
            return Err(Error::new(
                "route_check",
                "Linux 拒绝检查目标地址的现有路由",
                true,
            ));
        }
        parse_linux_route_conflict(&output.stdout, target, &self.settings.interface).ok_or_else(
            || {
                Error::new(
                    "route_check",
                    "Linux 返回了无法确认的路由信息，已取消添加路由",
                    true,
                )
            },
        )
    }

    async fn preflight_route_add(&self, route: &RouteState) -> Result<()> {
        let target = route
            .destination
            .parse::<std::net::Ipv4Addr>()
            .map_err(|_| Error::protocol("服务端路由快照包含无效地址"))?;
        #[cfg(windows)]
        {
            let tinc_index =
                platform_windows::interface_index(&self.settings.interface).map_err(|_| {
                    Error::new(
                        "route_check",
                        "无法确认 Tinc 虚拟网卡，已取消添加路由；请检查网卡安装状态",
                        true,
                    )
                })?;
            if platform_windows::exact_ipv4_route(tinc_index, target, route.prefix).map_err(
                |_| {
                    Error::new(
                        "route_check",
                        "无法核对 Tinc 虚拟网卡上的精确路由，已取消添加路由",
                        true,
                    )
                },
            )? {
                return Err(Error::new(
                    "route_conflict",
                    format!(
                        "目标地址 {target}/32 已存在于 Tinc 接口，但不属于当前 Agent；为避免误删，未接管该路由"
                    ),
                    false,
                ));
            }
            let best = platform_windows::best_ipv4_route(target).map_err(|_| {
                Error::new(
                    "route_check",
                    "无法检查目标地址的现有系统路由，已取消添加路由",
                    true,
                )
            })?;
            if windows_route_conflicts(best, tinc_index) {
                return Err(Error::new(
                    "route_conflict",
                    format!(
                        "目标地址 {target}/32 与本机现有网络路由冲突，未修改系统路由；请断开冲突网络或联系管理员调整 VPN 地址"
                    ),
                    false,
                ));
            }
        }
        #[cfg(unix)]
        {
            if self.linux_route_exists(route).await? {
                return Err(Error::new(
                    "route_conflict",
                    format!(
                        "目标地址 {target}/32 已存在于 Tinc 接口，但不属于当前 Agent；为避免误删，未接管该路由"
                    ),
                    false,
                ));
            }
            if self.linux_route_conflicts(target).await? {
                return Err(Error::new(
                    "route_conflict",
                    format!(
                        "目标地址 {target}/32 与本机现有网络路由冲突，未修改系统路由；请断开冲突网络或联系管理员调整 VPN 地址"
                    ),
                    false,
                ));
            }
        }
        Ok(())
    }

    async fn reconcile_routes(&mut self, desired: &[RouteState]) -> Result<()> {
        let ip = self
            .active
            .as_ref()
            .ok_or_else(|| Error::new("route_sync", "有效配置不存在，无法同步路由", false))?
            .identity
            .node_ip
            .clone();
        let changed = !self.route_manager.pending().is_empty()
            || self.route_manager.desired() != desired
            || self.route_manager.owned() != desired;
        self.apply_pending_routes(&ip).await?;
        if self.route_manager.desired() != desired || self.route_manager.owned() != desired {
            let additions: Vec<_> = desired
                .iter()
                .filter(|route| !self.route_manager.owned().contains(route))
                .cloned()
                .collect();
            for route in additions {
                // Inspect before journaling ownership. Existing routes are not
                // adopted, so later cleanup cannot delete administrator state.
                self.preflight_route_add(&route).await?;
            }
            self.route_manager.plan(desired);
            // Persist the complete intent before the first OS mutation.
            self.write_adapter_state(&ip)?;
            self.apply_pending_routes(&ip).await?;
        }
        if changed {
            self.log("已根据服务端授权快照同步节点路由").await;
        }
        let mut status = self.status.write().await;
        if status
            .last_error
            .as_ref()
            .is_some_and(|error| error.code == "route_sync")
        {
            status.last_error = None;
        }
        Ok(())
    }

    async fn apply_pending_routes(&mut self, ip: &str) -> Result<()> {
        for change in self.route_manager.pending().to_vec() {
            match change {
                RouteOperation::Remove(route) => {
                    self.remove_managed_route(ip, &route).await.map_err(|_| {
                        Error::new("route_sync", "撤销过期节点路由失败，将在下次心跳重试", true)
                    })?;
                }
                RouteOperation::Add(route) => {
                    self.install_managed_route(ip, &route)
                        .await
                        .map_err(|error| match error.code.as_str() {
                            "route_conflict" | "route_check" => error,
                            _ => Error::new(
                                "route_sync",
                                "新增授权节点路由失败，将在下次心跳重试",
                                true,
                            ),
                        })?;
                }
            }
        }
        Ok(())
    }

    fn adapter_cleanup_pending(&self) -> bool {
        self.adapter_address_configured || !self.route_manager.is_empty() || {
            #[cfg(unix)]
            {
                self.adapter_link_touched
            }
            #[cfg(not(unix))]
            {
                false
            }
        }
    }

    fn write_adapter_state(&self, ip: &str) -> Result<()> {
        let active = self
            .active
            .as_ref()
            .ok_or_else(|| adapter_state_error("写入时有效配置不存在"))?;
        if active.identity.node_ip != ip {
            return Err(adapter_state_error("写入地址与有效配置不一致"));
        }
        let state = AdapterState::current(
            &self.settings,
            active,
            &self.route_manager,
            self.route_revision.as_deref(),
            {
                #[cfg(unix)]
                {
                    self.adapter_link_touched
                }
                #[cfg(not(unix))]
                {
                    false
                }
            },
        );
        keys::atomic_write(
            &self.settings.data_dir.join(ADAPTER_STATE_FILE),
            &serde_json::to_vec(&state).map_err(|_| adapter_state_error("JSON 编码失败"))?,
        )
    }

    fn remove_adapter_state(&self) -> Result<()> {
        match fs::remove_file(self.settings.data_dir.join(ADAPTER_STATE_FILE)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::io(error)),
        }
    }

    pub async fn recover_adapter(&mut self) -> Result<()> {
        if !self.adapter_cleanup_pending() {
            return Ok(());
        }
        self.log("检测到上次异常终止遗留的专用网卡状态，正在按精确地址恢复清理")
            .await;
        let ip = self
            .active
            .as_ref()
            .ok_or_else(|| adapter_state_error("恢复时有效配置不存在"))?
            .identity
            .node_ip
            .clone();
        self.clear_adapter(&ip).await?;
        {
            let mut status = self.status.write().await;
            status.phase = Phase::Stopped;
            status.stage = "已清理上次异常终止遗留的专用网卡状态".into();
            status.last_error = None;
        }
        self.log("上次异常终止遗留的专用网卡状态已清理").await;
        Ok(())
    }

    async fn clear_adapter(&mut self, ip: &str) -> Result<()> {
        let mut first_error = None;
        self.route_manager.begin_cleanup();
        self.write_adapter_state(ip)?;
        for operation in self.route_manager.pending().to_vec() {
            let RouteOperation::Remove(route) = operation else {
                continue;
            };
            if let Err(error) = self.remove_managed_route(ip, &route).await {
                first_error.get_or_insert(error);
            }
        }
        if self.adapter_address_configured {
            #[cfg(windows)]
            {
                let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
                let mut c = Command::new(PathBuf::from(system).join("System32/netsh.exe"));
                c.args(windows_clear_adapter_args(&self.settings.interface, ip));
                match run_adapter(c).await {
                    Ok(()) => self.adapter_address_configured = false,
                    Err(error) => first_error = Some(error),
                }
            }
            #[cfg(unix)]
            {
                let mut c = Command::new("/usr/sbin/ip");
                c.args(linux_clear_address_args(
                    &self.settings.interface,
                    ip,
                    self.settings.interface_prefix,
                ));
                match run_adapter(c).await {
                    Ok(()) => self.adapter_address_configured = false,
                    Err(error) => first_error = Some(error),
                }
            }
        }
        #[cfg(unix)]
        if self.adapter_link_touched {
            let mut c = Command::new("/usr/sbin/ip");
            c.args(linux_link_args(&self.settings.interface, "down"));
            match run_adapter(c).await {
                Ok(()) => self.adapter_link_touched = false,
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => (),
            }
        }
        match first_error {
            Some(_) => Err(Error::new(
                "adapter_cleanup",
                "Tinc 已停止，但专用网卡地址或状态清理失败；请重试停止并检查服务权限",
                true,
            )),
            None => self.remove_adapter_state().map_err(|_| {
                Error::new(
                    "adapter_cleanup",
                    "专用网卡已恢复空闲，但无法清除恢复记录；请检查服务数据目录权限后重试停止",
                    true,
                )
            }),
        }
    }
    pub async fn stop(&mut self) -> Result<()> {
        self.vpn_epoch = self.vpn_epoch.wrapping_add(1);
        self.active_keepalive_request = None;
        if let Some(child) = &mut self.child {
            tokio::time::timeout(Duration::from_secs(10), child.kill())
                .await
                .map_err(|_| Error::new("stop_timeout", "停止 Tinc 超时，请刷新实际状态", true))?
                .map_err(|_| Error::new("stop_failed", "停止 Tinc 失败", true))?;
            self.child = None;
        }
        let adapter_ip = self
            .active
            .as_ref()
            .map(|active| active.identity.node_ip.clone());
        if let Some(ip) = adapter_ip {
            self.clear_adapter(&ip).await?;
        }
        {
            let mut s = self.status.write().await;
            s.process_running = false;
            s.connection = "unknown".into();
        }
        self.probe_target = None;
        self.last_probe = None;
        self.probe_started_at = None;
        self.consecutive_probe_failures = 0;
        self.route_revision = None;
        self.route_lease_deadline = None;
        self.keepalive_failure_streak = 0;
        self.stage(
            if self.active.is_some() {
                Phase::Stopped
            } else {
                Phase::Unconfigured
            },
            "VPN 已停止",
        )
        .await;
        Ok(())
    }

    async fn probe_connection(&mut self) {
        let Some(target) = self.probe_target else {
            return;
        };
        self.last_probe = Some(tokio::time::Instant::now());
        let result = connectivity_probe(target).await;
        let within_grace = self
            .probe_started_at
            .is_some_and(|started| started.elapsed() < CONNECTIVITY_GRACE_PERIOD);
        let current = self.status.read().await.connection.clone();
        let (failures, connection) = next_probe_connection(
            result,
            within_grace,
            &current,
            self.consecutive_probe_failures,
        );
        self.consecutive_probe_failures = failures;
        let message = match result {
            Some(true) => format!("VPN 隧道已验证：服务端节点 {target} 可达"),
            Some(false) if within_grace && current != "connected" => {
                format!("正在建立安全隧道；服务端节点 {target} 尚未响应")
            }
            Some(false) if connection == "connected" => format!(
                "服务端节点 {target} 单次探测未响应（连续 {failures}/{CONNECTIVITY_FAILURE_THRESHOLD}），正在复核；Tinc 进程仍在运行"
            ),
            Some(false) if connection == "unknown" => format!(
                "正在复核服务端节点 {target} 连通性（连续 {failures}/{CONNECTIVITY_FAILURE_THRESHOLD}）"
            ),
            Some(false) => format!("Tinc 进程运行中；服务端节点 {target} 暂不可达"),
            None => "Tinc 进程运行中；系统连通性探测工具不可用".into(),
        };
        let changed = {
            let mut status = self.status.write().await;
            let changed = status.connection != connection;
            status.connection = connection.into();
            status.stage = message.clone();
            changed
        };
        if changed {
            self.log(&message).await;
        }
    }
}

async fn connectivity_probe(target: std::net::Ipv4Addr) -> Option<bool> {
    #[cfg(windows)]
    let mut command = {
        let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let mut command = Command::new(PathBuf::from(system).join("System32/ping.exe"));
        command.args(["-n", "1", "-w", "1000", &target.to_string()]);
        command
    };
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new("/usr/bin/ping");
        command.args(["-c", "1", "-W", "1", &target.to_string()]);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    hide(&mut command);
    match tokio::time::timeout(Duration::from_secs(2), command.status()).await {
        Ok(Ok(status)) => Some(status.success()),
        Ok(Err(_)) => None,
        Err(_) => Some(false),
    }
}

#[cfg(windows)]
fn windows_set_adapter_args(interface: &str, ip: &str, prefix: u8) -> Vec<String> {
    let mask = std::net::Ipv4Addr::from(u32::MAX << (32 - prefix));
    vec![
        "interface".into(),
        "ipv4".into(),
        "set".into(),
        "address".into(),
        format!("name={interface}"),
        "source=static".into(),
        format!("address={ip}"),
        format!("mask={mask}"),
        "gateway=none".into(),
        "store=active".into(),
    ]
}

#[cfg(windows)]
fn windows_clear_adapter_args(interface: &str, ip: &str) -> Vec<String> {
    vec![
        "interface".into(),
        "ipv4".into(),
        "delete".into(),
        "address".into(),
        format!("name={interface}"),
        format!("address={ip}"),
        "store=active".into(),
    ]
}

#[cfg(windows)]
fn windows_add_route_args(interface: &str, target: std::net::Ipv4Addr) -> Vec<String> {
    vec![
        "interface".into(),
        "ipv4".into(),
        "add".into(),
        "route".into(),
        format!("prefix={target}/32"),
        format!("interface={interface}"),
        "nexthop=0.0.0.0".into(),
        "store=active".into(),
    ]
}

#[cfg(windows)]
fn windows_route_conflicts(best: platform_windows::Ipv4Route, tinc_index: u32) -> bool {
    best.prefix_length > 0 && best.interface_index != tinc_index
}

#[cfg(windows)]
fn windows_delete_route_args(interface: &str, target: &str, prefix: u8) -> Vec<String> {
    vec![
        "interface".into(),
        "ipv4".into(),
        "delete".into(),
        "route".into(),
        format!("prefix={target}/{prefix}"),
        format!("interface={interface}"),
        "store=active".into(),
    ]
}

#[cfg(unix)]
fn linux_set_address_args(interface: &str, ip: &str, prefix: u8) -> Vec<String> {
    vec![
        "address".into(),
        "replace".into(),
        format!("{ip}/{prefix}"),
        "dev".into(),
        interface.into(),
    ]
}

#[cfg(unix)]
fn linux_clear_address_args(interface: &str, ip: &str, prefix: u8) -> Vec<String> {
    vec![
        "address".into(),
        "delete".into(),
        format!("{ip}/{prefix}"),
        "dev".into(),
        interface.into(),
    ]
}

#[cfg(unix)]
fn linux_inspect_route_args(interface: &str, target: &str, prefix: u8) -> Vec<String> {
    vec![
        "route".into(),
        "show".into(),
        "exact".into(),
        format!("{target}/{prefix}"),
        "dev".into(),
        interface.into(),
    ]
}

#[cfg(unix)]
fn linux_matching_route_args(target: std::net::Ipv4Addr) -> Vec<String> {
    vec![
        "-j".into(),
        "-4".into(),
        "route".into(),
        "show".into(),
        "table".into(),
        "all".into(),
        "match".into(),
        format!("{target}/32"),
    ]
}

#[cfg(any(unix, test))]
fn parse_linux_route_conflict(
    bytes: &[u8],
    target: std::net::Ipv4Addr,
    interface: &str,
) -> Option<bool> {
    let routes = serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()?
        .as_array()?
        .clone();
    let target = u32::from(target);
    let mut best_prefix = 0u8;
    let mut best_conflicts = false;
    for route in routes {
        let object = route.as_object()?;
        let destination = object.get("dst")?.as_str()?;
        let (network, prefix) = if destination == "default" {
            (0, 0)
        } else if let Some((network, prefix)) = destination.split_once('/') {
            let network = u32::from(network.parse::<std::net::Ipv4Addr>().ok()?);
            let prefix = prefix.parse::<u8>().ok().filter(|prefix| *prefix <= 32)?;
            (network, prefix)
        } else {
            (
                u32::from(destination.parse::<std::net::Ipv4Addr>().ok()?),
                32,
            )
        };
        let mask = if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - prefix)
        };
        if target & mask != network & mask || prefix < best_prefix {
            continue;
        }
        let conflicts =
            prefix > 0 && object.get("dev").and_then(serde_json::Value::as_str) != Some(interface);
        if prefix > best_prefix {
            best_prefix = prefix;
            best_conflicts = conflicts;
        } else {
            best_conflicts |= conflicts;
        }
    }
    Some(best_conflicts)
}

#[cfg(any(unix, test))]
fn linux_add_route_args(interface: &str, target: std::net::Ipv4Addr) -> Vec<String> {
    vec![
        "route".into(),
        "add".into(),
        format!("{target}/32"),
        "dev".into(),
        interface.into(),
    ]
}

#[cfg(unix)]
fn linux_delete_route_args(interface: &str, target: &str, prefix: u8) -> Vec<String> {
    vec![
        "route".into(),
        "delete".into(),
        format!("{target}/{prefix}"),
        "dev".into(),
        interface.into(),
    ]
}

#[cfg(unix)]
fn linux_link_args(interface: &str, state: &str) -> Vec<String> {
    vec![
        "link".into(),
        "set".into(),
        "dev".into(),
        interface.into(),
        state.into(),
    ]
}

fn hide(c: &mut Command) {
    #[cfg(windows)]
    {
        c.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    let _ = c;
}

async fn run_adapter(mut c: Command) -> Result<()> {
    c.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    hide(&mut c);
    let output = tokio::time::timeout(Duration::from_secs(10), c.output())
        .await
        .map_err(|_| Error::new("adapter_timeout", "虚拟网卡设置超时", true))?
        .map_err(|_| Error::new("adapter", "无法执行系统网卡配置工具", false))?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = format!(
            "虚拟网卡配置失败 (exit={})。stdout: {}; stderr: {}",
            output.status.code().unwrap_or(-1),
            stdout.trim(),
            stderr.trim(),
        );
        return Err(Error::new("adapter", &detail, true));
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn adapter_commands_use_confirmed_prefix_and_delete_only_owned_address() {
        assert_eq!(
            windows_set_adapter_args("tinc0", "10.42.0.2", 32),
            [
                "interface",
                "ipv4",
                "set",
                "address",
                "name=tinc0",
                "source=static",
                "address=10.42.0.2",
                "mask=255.255.255.255",
                "gateway=none",
                "store=active",
            ]
        );
        assert_eq!(
            windows_clear_adapter_args("tinc0", "10.42.0.2"),
            [
                "interface",
                "ipv4",
                "delete",
                "address",
                "name=tinc0",
                "address=10.42.0.2",
                "store=active",
            ]
        );
        assert_eq!(
            windows_add_route_args("tinc0", "10.42.0.1".parse().unwrap()),
            [
                "interface",
                "ipv4",
                "add",
                "route",
                "prefix=10.42.0.1/32",
                "interface=tinc0",
                "nexthop=0.0.0.0",
                "store=active",
            ]
        );
        assert_eq!(
            windows_delete_route_args("tinc0", "10.42.0.1", 32),
            [
                "interface",
                "ipv4",
                "delete",
                "route",
                "prefix=10.42.0.1/32",
                "interface=tinc0",
                "store=active",
            ]
        );
    }

    #[test]
    fn route_conflict_ignores_default_route_and_owned_tinc_route() {
        assert!(!windows_route_conflicts(
            platform_windows::Ipv4Route {
                interface_index: 7,
                prefix_length: 0,
            },
            42,
        ));
        assert!(!windows_route_conflicts(
            platform_windows::Ipv4Route {
                interface_index: 42,
                prefix_length: 32,
            },
            42,
        ));
    }

    #[test]
    fn route_conflict_rejects_more_specific_route_on_another_interface() {
        assert!(windows_route_conflicts(
            platform_windows::Ipv4Route {
                interface_index: 7,
                prefix_length: 24,
            },
            42,
        ));
        assert!(windows_route_conflicts(
            platform_windows::Ipv4Route {
                interface_index: 7,
                prefix_length: 32,
            },
            42,
        ));
    }
}

#[cfg(test)]
mod linux_route_parser_tests {
    use super::*;

    #[test]
    fn default_route_does_not_block_an_exact_vpn_route() {
        let routes = br#"[{"dst":"default","gateway":"192.0.2.1","dev":"eth0"}]"#;
        assert_eq!(
            parse_linux_route_conflict(routes, "10.42.0.3".parse().unwrap(), "tinc0"),
            Some(false)
        );
    }

    #[test]
    fn local_lan_route_on_another_interface_is_a_conflict() {
        let routes = br#"[
            {"dst":"default","gateway":"192.0.2.1","dev":"eth0"},
            {"dst":"10.42.0.0/24","dev":"eth0","protocol":"kernel"}
        ]"#;
        assert_eq!(
            parse_linux_route_conflict(routes, "10.42.0.3".parse().unwrap(), "tinc0"),
            Some(true)
        );
    }

    #[test]
    fn a_more_specific_owned_route_wins_over_a_lan_route() {
        let routes = br#"[
            {"dst":"10.42.0.0/24","dev":"eth0"},
            {"dst":"10.42.0.3","dev":"tinc0"}
        ]"#;
        assert_eq!(
            parse_linux_route_conflict(routes, "10.42.0.3".parse().unwrap(), "tinc0"),
            Some(false)
        );
    }

    #[test]
    fn malformed_linux_route_json_fails_closed() {
        assert_eq!(
            parse_linux_route_conflict(b"not-json", "10.42.0.3".parse().unwrap(), "tinc0"),
            None
        );
    }

    #[test]
    fn linux_add_never_replaces_an_existing_route() {
        assert_eq!(
            linux_add_route_args("tinc0", "10.42.0.3".parse().unwrap()),
            ["route", "add", "10.42.0.3/32", "dev", "tinc0"]
        );
    }
}

#[cfg(all(test, unix))]
mod linux_tests {
    use super::*;

    #[test]
    fn adapter_commands_delete_only_owned_address_then_lower_link() {
        assert_eq!(
            linux_set_address_args("tinc0", "10.42.0.2", 32),
            ["address", "replace", "10.42.0.2/32", "dev", "tinc0"]
        );
        assert_eq!(
            linux_clear_address_args("tinc0", "10.42.0.2", 32),
            ["address", "delete", "10.42.0.2/32", "dev", "tinc0"]
        );
        assert_eq!(
            linux_link_args("tinc0", "down"),
            ["link", "set", "dev", "tinc0", "down"]
        );
        assert_eq!(
            linux_inspect_route_args("tinc0", "10.42.0.1", 32),
            ["route", "show", "exact", "10.42.0.1/32", "dev", "tinc0"]
        );
        assert_eq!(
            linux_add_route_args("tinc0", "10.42.0.1".parse().unwrap()),
            ["route", "add", "10.42.0.1/32", "dev", "tinc0"]
        );
        assert_eq!(
            linux_delete_route_args("tinc0", "10.42.0.1", 32),
            ["route", "delete", "10.42.0.1/32", "dev", "tinc0"]
        );
    }
}

#[cfg(test)]
mod tinc_log_tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn sanitizes_tinc_output_before_it_reaches_the_activity_log() {
        assert_eq!(sanitize_tinc_output("\r\n\t"), None);
        assert_eq!(
            sanitize_tinc_output("connected\u{1b}[31m peer"),
            Some("connected[31m peer".into())
        );
        assert_eq!(
            sanitize_tinc_output("token=must-not-appear"),
            Some("已省略可能包含敏感字段的 Tinc 输出".into())
        );
        let long = "x".repeat(MAX_TINC_LOG_CHARS + 1);
        let sanitized = sanitize_tinc_output(&long).unwrap();
        assert_eq!(sanitized.chars().count(), MAX_TINC_LOG_CHARS + 1);
        assert!(sanitized.ends_with('…'));
    }

    #[test]
    fn starts_tinc_with_connection_level_diagnostics() {
        assert_eq!(
            tinc_start_args(Path::new("C:/protected/generation")),
            [
                OsString::from("-D"),
                OsString::from("--debug=1"),
                OsString::from("-c"),
                OsString::from("C:/protected/generation"),
            ]
        );
    }

    #[tokio::test]
    async fn captures_bounded_prefixed_lines_without_exposing_sensitive_values() {
        let (mut writer, reader) = tokio::io::duplex(1024);
        let logs = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn(capture_tinc_output(reader, "stderr", logs.clone()));
        writer
            .write_all(b"peer connected\npassword=secret\n")
            .await
            .unwrap();
        drop(writer);
        task.await.unwrap();

        let logs = logs.lock().await;
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].message, "[Tinc stderr] peer connected");
        assert_eq!(
            logs[1].message,
            "[Tinc stderr] 已省略可能包含敏感字段的 Tinc 输出"
        );
        assert!(!logs.iter().any(|entry| entry.message.contains("secret")));
    }

    #[tokio::test]
    async fn activity_log_keeps_only_the_latest_bounded_entries() {
        let logs = Arc::new(Mutex::new(Vec::new()));
        for index in 0..MAX_LOG_ENTRIES + 5 {
            append_log(&logs, format!("entry-{index}")).await;
        }
        let logs = logs.lock().await;
        assert_eq!(logs.len(), MAX_LOG_ENTRIES);
        assert_eq!(logs.first().unwrap().message, "entry-5");
        assert_eq!(logs.last().unwrap().message, "entry-204");
    }
}

#[cfg(test)]
mod connectivity_probe_tests {
    use super::*;

    #[test]
    fn transient_ping_loss_requires_three_consecutive_failures() {
        let (failures, connection) = next_probe_connection(Some(false), false, "connected", 0);
        assert_eq!((failures, connection), (1, "connected"));
        let (failures, connection) =
            next_probe_connection(Some(false), false, connection, failures);
        assert_eq!((failures, connection), (2, "connected"));
        let (failures, connection) =
            next_probe_connection(Some(false), false, connection, failures);
        assert_eq!((failures, connection), (3, "unreachable"));
        assert_eq!(
            next_probe_connection(Some(true), false, connection, failures),
            (0, "connected")
        );
    }

    #[test]
    fn startup_grace_does_not_downgrade_an_already_verified_gateway() {
        assert_eq!(
            next_probe_connection(Some(false), true, "unknown", 0),
            (1, "unknown")
        );
        assert_eq!(
            next_probe_connection(Some(false), true, "connected", 0),
            (1, "connected")
        );
        assert_eq!(
            next_probe_connection(None, false, "connected", 2),
            (0, "unknown")
        );
    }
}

#[cfg(test)]
mod route_lifecycle_tests {
    use super::*;

    fn route(last: u8) -> RouteState {
        RouteState {
            destination: format!("10.0.11.{last}"),
            prefix: 32,
        }
    }

    #[test]
    fn lease_check_starts_only_after_tinc_is_fully_running() {
        let now = std::time::Instant::now();
        assert!(!running_route_lease_expired(true, false, None, now));
        assert!(running_route_lease_expired(true, true, None, now));
        assert!(!running_route_lease_expired(
            true,
            true,
            Some(now + Duration::from_secs(1)),
            now
        ));
        assert!(running_route_lease_expired(true, true, Some(now), now));
        assert!(!running_route_lease_expired(false, true, None, now));
    }

    #[test]
    fn lease_is_invalid_at_the_deadline_and_valid_until_just_before_it() {
        let now = std::time::Instant::now();
        assert!(!route_lease_is_valid(None, now));
        assert!(!route_lease_is_valid(Some(now), now));
        assert!(route_lease_is_valid(
            Some(now + Duration::from_nanos(1)),
            now
        ));
    }

    #[test]
    fn keepalive_diagnostics_are_bounded_to_safe_categories_and_monotonic_time() {
        let now = std::time::Instant::now();
        assert_eq!(keepalive_failure_label("network_timeout"), "网络请求超时");
        assert_eq!(
            keepalive_failure_label("server-secret-detail"),
            "未知临时错误"
        );
        assert_eq!(
            route_lease_remaining_seconds(Some(now + Duration::from_millis(1)), now),
            1
        );
        assert_eq!(route_lease_remaining_seconds(Some(now), now), 0);
        assert_eq!(route_lease_remaining_seconds(None, now), 0);
        assert_eq!(keepalive_recovery_message(0, Some(now), now), None);
        assert_eq!(
            keepalive_recovery_message(2, Some(now + Duration::from_secs(120)), now),
            Some("路由授权续期已恢复（此前连续失败 2 次）；新租约剩余约 120 秒".into())
        );
    }

    #[test]
    fn unchanged_revision_and_routes_skip_reconciliation() {
        let current = vec![route(1), route(212)];
        assert!(!route_snapshot_needs_reconcile(
            Some("routes-fixture"),
            &current,
            false,
            "routes-fixture",
            &current,
        )
        .unwrap());
        assert!(route_snapshot_needs_reconcile(
            Some("routes-fixture"),
            &current,
            false,
            "routes-next",
            &current,
        )
        .unwrap());
    }

    #[test]
    fn pending_operations_force_reconciliation_of_an_unchanged_snapshot() {
        let current = vec![route(1), route(212)];
        assert!(route_snapshot_needs_reconcile(
            Some("routes-fixture"),
            &current,
            true,
            "routes-fixture",
            &current,
        )
        .unwrap());
    }

    #[test]
    fn same_revision_with_different_routes_is_rejected() {
        let current = vec![route(1), route(212)];
        let error = route_snapshot_needs_reconcile(
            Some("routes-fixture"),
            &current,
            false,
            "routes-fixture",
            &[route(1), route(211)],
        )
        .unwrap_err();
        assert_eq!(error.code, "protocol");
    }

    #[test]
    fn stale_or_out_of_order_keepalive_ticket_is_never_current() {
        let current = KeepaliveTicket {
            session_epoch: 4,
            vpn_epoch: 7,
            request_id: 10,
        };
        assert!(keepalive_ticket_is_current(current, 4, 7, Some(10)));
        assert!(!keepalive_ticket_is_current(current, 5, 7, Some(10)));
        assert!(!keepalive_ticket_is_current(current, 4, 8, Some(10)));
        assert!(!keepalive_ticket_is_current(current, 4, 7, Some(11)));
    }

    #[test]
    fn missing_node_immediately_invalidates_the_session_without_becoming_revocation() {
        assert!(session_authorization_lost("auth_expired"));
        assert!(session_authorization_lost("assignment_missing"));
        assert!(!session_authorization_lost("node_revoked"));
        assert!(keepalive_error_requires_immediate_failure("node_revoked"));
        assert!(keepalive_error_requires_immediate_failure(
            "assignment_missing"
        ));
        assert!(!keepalive_error_requires_immediate_failure(
            "route_snapshot_unavailable"
        ));
    }
}

#[cfg(test)]
mod adapter_state_tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            data_dir: PathBuf::from("/protected/data"),
            tinc_binary: PathBuf::from("/protected/tincd"),
            interface: "tinc0".into(),
            owner: "owner".into(),
            interface_prefix: 32,
        }
    }

    fn active() -> Active {
        Active {
            generation: "gen-safe".into(),
            identity: Identity {
                sid: "client_1".into(),
                net_name: "office".into(),
                node_ip: "10.42.0.2".into(),
                node_id: None,
                network_id: None,
                access_code: None,
            },
            profile: "a".repeat(64),
            revoked: false,
        }
    }

    #[test]
    fn recovery_state_requires_exact_current_configuration() {
        let settings = settings();
        let active = active();
        let manager = RouteManager::default();
        let mut state = AdapterState::current(&settings, &active, &manager, None, true);
        state.validate(&settings, Some(&active)).unwrap();

        state.address = "10.42.0.3".into();
        assert_eq!(
            state.validate(&settings, Some(&active)).unwrap_err().code,
            "adapter_state_invalid"
        );
        assert!(
            AdapterState::current(&settings, &active, &manager, None, true)
                .validate(&settings, None)
                .is_err()
        );
    }

    #[test]
    fn recovery_state_rejects_unknown_json_fields() {
        let settings = settings();
        let active = active();
        let manager = RouteManager::default();
        let mut value = serde_json::to_value(AdapterState::current(
            &settings, &active, &manager, None, true,
        ))
        .unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("unexpected".into(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<AdapterState>(value).is_err());
    }

    #[test]
    fn version_two_state_serializes_stable_ids_and_explicit_route_sets() {
        let settings = settings();
        let mut active = active();
        active.identity.node_id = Some(211);
        active.identity.network_id = Some(11);
        let gateway = RouteState {
            destination: "10.42.0.1".into(),
            prefix: 32,
        };
        let manager = RouteManager::restore(
            vec![gateway.clone()],
            Vec::new(),
            vec![RouteOperation::Add(gateway)],
        );

        let value = serde_json::to_value(AdapterState::current(
            &settings,
            &active,
            &manager,
            Some("network-11-fixture"),
            true,
        ))
        .unwrap();

        assert_eq!(value["version"], 2);
        assert_eq!(value["node_id"], 211);
        assert_eq!(value["network_id"], 11);
        assert!(value.get("desired_routes").is_some());
        assert!(value.get("owned_routes").is_some());
        assert!(value.get("pending_operations").is_some());
        assert_eq!(value["route_revision"], "network-11-fixture");
        assert!(value.get("routes").is_none());
        #[cfg(unix)]
        assert_eq!(value["link_touched"], true);
    }

    #[test]
    fn version_two_current_state_round_trips() {
        let settings = settings();
        let active = active();
        let manager = RouteManager::default();
        let value = serde_json::to_value(AdapterState::current(
            &settings, &active, &manager, None, false,
        ))
        .unwrap();

        let state: AdapterState = serde_json::from_value(value).unwrap();
        state.validate(&settings, Some(&active)).unwrap();
        let restored = state.route_manager();
        assert!(restored.desired().is_empty());
        assert!(restored.owned().is_empty());
        assert!(restored.pending().is_empty());
        #[cfg(unix)]
        assert!(!state.owns_link_state());
    }

    #[cfg(unix)]
    #[test]
    fn version_two_state_requires_explicit_link_ownership() {
        let settings = settings();
        let active = active();
        let manager = RouteManager::default();
        let mut value = serde_json::to_value(AdapterState::current(
            &settings, &active, &manager, None, false,
        ))
        .unwrap();
        value.as_object_mut().unwrap().remove("link_touched");

        let state: AdapterState = serde_json::from_value(value).unwrap();
        assert_eq!(
            state.validate(&settings, Some(&active)).unwrap_err().code,
            "adapter_state_invalid"
        );
    }

    #[test]
    fn managed_route_state_accepts_exact_peers_and_rejects_unsafe_entries() {
        let gateway = "10.42.0.1".parse().unwrap();
        let own = "10.42.0.2".parse().unwrap();
        let routes = vec![
            RouteState {
                destination: "10.42.0.1".into(),
                prefix: 32,
            },
            RouteState {
                destination: "10.42.0.3".into(),
                prefix: 32,
            },
        ];
        validate_route_states(&routes, own, Some(gateway)).unwrap();

        let mut invalid = routes.clone();
        invalid[1].prefix = 24;
        assert!(validate_route_states(&invalid, own, Some(gateway)).is_err());

        let mut missing_gateway = routes.clone();
        missing_gateway.remove(0);
        assert!(validate_route_states(&missing_gateway, own, Some(gateway)).is_err());

        let mut duplicate = routes.clone();
        duplicate.push(routes[1].clone());
        assert!(validate_route_states(&duplicate, own, Some(gateway)).is_err());
    }

    #[test]
    fn completed_route_cleanup_forgets_only_the_deleted_route() {
        let gateway = RouteState {
            destination: "10.42.0.1".into(),
            prefix: 32,
        };
        let peer = RouteState {
            destination: "10.42.0.3".into(),
            prefix: 32,
        };
        let mut manager = RouteManager::restore(
            vec![gateway.clone(), peer.clone()],
            vec![gateway.clone(), peer.clone()],
            vec![RouteOperation::Remove(gateway.clone())],
        );
        manager.complete(&RouteOperation::Remove(gateway));

        assert_eq!(manager.owned(), &[peer]);
    }

    #[test]
    fn managed_routes_keep_gateway_first_and_deduplicate_it() {
        let gateway = "10.42.0.1".parse().unwrap();
        let allowed = vec![
            AllowedRoute {
                destination: gateway,
                prefix: 32,
            },
            AllowedRoute {
                destination: "10.42.0.3".parse().unwrap(),
                prefix: 32,
            },
        ];

        let routes = managed_routes(gateway, &allowed).unwrap();

        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].destination, "10.42.0.1");
        assert_eq!(routes[1].destination, "10.42.0.3");
    }

    #[test]
    fn version_one_state_restores_routes_as_desired_and_owned() {
        let root = tempfile::tempdir().unwrap();
        let mut settings = settings();
        settings.data_dir = root.path().to_path_buf();
        let active = active();
        let hosts = root.path().join("generations/gen-safe/hosts");
        std::fs::create_dir_all(&hosts).unwrap();
        std::fs::write(
            hosts.join("server_master"),
            include_str!("../tests/fixtures/backend-config/server_master"),
        )
        .unwrap();
        let legacy = serde_json::json!({
            "version": 1,
            "interface": "tinc0",
            "address": "10.42.0.2",
            "prefix": 32,
            "generation": "gen-safe",
            "profile": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "routes": [
                {"destination": "10.254.0.1", "prefix": 32},
                {"destination": "10.42.0.3", "prefix": 32}
            ]
        });
        let state: AdapterState = serde_json::from_value(legacy).unwrap();

        state.validate(&settings, Some(&active)).unwrap();
        #[cfg(unix)]
        assert!(!state.owns_link_state());
        let restored = state.route_manager();
        assert_eq!(restored.desired(), restored.owned());
        assert!(restored.pending().is_empty());
    }

    #[test]
    fn background_keepalive_waits_for_fully_running_state() {
        assert!(!background_keepalive_due(true, false, None, false));
        assert!(background_keepalive_due(true, true, None, false));
        assert!(!background_keepalive_due(true, true, None, true));
        assert!(!background_keepalive_due(
            true,
            true,
            Some(tokio::time::Instant::now()),
            false,
        ));
    }
}
