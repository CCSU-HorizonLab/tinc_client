use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use vpn_core::{Error, Reply, Request, Result};

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("vpn-ipc currently supports Windows and Linux desktop clients only");
pub const MAX_FRAME: usize = 64 * 1024;
pub fn development() -> bool {
    cfg!(debug_assertions) && std::env::var("TINC_CLIENT_DEV").as_deref() == Ok("1")
}
fn development_instance() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("TINC_CLIENT_DEV_INSTANCE")
        .ok()
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
}
pub fn endpoint(dev: bool) -> String {
    #[cfg(windows)]
    {
        if dev {
            match development_instance() {
                Some(instance) => format!(r"\\.\pipe\tinc-client-dev-v1-{instance}"),
                None => r"\\.\pipe\tinc-client-dev-v1".into(),
            }
        } else {
            r"\\.\pipe\tinc-client-v1".into()
        }
    }
    #[cfg(target_os = "linux")]
    {
        if dev {
            match development_instance() {
                Some(instance) => {
                    format!("/tmp/tinc-client-{}-{instance}.sock", platform_linux::uid())
                }
                None => format!("/tmp/tinc-client-{}.sock", platform_linux::uid()),
            }
        } else {
            "/run/tinc-client/agent.sock".into()
        }
    }
}
pub async fn read_frame<T: serde::de::DeserializeOwned>(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<T> {
    let len = stream.read_u32().await.map_err(|_| ipc_error())? as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(Error::protocol("IPC 消息长度非法"));
    }
    let mut bytes = zero_buffer(len);
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|_| ipc_error())?;
    let parsed = serde_json::from_slice(&bytes).map_err(|_| Error::protocol("IPC 消息格式无效"));
    bytes.fill(0);
    parsed
}
fn zero_buffer(len: usize) -> Vec<u8> {
    vec![0; len]
}
pub async fn write_frame<T: serde::Serialize>(
    stream: &mut (impl AsyncWrite + Unpin),
    value: &T,
) -> Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| Error::protocol("IPC 编码失败"))?;
    if bytes.len() > MAX_FRAME {
        return Err(Error::protocol("IPC 消息超过限制"));
    }
    let result = async {
        stream.write_u32(bytes.len() as u32).await?;
        stream.write_all(&bytes).await?;
        stream.flush().await
    }
    .await;
    bytes.fill(0);
    result.map_err(|_: std::io::Error| ipc_error())
}
fn ipc_error() -> Error {
    #[cfg(windows)]
    let message = "无法连接本地 Agent；请确认 Tinc Client Agent Windows 服务已安装并正在运行，当前用户与安装时授权用户一致";
    #[cfg(target_os = "linux")]
    let message = "无法连接本地 Agent；请确认 tinc-client-agent systemd 服务已启动，当前用户 UID 与 /etc/tinc-client/settings.json 中 owner 一致";
    Error::new("agent_unavailable", message, true)
}
fn ipc_identity_error() -> Error {
    #[cfg(windows)]
    let message = "本地 Agent 身份验证失败；服务进程必须由 LocalSystem 运行，请重新运行当前版本安装程序修复服务";
    #[cfg(target_os = "linux")]
    let message = "本地 Agent 身份验证失败；生产 socket 必须由 root Agent 创建，请检查 systemd 服务和 socket 权限";
    Error::new("ipc_identity", message, false)
}
pub async fn call(request: Request) -> Result<Reply> {
    tokio::time::timeout(Duration::from_secs(150), async {
        let dev = development();
        #[cfg(windows)]
        let mut stream = {
            use std::os::windows::io::AsRawHandle;
            let mut attempts = 0;
            let pipe = loop {
                match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint(dev)) {
                    Ok(p) => break p,
                    Err(e) if e.raw_os_error() == Some(231) && attempts < 20 => {
                        attempts += 1;
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    Err(_) => return Err(ipc_error()),
                }
            };
            unsafe { platform_windows::verify_server(pipe.as_raw_handle(), dev) }
                .map_err(|_| ipc_identity_error())?;
            pipe
        };
        #[cfg(target_os = "linux")]
        let mut stream = {
            let socket = tokio::net::UnixStream::connect(endpoint(dev))
                .await
                .map_err(|_| ipc_error())?;
            platform_linux::verify_peer(&socket, if dev { platform_linux::uid() } else { 0 })
                .map_err(|_| ipc_identity_error())?;
            socket
        };
        write_frame(&mut stream, &request).await?;
        read_frame(&mut stream).await
    })
    .await
    .map_err(|_| {
        Error::new(
            "ipc_timeout",
            "本地服务响应超时，请刷新实际状态后重试",
            true,
        )
    })?
}

#[cfg(test)]
mod tests {
    use super::{ipc_error, ipc_identity_error};

    #[test]
    fn platform_connection_error_is_actionable() {
        let error = ipc_error();
        assert_eq!(error.code, "agent_unavailable");
        assert!(error.retryable);
        #[cfg(windows)]
        assert!(error.message.contains("Windows 服务"));
        #[cfg(target_os = "linux")]
        assert!(error.message.contains("systemd 服务"));
    }

    #[test]
    fn platform_identity_error_is_actionable_and_not_retryable() {
        let error = ipc_identity_error();
        assert_eq!(error.code, "ipc_identity");
        assert!(!error.retryable);
        #[cfg(windows)]
        assert!(error.message.contains("LocalSystem"));
        #[cfg(target_os = "linux")]
        assert!(error.message.contains("root Agent"));
    }
}
