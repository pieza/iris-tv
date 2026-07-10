use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use iris::config::{AppConfig, ConfiguredDevice};
use iris::ir::MockTransmitter;
use iris::profiles::Profile;
use iris::server::{RegisteredDevice, build_router, build_router_for_tests};
use std::sync::Arc;
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
    assert_eq!(health_json["api_version"], 2);
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

#[tokio::test]
async fn device_inventory_and_routing_keep_tv_and_fan_separate() {
    let tv = Profile::from_toml_str(TELSTAR_PROFILE).expect("tv profile");
    let fan = Profile::from_toml_str(
        r#"
brand = "generic"
model = "fan"
device_type = "fan"

[home_assistant.fan]
power_on = "power_on"

[home_assistant.fan.presets]
low = "speed_low"

[commands]
power_on = { type = "raw", frequency = 38000, pulses = [500, 500] }
speed_low = { type = "raw", frequency = 38000, pulses = [600, 600] }
"#,
    )
    .expect("fan profile");
    let config = AppConfig {
        api_token: Some("secret".to_string()),
        default_device: Some("tv".to_string()),
        devices: vec![
            ConfiguredDevice {
                id: "tv".to_string(),
                name: "Living Room TV".to_string(),
                profile: tv.id(),
            },
            ConfiguredDevice {
                id: "fan".to_string(),
                name: "Bedroom Fan".to_string(),
                profile: fan.id(),
            },
        ],
        ..AppConfig::default()
    };
    let app = build_router(
        vec![
            RegisteredDevice {
                config: config.devices[0].clone(),
                profile: tv,
            },
            RegisteredDevice {
                config: config.devices[1].clone(),
                profile: fan,
            },
        ],
        config,
        Arc::new(MockTransmitter::new()),
    );

    let inventory = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/devices")
                .header("authorization", "Bearer secret")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(inventory.status(), StatusCode::OK);
    let body = to_bytes(inventory.into_body(), 4096).await.expect("body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let devices = body["devices"].as_array().expect("devices");
    assert_eq!(devices.len(), 2);
    let fan = devices
        .iter()
        .find(|device| device["id"] == "fan")
        .expect("fan device");
    assert_eq!(fan["device_type"], "fan");
    assert_eq!(fan["home_assistant"]["fan"]["presets"]["low"], "speed_low");

    let sent = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/devices/fan/send/speed_low")
                .header("authorization", "Bearer secret")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(sent.status(), StatusCode::OK);
}
