use axum::{http::StatusCode, routing::post, Json, Router};
use serde_json::json;
use std::sync::{Arc, Mutex};
use vpn_core::api::Session;
const UPLOAD: &str = "Subnet = 10.42.0.2/32\n\n-----BEGIN RSA PUBLIC KEY-----\nfixture\n-----END RSA PUBLIC KEY-----\n";
async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    (
        format!("http://{address}"),
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }),
    )
}
#[tokio::test]
async fn distinguishes_rejection_and_malformed_success_without_leaking_server_body() {
    for (value, code) in [
        (
            json!({"status":0,"msg":"password=secret token=secret"}),
            "auth_failed",
        ),
        (json!({"code":200,"token":"secret"}), "protocol"),
        (
            json!({"status":1,"sid":"other","token":"secret","net_name":"net","node_ip":"10.0.0.2"}),
            "protocol",
        ),
    ] {
        let (url, task) = serve(Router::new().route(
            "/api/tinc/client/login",
            post(move || async move { Json(value) }),
        ))
        .await;
        let error = Session::login(&url, None, Some("client_a"), "password with spaces", true)
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, code);
        assert!(!error.message.contains("secret"));
        task.abort();
    }
}
#[tokio::test]
async fn follows_observed_request_fields_and_surfaces_expired_download() {
    let app = Router::new()
        .route(
            "/api/tinc/client/login",
            post(|Json(v): Json<serde_json::Value>| async move {
                assert_eq!(v, json!({"sid":"client_a","password":" with spaces "}));
                Json(json!({"status":1,"sid":"client_a","token":"opaque","net_name":"office","node_ip":"10.42.0.2","node_id":101,"network_id":20}))
            }),
        )
        .route(
            "/api/tinc/client/config/download",
            post(|Json(v): Json<serde_json::Value>| async move {
                assert_eq!(v, json!({"sid":"client_a","token":"opaque"}));
                axum::http::StatusCode::UNAUTHORIZED
            }),
        )
        .route(
            "/api/tinc/client/key/upload",
            post(|Json(v): Json<serde_json::Value>| async move {
                assert_eq!(
                    v,
                    json!({"sid":"client_a","token":"opaque","content":UPLOAD,"action":"exchangeFile"})
                );
                (
                    [
                        ("content-type", "text/plain; charset=UTF-8"),
                        ("cache-control", "no-store"),
                    ],
                    "host content",
                )
            }),
        );
    let (url, task) = serve(app).await;
    let session = Session::login(&url, None, Some("client_a"), " with spaces ", true)
        .await
        .unwrap();
    assert_eq!(session.identity.node_id, Some(101));
    assert_eq!(session.identity.network_id, Some(20));
    assert_eq!(session.download().await.unwrap_err().code, "auth_expired");
    assert_eq!(session.upload(UPLOAD).await.unwrap(), "host content");
    task.abort();
}
#[tokio::test]
async fn upload_marks_token_rejection_as_an_expired_session() {
    let app = Router::new()
        .route(
            "/api/tinc/client/login",
            post(|| async {
                Json(json!({"status":1,"sid":"client_a","token":"opaque","net_name":"office","node_ip":"10.42.0.2"}))
            }),
        )
        .route("/api/tinc/client/key/upload", post(|| async { StatusCode::UNAUTHORIZED }));
    let (url, task) = serve(app).await;
    let session = Session::login(&url, None, Some("client_a"), "password", true)
        .await
        .unwrap();
    assert_eq!(
        session.upload(UPLOAD).await.unwrap_err().code,
        "auth_expired"
    );
    task.abort();
}
#[tokio::test]
async fn upload_rejects_a_non_plain_text_success_response() {
    let app = Router::new()
        .route(
            "/api/tinc/client/login",
            post(|| async {
                Json(json!({"status":1,"sid":"client_a","token":"opaque","net_name":"office","node_ip":"10.42.0.2"}))
            }),
        )
        .route(
            "/api/tinc/client/key/upload",
            post(|| async { ([("content-type", "application/json")], "host content") }),
        );
    let (url, task) = serve(app).await;
    let session = Session::login(&url, None, Some("client_a"), "password", true)
        .await
        .unwrap();
    assert_eq!(session.upload(UPLOAD).await.unwrap_err().code, "protocol");
    task.abort();
}
#[tokio::test]
async fn download_rejects_zip_bytes_with_an_unexpected_media_type() {
    let app = Router::new()
        .route(
            "/api/tinc/client/login",
            post(|| async {
                Json(json!({"status":1,"sid":"client_a","token":"opaque","net_name":"office","node_ip":"10.42.0.2"}))
            }),
        )
        .route(
            "/api/tinc/client/config/download",
            post(|| async { ([("content-type", "application/octet-stream")], b"PK\x03\x04payload") }),
        );
    let (url, task) = serve(app).await;
    let session = Session::login(&url, None, Some("client_a"), "password", true)
        .await
        .unwrap();
    assert_eq!(session.download().await.unwrap_err().code, "protocol");
    task.abort();
}
#[tokio::test]
async fn upload_maps_only_known_backend_error_codes_and_never_exposes_msg() {
    for (status, backend_code, expected, retryable) in [
        (
            StatusCode::BAD_REQUEST,
            "CLIENT_PUBLIC_KEY_INVALID",
            "key_rejected",
            false,
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "GATEWAY_HOST_UNAVAILABLE",
            "gateway_unavailable",
            true,
        ),
        (
            StatusCode::BAD_GATEWAY,
            "GATEWAY_KEY_APPLY_FAILED",
            "gateway_apply",
            true,
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "CLIENT_SESSION_UNAVAILABLE",
            "session_unavailable",
            true,
        ),
        (
            StatusCode::GONE,
            "CLIENT_NODE_REVOKED",
            "node_revoked",
            false,
        ),
    ] {
        let response = format!(r#"{{"code":"{backend_code}","msg":"token=secret"}}"#);
        let app = Router::new()
            .route(
                "/api/tinc/client/login",
                post(|| async {
                    Json(json!({"status":1,"sid":"client_a","token":"opaque","net_name":"office","node_ip":"10.42.0.2"}))
                }),
            )
            .route(
                "/api/tinc/client/key/upload",
                post(move || {
                    let response = response.clone();
                    async move { (status, [("content-type", "application/json")], response) }
                }),
            );
        let (url, task) = serve(app).await;
        let session = Session::login(&url, None, Some("client_a"), "password", true)
            .await
            .unwrap();
        let error = session.upload(UPLOAD).await.unwrap_err();
        assert_eq!(error.code, expected);
        assert_eq!(error.retryable, retryable);
        assert!(!error.message.contains("secret"));
        task.abort();
    }
}

#[tokio::test]
async fn keepalive_maps_backend_failure_as_retryable() {
    let app = Router::new()
        .route(
            "/api/tinc/client/login",
            post(|| async {
                Json(json!({"status":1,"sid":"client_a","token":"opaque","net_name":"office","node_ip":"10.42.0.2"}))
            }),
        )
        .route(
            "/api/tinc/client/keepalive",
            post(|| async {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "code": "CLIENT_KEEPALIVE_FAILED",
                        "msg": "token=secret"
                    })),
                )
            }),
        );
    let (url, task) = serve(app).await;
    let session = Session::login(&url, None, Some("client_a"), "password", true)
        .await
        .unwrap();

    let error = session.keepalive().await.unwrap_err();

    assert_eq!(error.code, "keepalive_failed");
    assert!(error.retryable);
    assert!(!error.message.contains("secret"));
    task.abort();
}

#[tokio::test]
async fn keepalive_maps_route_snapshot_unavailable_as_retryable() {
    let app = Router::new()
        .route(
            "/api/tinc/client/login",
            post(|| async {
                Json(json!({"status":1,"sid":"client_a","token":"opaque","net_name":"office","node_ip":"10.42.0.2"}))
            }),
        )
        .route(
            "/api/tinc/client/keepalive",
            post(|| async {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "code": "CLIENT_ROUTE_SNAPSHOT_UNAVAILABLE",
                        "msg": "token=secret"
                    })),
                )
            }),
        );
    let (url, task) = serve(app).await;
    let session = Session::login(&url, None, Some("client_a"), "password", true)
        .await
        .unwrap();

    let error = session.keepalive().await.unwrap_err();

    assert_eq!(error.code, "route_snapshot_unavailable");
    assert!(error.retryable);
    assert!(!error.message.contains("secret"));
    task.abort();
}

#[tokio::test]
async fn parses_login_and_keepalive_route_snapshots_without_guessing_subnets() {
    let app = Router::new()
        .route(
            "/api/tinc/client/login",
            post(|| async {
                Json(json!({
                    "status": 1,
                    "sid": "client_a",
                    "token": "opaque",
                    "net_name": "office",
                    "node_ip": "10.42.0.2",
                    "allowed_routes": ["10.42.0.4/32", "10.42.0.3/32", "10.42.0.3/32"],
                    "route_revision": "network-20-login",
                    "route_lease_seconds": 120
                }))
            }),
        )
        .route(
            "/api/tinc/client/keepalive",
            post(|| async {
                Json(json!({
                    "status": "success",
                    "allowed_routes": ["10.42.0.5/32"],
                    "route_revision": "network-20-keepalive",
                    "route_lease_seconds": 120
                }))
            }),
        );
    let (url, task) = serve(app).await;
    let session = Session::login(&url, None, Some("client_a"), "password", true)
        .await
        .unwrap();

    assert_eq!(session.allowed_routes().len(), 2);
    assert_eq!(
        session.allowed_routes()[0].destination.to_string(),
        "10.42.0.3"
    );
    assert_eq!(session.allowed_routes()[0].prefix, 32);
    let initial = session.route_snapshot().unwrap();
    assert_eq!(initial.revision(), Some("network-20-login"));
    assert!(initial.lease_deadline().unwrap() > std::time::Instant::now());
    let refreshed = session.keepalive().await.unwrap().unwrap();
    assert_eq!(refreshed.revision(), Some("network-20-keepalive"));
    assert!(refreshed.lease_deadline().unwrap() > std::time::Instant::now());
    assert_eq!(refreshed.routes().len(), 1);
    assert_eq!(refreshed.routes()[0].destination.to_string(), "10.42.0.5");
    task.abort();
}

#[tokio::test]
async fn two_nodes_receive_only_the_other_nodes_exact_route() {
    let app = Router::new().route(
        "/api/tinc/client/login",
        post(|Json(value): Json<serde_json::Value>| async move {
            let sid = value["sid"].as_str().unwrap();
            let (node_ip, peer_ip, node_id) = match sid {
                "client_a" => ("10.0.11.211", "10.0.11.212/32", 211),
                "client_b" => ("10.0.11.212", "10.0.11.211/32", 212),
                _ => panic!("unexpected fixture node"),
            };
            Json(json!({
                "status": 1,
                "sid": sid,
                "token": format!("token-{sid}"),
                "net_name": "office",
                "node_ip": node_ip,
                "node_id": node_id,
                "network_id": 11,
                "allowed_routes": [peer_ip],
                "route_revision": "network-11-fixture",
                "route_lease_seconds": 120
            }))
        }),
    );
    let (url, task) = serve(app).await;

    let a = Session::login(&url, None, Some("client_a"), "password", true)
        .await
        .unwrap();
    let b = Session::login(&url, None, Some("client_b"), "password", true)
        .await
        .unwrap();

    assert_eq!(a.identity.node_ip, "10.0.11.211");
    assert_eq!(a.identity.network_id, Some(11));
    assert_eq!(a.allowed_routes()[0].destination.to_string(), "10.0.11.212");
    assert_eq!(b.identity.node_ip, "10.0.11.212");
    assert_eq!(b.identity.network_id, Some(11));
    assert_eq!(b.allowed_routes()[0].destination.to_string(), "10.0.11.211");
    assert_ne!(
        a.allowed_routes()[0].destination.to_string(),
        a.identity.node_ip
    );
    assert_ne!(
        b.allowed_routes()[0].destination.to_string(),
        b.identity.node_ip
    );
    task.abort();
}

#[tokio::test]
async fn rejects_unsafe_or_broad_routes_from_the_backend() {
    for route in [
        "10.42.0.0/24",
        "10.42.0.2/32",
        "127.0.0.1/32",
        "169.254.1.2/32",
    ] {
        let app = Router::new().route(
            "/api/tinc/client/login",
            post(move || async move {
                Json(json!({
                    "status": 1,
                    "sid": "client_a",
                    "token": "opaque",
                    "net_name": "office",
                    "node_ip": "10.42.0.2",
                    "allowed_routes": [route]
                }))
            }),
        );
        let (url, task) = serve(app).await;

        let error = Session::login(&url, None, Some("client_a"), "password", true)
            .await
            .err()
            .unwrap();

        assert_eq!(error.code, "protocol");
        task.abort();
    }
}

#[tokio::test]
async fn rejects_incomplete_or_unsafe_route_snapshot_metadata() {
    for snapshot in [
        json!({
            "allowed_routes": ["10.42.0.3/32"],
            "route_revision": "network-20-only-revision"
        }),
        json!({
            "allowed_routes": ["10.42.0.3/32"],
            "route_lease_seconds": 120
        }),
        json!({
            "allowed_routes": ["10.42.0.3/32"],
            "route_revision": "network-20-too-short",
            "route_lease_seconds": 29
        }),
        json!({
            "allowed_routes": ["10.42.0.3/32"],
            "route_revision": " network-20-spaced ",
            "route_lease_seconds": 120
        }),
        json!({
            "route_revision": "network-20-no-routes",
            "route_lease_seconds": 120
        }),
    ] {
        let app = Router::new().route(
            "/api/tinc/client/login",
            post(move || async move {
                let mut response = json!({
                    "status": 1,
                    "sid": "client_a",
                    "token": "opaque",
                    "net_name": "office",
                    "node_ip": "10.42.0.2"
                });
                response
                    .as_object_mut()
                    .unwrap()
                    .extend(snapshot.as_object().unwrap().clone());
                Json(response)
            }),
        );
        let (url, task) = serve(app).await;

        let error = Session::login(&url, None, Some("client_a"), "password", true)
            .await
            .err()
            .unwrap();

        assert_eq!(error.code, "protocol");
        task.abort();
    }
}
#[tokio::test]
async fn repeated_uploads_keep_the_same_authenticated_public_content() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let app = Router::new()
        .route(
            "/api/tinc/client/login",
            post(|| async { Json(json!({"status":1,"sid":"client_a","token":"opaque","net_name":"office","node_ip":"10.42.0.2"})) }),
        )
        .route("/api/tinc/client/key/upload", post(move |Json(value): Json<serde_json::Value>| {
            let captured = captured.clone();
            async move {
                captured.lock().unwrap().push(value);
                ([("content-type", "text/plain; charset=UTF-8"), ("cache-control", "no-store")], "host content")
            }
        }));
    let (url, task) = serve(app).await;
    let session = Session::login(&url, None, Some("client_a"), "password", true)
        .await
        .unwrap();
    assert_eq!(session.upload(UPLOAD).await.unwrap(), "host content");
    assert_eq!(session.upload(UPLOAD).await.unwrap(), "host content");
    let values = seen.lock().unwrap();
    assert_eq!(values.len(), 2);
    assert_eq!(
        values[0],
        json!({"sid":"client_a","token":"opaque","content":UPLOAD,"action":"exchangeFile"})
    );
    assert_eq!(values[0], values[1]);
    task.abort();
}
#[tokio::test]
async fn never_follows_redirect_with_credentials() {
    let (url, task) = serve(Router::new().route(
        "/api/tinc/client/login",
        post(|| async {
            (
                axum::http::StatusCode::TEMPORARY_REDIRECT,
                [("location", "http://127.0.0.1:1/capture")],
            )
        }),
    ))
    .await;
    let err = Session::login(&url, None, Some("client_a"), "secret", true)
        .await
        .err()
        .unwrap();
    assert_eq!(err.code, "http");
    assert!(err.message.contains("307"));
    task.abort();
}

#[test]
fn validates_access_code_format_and_normalization() {
    use vpn_core::api::validate_access_code;

    // Valid formats normalize to uppercase
    assert_eq!(
        validate_access_code("TL-7K9M-2Q4F").unwrap(),
        "TL-7K9M-2Q4F"
    );
    assert_eq!(
        validate_access_code("tl-7k9m-2q4f").unwrap(),
        "TL-7K9M-2Q4F"
    );
    assert_eq!(
        validate_access_code("  tl-7k9m-2q4f  ").unwrap(),
        "TL-7K9M-2Q4F"
    );

    // Invalid length
    assert!(validate_access_code("TL-7K9M").is_err());
    assert!(validate_access_code("TL-7K9M-2Q4F1").is_err());

    // Invalid prefix or separator
    assert!(validate_access_code("XX-7K9M-2Q4F").is_err());
    assert!(validate_access_code("TL7K9M-2Q4F_").is_err());
    assert!(validate_access_code("TL-7K9M_2Q4F").is_err());

    // Invalid characters outside Crockford Base32 (no 0, 1, I, L, O)
    assert!(validate_access_code("TL-7K90-2Q4F").is_err());
    assert!(validate_access_code("TL-7K91-2Q4F").is_err());
    assert!(validate_access_code("TL-7K9I-2Q4F").is_err());
    assert!(validate_access_code("TL-7K9L-2Q4F").is_err());
    assert!(validate_access_code("TL-7K9O-2Q4F").is_err());
}

#[tokio::test]
async fn access_code_login_sends_contract_json_without_sid_and_parses_response() {
    let captured_request = Arc::new(Mutex::new(None));
    let captured_clone = captured_request.clone();

    let app = Router::new().route(
        "/api/tinc/client/login",
        post(move |Json(v): Json<serde_json::Value>| {
            let captured = captured_clone.clone();
            async move {
                *captured.lock().unwrap() = Some(v);
                Json(json!({
                    "status": 1,
                    "msg": "登录成功",
                    "access_code": "TL-7K9M-2Q4F",
                    "sid": "client_a",
                    "token": "tok_xyz_123",
                    "node_id": 101,
                    "network_id": 20,
                    "net_name": "net_office",
                    "node_ip": "10.42.0.2"
                }))
            }
        }),
    );
    let (url, task) = serve(app).await;

    // Login with lowercase, untrimmed access code; verify it normalizes and sends no sid
    let session = Session::login(&url, Some("  tl-7k9m-2q4f  "), None, "node-secret", true)
        .await
        .unwrap();

    let sent = captured_request.lock().unwrap().take().unwrap();
    assert_eq!(
        sent,
        json!({
            "accessCode": "TL-7K9M-2Q4F",
            "password": "node-secret"
        })
    );
    // Crucial: verify "sid" is NOT present in request payload
    assert!(sent.get("sid").is_none());

    // Verify session identity fields
    assert_eq!(session.identity.sid, "client_a");
    assert_eq!(session.identity.net_name, "net_office");
    assert_eq!(session.identity.node_ip, "10.42.0.2");
    assert_eq!(session.identity.node_id, Some(101));
    assert_eq!(session.identity.network_id, Some(20));
    assert_eq!(
        session.identity.access_code.as_deref(),
        Some("TL-7K9M-2Q4F")
    );

    task.abort();
}

#[tokio::test]
async fn access_code_login_maps_backend_rejections() {
    for (backend_code, expected_client_code, retryable) in [
        ("CLIENT_CREDENTIAL_INVALID", "credential_invalid", false),
        ("CLIENT_LOGIN_RATE_LIMITED", "login_rate_limited", true),
        (
            "CLIENT_LOGIN_LIMITER_UNAVAILABLE",
            "login_unavailable",
            true,
        ),
        ("CLIENT_LEGACY_LOGIN_AMBIGUOUS", "login_ambiguous", false),
        ("CLIENT_NETWORK_IP_INVALID", "assignment_missing", false),
        ("CLIENT_NETWORK_NOT_FOUND", "assignment_missing", false),
        ("SYSTEM_ERROR", "server_error", true),
    ] {
        let app = Router::new().route(
            "/api/tinc/client/login",
            post(move || async move {
                Json(json!({
                    "status": 0,
                    "code": backend_code,
                    "msg": format!("backend rejection for {backend_code}")
                }))
            }),
        );
        let (url, task) = serve(app).await;
        let err = Session::login(&url, Some("TL-7K9M-2Q4F"), None, "wrong-pwd", true)
            .await
            .err()
            .unwrap();

        assert_eq!(err.code, expected_client_code);
        assert_eq!(err.retryable, retryable);
        task.abort();
    }
}

#[test]
fn access_code_rotation_preserves_profile_id_and_identity_equality() {
    use vpn_core::{keys, Identity};

    let origin = "http://127.0.0.1:8080";
    let identity_v1 = Identity {
        sid: "client_a".into(),
        net_name: "office_vpn".into(),
        node_ip: "10.42.0.2".into(),
        node_id: Some(101),
        network_id: Some(20),
        access_code: Some("TL-7K9M-2Q4F".into()),
    };

    let identity_v2 = Identity {
        sid: "client_a".into(),
        net_name: "office_vpn".into(),
        node_ip: "10.42.0.2".into(),
        node_id: Some(101),
        network_id: Some(20),
        access_code: Some("TL-9A8B-7C6D".into()), // Rotated access code
    };

    // Equality: identity equality ignores access_code changes
    assert_eq!(identity_v1, identity_v2);

    // Profile ID: profile_id remains completely invariant across access code rotation
    let profile_1 = keys::profile_id(origin, &identity_v1);
    let profile_2 = keys::profile_id(origin, &identity_v2);
    assert_eq!(profile_1, profile_2);
}

#[test]
fn same_node_name_across_different_networks_isolate_profiles() {
    use vpn_core::{keys, Identity};

    let origin = "http://127.0.0.1:8080";

    // Two nodes with the same display name ("a") in different networks
    let node_net_a = Identity {
        sid: "a".into(),
        net_name: "network_a".into(),
        node_ip: "10.42.0.2".into(),
        node_id: Some(10),
        network_id: Some(1),
        access_code: Some("TL-AAAA-1111".into()),
    };

    let node_net_b = Identity {
        sid: "a".into(),
        net_name: "network_b".into(),
        node_ip: "10.42.0.2".into(),
        node_id: Some(20),
        network_id: Some(2),
        access_code: Some("TL-BBBB-2222".into()),
    };

    assert_ne!(node_net_a, node_net_b);

    let profile_a = keys::profile_id(origin, &node_net_a);
    let profile_b = keys::profile_id(origin, &node_net_b);

    // Profiles are distinct; different networks/nodes never overwrite keys or configurations
    assert_ne!(profile_a, profile_b);
}
