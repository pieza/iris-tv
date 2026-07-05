use crate::config::AppConfig;
use crate::errors::IrisError;
use crate::ir::{IrTransmitter, MockTransmitter};
use crate::profiles::Profile;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

#[derive(Clone)]
pub struct ServerState {
    profile: Profile,
    config: AppConfig,
    transmitter: Arc<dyn IrTransmitter>,
}

pub fn build_router(
    profile: Profile,
    config: AppConfig,
    transmitter: Arc<dyn IrTransmitter>,
) -> Router {
    let state = ServerState {
        profile,
        config,
        transmitter,
    };

    Router::new()
        .route("/health", get(health))
        .route("/profile", get(profile_handler))
        .route("/send/{command}", post(send))
        .with_state(state)
}

pub fn build_router_for_tests(profile: Profile, config: AppConfig) -> Router {
    build_router(profile, config, Arc::new(MockTransmitter::new()))
}

pub async fn serve(
    profile: Profile,
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
    axum::serve(listener, build_router(profile, config, transmitter)).await?;
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

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

async fn profile_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(err) = authorize(&state.config, &headers) {
        return error_response(err);
    }

    (
        StatusCode::OK,
        Json(json!({
            "brand": state.profile.brand,
            "model": state.profile.model,
            "device_type": state.profile.device_type,
            "commands": state.profile.commands.keys().collect::<Vec<_>>()
        })),
    )
        .into_response()
}

async fn send(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(command): Path<String>,
) -> impl IntoResponse {
    if let Err(err) = authorize(&state.config, &headers) {
        return error_response(err);
    }

    let repeat = state.config.default_repeat.max(1);
    match state
        .profile
        .signal_for(&command)
        .and_then(|signal| state.transmitter.send(signal, repeat))
    {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "sent": command, "profile": state.profile.id() })),
        )
            .into_response(),
        Err(err) => error_response(err),
    }
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
        IrisError::CommandNotFound { .. } => StatusCode::NOT_FOUND,
        _ => StatusCode::BAD_REQUEST,
    };
    (status, Json(json!({ "error": err.to_string() }))).into_response()
}
