use std::{io::Write, path::Path, sync::OnceLock};

use serde_json::{Map, Value};
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};

use crate::{state::ControllerIdentity, util::iso_now};

pub(crate) struct LogEvent {
    event: &'static str,
    level: &'static str,
    fields: Map<String, Value>,
}

impl LogEvent {
    pub(crate) fn new(event: &'static str) -> Self {
        Self {
            event,
            level: "info",
            fields: Map::new(),
        }
    }

    pub(crate) fn warn(event: &'static str) -> Self {
        Self {
            event,
            level: "warn",
            fields: Map::new(),
        }
    }

    pub(crate) fn error(event: &'static str) -> Self {
        Self {
            event,
            level: "error",
            fields: Map::new(),
        }
    }

    pub(crate) fn request_id(mut self, request_id: impl Into<Option<String>>) -> Self {
        self.insert_opt("requestId", request_id.into());
        self
    }

    pub(crate) fn request_id_str(self, request_id: &str) -> Self {
        self.request_id(non_empty(request_id))
    }

    pub(crate) fn client_id(mut self, client_id: impl Into<Option<String>>) -> Self {
        self.insert_opt("clientId", client_id.into());
        self
    }

    pub(crate) fn client_id_str(self, client_id: &str) -> Self {
        self.client_id(non_empty(client_id))
    }

    pub(crate) fn controller(mut self, controller: &ControllerIdentity) -> Self {
        self.fields.insert(
            "controllerId".to_string(),
            Value::String(controller.id.clone()),
        );
        self.fields.insert(
            "controllerType".to_string(),
            Value::String(controller.controller_type.clone()),
        );
        self
    }

    pub(crate) fn controller_id(mut self, controller_id: impl Into<Option<String>>) -> Self {
        self.insert_opt("controllerId", controller_id.into());
        self
    }

    pub(crate) fn sequence_id(mut self, sequence_id: impl Into<Option<String>>) -> Self {
        self.insert_opt("sequenceId", sequence_id.into());
        self
    }

    pub(crate) fn field(mut self, key: &'static str, value: impl serde::Serialize) -> Self {
        let value = serde_json::to_value(value).unwrap_or(Value::Null);
        self.fields.insert(key.to_string(), value);
        self
    }

    pub(crate) fn field_opt(mut self, key: &'static str, value: impl Into<Option<String>>) -> Self {
        self.insert_opt(key, value.into());
        self
    }

    pub(crate) fn emit(self) {
        let mut payload = Map::new();
        payload.insert("time".to_string(), Value::String(iso_now()));
        payload.insert("level".to_string(), Value::String(self.level.to_string()));
        payload.insert("event".to_string(), Value::String(self.event.to_string()));
        for (key, value) in self.fields {
            payload.insert(key, value);
        }
        let line = Value::Object(payload).to_string();
        println!("{line}");
        if let Some(logger) = FILE_LOGGER.get() {
            let mut writer = logger.writer.clone();
            if let Err(error) = writeln!(writer, "{line}") {
                eprintln!("failed to write structured log file: {error}");
            }
        }
    }

    fn insert_opt(&mut self, key: &'static str, value: Option<String>) {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            self.fields.insert(key.to_string(), Value::String(value));
        }
    }
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

static FILE_LOGGER: OnceLock<FileLogger> = OnceLock::new();

struct FileLogger {
    writer: NonBlocking,
    _guard: WorkerGuard,
}

pub(crate) fn init_daily_file_logger(
    directory: impl AsRef<Path>,
    file_prefix: &str,
) -> Result<(), String> {
    let directory = directory.as_ref();
    std::fs::create_dir_all(directory).map_err(|error| {
        format!(
            "Failed to create log directory {}: {error}",
            directory.display()
        )
    })?;

    let appender = tracing_appender::rolling::daily(directory, file_prefix);
    let (writer, guard) = tracing_appender::non_blocking(appender);
    FILE_LOGGER
        .set(FileLogger {
            writer,
            _guard: guard,
        })
        .map_err(|_| "File logger has already been initialized.".to_string())
}
