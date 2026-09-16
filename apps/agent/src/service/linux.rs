use std::{path::PathBuf, sync::Arc};
use tokio::sync::Notify;

pub fn settings_path() -> PathBuf {
    PathBuf::from("/etc/tinc-client/settings.json")
}

pub async fn wait_for_shutdown(signal: Arc<Notify>) {
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! {
            _ = terminate.recv() => (),
            _ = tokio::signal::ctrl_c() => (),
        }
    }
    signal.notify_one();
}
