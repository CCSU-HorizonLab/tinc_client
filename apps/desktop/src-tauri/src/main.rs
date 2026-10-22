#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use vpn_core::{Error, LogEntry, Reply, Request, Result, Status};

#[cfg(target_os = "linux")]
mod linux_setup {
    use std::{
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::Path,
        process::Command,
    };
    use vpn_core::{Error, Result};

    const PAYLOAD_ROOT: &str = "/usr/share/tinc-connect/payload";
    const SETUP_SCRIPT: &str = "/usr/share/tinc-connect/payload/setup-agent.sh";

    fn verify_root_owned(path: &Path, directory: bool, executable: bool) -> Result<()> {
        let metadata = std::fs::symlink_metadata(path).map_err(|_| {
            Error::new(
                "agent_payload_missing",
                "Linux Agent 安装资源不完整，请重新安装当前 DEB 软件包",
                false,
            )
        })?;
        let kind_ok = if directory {
            metadata.file_type().is_dir()
        } else {
            metadata.file_type().is_file() && !metadata.file_type().is_symlink()
        };
        let safe_mode = metadata.uid() == 0 && metadata.permissions().mode() & 0o022 == 0;
        let executable_ok = !executable || metadata.permissions().mode() & 0o111 != 0;
        if !kind_ok || !safe_mode || !executable_ok {
            return Err(Error::new(
                "agent_payload_untrusted",
                "Linux Agent 安装资源的所有者或权限不安全，请重新安装当前 DEB 软件包",
                false,
            ));
        }
        Ok(())
    }

    pub fn install() -> Result<()> {
        verify_root_owned(Path::new("/usr/share/tinc-connect"), true, false)?;
        verify_root_owned(Path::new(PAYLOAD_ROOT), true, false)?;
        verify_root_owned(Path::new(SETUP_SCRIPT), false, false)?;
        verify_root_owned(
            Path::new("/usr/share/tinc-connect/payload/install-agent.sh"),
            false,
            false,
        )?;
        verify_root_owned(
            Path::new("/usr/share/tinc-connect/payload/tinc-client-agent.service"),
            false,
            false,
        )?;
        verify_root_owned(
            Path::new("/usr/share/tinc-connect/payload/vpn-agent"),
            false,
            true,
        )?;
        verify_root_owned(Path::new("/usr/bin/pkexec"), false, true).map_err(|_| {
            Error::new(
                "privilege_helper_missing",
                "系统缺少受支持的 pkexec 提权组件，无法从界面安装本地 Agent",
                false,
            )
        })?;

        let status = Command::new("/usr/bin/pkexec")
            .args(["/bin/sh", SETUP_SCRIPT])
            .status()
            .map_err(|_| {
                Error::new(
                    "agent_setup_failed",
                    "无法启动 Linux Agent 安装授权，请确认桌面会话支持系统授权窗口",
                    true,
                )
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::new(
                "agent_setup_failed",
                "Linux Agent 安装未完成；若已取消授权可重试，否则请检查 Tinc 1.0.36、iproute2、ping、systemd 与 TUN",
                true,
            ))
        }
    }
}

async fn status_request(request: Request) -> Result<Status> {
    match vpn_ipc::call(request).await? {
        Reply::Status(s) => Ok(*s),
        Reply::Error(e) => Err(e),
        _ => Err(Error::protocol("服务返回类型错误")),
    }
}
#[tauri::command]
fn get_platform() -> &'static str {
    #[cfg(windows)]
    return "windows";
    #[cfg(target_os = "linux")]
    return "linux";
}
#[tauri::command]
async fn setup_linux_agent() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return tauri::async_runtime::spawn_blocking(linux_setup::install)
            .await
            .map_err(|_| {
                Error::new(
                    "agent_setup_failed",
                    "Linux Agent 安装任务异常结束，请重试",
                    true,
                )
            })?;
    }
    #[cfg(windows)]
    Err(Error::new(
        "unsupported_platform_action",
        "Windows Agent 必须通过当前版本安装程序安装或升级",
        false,
    ))
}
#[tauri::command]
async fn login(
    server: String,
    access_code: Option<String>,
    sid: Option<String>,
    password: String,
    allow_http: bool,
) -> Result<Status> {
    status_request(Request::Login {
        server,
        access_code,
        sid,
        password: zeroize::Zeroizing::new(password),
        allow_http,
    })
    .await
}
#[tauri::command]
async fn configure() -> Result<Status> {
    status_request(Request::Configure).await
}
#[tauri::command]
async fn rotate_key() -> Result<Status> {
    status_request(Request::RotateKey).await
}
#[tauri::command]
async fn start_vpn() -> Result<Status> {
    status_request(Request::StartVpn).await
}
#[tauri::command]
async fn stop_vpn() -> Result<Status> {
    status_request(Request::StopVpn).await
}
#[tauri::command]
async fn get_status() -> Result<Status> {
    status_request(Request::GetStatus).await
}
#[tauri::command]
async fn logout() -> Result<Status> {
    status_request(Request::Logout).await
}
#[tauri::command]
async fn get_recent_logs() -> Result<Vec<LogEntry>> {
    match vpn_ipc::call(Request::GetRecentLogs).await? {
        Reply::Logs(l) => Ok(l),
        Reply::Error(e) => Err(e),
        _ => Err(Error::protocol("服务返回类型错误")),
    }
}
fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            login,
            configure,
            rotate_key,
            start_vpn,
            stop_vpn,
            get_status,
            get_recent_logs,
            logout,
            get_platform,
            setup_linux_agent
        ])
        .run(tauri::generate_context!())
        .expect("desktop runtime failed");
}
