use inquire::Select;
use lpdwise_core::{Language, PromptTemplate};

/// Interactively select source language if not specified via CLI.
pub(crate) fn select_language() -> Result<Language, inquire::InquireError> {
    let options = vec![
        ("自动检测", Language::Auto),
        ("中文", Language::Chinese),
        ("English", Language::English),
        ("日本語", Language::Japanese),
    ];

    let labels: Vec<&str> = options.iter().map(|(l, _)| *l).collect();
    let selected = Select::new("选择源语言:", labels).prompt()?;

    Ok(options
        .into_iter()
        .find(|(l, _)| *l == selected)
        .map(|(_, lang)| lang)
        .unwrap_or(Language::Auto))
}

/// Interactively select prompt template.
pub(crate) fn select_template() -> Result<PromptTemplate, inquire::InquireError> {
    let templates = PromptTemplate::ALL;
    let labels: Vec<String> = templates.iter().map(|t| t.to_string()).collect();

    let selected = Select::new("选择处理模板:", labels.clone()).prompt()?;

    Ok(templates
        .into_iter()
        .zip(labels.iter())
        .find(|(_, l)| l.as_str() == selected)
        .map(|(t, _)| t)
        .unwrap_or(PromptTemplate::Standard))
}

/// Display engine recommendations and let user pick.
pub(crate) fn select_engine(
    recommendations: &[lpdwise_core::EngineRecommendation],
) -> Result<lpdwise_core::EngineKind, inquire::InquireError> {
    if recommendations.is_empty() {
        // No interactive choice possible — caller handles this
        return Ok(lpdwise_core::EngineKind::GroqWhisper);
    }

    let labels: Vec<String> = recommendations
        .iter()
        .map(|r| format!("{:?} — {}", r.engine, r.reason))
        .collect();

    let selected = Select::new("选择 ASR 引擎:", labels.clone())
        .with_help_message("按推荐优先级排序")
        .prompt()?;

    Ok(recommendations
        .iter()
        .zip(labels.iter())
        .find(|(_, l)| l.as_str() == selected)
        .map(|(r, _)| r.engine)
        .unwrap_or(recommendations[0].engine))
}

#[cfg(test)]
mod tests {
    use lpdwise_core::PromptTemplate;

    #[test]
    fn test_all_templates_have_display() {
        for t in PromptTemplate::ALL {
            let label = t.to_string();
            assert!(!label.is_empty());
        }
    }
}
