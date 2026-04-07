use std::time::Duration;

use lpdwise_process::runner::{CommandRunner, ProcessRunner};
use lpdwise_process::ProcessError;
use tracing::debug;

/// External tool dependency for lpdwise.
struct ToolCheck {
    name: &'static str,
    install_hint: &'static str,
    probe_args: &'static [&'static str],
    ok_hint: &'static str,
    allow_nonzero_probe: bool,
    prefer_mise: bool,
}

const REQUIRED_TOOLS: &[ToolCheck] = &[
    ToolCheck {
        name: "mise",
        install_hint: "https://mise.jdx.dev/getting-started.html",
        probe_args: &["--version"],
        ok_hint: "mise available",
        allow_nonzero_probe: false,
        prefer_mise: false,
    },
    ToolCheck {
        name: "yt-dlp",
        install_hint: "mise use -g yt-dlp",
        probe_args: &["--version"],
        ok_hint: "yt-dlp available",
        allow_nonzero_probe: false,
        prefer_mise: false,
    },
    ToolCheck {
        name: "ffmpeg",
        install_hint: "mise use -g ffmpeg",
        probe_args: &["-version"],
        ok_hint: "ffmpeg available",
        allow_nonzero_probe: false,
        prefer_mise: false,
    },
    ToolCheck {
        name: "ffprobe",
        install_hint: "mise use -g ffmpeg  (ffprobe is included with ffmpeg)",
        probe_args: &["-version"],
        ok_hint: "ffprobe available",
        allow_nonzero_probe: false,
        prefer_mise: false,
    },
    ToolCheck {
        name: "llmfit",
        install_hint: "mise use -g cargo:llmfit@latest",
        probe_args: &["system", "--json"],
        ok_hint: "system --json ok",
        allow_nonzero_probe: false,
        prefer_mise: true,
    },
    ToolCheck {
        name: "sherpa-onnx-offline",
        install_hint: "mise use -g github:k2-fsa/sherpa-onnx",
        probe_args: &["--help"],
        ok_hint: "binary responded to --help",
        allow_nonzero_probe: true,
        prefer_mise: true,
    },
];

/// Result of checking a single tool.
pub(crate) struct ToolStatus {
    pub(crate) name: &'static str,
    pub(crate) found: bool,
    pub(crate) version: Option<String>,
    pub(crate) install_hint: &'static str,
    pub(crate) source: Option<&'static str>,
    pub(crate) note: Option<String>,
}

/// Run the doctor check: verify all required external tools are available.
pub(crate) async fn run_doctor() -> Vec<ToolStatus> {
    let runner = CommandRunner::new(Duration::from_secs(5));
    let mut results = Vec::with_capacity(REQUIRED_TOOLS.len());

    for tool in REQUIRED_TOOLS {
        let status = check_tool(tool, &runner).await;
        results.push(status);
    }

    results
}

async fn check_tool(tool: &ToolCheck, runner: &CommandRunner) -> ToolStatus {
    let mut failures = Vec::new();

    if tool.prefer_mise {
        if let Some(path) = resolve_mise_binary(tool.name, runner).await {
            let probe = probe_command(path.as_str(), tool, runner).await;
            match probe {
                ProbeStatus::Ok(summary) => {
                    return ToolStatus {
                        name: tool.name,
                        found: true,
                        version: Some(summary),
                        install_hint: tool.install_hint,
                        source: Some("mise"),
                        note: None,
                    };
                }
                ProbeStatus::Unavailable(reason) => {
                    failures.push(format!("mise-managed binary failed: {reason}"));
                }
            }
        }
    }

    let probe = probe_command(tool.name, tool, runner).await;
    let (found, version) = match probe {
        ProbeStatus::Ok(summary) => (true, Some(summary)),
        ProbeStatus::Unavailable(reason) => {
            failures.push(reason);
            (false, None)
        }
    };

    debug!(
        tool = tool.name,
        found,
        version = version.as_deref().unwrap_or("unknown"),
        "tool check"
    );

    ToolStatus {
        name: tool.name,
        found,
        version,
        install_hint: tool.install_hint,
        source: found.then_some("PATH"),
        note: (!found).then(|| failures.join(" | ")),
    }
}

async fn resolve_mise_binary(tool_name: &str, runner: &CommandRunner) -> Option<String> {
    let output = runner
        .run_checked("mise", &["which", tool_name])
        .await
        .ok()?;
    let path = output.stdout.trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

enum ProbeStatus {
    Ok(String),
    Unavailable(String),
}

async fn probe_command(program: &str, tool: &ToolCheck, runner: &CommandRunner) -> ProbeStatus {
    let result = runner.run_checked(program, tool.probe_args).await;

    match result {
        Ok(output) => ProbeStatus::Ok(summarize_probe_output(tool, &output.stdout, &output.stderr)),
        Err(ProcessError::NonZeroExit { output, .. }) if tool.allow_nonzero_probe => {
            ProbeStatus::Ok(summarize_probe_output(tool, &output.stdout, &output.stderr))
        }
        Err(ProcessError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            ProbeStatus::Unavailable(format!("{program} not found"))
        }
        Err(ProcessError::Timeout(timeout)) => {
            ProbeStatus::Unavailable(format!("{program} timed out after {timeout:?}"))
        }
        Err(ProcessError::NonZeroExit { output, status }) => ProbeStatus::Unavailable(format!(
            "{program} exited with {status}{}",
            command_context(&output.stdout, &output.stderr)
        )),
        Err(ProcessError::Io(e)) => ProbeStatus::Unavailable(format!("{program} failed: {e}")),
        Err(ProcessError::NoPid) => {
            ProbeStatus::Unavailable(format!("{program} failed to report a process id"))
        }
    }
}

fn summarize_probe_output(tool: &ToolCheck, stdout: &str, stderr: &str) -> String {
    let summary = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(tool.ok_hint);

    if summary.starts_with('{') {
        tool.ok_hint.to_string()
    } else {
        summary.to_string()
    }
}

fn command_context(stdout: &str, stderr: &str) -> String {
    let detail = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| !line.is_empty());

    match detail {
        Some(line) => format!(": {line}"),
        None => String::new(),
    }
}

/// Print doctor results to stdout with visual indicators.
pub(crate) fn print_doctor_results(results: &[ToolStatus]) {
    println!("lpdwise doctor — checking external dependencies\n");

    let mut all_ok = true;
    for status in results {
        if status.found {
            let ver = status.version.as_deref().unwrap_or("(version unknown)");
            match status.source {
                Some(source) => println!("  [ok]   {} — {} ({})", status.name, ver, source),
                None => println!("  [ok]   {} — {}", status.name, ver),
            }
        } else {
            all_ok = false;
            let note = status.note.as_deref().unwrap_or("not found");
            println!("  [MISS] {} — {}", status.name, note);
            println!("         Install with: {}", status.install_hint);
        }
    }

    println!();
    if all_ok {
        println!("All dependencies satisfied.");
    } else {
        println!("Some dependencies are missing. Install them to use all features.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_doctor_returns_results_for_all_tools() {
        let results = run_doctor().await;
        assert_eq!(results.len(), REQUIRED_TOOLS.len());

        // Each result should have a non-empty name and install hint
        for result in &results {
            assert!(!result.name.is_empty());
            assert!(!result.install_hint.is_empty());
        }
    }

    #[tokio::test]
    async fn test_check_echo_found() {
        let runner = CommandRunner::with_default_timeout();
        let tool = ToolCheck {
            name: "echo",
            install_hint: "built-in",
            probe_args: &["hello"],
            ok_hint: "echo available",
            allow_nonzero_probe: false,
            prefer_mise: false,
        };
        let status = check_tool(&tool, &runner).await;
        assert!(status.found);
    }

    #[tokio::test]
    async fn test_check_nonexistent_tool() {
        let runner = CommandRunner::with_default_timeout();
        let tool = ToolCheck {
            name: "definitely_not_a_real_tool_xyz",
            install_hint: "n/a",
            probe_args: &["--version"],
            ok_hint: "n/a",
            allow_nonzero_probe: false,
            prefer_mise: false,
        };
        let status = check_tool(&tool, &runner).await;
        assert!(!status.found);
        assert!(status.version.is_none());
    }

    #[test]
    fn test_required_tools_include_local_runtime_dependencies() {
        assert!(REQUIRED_TOOLS.iter().any(|tool| tool.name == "llmfit"));
        assert!(REQUIRED_TOOLS
            .iter()
            .any(|tool| tool.name == "sherpa-onnx-offline"));
    }
}
