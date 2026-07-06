use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use iris::config::AppConfig;
use iris::profiles::Profile;
use iris::server::build_router_for_tests;
use tower::ServiceExt;

const TELSTAR_PROFILE: &str = r#"
brand = "telstar"
model = "generic"
device_type = "tv"
carrier_frequency = 38000
protocol = "nec"

[commands]
power = { type = "nec", address = "0x00FF", command = "0xA25D" }
volume_up = { type = "nec", address = "0x00FF", command = "0x629D" }
"#;

#[tokio::test]
async fn server_health_profile_and_send_work_with_token() {
    let profile = Profile::from_toml_str(TELSTAR_PROFILE).expect("profile");
    let config = AppConfig {
        api_token: Some("secret".to_string()),
        ..AppConfig::default()
    };
    let app = build_router_for_tests(profile, config);

    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(health.status(), StatusCode::OK);
    let health_body = to_bytes(health.into_body(), 1024).await.expect("body");
    let health_json: serde_json::Value = serde_json::from_slice(&health_body).expect("health json");
    assert_eq!(health_json["status"], "ok");
    assert_eq!(health_json["api_version"], 1);
    assert_eq!(health_json["auth_required"], true);

    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/profile")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let sent = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/send/power")
                .header("authorization", "Bearer secret")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(sent.status(), StatusCode::OK);
}

#[tokio::test]
async fn health_is_public_and_profile_includes_id_with_token() {
    let profile = Profile::from_toml_str(TELSTAR_PROFILE).expect("profile");
    let config = AppConfig {
        api_token: Some("secret".to_string()),
        device_id: Some("iris-test-device".to_string()),
        device_name: "Living Room IRIS".to_string(),
        ..AppConfig::default()
    };
    let app = build_router_for_tests(profile, config);

    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(health.status(), StatusCode::OK);
    let health_body = to_bytes(health.into_body(), 1024).await.expect("body");
    let health_json: serde_json::Value = serde_json::from_slice(&health_body).expect("health json");
    assert_eq!(health_json["device_id"], "iris-test-device");
    assert_eq!(health_json["device_name"], "Living Room IRIS");

    let profile = app
        .oneshot(
            Request::builder()
                .uri("/profile")
                .header("authorization", "Bearer secret")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(profile.status(), StatusCode::OK);
    let profile_body = to_bytes(profile.into_body(), 4096).await.expect("body");
    let profile_json: serde_json::Value =
        serde_json::from_slice(&profile_body).expect("profile json");
    assert_eq!(profile_json["id"], "telstar/generic");
    assert_eq!(profile_json["brand"], "telstar");
    assert_eq!(profile_json["model"], "generic");
    assert_eq!(profile_json["device_type"], "tv");
    assert!(
        profile_json["commands"]
            .as_array()
            .expect("commands")
            .contains(&serde_json::json!("power"))
    );
}
