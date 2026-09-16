use crate::{identifier, Error, Identity, Result};
use reqwest::{redirect::Policy, Client};
use serde_json::{json, Value};
use std::time::Duration;
use url::Url;
use zeroize::Zeroizing;

struct PostResponse {
    body: Vec<u8>,
    content_type: Option<String>,
}

#[derive(serde::Deserialize)]
struct BackendErrorCode {
    code: Option<String>,
}

fn has_media_type(value: Option<&str>, expected: &str) -> bool {
    value.is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case(expected))
    })
}

pub struct Session {
    pub identity: Identity,
    pub origin: String,
    token: Zeroizing<String>,
    client: Client,
}

pub fn origin(value: &str, allow_http: bool) -> Result<Url> {
    let url = Url::parse(value)
        .map_err(|_| Error::protocol("请输入完整服务端地址，例如 https://vpn.example.com"))?;
    if !matches!(url.scheme(), "https" | "http")
        || (url.scheme() == "http" && !allow_http)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(Error::protocol("地址必须为 HTTPS 服务根地址；使用 HTTP 需要明确允许明文传输，不支持路径、凭据或查询参数"));
    }
    Ok(url)
}
fn network(e: reqwest::Error) -> Error {
    if e.is_timeout() {
        Error::new("network_timeout", "服务端请求超时，可以重试", true)
    } else {
        Error::new(
            "network",
            "无法完成服务端请求，请检查地址、网络和 TLS 证书",
            true,
        )
    }
}
async fn response_error(mut response: reqwest::Response) -> Error {
    let status = response.status().as_u16();
    if matches!(status, 401 | 403) {
        return Error::new("auth_expired", "鉴权失败或会话已失效，请重新登录", false);
    }
    let json = has_media_type(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        "application/json",
    );
    let code = if json && response.content_length().unwrap_or(0) <= 8 * 1024 {
        let mut body = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) if body.len() + chunk.len() <= 8 * 1024 => {
                    body.extend_from_slice(&chunk);
                }
                Ok(Some(_)) | Err(_) => {
                    body.clear();
                    break;
                }
                Ok(None) => break,
            }
        }
        serde_json::from_slice::<BackendErrorCode>(&body)
            .ok()
            .and_then(|value| value.code)
    } else {
        None
    };
    match code.as_deref() {
        Some("CLIENT_TOKEN_INVALID") => {
            Error::new("auth_expired", "鉴权失败或会话已失效，请重新登录", false)
        }
        Some("CLIENT_SESSION_UNAVAILABLE") => Error::new(
            "session_unavailable",
            "后台会话服务暂时不可用，可以稍后重试",
            true,
        ),
        Some("GATEWAY_HOST_UNAVAILABLE") | Some("GATEWAY_UNAVAILABLE") => Error::new(
            "gateway_unavailable",
            "网关配置暂时不可用，请联系管理员检查网关",
            true,
        ),
        Some("GATEWAY_KEY_APPLY_FAILED") => Error::new(
            "gateway_apply",
            "网关公钥写入或重载失败，旧配置已保留，可以重试",
            true,
        ),
        Some("CLIENT_PUBLIC_KEY_INVALID") => Error::new(
            "key_rejected",
            "后台拒绝本机公钥格式，请联系管理员核对客户端与后端版本",
            false,
        ),
        Some("CLIENT_NODE_NOT_FOUND") | Some("CLIENT_NETWORK_NOT_FOUND") => Error::new(
            "assignment_missing",
            "后台找不到节点或网络关联，请联系管理员检查绑定关系",
            false,
        ),
        Some("CLIENT_CONTENT_TYPE_INVALID")
        | Some("CLIENT_REQUEST_INVALID")
        | Some("CLIENT_ACTION_INVALID")
        | Some("CLIENT_SID_INVALID") => Error::new(
            "protocol",
            "客户端请求与后台契约不一致，请联系管理员核对版本",
            false,
        ),
        _ => Error::new(
            "http",
            format!("服务端返回 HTTP {status}"),
            status >= 500 || status == 429,
        ),
    }
}
async fn post(
    client: &Client,
    origin: &str,
    endpoint: &str,
    body: Value,
    limit: usize,
) -> Result<PostResponse> {
    let mut r = client
        .post(format!("{}{}", origin.trim_end_matches('/'), endpoint))
        .json(&body)
        .send()
        .await
        .map_err(network)?;
    if !r.status().is_success() {
        return Err(response_error(r).await);
    }
    if r.content_length().unwrap_or(0) > limit as u64 {
        return Err(Error::protocol("服务端响应超过大小限制"));
    }
    let content_type = r
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut data = Vec::new();
    while let Some(chunk) = r.chunk().await.map_err(network)? {
        if data.len() + chunk.len() > limit {
            return Err(Error::protocol("服务端响应超过大小限制"));
        }
        data.extend_from_slice(&chunk);
    }
    Ok(PostResponse {
        body: data,
        content_type,
    })
}
impl Session {
    pub async fn login(server: &str, sid: &str, password: &str, allow_http: bool) -> Result<Self> {
        identifier(sid)?;
        if sid.eq_ignore_ascii_case("server_master") {
            return Err(Error::protocol("server_master 为协议保留的服务端主机名"));
        }
        if password.is_empty() || password.len() > 1024 {
            return Err(Error::protocol("密码不能为空或超过长度限制"));
        }
        let origin = origin(server, allow_http)?.to_string();
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(network)?;
        let response = post(
            &client,
            &origin,
            "/api/tinc/client/login",
            json!({"sid":sid,"password":password}),
            64 * 1024,
        )
        .await?;
        let v: Value = serde_json::from_slice(&response.body)
            .map_err(|_| Error::protocol("登录响应不是有效 JSON"))?;
        if v == json!(0) || v["status"] == json!(0) {
            return Err(Error::new(
                "auth_failed",
                "登录被拒绝，请核对节点名、密码及后台节点关联",
                false,
            ));
        }
        if !v["status"].as_i64().is_some_and(|s| s > 0) {
            return Err(Error::protocol("登录响应缺少有效 status，不能确认登录成功"));
        }
        let field = |key: &str| {
            v[key]
                .as_str()
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| Error::protocol("登录响应缺少 sid/token/net_name/node_ip"))
        };
        let identity = Identity {
            sid: field("sid")?,
            net_name: field("net_name")?,
            node_ip: field("node_ip")?,
        };
        identifier(&identity.sid)?;
        identifier(&identity.net_name)?;
        if identity.sid != sid {
            return Err(Error::protocol("登录返回的节点名与请求不一致"));
        }
        identity
            .node_ip
            .parse::<std::net::Ipv4Addr>()
            .map_err(|_| Error::protocol("后台分配的 node_ip 必须是 IPv4 地址"))?;
        Ok(Self {
            identity,
            origin,
            token: Zeroizing::new(field("token")?),
            client,
        })
    }
    pub async fn download(&self) -> Result<Vec<u8>> {
        let response = post(
            &self.client,
            &self.origin,
            "/api/tinc/client/config/download",
            json!({"sid":self.identity.sid,"token":self.token.as_str()}),
            4 * 1024 * 1024,
        )
        .await?;
        if !has_media_type(response.content_type.as_deref(), "application/zip") {
            return Err(Error::protocol(
                "配置下载成功响应必须是 application/zip，不能确认配置包来源",
            ));
        }
        if response.body.starts_with(b"PK\x03\x04") {
            Ok(response.body)
        } else {
            Err(Error::protocol(
                "下载结果不是 ZIP 配置包；现有协议未定义待确认响应，需后台检查关联/审批及会话",
            ))
        }
    }
    pub async fn upload(&self, public: &str) -> Result<String> {
        let response = post(
            &self.client,
            &self.origin,
            "/api/tinc/client/key/upload",
            json!({
                "sid": self.identity.sid,
                "token": self.token.as_str(),
                "content": public,
                "action": "exchangeFile"
            }),
            64 * 1024,
        )
        .await?;
        let plain_text = has_media_type(response.content_type.as_deref(), "text/plain");
        if !plain_text {
            return Err(Error::protocol(
                "公钥上传成功响应必须是 text/plain 主机配置，不能确认密钥交换结果",
            ));
        }
        String::from_utf8(response.body)
            .map_err(|_| Error::protocol("公钥上传响应不是 UTF-8 主机配置"))
    }
}
