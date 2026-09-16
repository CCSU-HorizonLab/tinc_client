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
        let error = Session::login(&url, "client_a", "password with spaces", true)
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
    let app=Router::new().route("/api/tinc/client/login",post(|Json(v):Json<serde_json::Value>|async move{
        assert_eq!(v,json!({"sid":"client_a","password":" with spaces "}));Json(json!({"status":1,"sid":"client_a","token":"opaque","net_name":"office","node_ip":"10.42.0.2"}))
    })).route("/api/tinc/client/config/download",post(|Json(v):Json<serde_json::Value>|async move{
        assert_eq!(v,json!({"sid":"client_a","token":"opaque"}));axum::http::StatusCode::UNAUTHORIZED
    })).route("/api/tinc/client/key/upload",post(|Json(v):Json<serde_json::Value>|async move{
        assert_eq!(v,json!({"sid":"client_a","token":"opaque","content":UPLOAD,"action":"exchangeFile"}));
        ([("content-type", "text/plain; charset=UTF-8"), ("cache-control", "no-store")], "host content")
    }));
    let (url, task) = serve(app).await;
    let session = Session::login(&url, "client_a", " with spaces ", true)
        .await
        .unwrap();
    assert_eq!(session.download().await.unwrap_err().code, "auth_expired");
    assert_eq!(session.upload(UPLOAD).await.unwrap(), "host content");
    task.abort();
}
#[tokio::test]
async fn upload_marks_token_rejection_as_an_expired_session() {
    let app = Router::new()
        .route(
            "/api/tinc/client/login",
            post(|| async { Json(json!({"status":1,"sid":"client_a","token":"opaque","net_name":"office","node_ip":"10.42.0.2"})) }),
        )
        .route("/api/tinc/client/key/upload", post(|| async { StatusCode::UNAUTHORIZED }));
    let (url, task) = serve(app).await;
    let session = Session::login(&url, "client_a", "password", true)
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
    let session = Session::login(&url, "client_a", "password", true)
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
    let session = Session::login(&url, "client_a", "password", true)
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
        let session = Session::login(&url, "client_a", "password", true)
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
    let session = Session::login(&url, "client_a", "password", true)
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
    let err = Session::login(&url, "client_a", "secret", true)
        .await
        .err()
        .unwrap();
    assert_eq!(err.code, "http");
    assert!(err.message.contains("307"));
    task.abort();
}
