use std::path::Path;
use std::process::ExitStatus;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tracing::{debug, warn};

/// Default subprocess timeout: 30 minutes.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Outcome of a subprocess execution.
pub struct ProcessOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Abstraction over subprocess execution with RAII lifecycle management.
pub trait ProcessRunner {
    /// Run a command to completion, collecting all output.
    ///
    /// Returns an error on non-zero exit, timeout, or I/O failure.
    fn run_checked(
        &self,
        program: &str,
        args: &[&str],
    ) -> impl std::future::Future<Output = Result<ProcessOutput, ProcessError>>;

    /// Run a command with real-time streaming of stdout/stderr.
    ///
    /// Output is simultaneously:
    /// - Logged via tracing (terminal visibility)
    /// - Appended to `log_file` if provided
    /// - Buffered in the returned `ProcessOutput`
    fn run_streaming(
        &self,
        program: &str,
        args: &[&str],
        log_file: Option<&Path>,
    ) -> impl std::future::Future<Output = Result<ProcessOutput, ProcessError>>;
}

/// A runner that delegates to `tokio::process::Command` with process-group
/// lifecycle management.
pub struct CommandRunner {
    timeout: Duration,
}

impl CommandRunner {
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Create a runner with the default 30-minute timeout.
    pub fn with_default_timeout() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Build a `Command` with process-group isolation via `setsid`.
    fn build_command(program: &str, args: &[&str]) -> Command {
        let mut cmd = Command::new(program);
        cmd.args(args);

        // Create a new session so we can kill the entire process group on
        // timeout. This prevents orphaned grandchild processes.
        // SAFETY: `libc::setsid()` is async-signal-safe per POSIX and has no
        // memory-safety implications. It simply creates a new session for the
        // child process, which is the standard approach for process-group
        // management.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }

        cmd
    }

    /// Kill the entire process group rooted at `pid`, first with SIGTERM, then
    /// SIGKILL after a grace period.
    async fn kill_process_group(pid: u32) {
        let pgid = -(pid as i32);

        // SIGTERM for graceful shutdown
        // SAFETY: Sending a signal to a process group is a standard POSIX
        // operation with no memory-safety concerns.
        unsafe {
            libc::kill(pgid, libc::SIGTERM);
        }

        // Grace period before force-kill
        tokio::time::sleep(Duration::from_secs(2)).await;

        // SAFETY: Same as above — sending SIGKILL to the process group.
        unsafe {
            libc::kill(pgid, libc::SIGKILL);
        }
    }
}

impl Default for CommandRunner {
    fn default() -> Self {
        Self::with_default_timeout()
    }
}

impl ProcessRunner for CommandRunner {
    async fn run_checked(
        &self,
        program: &str,
        args: &[&str],
    ) -> Result<ProcessOutput, ProcessError> {
        debug!(program, ?args, "spawning checked command");

        let mut cmd = Self::build_command(program, args);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;
        let pid = child.id().ok_or(ProcessError::NoPid)?;

        // Take stdout/stderr handles before waiting so we can still access
        // the child for cleanup on timeout.
        let child_stdout = child.stdout.take();
        let child_stderr = child.stderr.take();

        let collect_and_wait = async {
            // Read stdout and stderr concurrently to prevent pipe deadlock.
            // Sequential reads can deadlock if the child writes >64KB to stderr
            // while stdout reading is still in progress (or vice versa).
            let stdout_fut = async {
                let mut buf = Vec::new();
                if let Some(out) = child_stdout {
                    tokio::io::AsyncReadExt::read_to_end(
                        &mut tokio::io::BufReader::new(out),
                        &mut buf,
                    )
                    .await?;
                }
                Ok::<_, std::io::Error>(buf)
            };

            let stderr_fut = async {
                let mut buf = Vec::new();
                if let Some(err) = child_stderr {
                    tokio::io::AsyncReadExt::read_to_end(
                        &mut tokio::io::BufReader::new(err),
                        &mut buf,
                    )
                    .await?;
                }
                Ok::<_, std::io::Error>(buf)
            };

            let (stdout_result, stderr_result) = tokio::join!(stdout_fut, stderr_fut);
            let stdout_buf = stdout_result?;
            let stderr_buf = stderr_result?;

            let status = child.wait().await?;
            Ok::<_, std::io::Error>((status, stdout_buf, stderr_buf))
        };

        let result = tokio::time::timeout(self.timeout, collect_and_wait).await;

        match result {
            Ok(Ok((status, stdout_buf, stderr_buf))) => {
                let code = status.code().unwrap_or(-1);
                let proc_output = ProcessOutput {
                    stdout: String::from_utf8_lossy(&stdout_buf).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr_buf).into_owned(),
                    exit_code: code,
                };

                if status.success() {
                    Ok(proc_output)
                } else {
                    Err(ProcessError::NonZeroExit {
                        status,
                        output: proc_output,
                    })
                }
            }
            Ok(Err(io_err)) => {
                Self::kill_process_group(pid).await;
                Err(ProcessError::Io(io_err))
            }
            Err(_elapsed) => {
                Self::kill_process_group(pid).await;
                Err(ProcessError::Timeout(self.timeout))
            }
        }
    }

    async fn run_streaming(
        &self,
        program: &str,
        args: &[&str],
        log_file: Option<&Path>,
    ) -> Result<ProcessOutput, ProcessError> {
        debug!(program, ?args, "spawning streaming command");

        let mut cmd = Self::build_command(program, args);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;
        let pid = child.id().ok_or(ProcessError::NoPid)?;

        let child_stdout = child.stdout.take();
        let child_stderr = child.stderr.take();

        let mut log_writer = match log_file {
            Some(path) => {
                let file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .await?;
                Some(file)
            }
            None => None,
        };

        // Read stdout and stderr concurrently, buffering and streaming each
        // line as it arrives.
        let stdout_handle = tokio::spawn(async move {
            let mut lines = Vec::new();
            if let Some(out) = child_stdout {
                let mut reader = BufReader::new(out).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    lines.push(line);
                }
            }
            lines
        });

        let stderr_handle = tokio::spawn(async move {
            let mut lines = Vec::new();
            if let Some(err) = child_stderr {
                let mut reader = BufReader::new(err).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    lines.push(line);
                }
            }
            lines
        });

        let wait_future = async {
            let stdout_lines = stdout_handle.await.map_err(std::io::Error::other)?;
            let stderr_lines = stderr_handle.await.map_err(std::io::Error::other)?;

            // Stream to terminal via tracing and optionally to log file
            for line in &stdout_lines {
                debug!(stream = "stdout", "{}", line);
            }
            for line in &stderr_lines {
                warn!(stream = "stderr", "{}", line);
            }

            if let Some(ref mut writer) = log_writer {
                for line in &stdout_lines {
                    writer
                        .write_all(format!("[stdout] {line}\n").as_bytes())
                        .await?;
                }
                for line in &stderr_lines {
                    writer
                        .write_all(format!("[stderr] {line}\n").as_bytes())
                        .await?;
                }
                writer.flush().await?;
            }

            let status = child.wait().await?;

            Ok::<(ExitStatus, String, String), std::io::Error>((
                status,
                stdout_lines.join("\n"),
                stderr_lines.join("\n"),
            ))
        };

        let result = tokio::time::timeout(self.timeout, wait_future).await;

        match result {
            Ok(Ok((status, stdout, stderr))) => {
                let code = status.code().unwrap_or(-1);
                let proc_output = ProcessOutput {
                    stdout,
                    stderr,
                    exit_code: code,
                };

                if status.success() {
                    Ok(proc_output)
                } else {
                    Err(ProcessError::NonZeroExit {
                        status,
                        output: proc_output,
                    })
                }
            }
            Ok(Err(io_err)) => {
                Self::kill_process_group(pid).await;
                let _ = child.wait().await;
                Err(ProcessError::Io(io_err))
            }
            Err(_elapsed) => {
                Self::kill_process_group(pid).await;
                let _ = child.wait().await;
                Err(ProcessError::Timeout(self.timeout))
            }
        }
    }
}

/// Errors from subprocess execution.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("process timed out after {0:?}")]
    Timeout(Duration),

    #[error("process exited with non-zero status: {status}")]
    NonZeroExit {
        status: ExitStatus,
        output: ProcessOutput,
    },

    #[error("failed to obtain child process ID")]
    NoPid,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Allow Debug for ProcessOutput (needed since ProcessError derives Debug).
impl std::fmt::Debug for ProcessOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessOutput")
            .field("exit_code", &self.exit_code)
            .field("stdout_len", &self.stdout.len())
            .field("stderr_len", &self.stderr.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_run_checked_echo() {
        let runner = CommandRunner::with_default_timeout();
        let output = runner.run_checked("echo", &["hello"]).await.unwrap();

        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn test_run_checked_nonzero_exit() {
        let runner = CommandRunner::with_default_timeout();
        let err = runner.run_checked("false", &[]).await.unwrap_err();

        match err {
            ProcessError::NonZeroExit { output, .. } => {
                assert_ne!(output.exit_code, 0);
            }
            other => panic!("expected NonZeroExit, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_run_checked_timeout() {
        let runner = CommandRunner::new(Duration::from_millis(100));
        let err = runner.run_checked("sleep", &["10"]).await.unwrap_err();

        assert!(
            matches!(err, ProcessError::Timeout(_)),
            "expected Timeout, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_run_checked_captures_stderr() {
        let runner = CommandRunner::with_default_timeout();
        let err = runner
            .run_checked("sh", &["-c", "echo oops >&2; exit 1"])
            .await
            .unwrap_err();

        match err {
            ProcessError::NonZeroExit { output, .. } => {
                assert!(output.stderr.contains("oops"));
            }
            other => panic!("expected NonZeroExit, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_run_streaming_echo() {
        let runner = CommandRunner::with_default_timeout();
        let output = runner
            .run_streaming("echo", &["streaming"], None)
            .await
            .unwrap();

        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("streaming"));
    }

    #[tokio::test]
    async fn test_run_streaming_writes_log_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let runner = CommandRunner::with_default_timeout();

        runner
            .run_streaming("echo", &["logged"], Some(tmp.path()))
            .await
            .unwrap();

        let log_content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(log_content.contains("logged"));
    }

    #[tokio::test]
    async fn test_run_streaming_timeout() {
        let runner = CommandRunner::new(Duration::from_millis(100));
        let err = runner
            .run_streaming("sleep", &["10"], None)
            .await
            .unwrap_err();

        assert!(
            matches!(err, ProcessError::Timeout(_)),
            "expected Timeout, got: {err:?}"
        );
    }
}
