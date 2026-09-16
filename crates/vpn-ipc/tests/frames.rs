use tokio::io::AsyncWriteExt;
use vpn_core::Request;
#[test]
fn debug_instance_isolated_without_changing_production_endpoint() {
    let production = vpn_ipc::endpoint(false);
    std::env::set_var("TINC_CLIENT_DEV_INSTANCE", "test-123");
    let development = vpn_ipc::endpoint(true);
    std::env::remove_var("TINC_CLIENT_DEV_INSTANCE");
    assert!(development.contains("test-123"));
    assert_eq!(production, vpn_ipc::endpoint(false));
    assert!(!production.contains("test-123"));
}
#[tokio::test]
async fn rejects_oversized_frame_before_allocating_payload() {
    let (mut a, mut b) = tokio::io::duplex(32);
    a.write_u32(1_000_000).await.unwrap();
    assert!(vpn_ipc::read_frame::<Request>(&mut b).await.is_err());
}
#[tokio::test]
async fn rejects_arbitrary_command_and_extra_fields() {
    for payload in [
        r#"{"command":"shell","args":"whoami"}"#,
        r#"{"command":"login","server":"https://a","sid":"n","password":"p","allow_http":false,"path":"C:/secret"}"#,
    ] {
        let (mut a, mut b) = tokio::io::duplex(2048);
        a.write_u32(payload.len() as u32).await.unwrap();
        a.write_all(payload.as_bytes()).await.unwrap();
        assert!(vpn_ipc::read_frame::<Request>(&mut b).await.is_err());
    }
}
