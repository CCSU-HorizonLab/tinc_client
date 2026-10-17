use crate::{identifier, AllowedRoute, Error, Identity, Result};
use reqwest::{redirect::Policy, Client};
use serde_json::{json, Value};
use std::{collections::BTreeSet, net::Ipv4Addr, time::Duration};
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

#[derive(Clone)]
pub struct Session {
    pub identity: Identity,
    pub origin: String,
    allowed_routes: Vec<AllowedRoute>,
    token: Zeroizing<String>,
    client: Client,
}

const MAX_ALLOWED_ROUTES: usize = 1024;

fn parse_allowed_routes(value: &Value, own_ip: Ipv4Addr) -> Result<Option<Vec<AllowedRoute>>> {
    let Some(routes) = value.get("allowed_routes") else {
        return Ok(None);
    };
    let routes = routes
        .as_array()
        .ok_or_else(|| Error::protocol("allowed_routes 必须是 IPv4 /32 路由数组"))?;
    if routes.len() > MAX_ALLOWED_ROUTES {
        return Err(Error::protocol("allowed_routes 超过客户端安全上限"));
    }
    let mut parsed = BTreeSet::new();
    for route in routes {
        let route = route
            .as_str()
            .filter(|route| !route.is_empty() && route.trim() == *route)
            .ok_or_else(|| Error::protocol("allowed_routes 包含无效路由"))?;
        let (address, _) = route
            .split_once('/')
            .filter(|(_, prefix)| *prefix == "32")
            .ok_or_else(|| Error::protocol("allowed_routes 仅支持 IPv4 /32 路由"))?;
        if address.contains('/') {
            return Err(Error::protocol("allowed_routes 包含无效路由"));
        }
        let destination = address
            .parse::<Ipv4Addr>()
            .map_err(|_| Error::protocol("allowed_routes 包含无效 IPv4 地址"))?;
        let octets = destination.octets();
        if destination == own_ip
            || destination.is_unspecified()
            || destination.is_loopback()
            || destination.is_multicast()
            || octets == [255, 255, 255, 255]
            || (octets[0] == 169 && octets[1] == 254)
        {
            return Err(Error::protocol("allowed_routes 包含不允许的目标地址"));
        }
        parsed.insert(AllowedRoute {
            destination,
            prefix: 32,
        });
    }
    Ok(Some(parsed.into_iter().collect()))
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
pub fn validate_access_code(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.len() != 12 {
        return Err(Error::protocol(
            "接入码格式无效，必须为 12 位，例如 TL-7K9M-2Q4F",
        ));
    }
    let upper = trimmed.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if !bytes.starts_with(b"TL-") || bytes[7] != b'-' {
        return Err(Error::protocol(
            "接入码格式无效，必须以 TL- 开头并包含连字符，例如 TL-7K9M-2Q4F",
        ));
    }
    const VALID_CHARS: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
    for (i, &b) in bytes.iter().enumerate() {
        if i < 3 || i == 7 {
            continue;
        }
        if !VALID_CHARS.contains(&b) {
            return Err(Error::protocol(
                "接入码包含无效字符，支持字符集为 23456789ABCDEFGHJKMNPQRSTUVWXYZ",
            ));
        }
    }
    Ok(upper)
}

pub fn map_backend_error(code: &str, fallback_msg: Option<&str>) -> Error {
    match code {
        "CLIENT_CREDENTIAL_INVALID" => Error::new(
            "credential_invalid",
            fallback_msg.unwrap_or("接入码或密码错误，请核对后重试"),
            false,
        ),
        "CLIENT_LOGIN_RATE_LIMITED" => {
            Error::new("login_rate_limited", "登录尝试过于频繁，请稍后重试", true)
        }
        "CLIENT_LOGIN_LIMITER_UNAVAILABLE" => Error::new(
            "login_unavailable",
            "登录保护服务暂时不可用，请稍后重试",
            true,
        ),
        "CLIENT_LEGACY_LOGIN_AMBIGUOUS" => Error::new(
            "login_ambiguous",
            fallback_msg.unwrap_or("检测到多个同名节点，请使用接入码登录"),
            false,
        ),
        "CLIENT_SID_INVALID" => Error::new(
            "protocol",
            fallback_msg.unwrap_or("节点标识格式无效"),
            false,
        ),
        "CLIENT_NETWORK_IP_INVALID" => Error::new(
            "assignment_missing",
            fallback_msg.unwrap_or("节点网络 IP 配置无效，请联系管理员"),
            false,
        ),
        "CLIENT_NETWORK_NOT_FOUND" => Error::new(
            "assignment_missing",
            fallback_msg.unwrap_or("后台找不到节点或网络关联，请联系管理员检查绑定关系"),
            false,
        ),
        "SYSTEM_ERROR" => Error::new(
            "server_error",
            fallback_msg.unwrap_or("服务端内部错误，可以稍后重试"),
            true,
        ),
        "CLIENT_TOKEN_INVALID" => {
            Error::new("auth_expired", "鉴权失败或会话已失效，请重新登录", false)
        }
        "CLIENT_SESSION_UNAVAILABLE" => Error::new(
            "session_unavailable",
            "后台会话服务暂时不可用，可以稍后重试",
            true,
        ),
        "CLIENT_KEEPALIVE_FAILED" => {
            Error::new("keepalive_failed", "心跳服务暂时不可用，可以稍后重试", true)
        }
        "GATEWAY_HOST_UNAVAILABLE" | "GATEWAY_UNAVAILABLE" => Error::new(
            "gateway_unavailable",
            "网关配置暂时不可用，请联系管理员检查网关",
            true,
        ),
        "GATEWAY_KEY_APPLY_FAILED" => Error::new(
            "gateway_apply",
            "网关公钥写入或重载失败，旧配置已保留，可以重试",
            true,
        ),
        "CLIENT_PUBLIC_KEY_INVALID" => Error::new(
            "key_rejected",
            "后台拒绝本机公钥格式，请联系管理员核对客户端与版本",
            false,
        ),
        "CLIENT_NODE_NOT_FOUND" => Error::new(
            "assignment_missing",
            "后台找不到节点或网络关联，请联系管理员检查绑定关系",
            false,
        ),
        "CLIENT_NODE_REVOKED" => Error::new(
            "node_revoked",
            "该节点已被后台撤销，VPN 已停止；本机私钥将保留但不能继续连接",
            false,
        ),
        "CLIENT_CONTENT_TYPE_INVALID" | "CLIENT_REQUEST_INVALID" | "CLIENT_ACTION_INVALID" => {
            Error::new(
                "protocol",
                "客户端请求与后台契约不一致，请联系管理员核对版本",
                false,
            )
        }
        _ => Error::new(
            "server_error",
            fallback_msg.unwrap_or("服务端返回未知业务错误"),
            false,
        ),
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
    if let Some(code) = code.as_deref() {
        map_backend_error(code, None)
    } else {
        Error::new(
            "http",
            format!("服务端返回 HTTP {status}"),
            status >= 500 || status == 429,
        )
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
    pub async fn login(
        server: &str,
        access_code: Option<&str>,
        sid: Option<&str>,
        password: &str,
        allow_http: bool,
    ) -> Result<Self> {
        let normalized_access_code = match access_code {
            Some(code) if !code.trim().is_empty() => Some(validate_access_code(code)?),
            _ => None,
        };

        let body = if let Some(code) = &normalized_access_code {
            json!({
                "accessCode": code,
                "password": password
            })
        } else if let Some(s) = sid.filter(|s| !s.trim().is_empty()) {
            identifier(s)?;
            if s.eq_ignore_ascii_case("server_master") {
                return Err(Error::protocol("server_master 为协议保留的服务端主机名"));
            }
            json!({
                "sid": s,
                "password": password
            })
        } else {
            return Err(Error::protocol("请提供接入码或节点名称"));
        };

        if password.is_empty() || password.len() > 1024 {
            return Err(Error::protocol("密码不能为空或超过长度限制"));
        }
        let origin = origin(server, allow_http)?.to_string();
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            // RSA-3072 generation can outlive a server's keep-alive window. Drop
            // idle pooled sockets early instead of reusing a half-closed one for
            // the subsequent public-key upload.
            .pool_idle_timeout(Duration::from_secs(2))
            .build()
            .map_err(network)?;
        let response = post(&client, &origin, "/api/tinc/client/login", body, 64 * 1024).await?;
        let v: Value = serde_json::from_slice(&response.body)
            .map_err(|_| Error::protocol("登录响应不是有效 JSON"))?;
        if v == json!(0) || v["status"] == json!(0) {
            if let Some(code) = v["code"].as_str().filter(|s| !s.is_empty()) {
                return Err(map_backend_error(code, v["msg"].as_str()));
            }
            return Err(Error::new(
                "auth_failed",
                "登录被拒绝，请核对接入码、密码及后台节点关联",
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
        let returned_sid = field("sid")?;
        identifier(&returned_sid)?;
        let returned_net_name = field("net_name")?;
        identifier(&returned_net_name)?;
        let returned_access_code = v["access_code"]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_owned);

        if let Some(expected_code) = &normalized_access_code {
            if let Some(actual_code) = &returned_access_code {
                if !actual_code.eq_ignore_ascii_case(expected_code) {
                    return Err(Error::protocol("登录返回的接入码与请求不一致"));
                }
            }
        } else if let Some(expected_sid) = sid {
            if returned_sid != expected_sid {
                return Err(Error::protocol("登录返回的节点名与请求不一致"));
            }
        }

        let node_ip = field("node_ip")?;
        let node_address = node_ip
            .parse::<Ipv4Addr>()
            .map_err(|_| Error::protocol("后台分配的 node_ip 必须是 IPv4 地址"))?;
        let allowed_routes = parse_allowed_routes(&v, node_address)?.unwrap_or_default();
        let identity = Identity {
            sid: returned_sid,
            net_name: returned_net_name,
            node_ip,
            node_id: v["node_id"].as_u64(),
            network_id: v["network_id"].as_u64(),
            access_code: returned_access_code.or(normalized_access_code),
        };
        Ok(Self {
            identity,
            origin,
            allowed_routes,
            token: Zeroizing::new(field("token")?),
            client,
        })
    }
    pub fn allowed_routes(&self) -> &[AllowedRoute] {
        &self.allowed_routes
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
    pub async fn keepalive(&self) -> Result<Option<Vec<AllowedRoute>>> {
        let response = post(
            &self.client,
            &self.origin,
            "/api/tinc/client/keepalive",
            json!({"sid":self.identity.sid,"token":self.token.as_str()}),
            64 * 1024,
        )
        .await?;
        if !has_media_type(response.content_type.as_deref(), "application/json") {
            return Err(Error::protocol("心跳成功响应必须是 application/json"));
        }
        let value: Value = serde_json::from_slice(&response.body)
            .map_err(|_| Error::protocol("心跳响应不是有效 JSON"))?;
        if value["status"] != json!("success") {
            return Err(Error::protocol("心跳响应缺少成功状态"));
        }
        let own_ip = self
            .identity
            .node_ip
            .parse::<Ipv4Addr>()
            .map_err(|_| Error::protocol("活动节点地址无效"))?;
        parse_allowed_routes(&value, own_ip)
    }
}
