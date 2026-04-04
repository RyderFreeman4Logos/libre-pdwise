use crate::language::Language;
use crate::types::{PromptPayload, Transcript};

/// Built-in prompt templates for knowledge extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptTemplate {
    /// Structured summary with outline, key points, and source quotes.
    Standard,
    /// Extract contrarian insights — counterintuitive claims with evidence.
    Contrarian,
    /// Political-economic logic decomposition: actors, interests, game theory, predictions.
    Political,
    /// Full faithful translation into Chinese, with terminology annotations.
    Translation,
}

// -- Chinese prompt templates (const) --

const STANDARD_TEMPLATE: &str = "\
请对以上转录稿进行结构化总结，要求如下：

1. **大纲**：列出主要话题和子话题的层级结构
2. **核心要点**：每个话题提炼 2-3 个关键观点，用简洁的陈述句表达
3. **引用原文**：每个要点附上最相关的原文片段作为佐证（标注时间戳）
4. **总结**：用 3-5 句话概括全文核心信息

输出格式：Markdown，使用标题层级区分结构。";

const CONTRARIAN_TEMPLATE: &str = "\
请从以上转录稿中提取反常识、违反直觉的干货内容，要求如下：

1. **反常识观点**：列出所有与主流认知相悖的观点或结论
2. **论据链**：每个观点给出演讲者提供的论据和推理过程
3. **可信度评估**：简要评估每个观点的论据强度（强/中/弱）
4. **启发**：这些观点对读者的实际决策有何参考价值

只提取有实质论据支撑的观点，忽略纯粹的修辞或煽动性表述。
输出格式：Markdown，每个观点作为独立小节。";

const POLITICAL_TEMPLATE: &str = "\
请对以上转录稿进行政经逻辑拆解，要求如下：

1. **主体识别**：列出所有相关的政治/经济行为主体（国家、组织、个人）
2. **利益分析**：每个主体的核心利益诉求和约束条件
3. **博弈结构**：主体之间的合作与对抗关系，关键博弈点
4. **因果链条**：从已知事实到分析结论的推理过程
5. **预测与风险**：基于上述分析的可能走向和关键变量

保持分析客观中立，区分事实陈述和个人判断。
输出格式：Markdown，按分析维度组织。";

const TRANSLATION_TEMPLATE: &str = "\
请将以上转录稿全文翻译为中文，要求如下：

1. **忠实原义**：准确传达原文含义，不增删内容
2. **自然流畅**：符合中文表达习惯，避免翻译腔
3. **术语注释**：专业术语首次出现时在括号内标注原文，如：量化宽松（Quantitative Easing）
4. **段落对应**：保持与原文相同的段落划分

输出格式：纯中文译文，专业术语带括号注释。";

/// Context prefix for Chinese transcript (first-round: produce corrected transcript).
const CHINESE_CONTEXT: &str = "\
你是一位专业的中文内容分析助手。以下是一段中文音视频的自动转录稿，\
可能包含语音识别错误。";

/// Context prefix for foreign-language transcript.
const FOREIGN_CONTEXT: &str = "\
你是一位专业的多语言内容分析助手。以下是一段外语音视频的自动转录稿，\
可能包含语音识别错误。";

/// Chinese-transcript first-round instruction: output corrected transcript then ask.
const CHINESE_FIRST_ROUND: &str = "\
请先输出修正后的纯净转录稿（修正明显的语音识别错误，保留原始表述），\
然后询问用户需要哪种后续处理。";

/// Foreign-transcript first-round instruction: output corrected transcript in original language, ask in Chinese.
const FOREIGN_FIRST_ROUND: &str = "\
请先输出原语言的修正转录稿（修正明显的语音识别错误，保留原始表述），\
然后用中文询问用户需要哪种后续处理。";

impl PromptTemplate {
    /// All available templates for UI enumeration.
    pub const ALL: [PromptTemplate; 4] = [
        Self::Standard,
        Self::Contrarian,
        Self::Political,
        Self::Translation,
    ];

    /// Human-readable label (Chinese).
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "结构化总结",
            Self::Contrarian => "反常识提取",
            Self::Political => "政经逻辑拆解",
            Self::Translation => "全文翻译",
        }
    }

    /// CLI value string.
    pub fn cli_value(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Contrarian => "contrarian",
            Self::Political => "political",
            Self::Translation => "translation",
        }
    }

    /// Parse from CLI value string.
    pub fn from_cli_value(s: &str) -> Option<Self> {
        match s {
            "standard" => Some(Self::Standard),
            "contrarian" => Some(Self::Contrarian),
            "political" => Some(Self::Political),
            "translation" => Some(Self::Translation),
            _ => None,
        }
    }

    /// The core instruction template for this prompt type.
    fn instruction_template(self) -> &'static str {
        match self {
            Self::Standard => STANDARD_TEMPLATE,
            Self::Contrarian => CONTRARIAN_TEMPLATE,
            Self::Political => POLITICAL_TEMPLATE,
            Self::Translation => TRANSLATION_TEMPLATE,
        }
    }

    /// Render this template into a prompt payload with language-aware assembly.
    ///
    /// Strategy:
    /// - Chinese transcript → Chinese context + corrected transcript request
    /// - Foreign transcript → foreign context + original-language correction + Chinese Q&A
    /// - Core instruction placed at tail (after transcript body) to prevent
    ///   "lost in the middle" attention degradation.
    pub fn render(self, transcript: &Transcript, language: Language) -> PromptPayload {
        let is_chinese = matches!(language, Language::Chinese);
        let body = transcript.pure_text();

        let context = if is_chinese {
            CHINESE_CONTEXT.to_string()
        } else {
            FOREIGN_CONTEXT.to_string()
        };

        let first_round = if is_chinese {
            CHINESE_FIRST_ROUND
        } else {
            FOREIGN_FIRST_ROUND
        };

        // Tail-placement: instruction goes after the transcript body.
        let instruction = format!(
            "{}\n\n如果用户选择了具体处理方式，则按以下模板执行：\n\n{}",
            first_round,
            self.instruction_template(),
        );

        PromptPayload {
            context,
            body,
            instruction,
        }
    }
}

impl std::fmt::Display for PromptTemplate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::types::TranscriptSegment;

    fn sample_transcript() -> Transcript {
        Transcript {
            segments: vec![
                TranscriptSegment {
                    text: "Hello world".into(),
                    start: Duration::from_secs(0),
                    end: Duration::from_secs(5),
                },
                TranscriptSegment {
                    text: "This is a test".into(),
                    start: Duration::from_secs(5),
                    end: Duration::from_secs(10),
                },
            ],
        }
    }

    #[test]
    fn test_render_standard_chinese_has_tail_instruction() {
        let payload =
            PromptTemplate::Standard.render(&sample_transcript(), Language::Chinese);
        assert!(payload.context.contains("中文"));
        assert!(payload.body.contains("Hello world"));
        assert!(payload.instruction.contains("结构化总结"));
        assert!(payload.instruction.contains("修正后的纯净转录稿"));
    }

    #[test]
    fn test_render_standard_english_uses_foreign_context() {
        let payload =
            PromptTemplate::Standard.render(&sample_transcript(), Language::English);
        assert!(payload.context.contains("外语"));
        assert!(payload.instruction.contains("原语言"));
    }

    #[test]
    fn test_render_contrarian_contains_template() {
        let payload =
            PromptTemplate::Contrarian.render(&sample_transcript(), Language::Chinese);
        assert!(payload.instruction.contains("反常识"));
    }

    #[test]
    fn test_render_political_contains_template() {
        let payload =
            PromptTemplate::Political.render(&sample_transcript(), Language::Chinese);
        assert!(payload.instruction.contains("博弈"));
    }

    #[test]
    fn test_render_translation_contains_template() {
        let payload =
            PromptTemplate::Translation.render(&sample_transcript(), Language::English);
        assert!(payload.instruction.contains("翻译"));
    }

    #[test]
    fn test_assemble_ordering() {
        let payload =
            PromptTemplate::Standard.render(&sample_transcript(), Language::Chinese);
        let assembled = payload.assemble();
        let ctx_pos = assembled.find("中文").unwrap();
        let body_pos = assembled.find("Hello world").unwrap();
        let instr_pos = assembled.find("结构化总结").unwrap();
        // Verify tail-placement: context < body < instruction
        assert!(ctx_pos < body_pos);
        assert!(body_pos < instr_pos);
    }

    #[test]
    fn test_cli_value_roundtrip() {
        for tpl in PromptTemplate::ALL {
            let parsed = PromptTemplate::from_cli_value(tpl.cli_value());
            assert_eq!(parsed, Some(tpl));
        }
    }

    #[test]
    fn test_from_cli_value_invalid() {
        assert_eq!(PromptTemplate::from_cli_value("invalid"), None);
    }

    #[test]
    fn test_all_templates_have_labels() {
        for tpl in PromptTemplate::ALL {
            assert!(!tpl.label().is_empty());
            assert!(!tpl.cli_value().is_empty());
        }
    }
}
