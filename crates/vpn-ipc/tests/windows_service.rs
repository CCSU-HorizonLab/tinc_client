#![cfg(windows)]

use vpn_core::{Reply, Request};

// Run explicitly against an installed production service:
// cargo test -p vpn-ipc --test windows_service -- --ignored
#[tokio::test]
#[ignore = "requires a running local TincClientAgent Windows service"]
async fn authenticates_running_production_service() {
    let reply = vpn_ipc::call(Request::GetStatus)
        .await
        .expect("the production service identity and IPC request must be accepted");
    assert!(matches!(reply, Reply::Status(_)));
}
