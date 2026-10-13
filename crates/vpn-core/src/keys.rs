use crate::{Error, Identity, Result};
use rsa::{
    pkcs1::{DecodeRsaPrivateKey, EncodeRsaPrivateKey, EncodeRsaPublicKey, LineEnding},
    RsaPrivateKey, RsaPublicKey,
};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::Path};

pub fn profile_id(origin: &str, identity: &Identity) -> String {
    let scope = match (identity.network_id, identity.node_id) {
        (Some(network_id), Some(node_id)) => format!("id\0{network_id}\0{node_id}"),
        _ => format!("{}\0{}", identity.net_name, identity.sid),
    };
    format!("{:x}", Sha256::digest(format!("{origin}\0{scope}")))
}
pub fn legacy_profile_id(origin: &str, identity: &Identity) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{origin}\0{}\0{}", identity.net_name, identity.sid))
    )
}
pub fn migrate_legacy_key(root: &Path, legacy: &str, profile: &str) -> Result<()> {
    if legacy == profile {
        return Ok(());
    }
    let keys = root.join("keys");
    private_dir(&keys)?;
    let source = keys.join(legacy);
    let target = keys.join(profile);
    if target.exists() || !source.exists() {
        return Ok(());
    }
    fs::rename(source, target).map_err(Error::io)
}
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let mut tmp = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| Error::protocol("无效存储路径"))?,
    )
    .map_err(Error::io)?;
    tmp.write_all(data).map_err(Error::io)?;
    tmp.as_file().sync_all().map_err(Error::io)?;
    tmp.persist(path).map_err(|e| Error::io(e.error))?;
    Ok(())
}
pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(Error::io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(Error::io)?;
    }
    Ok(())
}
// Single durable private key is the source of truth. Retrying an upload never rotates it.
pub fn ensure_key(root: &Path, id: &str, identity: &Identity) -> Result<String> {
    let folder = root.join("keys").join(id);
    private_dir(&folder)?;
    let path = folder.join("rsa_key.priv");
    let private = if path.exists() {
        let pem = zeroize::Zeroizing::new(fs::read_to_string(&path).map_err(Error::io)?);
        RsaPrivateKey::from_pkcs1_pem(&pem).map_err(|_| {
            Error::new(
                "key_invalid",
                "本机私钥损坏，已停止操作，未重新生成或覆盖",
                false,
            )
        })?
    } else {
        let private = RsaPrivateKey::new(&mut rand::rngs::OsRng, 3072)
            .map_err(|_| Error::new("key_generation", "本机 RSA 密钥生成失败", true))?;
        let pem = private
            .to_pkcs1_pem(LineEnding::LF)
            .map_err(|_| Error::protocol("RSA 私钥编码失败"))?;
        atomic_write(&path, pem.as_bytes())?;
        private
    };
    let public = RsaPublicKey::from(&private)
        .to_pkcs1_pem(LineEnding::LF)
        .map_err(|_| Error::protocol("RSA 公钥编码失败"))?;
    Ok(format!("Subnet = {}/32\n\n{}", identity.node_ip, public))
}

pub struct CandidateKey {
    pub private_pem: zeroize::Zeroizing<String>,
    pub public_host: String,
}

pub fn generate_candidate(identity: &Identity) -> Result<CandidateKey> {
    let private = RsaPrivateKey::new(&mut rand::rngs::OsRng, 3072)
        .map_err(|_| Error::new("key_generation", "本机 RSA 密钥生成失败", true))?;
    let private_pem = private
        .to_pkcs1_pem(LineEnding::LF)
        .map_err(|_| Error::protocol("RSA 私钥编码失败"))?;
    let public = RsaPublicKey::from(&private)
        .to_pkcs1_pem(LineEnding::LF)
        .map_err(|_| Error::protocol("RSA 公钥编码失败"))?;
    Ok(CandidateKey {
        private_pem,
        public_host: format!("Subnet = {}/32\n\n{}", identity.node_ip, public),
    })
}

pub fn replace_private_key(root: &Path, id: &str, private_pem: &[u8]) -> Result<()> {
    let folder = root.join("keys").join(id);
    private_dir(&folder)?;
    atomic_write(&folder.join("rsa_key.priv"), private_pem)
}

pub fn store_candidate(root: &Path, id: &str, private_pem: &[u8]) -> Result<()> {
    let folder = root.join("keys").join(id);
    private_dir(&folder)?;
    atomic_write(&folder.join("rsa_key.candidate"), private_pem)
}

pub fn activate_candidate(root: &Path, id: &str) -> Result<zeroize::Zeroizing<Vec<u8>>> {
    let folder = root.join("keys").join(id);
    let current =
        zeroize::Zeroizing::new(fs::read(folder.join("rsa_key.priv")).map_err(Error::io)?);
    let candidate =
        zeroize::Zeroizing::new(fs::read(folder.join("rsa_key.candidate")).map_err(Error::io)?);
    atomic_write(&folder.join("rsa_key.priv"), &candidate)?;
    Ok(current)
}

pub fn candidate_exists(root: &Path, id: &str) -> bool {
    root.join("keys")
        .join(id)
        .join("rsa_key.candidate")
        .is_file()
}

pub fn discard_candidate(root: &Path, id: &str) -> Result<()> {
    match fs::remove_file(root.join("keys").join(id).join("rsa_key.candidate")) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(error)),
    }
}
