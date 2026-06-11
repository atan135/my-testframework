use std::sync::OnceLock;

use uuid::Uuid;

pub const DEFAULT_REGISTER_SERVER_URL: &str = "http://localhost:3000";
pub const QAMCP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const HTTP_TIMEOUT_MS: u64 = 10_000;
pub const WS_CONNECT_TIMEOUT_MS: u64 = 10_000;
pub const EXECUTE_TIMEOUT_MS: u64 = 70_000;
pub const SEQUENCE_TIMEOUT_MS: u64 = 180_000;
pub const WATCH_EVENTS_DURATION_MS: u64 = 10_000;
pub const WAIT_RESULT_TIMEOUT_MS: u64 = 180_000;
pub const EVENT_SESSION_TTL_MS: u64 = 600_000;
pub const EVENT_SESSION_MAX_EVENTS: usize = 500;
pub const MAX_SEQUENCE_STEP_DELAY_MS: u64 = 300_000;

static CONTROLLER_ID: OnceLock<String> = OnceLock::new();

pub fn controller_id() -> &'static str {
    CONTROLLER_ID
        .get_or_init(|| {
            std::env::var("QA_CONTROLLER_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("qamcp:{}", Uuid::new_v4()))
        })
        .as_str()
}
