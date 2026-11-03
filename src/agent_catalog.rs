//! Built-in presets transcribed from Tessel. See THIRD_PARTY_NOTICES.md (Orca).
#[derive(Debug)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub command: &'static str,
    pub executable: &'static str,
    pub args: &'static [&'static str],
    pub accent: &'static str,
    pub docs: Option<&'static str>,
}

pub const ALL: &[Preset] = &[
    Preset {
        id: "claude",
        name: "Claude Code",
        command: "claude",
        executable: "claude",
        args: &[],
        accent: "#d97757",
        docs: Some("https://code.claude.com/docs"),
    },
    Preset {
        id: "codex",
        name: "Codex CLI",
        command: "codex",
        executable: "codex",
        args: &[],
        accent: "#10a37f",
        docs: Some("https://developers.openai.com/codex/cli"),
    },
    Preset {
        id: "gemini",
        name: "Gemini CLI",
        command: "gemini",
        executable: "gemini",
        args: &[],
        accent: "#4285f4",
        docs: Some("https://geminicli.com/docs/"),
    },
    Preset {
        id: "opencode",
        name: "OpenCode",
        command: "opencode",
        executable: "opencode",
        args: &[],
        accent: "#e8e8e8",
        docs: Some("https://opencode.ai/docs/"),
    },
    Preset {
        id: "qwen",
        name: "Qwen Code",
        command: "qwen",
        executable: "qwen",
        args: &[],
        accent: "#7c5cff",
        docs: Some("https://qwenlm.github.io/qwen-code-docs/"),
    },
    Preset {
        id: "copilot",
        name: "GitHub Copilot CLI",
        command: "copilot",
        executable: "copilot",
        args: &[],
        accent: "#8957e5",
        docs: Some("https://docs.github.com/en/copilot/how-tos/use-copilot-agents/use-copilot-cli"),
    },
    Preset {
        id: "kimi",
        name: "Kimi Code",
        command: "kimi",
        executable: "kimi",
        args: &[],
        accent: "#3b82f6",
        docs: Some("https://www.kimi.com/code/docs/en/kimi-code-cli/getting-started.html"),
    },
    Preset {
        id: "ollama",
        name: "Ollama",
        command: "ollama",
        executable: "ollama",
        args: &[],
        accent: "#f5f5f5",
        docs: None,
    },
    Preset {
        id: "cline",
        name: "Cline",
        command: "cline",
        executable: "cline",
        args: &[],
        accent: "#6ea8fe",
        docs: Some("https://docs.cline.bot/"),
    },
    Preset {
        id: "amp",
        name: "Amp",
        command: "amp",
        executable: "amp",
        args: &[],
        accent: "#f34e3f",
        docs: Some("https://ampcode.com/manual"),
    },
    Preset {
        id: "cursor",
        name: "Cursor CLI",
        command: "cursor-agent",
        executable: "cursor-agent",
        args: &[],
        accent: "#a3a3a3",
        docs: Some("https://cursor.com/docs/cli/overview"),
    },
    Preset {
        id: "grok",
        name: "Grok Build",
        command: "grok",
        executable: "grok",
        args: &[],
        accent: "#b8b8b8",
        docs: Some("https://x.ai/cli"),
    },
    Preset {
        id: "pi",
        name: "Pi",
        command: "pi",
        executable: "pi",
        args: &[],
        accent: "#cf7cdb",
        docs: Some("https://pi.dev"),
    },
    Preset {
        id: "droid",
        name: "Factory Droid",
        command: "droid",
        executable: "droid",
        args: &[],
        accent: "#f08050",
        docs: Some("https://docs.factory.ai/cli/getting-started/quickstart"),
    },
    Preset {
        id: "crush",
        name: "Crush",
        command: "crush",
        executable: "crush",
        args: &[],
        accent: "#ff79c6",
        docs: Some("https://github.com/charmbracelet/crush"),
    },
    Preset {
        id: "goose",
        name: "Goose",
        command: "goose",
        executable: "goose",
        args: &[],
        accent: "#e4b65b",
        docs: Some("https://block.github.io/goose/docs/"),
    },
    Preset {
        id: "auggie",
        name: "Auggie",
        command: "auggie",
        executable: "auggie",
        args: &[],
        accent: "#67c5ad",
        docs: Some("https://docs.augmentcode.com/cli/overview"),
    },
    Preset {
        id: "aider",
        name: "Aider",
        command: "aider",
        executable: "aider",
        args: &[],
        accent: "#14b014",
        docs: Some("https://aider.chat/docs/"),
    },
    Preset {
        id: "openclaude",
        name: "OpenClaude",
        command: "openclaude",
        executable: "openclaude",
        args: &[],
        accent: "#d9a077",
        docs: Some("https://openclaude.gitlawb.com/"),
    },
    Preset {
        id: "kilo",
        name: "Kilo Code",
        command: "kilo",
        executable: "kilo",
        args: &[],
        accent: "#f8f675",
        docs: Some("https://kilo.ai/docs/cli"),
    },
    Preset {
        id: "kiro",
        name: "Kiro",
        command: "kiro-cli chat --tui",
        executable: "kiro-cli",
        args: &["chat", "--tui"],
        accent: "#9046ff",
        docs: Some("https://kiro.dev/docs/cli/"),
    },
    Preset {
        id: "continue",
        name: "Continue",
        command: "cn",
        executable: "cn",
        args: &[],
        accent: "#be1fff",
        docs: Some("https://docs.continue.dev/guides/cli"),
    },
    Preset {
        id: "codebuff",
        name: "Codebuff",
        command: "codebuff",
        executable: "codebuff",
        args: &[],
        accent: "#56c271",
        docs: Some("https://www.codebuff.com/docs/help/quick-start"),
    },
    Preset {
        id: "vibe",
        name: "Mistral Vibe",
        command: "vibe",
        executable: "vibe",
        args: &[],
        accent: "#fa520f",
        docs: Some("https://github.com/mistralai/mistral-vibe"),
    },
    Preset {
        id: "antigravity",
        name: "Antigravity",
        command: "agy",
        executable: "agy",
        args: &[],
        accent: "#5b8def",
        docs: Some("https://antigravity.google/docs/cli-overview"),
    },
    Preset {
        id: "rovo",
        name: "Rovo Dev",
        command: "rovo",
        executable: "rovo",
        args: &[],
        accent: "#1868db",
        docs: Some(
            "https://support.atlassian.com/rovo/docs/install-and-run-rovo-dev-cli-on-your-device/",
        ),
    },
    Preset {
        id: "hermes",
        name: "Hermes",
        command: "hermes --tui",
        executable: "hermes",
        args: &["--tui"],
        accent: "#c9a86a",
        docs: Some("https://hermes-agent.nousresearch.com/docs/"),
    },
    Preset {
        id: "devin",
        name: "Devin",
        command: "devin",
        executable: "devin",
        args: &[],
        accent: "#3fb68b",
        docs: Some("https://devin.ai/cli"),
    },
    Preset {
        id: "trae",
        name: "Trae",
        command: "traecli",
        executable: "traecli",
        args: &[],
        accent: "#ff4d4f",
        docs: Some("https://docs.trae.cn/cli_get-started-with-trae-cli"),
    },
    Preset {
        id: "zcode",
        name: "ZCode",
        command: "zcode",
        executable: "zcode",
        args: &[],
        accent: "#6c8cff",
        docs: Some("https://zcode.z.ai/en/docs"),
    },
    Preset {
        id: "autohand",
        name: "Autohand Code",
        command: "autohand",
        executable: "autohand",
        args: &[],
        accent: "#f59e0b",
        docs: Some("https://github.com/autohandai/code-cli"),
    },
    Preset {
        id: "commandcode",
        name: "Command Code",
        command: "command-code --trust",
        executable: "command-code",
        args: &["--trust"],
        accent: "#e5e7eb",
        docs: Some("https://commandcode.ai/docs/quickstart"),
    },
    Preset {
        id: "openclaw",
        name: "OpenClaw",
        command: "openclaw",
        executable: "openclaw",
        args: &[],
        accent: "#ef6b5b",
        docs: Some("https://github.com/openclaw/openclaw"),
    },
    Preset {
        id: "omp",
        name: "OMP",
        command: "omp",
        executable: "omp",
        args: &[],
        accent: "#c04fd8",
        docs: Some("https://omp.sh"),
    },
    Preset {
        id: "muse",
        name: "Muse",
        command: "muse --trust-workspace",
        executable: "muse",
        args: &["--trust-workspace"],
        accent: "#0668e1",
        docs: Some("https://dev.meta.ai/docs/muse-code"),
    },
    Preset {
        id: "opencode2",
        name: "OpenCode 2",
        command: "opencode2",
        executable: "opencode2",
        args: &[],
        accent: "#e8e8e8",
        docs: Some("https://opencode.ai/v2/docs/"),
    },
    Preset {
        id: "mimocode",
        name: "MiMo Code",
        command: "mimo",
        executable: "mimo",
        args: &[],
        accent: "#ff6900",
        docs: Some("https://mimo.xiaomi.com/coder"),
    },
    Preset {
        id: "primeagent",
        name: "Prime Agent",
        command: "prime-agent",
        executable: "prime-agent",
        args: &[],
        accent: "#8b8bf5",
        docs: Some("https://github.com/PrimeIntellect-ai/prime-agent"),
    },
    Preset {
        id: "ante",
        name: "Ante",
        command: "ante",
        executable: "ante",
        args: &[],
        accent: "#22c55e",
        docs: Some("https://github.com/AntigmaLabs/ante-preview"),
    },
    Preset {
        id: "qoder",
        name: "Qoder CLI",
        command: "qodercli",
        executable: "qodercli",
        args: &[],
        accent: "#2adb5c",
        docs: Some("https://docs.qoder.com/cli/overview"),
    },
    Preset {
        id: "freebuff",
        name: "Freebuff",
        command: "freebuff",
        executable: "freebuff",
        args: &[],
        accent: "#e5e7eb",
        docs: Some("https://freebuff.com/cli"),
    },
    Preset {
        id: "dsh",
        name: "DeepSeek Harness",
        command: "dsh-tui",
        executable: "dsh-tui",
        args: &[],
        accent: "#4d6bfe",
        docs: Some("https://deepseek-harness.github.io/deepseek-harness/"),
    },
];

pub fn find(id: &str) -> Option<&'static Preset> {
    let id = if id == "cursor-agent" { "cursor" } else { id };
    ALL.iter().find(|preset| preset.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_matches_all_reference_presets_in_order() {
        let reference: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/agent_catalog.json")).unwrap();
        let rows = reference["agents"].as_array().unwrap();
        assert_eq!(ALL.len(), rows.len());
        for (preset, row) in ALL.iter().zip(rows) {
            assert_eq!(preset.id, row["id"].as_str().unwrap());
            assert_eq!(preset.name, row["name"].as_str().unwrap());
            assert_eq!(preset.command, row["command"].as_str().unwrap());
            assert_eq!(preset.accent, row["accent"].as_str().unwrap());
            assert_eq!(preset.docs, row["docs"].as_str());
            let mut words = vec![preset.executable];
            words.extend_from_slice(preset.args);
            assert_eq!(words.join(" "), preset.command);
            assert!(
                preset
                    .executable
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            );
        }
    }
    #[test]
    fn aliases_and_commands_remain_distinct_without_accepting_arbitrary_input() {
        assert_eq!(find("cursor-agent").unwrap().id, "cursor");
        assert_eq!(find("cursor").unwrap().executable, "cursor-agent");
        assert_eq!(find("continue").unwrap().executable, "cn");
        assert_eq!(find("antigravity").unwrap().executable, "agy");
        assert_eq!(find("kiro").unwrap().args, &["chat", "--tui"]);
        assert!(find("cmd;evil").is_none());
        assert!(find("cn").is_none());
    }
}
