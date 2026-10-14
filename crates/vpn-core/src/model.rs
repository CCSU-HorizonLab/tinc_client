use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}
impl Error {
    pub fn new(code: &str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }
    pub fn io(_: std::io::Error) -> Self {
        Self::new(
            "storage",
            "本地文件操作失败，请检查服务目录权限及磁盘空间",
            true,
        )
    }
    pub fn protocol(message: &str) -> Self {
        Self::new("protocol", message, false)
    }
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct AllowedRoute {
    pub destination: Ipv4Addr,
    pub prefix: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Unconfigured,
    Configuring,
    WaitingApproval,
    Stopped,
    Starting,
    Running,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub sid: String,
    pub net_name: String,
    pub node_ip: String,
    #[serde(default)]
    pub node_id: Option<u64>,
    #[serde(default)]
    pub network_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_code: Option<String>,
}

impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        self.sid == other.sid
            && self.net_name == other.net_name
            && self.node_ip == other.node_ip
            && self.node_id == other.node_id
            && self.network_id == other.network_id
    }
}
impl Eq for Identity {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub phase: Phase,
    pub session: Option<Identity>,
    pub configured: Option<Identity>,
    pub process_running: bool,
    pub connection: String,
    pub confirmation: String,
    pub stage: String,
    pub last_error: Option<Error>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            phase: Phase::Unconfigured,
            session: None,
            configured: None,
            process_running: false,
            connection: "unknown".into(),
            confirmation: "unavailable".into(),
            stage: "等待配置".into(),
            last_error: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub time: u64,
    pub message: String,
}

// Never derive Debug on requests: the login password must not appear in diagnostics.
#[derive(Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Login {
        server: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access_code: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sid: Option<String>,
        password: zeroize::Zeroizing<String>,
        allow_http: bool,
    },
    Configure,
    RotateKey,
    StartVpn,
    StopVpn,
    GetStatus,
    GetRecentLogs,
    Logout,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Reply {
    Status(Box<Status>),
    Logs(Vec<LogEntry>),
    Error(Error),
}

pub fn identifier(value: &str) -> Result<()> {
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || reserved.contains(&value.to_ascii_uppercase().as_str())
    {
        return Err(Error::protocol(
            "节点/网络名称只能包含 1–64 位字母、数字、下划线，且不能为系统保留名称",
        ));
    }
    Ok(())
}
