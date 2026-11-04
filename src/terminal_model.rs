//! Session-local observations from visible CLI footer rows; never inferred from Settings.
use crate::pane_session_model::{self as model, ModelSource, ShownAgentModel};
use serde_json::Value;

/// Observe the live screen even while the user reads scrollback; restore their position.
pub fn live_footer(parser: &mut vt100::Parser) -> Vec<String> {
    let offset = parser.screen().scrollback();
    parser.set_scrollback(0);
    let (rows, cols) = parser.screen().size();
    let lines = parser
        .screen()
        .rows(0, cols)
        .skip(rows.saturating_sub(12) as usize)
        .collect();
    parser.set_scrollback(offset);
    lines
}

pub fn supported(agent: &str) -> bool {
    matches!(agent, "codex" | "cursor" | "antigravity" | "muse")
}

pub fn observe(agent: &str, lines: &[String], models: &[Value]) -> Option<ShownAgentModel> {
    let count = match agent {
        "codex" => 12,
        "cursor" => 8,
        _ => 6,
    };
    let lines = &lines[lines.len().saturating_sub(count)..];
    let (id, name, effort) = match agent {
        "codex" => {
            let seen = model::codex_model_on_screen(lines, models)?;
            (seen.model, seen.name, seen.effort)
        }
        "cursor" => {
            let seen = model::cursor_model_on_screen(lines, models)?;
            (seen.model, Some(seen.name), seen.effort)
        }
        "antigravity" => {
            let seen = model::antigravity_model_on_screen(lines, models)?;
            (seen.model, seen.name, seen.effort)
        }
        "muse" => {
            let seen = model::muse_footer_model(lines)?;
            (seen.model, None, Some(seen.effort))
        }
        _ => return None,
    };
    Some(ShownAgentModel {
        model: id,
        name,
        effort,
        chosen_effort: None,
        source: ModelSource::Screen,
        at: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_footer_preserves_scrollback_without_observing_historical_models() {
        let mut parser = vt100::Parser::new(5, 100, 100);
        parser.process(b"old model\r\n1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n8\r\n9");
        parser.process("\x1b[3;1H\x1b[2K› Ask Codex anything\x1b[4;1H\x1b[2K\x1b[5;1H\x1b[2KGPT-5.6 Sol xhigh fast".as_bytes());
        parser.set_scrollback(5);
        let before = parser.screen().contents();
        let offset = parser.screen().scrollback();
        let lines = live_footer(&mut parser);
        let models = crate::agent_model_catalog::models_for("codex", None);
        assert_eq!(
            observe("codex", &lines, &models).unwrap().model,
            "gpt-5.6-sol"
        );
        assert_eq!(parser.screen().scrollback(), offset);
        assert_eq!(parser.screen().contents(), before);
    }
    #[test]
    fn footer_observation_requires_cli_structure_and_does_not_guess_from_prose() {
        let models = crate::agent_model_catalog::models_for("codex", None);
        let lines = vec![
            "› Ask Codex anything".into(),
            String::new(),
            "GPT-5.6 Sol xhigh fast · C:\\fixture".into(),
        ];
        let seen = observe("codex", &lines, &models).unwrap();
        assert_eq!(seen.model, "gpt-5.6-sol");
        assert_eq!(seen.effort.as_deref(), Some("xhigh"));
        assert_eq!(seen.source, ModelSource::Screen);
        assert!(
            observe(
                "codex",
                &["The answer mentions GPT-5.6 Sol xhigh".into()],
                &models
            )
            .is_none()
        );
        assert!(observe("claude", &lines, &models).is_none());
        let mut old_footer = lines;
        old_footer.extend(std::iter::repeat_n(String::new(), 12));
        assert!(observe("codex", &old_footer, &models).is_none());
    }
}
