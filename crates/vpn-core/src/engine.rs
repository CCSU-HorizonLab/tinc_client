use crate::route_manager::{validate_routes, RouteManager, RouteOperation, RouteState};
use crate::{
    api::{RouteSnapshot, Session},
    config::{self, Active},
    keys, *,
};
use std::{
    fs,
    path::Path,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
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
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);
const MAX_MANAGED_ROUTES: usize = 1025;

fn background_keepalive_due(
    child_running: bool,
    process_running: bool,
    last_keepalive: Option<tokio::time::Instant>,
) -> bool {
    child_running
        && process_running
        && last_keepalive.is_none_or(|last| last.elapsed() >= KEEPALIVE_INTERVAL)
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
    link_touched: bool,
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
            link_touched: _link_touched,
        }
    }

    fn validate(&self, settings: &Settings, active: Option<&Active>) -> Result<()> {
        let active = active.ok_or_else(|| adapter_state_error("有效配置不存在"))?;
        let supported_version = matches!(self.version, 1 | ADAPTER_STATE_VERSION);
        let stable_identity_matches = self.version == 1
            || (self.node_id == active.identity.node_id
                && self.network_id == active.identity.network_id);
        let matches = supported_version
            && self.interface == settings.interface
            && self.address == active.identity.node_ip
            && self.prefix == settings.interface_prefix
            && self.generation == active.generation
            && self.profile == active.profile
            && stable_identity_matches;
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
    active: Option<Active>,
    child: Option<Child>,
    adapter_address_configured: bool,
    route_manager: RouteManager,
    probe_target: Option<std::net::Ipv4Addr>,
    last_probe: Option<tokio::time::Instant>,
    probe_started_at: Option<tokio::time::Instant>,
    last_keepalive: Option<tokio::time::Instant>,
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
            last_keepalive: None,
            route_revision: adapter_state
                .as_ref()
                .and_then(|state| state.route_revision.clone()),
            route_lease_deadline: None,
            #[cfg(unix)]
            adapter_link_touched: adapter_state
                .as_ref()
                .is_some_and(|state| state.link_touched),
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
        let mut logs = self.logs.lock().await;
        if logs.len() >= 200 {
            logs.remove(0);
        }
        logs.push(LogEntry {
            time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            message: message.into(),
        });
    }
    pub async fn fail(&mut self, error: Error) -> Reply {
        if error.code == "auth_expired" {
            if self.child.is_some() || self.adapter_cleanup_pending() {
                if let Err(cleanup_error) = self.stop().await {
                    self.log(&cleanup_error.message).await;
                }
            }
            self.session = None;
            self.session_epoch = self.session_epoch.wrapping_add(1);
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
        if self.child.is_some()
            && self.status.read().await.process_running
            && self
                .route_lease_deadline
                .is_none_or(|deadline| std::time::Instant::now() >= deadline)
        {
            return Err(Error::new(
                "route_lease_expired",
                "路由授权租约已到期，VPN 将停止并清理自有路由",
                false,
            ));
        }
        if let Some(child) = &mut self.child {
            if let Some(code) = child.try_wait().map_err(Error::io)? {
                self.child = None;
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
    pub async fn begin_background_keepalive(&mut self) -> Option<(Session, u64)> {
        let process_running = self.status.read().await.process_running;
        if background_keepalive_due(self.child.is_some(), process_running, self.last_keepalive) {
            self.last_keepalive = Some(tokio::time::Instant::now());
            self.session
                .clone()
                .map(|session| (session, self.session_epoch))
        } else {
            None
        }
    }

    pub async fn finish_background_keepalive(
        &mut self,
        session_epoch: u64,
        keepalive: Result<Option<RouteSnapshot>>,
    ) -> Result<()> {
        if session_epoch != self.session_epoch {
            return Ok(());
        }
        if let Err(error) = &keepalive {
            if error.code == "node_revoked" || error.code == "auth_expired" {
                return Err(error.clone());
            }
            // A transient or malformed response does not revoke an otherwise
            // valid lease. It becomes fail-closed only at the saved deadline.
            return self.ensure_route_lease_valid();
        }
        if self.child.is_none() || !self.status.read().await.process_running {
            return Ok(());
        }
        let Some(snapshot) = keepalive.ok().flatten() else {
            // A successful but legacy/malformed heartbeat must not erase an
            // existing authorization lease. Keep using the last snapshot only
            // until its original deadline, then fail closed in poll().
            return self.ensure_route_lease_valid();
        };
        self.apply_route_snapshot(&snapshot).await
    }

    fn ensure_route_lease_valid(&self) -> Result<()> {
        if self
            .route_lease_deadline
            .is_some_and(|deadline| std::time::Instant::now() < deadline)
        {
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
        let revision_changed = self.route_revision.as_deref() != Some(revision);
        self.route_lease_deadline = Some(deadline);
        if revision_changed || self.route_manager.desired() != desired {
            self.route_revision = Some(revision.to_owned());
            self.reconcile_routes(&desired).await?;
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
            .arg("-D")
            .arg("-c")
            .arg(config::generation_path(&self.settings.data_dir, &active))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        hide(&mut command);
        self.child = Some(command.spawn().map_err(|_| {
            Error::new(
                "start_failed",
                "Tinc 进程无法启动，请检查安装、权限和驱动",
                true,
            )
        })?);
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
        self.route_revision = None;
        self.route_lease_deadline = None;
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
        let (connection, message) = match result {
            Some(true) => (
                "connected",
                format!("VPN 隧道已验证：服务端节点 {target} 可达"),
            ),
            Some(false) if within_grace => (
                "unknown",
                format!("正在建立安全隧道；服务端节点 {target} 尚未响应"),
            ),
            Some(false) => (
                "unreachable",
                format!("Tinc 进程运行中；服务端节点 {target} 暂不可达"),
            ),
            None => (
                "unknown",
                "Tinc 进程运行中；系统连通性探测工具不可用".into(),
            ),
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
fn linux_add_route_args(interface: &str, target: std::net::Ipv4Addr) -> Vec<String> {
    vec![
        "route".into(),
        "replace".into(),
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
            ["route", "replace", "10.42.0.1/32", "dev", "tinc0"]
        );
        assert_eq!(
            linux_delete_route_args("tinc0", "10.42.0.1", 32),
            ["route", "delete", "10.42.0.1/32", "dev", "tinc0"]
        );
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
        let restored = state.route_manager();
        assert_eq!(restored.desired(), restored.owned());
        assert!(restored.pending().is_empty());
    }

    #[test]
    fn background_keepalive_waits_for_fully_running_state() {
        assert!(!background_keepalive_due(true, false, None));
        assert!(background_keepalive_due(true, true, None));
        assert!(!background_keepalive_due(
            true,
            true,
            Some(tokio::time::Instant::now())
        ));
    }
}
