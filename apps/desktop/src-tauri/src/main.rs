#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use vpn_core::{Error, LogEntry, Reply, Request, Result, Status};
async fn status_request(request: Request) -> Result<Status> {
    match vpn_ipc::call(request).await? {
        Reply::Status(s) => Ok(*s),
        Reply::Error(e) => Err(e),
        _ => Err(Error::protocol("服务返回类型错误")),
    }
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
            logout
        ])
        .run(tauri::generate_context!())
        .expect("desktop runtime failed");
}
