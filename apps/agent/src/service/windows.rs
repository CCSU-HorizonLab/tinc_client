use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Notify;
use vpn_core::Error;
use windows_service::{
    service::*,
    service_control_handler::{self, ServiceControlHandlerResult},
};

pub fn settings_path() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into()))
        .join("TincClient/settings.json")
}

pub async fn wait_for_shutdown(signal: Arc<Notify>) {
    let _ = tokio::signal::ctrl_c().await;
    signal.notify_one();
}

pub fn run() {
    if windows_service::service_dispatcher::start("TincClientAgent", ffi_service_main).is_err() {
        eprintln!("Windows Service dispatcher failed");
        std::process::exit(1);
    }
}

windows_service::define_windows_service!(ffi_service_main, service_main);

fn service_main(_: Vec<std::ffi::OsString>) {
    let shutdown = Arc::new(Notify::new());
    let signal = shutdown.clone();
    let Ok(handle) =
        service_control_handler::register("TincClientAgent", move |control| match control {
            ServiceControl::Stop => {
                signal.notify_one();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })
    else {
        return;
    };
    let state = |current_state, code| ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state,
        controls_accepted: if current_state == ServiceState::Running {
            ServiceControlAccept::STOP
        } else {
            ServiceControlAccept::empty()
        },
        exit_code: ServiceExitCode::Win32(code),
        checkpoint: 0,
        wait_hint: Duration::from_secs(120),
        process_id: None,
    };
    let _ = handle.set_service_status(state(ServiceState::StartPending, 0));
    let result = tokio::runtime::Runtime::new()
        .map_err(Error::io)
        .and_then(|runtime| {
            runtime.block_on(crate::server::serve(
                settings_path(),
                false,
                shutdown,
                || {
                    handle
                        .set_service_status(state(ServiceState::Running, 0))
                        .map_err(|_| Error::protocol("服务状态上报失败"))
                },
            ))
        });
    let _ = handle.set_service_status(state(
        ServiceState::Stopped,
        if result.is_ok() { 0 } else { 1 },
    ));
}
