use axum::body::Body;
use axum::http::{Request, StatusCode};
use iris::config::AppConfig;
use iris::profiles::Profile;
use iris::server::build_router_for_tests;
use tower::ServiceExt;

const TELSTAR_PROFILE: &str = r#"
brand = "telstar"
model = "xxx"
device_type = "tv"
carrier_frequency = 38000
protocol = "nec"

[commands]
power = { type = "nec", address = "0x00FF", command = "0xA25D" }
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
