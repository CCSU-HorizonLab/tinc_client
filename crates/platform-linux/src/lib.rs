#![cfg(target_os = "linux")]
use std::{
    io,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::Path,
};
pub fn uid() -> u32 {
    unsafe { libc::geteuid() }
}
pub fn secure_socket(path: &Path, owner: u32) -> io::Result<()> {
    let name = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    if unsafe { libc::chown(name.as_ptr(), owner, u32::MAX) } != 0 {
        return Err(io::Error::last_os_error());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}
pub fn verify_peer(stream: &tokio::net::UnixStream, expected: u32) -> io::Result<()> {
    if stream.peer_cred()?.uid() != expected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "IPC peer UID rejected",
        ));
    }
    Ok(())
}
