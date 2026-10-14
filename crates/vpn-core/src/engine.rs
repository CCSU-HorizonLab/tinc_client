use crate::{
    api::Session,
    config::{self, Active},
    keys, *,
};
use std::{
    collections::BTreeSet,
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

const ADAPTER_STATE_VERSION: u8 = 1;
const ADAPTER_STATE_FILE: &str = "adapter-state.json";
const CONNECTIVITY_PROBE_INTERVAL: Duration = Duration::from_secs(5);
const CONNECTIVITY_GRACE_PERIOD: Duration = Duration::from_secs(15);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);
const MAX_MANAGED_ROUTES: usize = 1025;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteState {
    destination: String,
    prefix: u8,
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
    #[serde(default)]
    routes: Vec<RouteState>,
    #[cfg(unix)]
    link_touched: bool,
}

impl AdapterState {
    fn current(
        settings: &Settings,
        active: &Active,
        routes: Vec<RouteState>,
        _link_touched: bool,
    ) -> Self {
        Self {
            version: ADAPTER_STATE_VERSION,
            interface: settings.interface.clone(),
            address: active.identity.node_ip.clone(),
            prefix: settings.interface_prefix,
            generation: active.generation.clone(),
            profile: active.profile.clone(),
            routes,
            #[cfg(unix)]
            link_touched: _link_touched,
        }
    }

    fn validate(&self, settings: &Settings, active: Option<&Active>) -> Result<()> {
        let active = active.ok_or_else(|| adapter_state_error("有效配置不存在"))?;
        let matches = self.version == ADAPTER_STATE_VERSION
            && self.interface == settings.interface
            && self.address == active.identity.node_ip
            && self.prefix == settings.interface_prefix
            && self.generation == active.generation
            && self.profile == active.profile;
        let address = self.address.parse::<std::net::Ipv4Addr>();
        if !matches || address.is_err() {
            return Err(adapter_state_error("记录与当前管理员设置或有效配置不一致"));
        }
        let required_gateway = if self.routes.is_empty() {
            None
        } else {
            let target = config::server_probe_target(&settings.data_dir, active)?;
            Some(target)
        };
        validate_route_states(&self.routes, address.unwrap(), required_gateway)?;
        Ok(())
    }
}

fn validate_route_states(
    routes: &[RouteState],
    own_ip: std::net::Ipv4Addr,
    required_gateway: Option<std::net::Ipv4Addr>,
) -> Result<()> {
    if routes.len() > MAX_MANAGED_ROUTES {
        return Err(adapter_state_error("记录中的路由数量超过安全上限"));
    }
    let mut destinations = BTreeSet::new();
    for route in routes {
        let destination = route
            .destination
            .parse::<std::net::Ipv4Addr>()
            .map_err(|_| adapter_state_error("记录包含无效 IPv4 路由"))?;
        let octets = destination.octets();
        if route.prefix != 32
            || destination == own_ip
            || destination.is_unspecified()
            || destination.is_loopback()
            || destination.is_multicast()
            || octets == [255, 255, 255, 255]
            || (octets[0] == 169 && octets[1] == 254)
            || !destinations.insert(destination)
        {
            return Err(adapter_state_error("记录包含非预期路由"));
        }
    }
    if required_gateway.is_some_and(|gateway| !destinations.contains(&gateway)) {
        return Err(adapter_state_error("记录缺少当前服务端节点路由"));
    }
    Ok(())
}

fn forget_route(routes: &mut Vec<RouteState>, deleted: &RouteState) {
    if let Some(index) = routes.iter().position(|route| route == deleted) {
        routes.remove(index);
    }
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

#[derive(Debug, PartialEq)]
struct RoutePlan {
    add: Vec<RouteState>,
    remove: Vec<RouteState>,
}

fn route_plan(current: &[RouteState], desired: &[RouteState]) -> RoutePlan {
    RoutePlan {
        add: desired
            .iter()
            .filter(|route| !current.contains(route))
            .cloned()
            .collect(),
        remove: current
            .iter()
            .filter(|route| !desired.contains(route))
            .cloned()
            .collect(),
    }
}

fn adapter_state_error(reason: &str) -> Error {
    Error::new(
        "adapter_state_invalid",
        format!("网卡恢复记录无效（{reason}）；已拒绝自动清理，请由管理员核查服务数据目录"),
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
    active: Option<Active>,
    child: Option<Child>,
    adapter_address_configured: bool,
    adapter_routes: Vec<RouteState>,
    probe_target: Option<std::net::Ipv4Addr>,
    last_probe: Option<tokio::time::Instant>,
    probe_started_at: Option<tokio::time::Instant>,
    last_keepalive: Option<tokio::time::Instant>,
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
            active,
            child: None,
            adapter_address_configured: adapter_state.is_some(),
            adapter_routes: adapter_state
                .as_ref()
                .map(|state| state.routes.clone())
                .unwrap_or_default(),
            probe_target: None,
            last_probe: None,
            probe_started_at: None,
            last_keepalive: None,
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
            let mut status = self.status.write().await;
            status.session = None;
            status.phase = Phase::Failed;
            status.stage = error.message.clone();
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
        if self.child.is_some()
            && self
                .last_keepalive
                .is_none_or(|last| last.elapsed() >= KEEPALIVE_INTERVAL)
        {
            let keepalive = if let Some(session) = self.session.as_ref() {
                self.last_keepalive = Some(tokio::time::Instant::now());
                Some(session.keepalive().await)
            } else {
                None
            };
            if let Some(keepalive) = keepalive {
                if let Err(error) = &keepalive {
                    if error.code == "node_revoked" || error.code == "auth_expired" {
                        return Err(error.clone());
                    }
                }
                if let Ok(Some(allowed_routes)) = keepalive {
                    let gateway = self.probe_target.ok_or_else(|| {
                        Error::new("route_sync", "VPN 运行状态缺少网关路由目标", true)
                    })?;
                    let desired = managed_routes(gateway, &allowed_routes)?;
                    self.reconcile_routes(&desired).await?;
                }
            }
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
        let refreshed_routes = session.keepalive().await?;
        if !self.settings.tinc_binary.is_file() {
            return Err(Error::new(
                "tinc_missing",
                "未安装管理员指定的 Tinc 核心程序",
                false,
            ));
        }
        let probe_target = config::server_probe_target(&self.settings.data_dir, &active)?;
        let routes = managed_routes(
            probe_target,
            refreshed_routes
                .as_deref()
                .unwrap_or_else(|| session.allowed_routes()),
        )?;
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
            for route in routes {
                self.install_managed_route(ip, route).await?;
            }
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
            for route in routes {
                self.install_managed_route(ip, route).await?;
            }
        }
        Ok(())
    }

    async fn install_managed_route(&mut self, ip: &str, route: &RouteState) -> Result<()> {
        if self.adapter_routes.contains(route) {
            return Ok(());
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
            let mut c = Command::new("/usr/sbin/ip");
            c.args(linux_add_route_args(&self.settings.interface, target));
            run_adapter(c).await?;
        }
        self.adapter_routes.push(route.clone());
        self.write_adapter_state(ip)
    }

    async fn remove_managed_route(&mut self, ip: &str, route: &RouteState) -> Result<()> {
        if !self.adapter_routes.contains(route) {
            return Ok(());
        }
        #[cfg(windows)]
        {
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
            let mut c = Command::new("/usr/sbin/ip");
            c.args(linux_delete_route_args(
                &self.settings.interface,
                &route.destination,
                route.prefix,
            ));
            run_adapter(c).await?;
        }
        forget_route(&mut self.adapter_routes, route);
        self.write_adapter_state(ip)
    }

    async fn reconcile_routes(&mut self, desired: &[RouteState]) -> Result<()> {
        let ip = self
            .active
            .as_ref()
            .ok_or_else(|| Error::new("route_sync", "有效配置不存在，无法同步路由", false))?
            .identity
            .node_ip
            .clone();
        let plan = route_plan(&self.adapter_routes, desired);
        let changed = !plan.add.is_empty() || !plan.remove.is_empty();
        for route in &plan.add {
            self.install_managed_route(&ip, route)
                .await
                .map_err(|error| match error.code.as_str() {
                    "route_conflict" | "route_check" => error,
                    _ => Error::new("route_sync", "新增授权节点路由失败，将在下次心跳重试", true),
                })?;
        }
        for route in &plan.remove {
            self.remove_managed_route(&ip, route).await.map_err(|_| {
                Error::new("route_sync", "撤销过期节点路由失败，将在下次心跳重试", true)
            })?;
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

    fn adapter_cleanup_pending(&self) -> bool {
        self.adapter_address_configured || !self.adapter_routes.is_empty() || {
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
        let state = AdapterState::current(&self.settings, active, self.adapter_routes.clone(), {
            #[cfg(unix)]
            {
                self.adapter_link_touched
            }
            #[cfg(not(unix))]
            {
                false
            }
        });
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
        for route in self.adapter_routes.clone() {
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
        let mut state = AdapterState::current(&settings, &active, Vec::new(), true);
        state.validate(&settings, Some(&active)).unwrap();

        state.address = "10.42.0.3".into();
        assert_eq!(
            state.validate(&settings, Some(&active)).unwrap_err().code,
            "adapter_state_invalid"
        );
        assert!(AdapterState::current(&settings, &active, Vec::new(), true)
            .validate(&settings, None)
            .is_err());
    }

    #[test]
    fn recovery_state_rejects_unknown_json_fields() {
        let settings = settings();
        let active = active();
        let mut value =
            serde_json::to_value(AdapterState::current(&settings, &active, Vec::new(), true))
                .unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("unexpected".into(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<AdapterState>(value).is_err());
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
    fn successful_route_cleanup_forgets_only_the_deleted_route() {
        let gateway = RouteState {
            destination: "10.42.0.1".into(),
            prefix: 32,
        };
        let peer = RouteState {
            destination: "10.42.0.3".into(),
            prefix: 32,
        };
        let mut routes = vec![gateway.clone(), peer.clone()];

        forget_route(&mut routes, &gateway);

        assert_eq!(routes, vec![peer]);
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
    fn route_plan_adds_new_routes_and_removes_only_stale_owned_routes() {
        let gateway = RouteState {
            destination: "10.42.0.1".into(),
            prefix: 32,
        };
        let stale = RouteState {
            destination: "10.42.0.3".into(),
            prefix: 32,
        };
        let added = RouteState {
            destination: "10.42.0.4".into(),
            prefix: 32,
        };

        let plan = route_plan(
            &[gateway.clone(), stale.clone()],
            &[gateway.clone(), added.clone()],
        );

        assert_eq!(plan.add, vec![added]);
        assert_eq!(plan.remove, vec![stale]);
    }
}
