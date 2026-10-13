use crate::{identifier, keys, Error, Identity, Result};
use rsa::{pkcs1::DecodeRsaPublicKey, traits::PublicKeyParts, RsaPublicKey};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Active {
    pub generation: String,
    pub identity: Identity,
    pub profile: String,
    #[serde(default)]
    pub revoked: bool,
}
pub struct Validated {
    pub files: BTreeMap<String, String>,
}

#[derive(Default)]
struct HostShape {
    addresses: usize,
    ports: usize,
    subnets: Vec<(std::net::Ipv4Addr, u8)>,
}

fn text(bytes: Vec<u8>) -> Result<String> {
    let s = String::from_utf8(bytes).map_err(|_| Error::protocol("配置文件必须为 UTF-8"))?;
    if s.chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(Error::protocol("配置包含非法控制字符"));
    }
    Ok(s)
}
fn directives(s: &str) -> Result<Vec<(String, String)>> {
    s.lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(|line| {
            let (k, v) = line
                .split_once('=')
                .ok_or_else(|| Error::protocol("配置指令必须使用 key = value 格式"))?;
            if v.trim().is_empty() {
                return Err(Error::protocol("配置指令缺少值"));
            }
            Ok((k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        })
        .collect()
}
fn validate_host(s: &str, require_key: bool) -> Result<HostShape> {
    if s.contains("PRIVATE KEY") {
        return Err(Error::protocol("服务端配置禁止包含私钥"));
    }
    let mut shape = HostShape::default();
    let (head, pem) = s
        .split_once("-----BEGIN RSA PUBLIC KEY-----")
        .unwrap_or((s, ""));
    for (key, value) in directives(head)? {
        match key.as_str() {
            "address" => {
                if value.len() > 253
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b".-:".contains(&b))
                {
                    return Err(Error::protocol("主机 Address 无效"));
                }
                shape.addresses += 1;
            }
            "port" => {
                if value.parse::<u16>().ok().filter(|v| *v > 0).is_none() {
                    return Err(Error::protocol("主机 Port 无效"));
                }
                shape.ports += 1;
            }
            "subnet" => {
                let (ip, prefix) = value
                    .split_once('/')
                    .ok_or_else(|| Error::protocol("Subnet 必须为 IPv4/CIDR"))?;
                let ip = ip
                    .parse::<std::net::Ipv4Addr>()
                    .map_err(|_| Error::protocol("Subnet 无效"))?;
                let prefix = prefix
                    .parse::<u8>()
                    .ok()
                    .filter(|n| *n <= 32)
                    .ok_or_else(|| Error::protocol("Subnet 无效"))?;
                shape.subnets.push((ip, prefix));
            }
            _ => return Err(Error::protocol("主机配置包含未允许的指令")),
        }
    }
    if !pem.is_empty() {
        let full = format!("-----BEGIN RSA PUBLIC KEY-----{pem}");
        let key = RsaPublicKey::from_pkcs1_pem(&full)
            .map_err(|_| Error::protocol("主机 RSA 公钥格式无效"))?;
        if !(2048..=8192).contains(&key.n().bits()) {
            return Err(Error::protocol("主机 RSA 公钥长度必须为 2048–8192 位"));
        }
    } else if require_key {
        return Err(Error::protocol(
            "服务端未返回有效 RSA 公钥，不能确认密钥交换完成",
        ));
    }
    Ok(shape)
}
pub fn host(s: &str, require_key: bool) -> Result<()> {
    validate_host(s, require_key).map(|_| ())
}
pub fn client_host(s: &str, expected_ip: &str) -> Result<()> {
    let expected_ip = expected_ip
        .parse::<std::net::Ipv4Addr>()
        .map_err(|_| Error::protocol("后台分配的客户端地址无效"))?;
    let shape = validate_host(s, true)?;
    if shape.addresses != 0 || shape.ports != 0 || shape.subnets.as_slice() != [(expected_ip, 32)] {
        return Err(Error::protocol(
            "客户端公钥 host 必须只包含后台分配的单个 /32 Subnet 和 RSA 公钥",
        ));
    }
    Ok(())
}
pub fn server_host(s: &str) -> Result<()> {
    let shape = validate_host(s, true)?;
    if shape.addresses != 1
        || shape.ports > 1
        || shape.subnets.len() != 1
        || shape.subnets[0].1 != 32
    {
        return Err(Error::protocol(
            "server_master 必须包含唯一 Address、唯一 /32 Subnet、至多一个 Port 和 RSA 公钥",
        ));
    }
    Ok(())
}

pub fn server_probe_target(root: &Path, active: &Active) -> Result<std::net::Ipv4Addr> {
    let contents = fs::read_to_string(generation_path(root, active).join("hosts/server_master"))
        .map_err(Error::io)?;
    let shape = validate_host(&contents, true)?;
    if shape.addresses != 1
        || shape.ports > 1
        || shape.subnets.len() != 1
        || shape.subnets[0].1 != 32
    {
        return Err(Error::protocol(
            "server_master 必须包含唯一 Address、唯一 /32 Subnet、至多一个 Port 和 RSA 公钥",
        ));
    }
    Ok(shape.subnets[0].0)
}
pub fn validate_zip(data: &[u8], identity: &Identity, interface: &str) -> Result<Validated> {
    identifier(&identity.sid)?;
    identifier(&identity.net_name)?;
    let mut zip =
        zip::ZipArchive::new(Cursor::new(data)).map_err(|_| Error::protocol("ZIP 结构无效"))?;
    if zip.len() > 128 {
        return Err(Error::protocol("配置包文件数量超过 128"));
    }
    let mut files = BTreeMap::new();
    let mut names = HashSet::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|_| Error::protocol("ZIP 条目无效"))?;
        let raw = entry.name().to_owned();
        if raw.contains('\\')
            || raw.contains(':')
            || raw.starts_with('/')
            || raw.split('/').any(|s| s == ".." || s == ".")
            || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err(Error::protocol("配置包包含不安全路径或符号链接"));
        }
        let name = raw
            .strip_prefix(&format!("{}/", identity.net_name))
            .unwrap_or(&raw)
            .trim_end_matches('/')
            .to_owned();
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(Error::protocol("配置包包含重复或大小写冲突路径"));
        }
        if entry.is_dir() {
            if name != "hosts" && name != identity.net_name && !name.is_empty() {
                return Err(Error::protocol("配置包包含非预期目录"));
            }
            continue;
        }
        let allowed = name == "tinc.conf"
            || name
                .strip_prefix("hosts/")
                .is_some_and(|s| identifier(s).is_ok());
        if !allowed {
            let visible_name: String = name
                .chars()
                .flat_map(char::escape_default)
                .take(160)
                .collect();
            return Err(Error::new(
                "unsafe_config",
                format!("配置包包含未允许的文件：{visible_name}。仅接收 tinc.conf 和 hosts；脚本、私钥及可执行文件被拒绝"),
                false,
            ));
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| Error::protocol("配置包过大"))?;
        if total > 2 * 1024 * 1024 || entry.size() > 64 * 1024 {
            return Err(Error::protocol("配置包解压大小超过限制"));
        }
        let mut bytes = Vec::new();
        (&mut entry)
            .take(64 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::protocol("配置包 CRC 校验或解压失败"))?;
        if bytes.len() > 64 * 1024 {
            return Err(Error::protocol("配置文件过大"));
        }
        let s = text(bytes)?;
        if name.starts_with("hosts/") {
            if name == "hosts/server_master" {
                server_host(&s)?;
            } else {
                host(&s, false)?;
            }
        }
        files.insert(name, s);
    }
    let conf = files
        .get("tinc.conf")
        .ok_or_else(|| Error::protocol("配置包缺少 tinc.conf"))?;
    let mut seen = HashSet::new();
    let mut peers = Vec::new();
    let mut output = String::new();
    for (key, value) in directives(conf)? {
        if key != "connectto" && !seen.insert(key.clone()) {
            return Err(Error::protocol("tinc.conf 存在重复指令"));
        }
        match key.as_str() {
            "name" => {
                if value != identity.sid {
                    return Err(Error::protocol("配置 Name 与登录节点不一致"));
                }
            }
            "connectto" => {
                identifier(&value)?;
                peers.push(value.clone());
            }
            "interface" => continue, // administrator-selected adapter; never trust downloaded adapter names
            "mode" if value == "router" => (),
            "addressfamily" if matches!(value.as_str(), "ipv4" | "any") => (),
            _ => {
                return Err(Error::protocol(
                    "tinc.conf 包含未允许的指令；文件路径、脚本、外部命令和设备覆盖被禁止",
                ))
            }
        }
        output.push_str(&format!("{key} = {value}\n"));
    }
    if !seen.contains("name") || peers.is_empty() {
        return Err(Error::protocol("配置必须包含 Name 和 ConnectTo"));
    }
    if interface.is_empty()
        || interface.len() > 128
        || interface
            .chars()
            .any(|c| c.is_control() || c == '=' || c == '"')
    {
        return Err(Error::protocol("管理员指定的网卡名称无效"));
    }
    for peer in peers {
        if peer != "server_master" && !files.contains_key(&format!("hosts/{peer}")) {
            return Err(Error::protocol("ConnectTo 指向缺失的主机"));
        }
    }
    output.push_str(&format!("Interface = {interface}\n"));
    files.insert("tinc.conf".into(), output);
    Ok(Validated { files })
}
pub fn read_active(root: &Path) -> Result<Option<Active>> {
    let path = root.join("active.json");
    if !path.exists() {
        return Ok(None);
    }
    let active: Active = serde_json::from_slice(&fs::read(path).map_err(Error::io)?)
        .map_err(|_| Error::protocol("有效配置索引损坏，已停止自动恢复"))?;
    if !active.generation.starts_with("gen-")
        || !active
            .generation
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || active.profile.len() != 64
        || !active.profile.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Error::protocol("有效配置索引路径无效"));
    }
    identifier(&active.identity.sid)?;
    identifier(&active.identity.net_name)?;
    if !generation_path(root, &active).join("tinc.conf").is_file() {
        return Err(Error::protocol("有效配置文件缺失"));
    }
    Ok(Some(active))
}
pub fn generation_path(root: &Path, active: &Active) -> PathBuf {
    root.join("generations").join(&active.generation)
}
pub fn apply(
    root: &Path,
    mut config: Validated,
    identity: &Identity,
    profile: &str,
    public: &str,
    master: &str,
) -> Result<Active> {
    server_host(master)?;
    client_host(public, &identity.node_ip)?;
    config
        .files
        .insert("hosts/server_master".into(), master.into());
    config
        .files
        .insert(format!("hosts/{}", identity.sid), public.into());
    for (name, data) in &config.files {
        if name == "hosts/server_master" {
            server_host(data)?;
        } else if name.starts_with("hosts/") {
            host(data, true)?;
        }
    }
    let generations = root.join("generations");
    keys::private_dir(&generations)?;
    let stage = tempfile::Builder::new()
        .prefix("gen-")
        .tempdir_in(&generations)
        .map_err(Error::io)?;
    keys::private_dir(&stage.path().join("hosts"))?;
    for (name, data) in config.files {
        keys::atomic_write(&stage.path().join(name), data.as_bytes())?;
    }
    let private = zeroize::Zeroizing::new(
        fs::read(root.join("keys").join(profile).join("rsa_key.priv")).map_err(Error::io)?,
    );
    keys::atomic_write(&stage.path().join("rsa_key.priv"), &private)?;
    let active = Active {
        generation: stage
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        identity: identity.clone(),
        profile: profile.into(),
        revoked: false,
    };
    // Commit immutable generation first, then atomically replace the small pointer.
    // A failed pointer write leaves the previous configuration fully usable.
    let _ = stage.keep();
    keys::atomic_write(
        &root.join("active.json"),
        &serde_json::to_vec(&active).map_err(|_| Error::protocol("配置索引编码失败"))?,
    )?;
    Ok(active)
}

pub fn write_active(root: &Path, active: &Active) -> Result<()> {
    keys::atomic_write(
        &root.join("active.json"),
        &serde_json::to_vec(active).map_err(|_| Error::protocol("配置索引编码失败"))?,
    )
}
