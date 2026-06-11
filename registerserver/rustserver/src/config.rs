use std::{env, net::IpAddr, path::PathBuf};

use crate::network_access::AccessScope;
use crate::static_files::default_client_dist_path;
use crate::util::{
    env_bool, env_positive_u16, env_positive_u32, env_positive_u64, env_positive_usize,
};

#[derive(Clone)]
pub(crate) struct Config {
    pub(crate) port: u16,
    pub(crate) listen_host: IpAddr,
    pub(crate) access_scope: AccessScope,
    pub(crate) heartbeat_interval_ms: u64,
    pub(crate) websocket_outbound_queue_size: usize,
    pub(crate) unity_heartbeat_stale_ms: u64,
    pub(crate) execution_timeout_ms: u64,
    pub(crate) method_refresh_min_count: usize,
    pub(crate) method_refresh_timeout_ms: u64,
    pub(crate) web_console_token: Option<String>,
    pub(crate) client_dist_dir: PathBuf,
    pub(crate) artifact_dir: PathBuf,
    pub(crate) artifact_max_bytes: usize,
    pub(crate) log_dir: Option<String>,
    pub(crate) log_prefix: String,
    pub(crate) archive_mysql_enabled: bool,
    pub(crate) archive_mysql_url: Option<String>,
    pub(crate) archive_mysql_max_connections: u32,
    pub(crate) archive_queue_size: usize,
}

impl Config {
    pub(crate) fn from_env() -> Self {
        load_dotenv();
        Self {
            port: env_positive_u16("PORT", 3000),
            listen_host: env_ip_addr("QA_LISTEN_HOST", IpAddr::from([0, 0, 0, 0])),
            access_scope: AccessScope::from_env_value(
                env::var("QA_ACCESS_SCOPE")
                    .ok()
                    .as_deref()
                    .unwrap_or("private"),
            ),
            heartbeat_interval_ms: env_positive_u64("WS_HEARTBEAT_INTERVAL_MS", 15_000),
            websocket_outbound_queue_size: env_positive_usize("QA_WS_OUTBOUND_QUEUE_SIZE", 1024),
            unity_heartbeat_stale_ms: env_positive_u64("UNITY_HEARTBEAT_STALE_MS", 45_000),
            execution_timeout_ms: env_positive_u64("EXECUTION_TIMEOUT_MS", 20_000),
            method_refresh_min_count: env_positive_usize("QA_METHOD_REFRESH_MIN_COUNT", 5),
            method_refresh_timeout_ms: env_positive_u64("QA_METHOD_REFRESH_TIMEOUT_MS", 1_500),
            web_console_token: env::var("QA_WEB_CONSOLE_TOKEN")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            client_dist_dir: env_path("QA_CLIENT_DIST_DIR")
                .unwrap_or_else(default_client_dist_path),
            artifact_dir: env_path("QA_ARTIFACT_DIR").unwrap_or_else(default_artifact_dir),
            artifact_max_bytes: env_positive_usize("QA_ARTIFACT_MAX_BYTES", 20 * 1024 * 1024),
            log_dir: env::var("QA_LOG_DIR")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            log_prefix: env::var("QA_LOG_PREFIX")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "registerserver".to_string()),
            archive_mysql_enabled: env_bool("QA_EXECUTION_ARCHIVE_MYSQL_ENABLED", false),
            archive_mysql_url: env::var("QA_EXECUTION_ARCHIVE_MYSQL_URL")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            archive_mysql_max_connections: env_positive_u32(
                "QA_EXECUTION_ARCHIVE_MYSQL_MAX_CONNECTIONS",
                5,
            ),
            archive_queue_size: env_positive_usize("QA_EXECUTION_ARCHIVE_QUEUE_SIZE", 10_000),
        }
    }
}

fn default_artifact_dir() -> PathBuf {
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("artifacts")
}

fn load_dotenv() {
    if dotenvy::dotenv().is_ok() {
        return;
    }

    if let Ok(exe_path) = env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        let _ = dotenvy::from_path(exe_dir.join(".env"));
    }
}

fn env_ip_addr(name: &str, fallback: IpAddr) -> IpAddr {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<IpAddr>().ok())
        .unwrap_or(fallback)
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}
