use crate::config::{AppConfig, ConfiguredDevice};
use crate::discovery;
use crate::errors::IrisError;
use crate::ir::{IrSignal, IrTransmitter, MockTransmitter};
use crate::profiles::Profile;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, mpsc};
use tokio::sync::oneshot;

const TRANSMIT_QUEUE_CAPACITY: usize = 32;

#[derive(Debug, Clone)]
pub struct RegisteredDevice {
    pub config: ConfiguredDevice,
    pub profile: Profile,
}

#[derive(Clone)]
struct ServerState {
    devices: Arc<BTreeMap<String, RegisteredDevice>>,
    config: AppConfig,
    dispatcher: TransmitDispatcher,
}

struct TransmitJob {
    signal: IrSignal,
    repeat: u32,
    carrier_frequency: u32,
    response: oneshot::Sender<Result<(), IrisError>>,
}

#[derive(Clone)]
struct TransmitDispatcher {
    sender: mpsc::SyncSender<TransmitJob>,
}

impl TransmitDispatcher {
    fn new(transmitter: Arc<dyn IrTransmitter>) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<TransmitJob>(TRANSMIT_QUEUE_CAPACITY);
        std::thread::spawn(move || {
            while let Ok(job) = receiver.recv() {
                let result =
                    transmitter.send_with_frequency(job.signal, job.repeat, job.carrier_frequency);
                let _ = job.response.send(result);
            }
        });
        Self { sender }
    }

    async fn send(
        &self,
        signal: IrSignal,
        repeat: u32,
        carrier_frequency: u32,
    ) -> Result<(), IrisError> {
        let (response, receiver) = oneshot::channel();
        self.sender
            .try_send(TransmitJob {
                signal,
                repeat,
                carrier_frequency,
                response,
            })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => IrisError::TransmitQueueFull,
                mpsc::TrySendError::Disconnected(_) => {
                    IrisError::IoPlain(std::io::Error::other("IR transmit worker stopped"))
                }
            })?;
        receiver
            .await
            .map_err(|_| IrisError::IoPlain(std::io::Error::other("IR transmit worker stopped")))?
    }
}

pub fn build_router(
    devices: Vec<RegisteredDevice>,
    config: AppConfig,
    transmitter: Arc<dyn IrTransmitter>,
) -> Router {
    let devices = devices
        .into_iter()
        .map(|device| (device.config.id.clone(), device))
        .collect();
    let state = ServerState {
        devices: Arc::new(devices),
        config,
        dispatcher: TransmitDispatcher::new(transmitter),
    };

    Router::new()
        .route("/health", get(health))
        .route("/devices", get(devices_handler))
        .route("/devices/{device}/send/{command}", post(send_device))
        .route("/devices/{device}", get(device_handler))
        .route("/profile", get(default_profile_handler))
        .route("/send/{command}", post(send_default))
        .with_state(state)
}

/// Compatibility helper retained for existing server tests and embedders.
pub fn build_router_for_tests(profile: Profile, mut config: AppConfig) -> Router {
    config.upsert_legacy_default(profile.id());
    let device = RegisteredDevice {
        config: config
            .device(None)
            .expect("legacy default device exists")
            .clone(),
        profile,
    };
    build_router(vec![device], config, Arc::new(MockTransmitter::new()))
}

pub async fn serve(
    devices: Vec<RegisteredDevice>,
    config: AppConfig,
    transmitter: Arc<dyn IrTransmitter>,
) -> Result<(), IrisError> {
    ensure_bind_is_safe(&config)?;
    let addr = format!("{}:{}", config.server_host, config.server_port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|source| IrisError::ServerBindFailure {
            addr: addr.clone(),
            source,
        })?;
    tracing::info!("IRIS server listening on {addr}");
    let _discovery = discovery::register(&config)?;
    axum::serve(listener, build_router(devices, config, transmitter)).await?;
    Ok(())
}

fn ensure_bind_is_safe(config: &AppConfig) -> Result<(), IrisError> {
    let parsed = config.server_host.parse::<IpAddr>();
    let is_loopback = parsed.as_ref().map(|ip| ip.is_loopback()).unwrap_or(false);
    if !is_loopback && config.api_token.as_deref().unwrap_or("").is_empty() {
        return Err(IrisError::MissingApiTokenForNetworkBind);
    }
    let _socket = SocketAddr::new(
        parsed.unwrap_or(IpAddr::from([127, 0, 0, 1])),
        config.server_port,
    );
    Ok(())
}

async fn health(State(state): State<ServerState>) -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "api_version": 2,
        "bridge_id": state.config.device_id.as_deref().unwrap_or(""),
        "device_id": state.config.device_id.as_deref().unwrap_or(""),
        "device_name": state.config.device_name,
        "auth_required": state.config.api_token.as_deref().is_some_and(|token| !token.is_empty())
    }))
}

async fn devices_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(err) = authorize(&state.config, &headers) {
        return error_response(err);
    }
    let devices = state.devices.values().map(device_json).collect::<Vec<_>>();
    (StatusCode::OK, Json(json!({ "devices": devices }))).into_response()
}

async fn device_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(device): Path<String>,
) -> impl IntoResponse {
    if let Err(err) = authorize(&state.config, &headers) {
        return error_response(err);
    }
    match state.devices.get(&device) {
        Some(device) => (StatusCode::OK, Json(device_json(device))).into_response(),
        None => error_response(IrisError::DeviceNotFound { device }),
    }
}

async fn default_profile_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(err) = authorize(&state.config, &headers) {
        return error_response(err);
    }
    match default_device(&state) {
        Ok(device) => (StatusCode::OK, Json(legacy_profile_json(device))).into_response(),
        Err(err) => error_response(err),
    }
}

fn legacy_profile_json(device: &RegisteredDevice) -> serde_json::Value {
    let mut payload = device_json(device);
    payload["id"] = json!(device.profile.id());
    payload["device_id"] = json!(device.config.id);
    payload
}

async fn send_default(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(command): Path<String>,
) -> impl IntoResponse {
    send_for_device(state, headers, None, command).await
}

async fn send_device(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path((device, command)): Path<(String, String)>,
) -> impl IntoResponse {
    send_for_device(state, headers, Some(device), command).await
}

async fn send_for_device(
    state: ServerState,
    headers: HeaderMap,
    device_id: Option<String>,
    command: String,
) -> axum::response::Response {
    if let Err(err) = authorize(&state.config, &headers) {
        return error_response(err);
    }
    let device = match device_id.as_deref().map_or_else(
        || default_device(&state),
        |id| {
            state
                .devices
                .get(id)
                .ok_or_else(|| IrisError::DeviceNotFound {
                    device: id.to_string(),
                })
        },
    ) {
        Ok(device) => device,
        Err(err) => return error_response(err),
    };
    let signal = match device.profile.signal_for(&command) {
        Ok(signal) => signal,
        Err(err) => return error_response(err),
    };
    match state
        .dispatcher
        .send(
            signal,
            state.config.default_repeat.max(1),
            device
                .profile
                .carrier_frequency
                .unwrap_or(state.config.carrier_frequency),
        )
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "sent": command, "device": device.config.id, "profile": device.profile.id() })),
        )
            .into_response(),
        Err(err) => error_response(err),
    }
}

fn default_device(state: &ServerState) -> Result<&RegisteredDevice, IrisError> {
    let id = state
        .config
        .default_device
        .as_deref()
        .ok_or(IrisError::DefaultDeviceMissing)?;
    state
        .devices
        .get(id)
        .ok_or_else(|| IrisError::DeviceNotFound {
            device: id.to_string(),
        })
}

fn device_json(device: &RegisteredDevice) -> serde_json::Value {
    let fan = device.profile.home_assistant.fan.as_ref().map(|fan| {
        json!({
            "power_on": fan.power_on,
            "power_off": fan.power_off,
            "oscillate": fan.oscillate,
            "presets": fan.presets,
        })
    });
    json!({
        "id": device.config.id,
        "name": device.config.name,
        "profile": device.profile.id(),
        "brand": device.profile.brand,
        "model": device.profile.model,
        "device_type": device.profile.device_type,
        "commands": device.profile.commands.keys().collect::<Vec<_>>(),
        "home_assistant": { "fan": fan },
    })
}

fn authorize(config: &AppConfig, headers: &HeaderMap) -> Result<(), IrisError> {
    let Some(token) = config
        .api_token
        .as_deref()
        .filter(|token| !token.is_empty())
    else {
        return Ok(());
    };
    let expected = format!("Bearer {token}");
    let actual = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    if actual == Some(expected.as_str()) {
        Ok(())
    } else {
        Err(IrisError::Unauthorized)
    }
}

fn error_response(err: IrisError) -> axum::response::Response {
    let status = match err {
        IrisError::Unauthorized => StatusCode::UNAUTHORIZED,
        IrisError::CommandNotFound { .. } | IrisError::DeviceNotFound { .. } => {
            StatusCode::NOT_FOUND
        }
        IrisError::TransmitQueueFull => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::BAD_REQUEST,
    };
    (status, Json(json!({ "error": err.to_string() }))).into_response()
}
