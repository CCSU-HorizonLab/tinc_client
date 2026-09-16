use std::{path::PathBuf, sync::Arc};
use tokio::sync::Notify;

#[cfg(unix)]
pub mod linux;
#[cfg(windows)]
pub mod windows;

pub fn settings_path() -> PathBuf {
    #[cfg(windows)]
    return windows::settings_path();
    #[cfg(unix)]
    return linux::settings_path();
}

pub async fn wait_for_shutdown(signal: Arc<Notify>) {
    #[cfg(windows)]
    windows::wait_for_shutdown(signal).await;
    #[cfg(unix)]
    linux::wait_for_shutdown(signal).await;
}
