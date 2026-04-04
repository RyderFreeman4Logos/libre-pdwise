use lpdwise_process::runner::{CommandRunner, ProcessRunner};
use tracing::debug;

/// External tool dependency for lpdwise.
struct ToolCheck {
    name: &'static str,
    install_hint: &'static str,
}

const REQUIRED_TOOLS: &[ToolCheck] = &[
    ToolCheck {
        name: "yt-dlp",
        install_hint: "mise use -g yt-dlp",
    },
    ToolCheck {
        name: "ffmpeg",
        install_hint: "mise use -g ffmpeg",
    },
    ToolCheck {
        name: "ffprobe",
        install_hint: "mise use -g ffmpeg  (ffprobe is included with ffmpeg)",
    },
];

/// Result of checking a single tool.
pub(crate) struct ToolStatus {
    pub(crate) name: &'static str,
    pub(crate) found: bool,
    pub(crate) version: Option<String>,
    pub(crate) install_hint: &'static str,
}

/// Run the doctor check: verify all required external tools are available.
pub(crate) async fn run_doctor() -> Vec<ToolStatus> {
    let runner = CommandRunner::with_default_timeout();
    let mut results = Vec::with_capacity(REQUIRED_TOOLS.len());

    for tool in REQUIRED_TOOLS {
        let status = check_tool(tool, &runner).await;
        results.push(status);
    }

    results
}

async fn check_tool(tool: &ToolCheck, runner: &CommandRunner) -> ToolStatus {
    // First check if the tool exists in PATH
    let found = runner.run_checked("which", &[tool.name]).await.is_ok();

    let version = if found {
        get_version(tool.name, runner).await
    } else {
        None
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
    }
}

async fn get_version(tool_name: &str, runner: &CommandRunner) -> Option<String> {
    let output = runner
        .run_checked(tool_name, &["--version"])
        .await
        .ok()?;

    // Take the first line of stdout (or stderr for some tools)
    let text = if output.stdout.trim().is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };

    text.lines().next().map(|l| l.trim().to_string())
}

/// Print doctor results to stdout with visual indicators.
pub(crate) fn print_doctor_results(results: &[ToolStatus]) {
    println!("lpdwise doctor — checking external dependencies\n");

    let mut all_ok = true;
    for status in results {
        if status.found {
            let ver = status.version.as_deref().unwrap_or("(version unknown)");
            println!("  [ok]   {} — {}", status.name, ver);
        } else {
            all_ok = false;
            println!("  [MISS] {} — not found", status.name);
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
        };
        let status = check_tool(&tool, &runner).await;
        assert!(!status.found);
        assert!(status.version.is_none());
    }
}
