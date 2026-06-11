mod api;
mod archive;
mod artifacts;
mod auth;
mod clients;
mod config;
mod execution;
mod heartbeat;
mod logging;
mod messaging;
mod network_access;
mod sequence;
mod state;
mod static_files;
mod util;
mod websocket;

use std::{net::SocketAddr, sync::Arc, time::Instant};

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{any, get, post},
};
use tokio::{net::TcpListener, sync::RwLock};
use tower_http::cors::{Any, CorsLayer};

use crate::{
    api::{
        api_execute, api_health, api_not_found, api_refresh_methods_if_needed, api_results,
        api_unity_clients,
    },
    archive::ExecutionArchive,
    artifacts::{api_create_artifact, api_download_artifact, api_get_artifact},
    auth::{api_web_auth, api_web_login, api_web_logout},
    config::Config,
    heartbeat::heartbeat_loop,
    logging::{LogEvent, init_daily_file_logger},
    network_access::enforce_network_access,
    state::{AppState, ServerState},
    static_files::static_service,
    websocket::ws_handler,
};

#[tokio::main]
async fn main() {
    let config = Config::from_env();
    if let Some(log_dir) = config.log_dir.as_deref()
        && let Err(error) = init_daily_file_logger(log_dir, &config.log_prefix)
    {
        eprintln!("{error}");
    }
    let archive = ExecutionArchive::initialize(&config).await;
    let state = Arc::new(AppState {
        started_at: Instant::now(),
        config,
        archive,
        inner: RwLock::new(ServerState::default()),
    });

    let heartbeat_task = tokio::spawn(heartbeat_loop(state.clone()));

    let client_dist_path = state.config.client_dist_dir.clone();
    let artifact_body_limit = state.config.artifact_max_bytes;
    let app = Router::new()
        .route("/api/health", get(api_health))
        .route("/api/web-auth", get(api_web_auth))
        .route("/api/web-login", post(api_web_login))
        .route("/api/web-logout", post(api_web_logout))
        .route("/api/unity-clients", get(api_unity_clients))
        .route("/api/results", get(api_results))
        .route(
            "/api/unity-clients/{client_id}/refresh-methods-if-needed",
            post(api_refresh_methods_if_needed),
        )
        .route("/api/unity-clients/{client_id}/execute", post(api_execute))
        .route(
            "/api/artifacts",
            post(api_create_artifact).layer(DefaultBodyLimit::max(artifact_body_limit)),
        )
        .route("/api/artifacts/{artifact_id}", get(api_get_artifact))
        .route(
            "/api/artifacts/{artifact_id}/download",
            get(api_download_artifact),
        )
        .route("/api/{*rest}", any(api_not_found))
        .route("/ws", get(ws_handler))
        .fallback_service(static_service(client_dist_path))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            enforce_network_access,
        ))
        .with_state(state.clone());

    let addr = SocketAddr::new(state.config.listen_host, state.config.port);
    let listener = TcpListener::bind(addr)
        .await
        .unwrap_or_else(|error| panic!("failed to bind {addr}: {error}"));

    LogEvent::new("server_started")
        .field("listenHost", state.config.listen_host.to_string())
        .field("port", state.config.port)
        .field("accessScope", state.config.access_scope.as_str())
        .field(
            "clientDistDir",
            state.config.client_dist_dir.to_string_lossy().to_string(),
        )
        .field(
            "webConsoleAuthRequired",
            state.config.web_console_token.is_some(),
        )
        .field("logDir", state.config.log_dir.clone())
        .field("logPrefix", state.config.log_prefix.clone())
        .field(
            "executionArchiveMysqlConfigured",
            state.config.archive_mysql_enabled,
        )
        .field("executionArchiveMysqlActive", state.archive.enabled())
        .field(
            "artifactDir",
            state.config.artifact_dir.to_string_lossy().to_string(),
        )
        .field("artifactMaxBytes", state.config.artifact_max_bytes)
        .emit();

    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal());
    if let Err(error) = server.await {
        LogEvent::error("server_failed")
            .field("error", error.to_string())
            .emit();
        heartbeat_task.abort();
        std::process::exit(1);
    }

    heartbeat_task.abort();
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        LogEvent::error("shutdown_signal_failed")
            .field("error", error.to_string())
            .emit();
        return;
    }

    LogEvent::new("shutdown_signal_received").emit();
}
