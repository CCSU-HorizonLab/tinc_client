use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Mutex, Notify, Semaphore};
use vpn_core::{
    engine::{Engine, Settings},
    *,
};

pub async fn serve(
    settings_path: PathBuf,
    dev: bool,
    shutdown: Arc<Notify>,
    ready: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let settings_bytes = std::fs::read(&settings_path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::new(
                "settings_missing",
                "找不到服务设置文件；首次开发请先运行 scripts/dev/init.ps1",
                false,
            )
        } else {
            Error::new(
                "settings_read",
                "无法读取服务设置文件，请检查文件权限",
                true,
            )
        }
    })?;
    let settings: Settings = serde_json::from_slice(&settings_bytes)
        .map_err(|_| Error::new("settings_invalid", "服务设置文件格式无效", false))?;
    keys::private_dir(&settings.data_dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(settings.data_dir.join("agent.lock"))
        .map_err(Error::io)?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| Error::new("agent_running", "此配置目录已经由另一个服务进程使用", false))?;
    let mut engine = Engine::new(settings.clone())?;
    if let Err(error) = engine.recover_adapter().await {
        engine.fail(error).await;
    }
    let status = engine.status.clone();
    let logs = engine.logs.clone();
    let engine = Arc::new(Mutex::new(engine));
    let limits = Arc::new(Semaphore::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    let tick_engine = engine.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let session = if let Ok(mut engine) = tick_engine.try_lock() {
                if let Err(error) = engine.poll().await {
                    engine.fail(error).await;
                    None
                } else {
                    engine.begin_background_keepalive().await
                }
            } else {
                None
            };
            let Some((session, session_epoch)) = session else {
                continue;
            };
            let keepalive = session.keepalive().await;
            let mut engine = tick_engine.lock().await;
            if let Err(error) = engine
                .finish_background_keepalive(session_epoch, keepalive)
                .await
            {
                engine.fail(error).await;
            }
        }
    });

    #[cfg(windows)]
    let mut listener = {
        let sid = if dev {
            platform_windows::current_sid().map_err(Error::io)?
        } else {
            settings.owner.clone()
        };
        make_pipe(&sid, dev, true)?
    };
    #[cfg(unix)]
    let listener = make_socket(&settings, dev).await?;

    ready()?;
    loop {
        #[cfg(windows)]
        let incoming = tokio::select! {
            _=shutdown.notified()=>break,
            result=listener.connect()=> {
                result.map_err(Error::io)?;
                let sid=if dev {platform_windows::current_sid().map_err(Error::io)?}else{settings.owner.clone()};
                let next=make_pipe(&sid,dev,false)?;
                std::mem::replace(&mut listener,next)
            }
        };
        #[cfg(unix)]
        let incoming = tokio::select! {
            _=shutdown.notified()=>break,
            result=listener.accept()=> {
                let (stream,_)=result.map_err(Error::io)?;
                let owner=if dev{platform_linux::uid()}else{settings.owner.parse().map_err(|_|Error::protocol("invalid owner UID"))?};
                if platform_linux::verify_peer(&stream,owner).is_err(){continue;}
                stream
            }
        };
        let permit = match limits.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => continue,
        };
        let engine = engine.clone();
        let status = status.clone();
        let logs = logs.clone();
        tasks.spawn(async move {
            let _permit = permit;
            let mut stream = incoming;
            let request = match tokio::time::timeout(
                Duration::from_secs(5),
                vpn_ipc::read_frame::<Request>(&mut stream),
            )
            .await
            {
                Ok(Ok(request)) => request,
                _ => return,
            };
            let reply = match request {
                Request::GetStatus => Reply::Status(Box::new(status.read().await.clone())),
                Request::GetRecentLogs => Reply::Logs(logs.lock().await.clone()),
                _ => match engine.try_lock() {
                    Ok(mut engine) => engine.handle(request).await,
                    Err(_) => Reply::Error(Error::new(
                        "busy",
                        "另一项操作正在进行，请等待完成后重试",
                        true,
                    )),
                },
            };
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                vpn_ipc::write_frame(&mut stream, &reply),
            )
            .await;
        });
        while tasks.try_join_next().is_some() {}
    }
    while tasks.join_next().await.is_some() {}
    ticker.abort();
    engine.lock().await.stop().await?;
    #[cfg(unix)]
    std::fs::remove_file(vpn_ipc::endpoint(dev)).map_err(Error::io)?;
    drop(lock);
    Ok(())
}

#[cfg(windows)]
fn make_pipe(
    sid: &str,
    dev: bool,
    first: bool,
) -> Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    let security = platform_windows::PipeSecurity::new(sid).map_err(Error::io)?;
    let mut attrs = security.attributes();
    unsafe {
        tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                vpn_ipc::endpoint(dev),
                std::ptr::from_mut(&mut attrs).cast(),
            )
    }
    .map_err(Error::io)
}

#[cfg(unix)]
async fn make_socket(settings: &Settings, dev: bool) -> Result<tokio::net::UnixListener> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let endpoint = vpn_ipc::endpoint(dev);
    let path = std::path::Path::new(&endpoint);
    let owner = if dev {
        platform_linux::uid()
    } else {
        settings
            .owner
            .parse()
            .map_err(|_| Error::protocol("invalid owner UID"))?
    };
    if path.exists() {
        let metadata = std::fs::symlink_metadata(path).map_err(Error::io)?;
        if !metadata.file_type().is_socket() || metadata.uid() != owner {
            return Err(Error::protocol("拒绝覆盖非本服务的 socket 路径"));
        }
        if tokio::net::UnixStream::connect(path).await.is_ok() {
            return Err(Error::protocol("IPC 地址已被占用"));
        }
        std::fs::remove_file(path).map_err(Error::io)?;
    }
    let listener = tokio::net::UnixListener::bind(path).map_err(Error::io)?;
    platform_linux::secure_socket(path, owner).map_err(Error::io)?;
    Ok(listener)
}
