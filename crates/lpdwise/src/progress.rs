use indicatif::{ProgressBar, ProgressStyle};

/// Pipeline stages for progress display.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Stage {
    Acquiring,
    Chunking,
    Transcribing,
    Assembling,
    Delivering,
}

impl Stage {
    /// Human-readable label for each stage.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Acquiring => "获取媒体",
            Self::Chunking => "音频分块",
            Self::Transcribing => "语音转录",
            Self::Assembling => "组装 Prompt",
            Self::Delivering => "交付到剪贴板",
        }
    }
}

/// Create a spinner for indeterminate-duration stages.
pub(crate) fn create_spinner(stage: Stage) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .expect("valid spinner template"),
    );
    pb.set_message(format!("{}...", stage.label()));
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb
}

/// Create a progress bar for determinate-count stages (e.g., chunk processing).
pub(crate) fn create_progress_bar(total: u64, stage: Stage) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.cyan} {msg} [{bar:30.cyan/dim}] {pos}/{len}")
            .expect("valid bar template")
            .progress_chars("=> "),
    );
    pb.set_message(stage.label().to_string());
    pb
}

/// Finish a progress bar with a success message.
pub(crate) fn finish_stage(pb: &ProgressBar, stage: Stage) {
    pb.finish_with_message(format!("{} ✓", stage.label()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_stages_have_labels() {
        let stages = [
            Stage::Acquiring,
            Stage::Chunking,
            Stage::Transcribing,
            Stage::Assembling,
            Stage::Delivering,
        ];
        for stage in stages {
            assert!(!stage.label().is_empty());
        }
    }

    #[test]
    fn test_create_spinner_does_not_panic() {
        let pb = create_spinner(Stage::Acquiring);
        finish_stage(&pb, Stage::Acquiring);
    }

    #[test]
    fn test_create_progress_bar_does_not_panic() {
        let pb = create_progress_bar(10, Stage::Transcribing);
        pb.inc(1);
        finish_stage(&pb, Stage::Transcribing);
    }
}
