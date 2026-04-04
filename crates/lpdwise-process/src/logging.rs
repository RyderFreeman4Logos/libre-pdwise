use std::path::PathBuf;

use tracing_subscriber::prelude::*;

/// Configuration for session-scoped dual-write logging.
///
/// Logs go to both stderr (coloured, concise) and a file (full timestamps + spans).
pub struct SessionLogConfig {
    session_dir: PathBuf,
}

impl SessionLogConfig {
    /// Create a new session logger under `logs_dir`.
    ///
    /// The session directory is named `<YYYYMMDD-HHMMSS>-<pid>` to ensure
    /// uniqueness across concurrent processes.
    pub fn new(logs_dir: PathBuf) -> Result<Self, LoggingError> {
        let now = time::OffsetDateTime::now_utc();
        let timestamp = now
            .format(
                &time::format_description::parse("[year][month][day]-[hour][minute][second]")
                    .map_err(|e| LoggingError::SubscriberInit(e.to_string()))?,
            )
            .map_err(|e| LoggingError::SubscriberInit(e.to_string()))?;

        let pid = std::process::id();
        let dir_name = format!("{timestamp}-{pid}");
        let session_dir = logs_dir.join(dir_name);

        std::fs::create_dir_all(&session_dir)?;

        Ok(Self { session_dir })
    }

    /// Initialize the tracing subscriber with dual-write (terminal + file).
    ///
    /// - Terminal layer: coloured, concise format (level + message).
    /// - File layer: full timestamps, level, span context → `session.log`.
    pub fn init_tracing(&self) -> Result<(), LoggingError> {
        let log_file = std::fs::File::create(self.session_dir.join("session.log"))?;

        // Terminal layer: coloured, compact
        let terminal_layer = tracing_subscriber::fmt::layer()
            .with_ansi(true)
            .with_target(false)
            .compact();

        // File layer: full detail, no ANSI
        let file_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(log_file)
            .with_target(true)
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE);

        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

        tracing_subscriber::registry()
            .with(filter)
            .with(terminal_layer)
            .with(file_layer)
            .try_init()
            .map_err(|e| LoggingError::SubscriberInit(e.to_string()))?;

        Ok(())
    }

    /// Return the session directory path (for error messages suggesting log attachment).
    pub fn session_dir(&self) -> &PathBuf {
        &self.session_dir
    }

    /// Format a user-facing hint for attaching logs to an issue report.
    pub fn issue_hint(&self) -> String {
        format!(
            "请将此日志附到 issue: {}/session.log",
            self.session_dir.display()
        )
    }
}

/// Errors from logging setup.
#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    #[error("failed to create log directory: {0}")]
    CreateDir(#[from] std::io::Error),

    #[error("tracing subscriber init failed: {0}")]
    SubscriberInit(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_dir_created_with_pid_suffix() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = SessionLogConfig::new(tmp.path().to_path_buf()).unwrap();

        assert!(config.session_dir().is_dir());

        // Directory name should end with the current PID
        let dir_name = config.session_dir().file_name().unwrap().to_str().unwrap();
        let expected_suffix = format!("-{}", std::process::id());
        assert!(
            dir_name.ends_with(&expected_suffix),
            "dir name {dir_name} should end with {expected_suffix}"
        );
    }

    #[test]
    fn test_issue_hint_contains_session_log_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = SessionLogConfig::new(tmp.path().to_path_buf()).unwrap();

        let hint = config.issue_hint();
        assert!(hint.contains("session.log"));
        assert!(hint.contains(&config.session_dir().display().to_string()));
    }
}
