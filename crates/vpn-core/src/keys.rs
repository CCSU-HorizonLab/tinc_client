use crate::{Error, Identity, Result};
use rsa::{
    pkcs1::{DecodeRsaPrivateKey, EncodeRsaPrivateKey, EncodeRsaPublicKey, LineEnding},
    RsaPrivateKey, RsaPublicKey,
};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::Path};

pub fn profile_id(origin: &str, identity: &Identity) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{origin}\0{}\0{}", identity.net_name, identity.sid))
    )
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
