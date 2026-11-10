#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    linux::run()
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        ffi::CString,
        fs, io,
        os::unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, PermissionsExt},
            net::UnixListener,
        },
        path::Path,
    };

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let mut args = std::env::args_os().skip(1);
        let directory = args.next().ok_or("missing socket directory")?;
        let owner = args
            .next()
            .ok_or("missing target owner UID")?
            .to_string_lossy()
            .parse::<u32>()?;
        if args.next().is_some() {
            return Err("unexpected fixture argument".into());
        }
        if owner == unsafe { libc::geteuid() } {
            return Err("target owner UID must differ from the service UID".into());
        }

        let legacy_path = Path::new(&directory).join("legacy.sock");
        let _legacy_listener = UnixListener::bind(&legacy_path)?;
        let legacy_error = chown_then_chmod(&legacy_path, owner)
            .expect_err("legacy chown-then-chmod order unexpectedly succeeded");
        if legacy_error.raw_os_error() != Some(libc::EPERM) {
            return Err(format!(
                "legacy chmod failed with unexpected errno: {:?}",
                legacy_error.raw_os_error()
            )
            .into());
        }
        println!("LEGACY_CHMOD_ERRNO={}", libc::EPERM);

        let fixed_path = Path::new(&directory).join("fixed.sock");
        let _fixed_listener = UnixListener::bind(&fixed_path)?;
        platform_linux::secure_socket(&fixed_path, owner)?;
        let metadata = fs::metadata(&fixed_path)?;
        let mode = metadata.permissions().mode() & 0o777;
        if mode != 0o600 {
            return Err(format!("fixed socket mode is {mode:o}, expected 600").into());
        }
        if metadata.uid() != owner {
            return Err(
                format!("fixed socket owner is {}, expected {owner}", metadata.uid()).into(),
            );
        }
        println!("SECURE_SOCKET_OK uid={owner} mode={mode:o}");
        Ok(())
    }

    fn chown_then_chmod(path: &Path, owner: u32) -> io::Result<()> {
        let name = CString::new(path.as_os_str().as_bytes())?;
        if unsafe { libc::chown(name.as_ptr(), owner, u32::MAX) } != 0 {
            return Err(io::Error::last_os_error());
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
    }
}
