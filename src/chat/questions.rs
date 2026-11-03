use crate::i18n::tr;
use eframe::egui;
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize)]
struct Choice {
    label: String,
    description: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Question {
    id: String,
    header: String,
    question: String,
    #[serde(default)]
    is_other: bool,
    #[serde(default)]
    is_secret: bool,
    options: Option<Vec<Choice>>,
    #[serde(skip)]
    selected: Option<usize>,
    #[serde(skip)]
    text: String,
}
pub struct Request {
    pub id: Value,
    questions: Vec<Question>,
}
impl Request {
    pub fn parse(id: Value, params: &Value) -> Result<Self, String> {
        let questions: Vec<Question> = serde_json::from_value(params["questions"].clone())
            .map_err(|_| "Invalid agent questions")?;
        if questions.is_empty() || questions.len() > 3 {
            return Err("Agent may ask one to three questions".into());
        }
        let mut ids = std::collections::HashSet::new();
        for q in &questions {
            if q.is_secret {
                return Err(
                    "Secret questions are unsupported; use the site's sign-in interface".into(),
                );
            }
            if q.id.is_empty()
                || q.id.len() > 128
                || q.id.chars().any(char::is_control)
                || !ids.insert(&q.id)
                || q.header.len() > 128
                || q.question.is_empty()
                || q.question.len() > 4096
            {
                return Err("Invalid agent question fields".into());
            }
            if let Some(opts) = &q.options
                && (opts.is_empty()
                    || opts.len() > 10
                    || opts.iter().any(|o| {
                        o.label.is_empty() || o.label.len() > 512 || o.description.len() > 4096
                    }))
            {
                return Err("Invalid agent question options".into());
            }
        }
        Ok(Self { id, questions })
    }
    fn answers(&self) -> Option<Value> {
        let mut answers = serde_json::Map::new();
        for q in &self.questions {
            let value = if let Some(options) = &q.options {
                match q.selected {
                    Some(i) if i < options.len() => options[i].label.as_str(),
                    Some(i) if i == options.len() && q.is_other => q.text.trim(),
                    _ => return None,
                }
            } else {
                q.text.trim()
            };
            if value.is_empty() || value.len() > 16384 {
                return None;
            }
            answers.insert(q.id.clone(), json!({"answers":[value]}));
        }
        Some(json!({"answers":answers}))
    }
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Value> {
        let mut response = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.strong(tr("Questions from the agent"));
            egui::ScrollArea::vertical()
                .id_salt("agent-questions")
                .max_height(300.0)
                .show(ui, |ui| {
                    for q in &mut self.questions {
                        ui.push_id(&q.id, |ui| {
                            ui.strong(&q.header);
                            ui.label(&q.question);
                            if let Some(options) = &q.options {
                                for (i, o) in options.iter().enumerate() {
                                    ui.radio_value(&mut q.selected, Some(i), &o.label)
                                        .on_hover_text(&o.description);
                                }
                                if q.is_other {
                                    ui.radio_value(
                                        &mut q.selected,
                                        Some(options.len()),
                                        tr("Your answer"),
                                    );
                                }
                            }
                            if q.options.is_none() || q.is_other {
                                let edited = ui
                                    .add(
                                        egui::TextEdit::multiline(&mut q.text)
                                            .desired_rows(2)
                                            .char_limit(4096),
                                    )
                                    .changed();
                                if edited && let Some(options) = &q.options {
                                    q.selected = Some(options.len());
                                }
                            }
                            ui.separator();
                        });
                    }
                });
            ui.horizontal(|ui| {
                let answers = self.answers();
                if ui
                    .add_enabled(answers.is_some(), egui::Button::new(tr("Send answers")))
                    .clicked()
                {
                    response = answers;
                }
                if ui.button(tr("Skip questions")).clicked() {
                    response = Some(json!({"answers":{}}));
                }
            });
        });
        response
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn questions_require_explicit_choices_and_keep_answer_ids() {
        let params = json!({"questions":[{"id":"first","header":"Choice","question":"Pick?","options":[{"label":"One","description":"First"}],"isOther":true},{"id":"second","header":"Text","question":"Details?"}]});
        let mut request = Request::parse(json!(12), &params).unwrap();
        assert!(request.answers().is_none());
        request.questions[0].selected = Some(0);
        assert!(request.answers().is_none());
        request.questions[1].text = "Typed answer".into();
        assert_eq!(
            request.answers().unwrap(),
            json!({"answers":{"first":{"answers":["One"]},"second":{"answers":["Typed answer"]}}})
        );
        request.questions[0].selected = Some(1);
        assert!(request.answers().is_none());
        request.questions[0].text = "Other text".into();
        assert_eq!(
            request.answers().unwrap()["answers"]["first"]["answers"][0],
            "Other text"
        );
    }
    #[test]
    fn malformed_duplicate_and_secret_questions_are_refused() {
        let q = json!({"id":"q","header":"H","question":"Q"});
        assert!(Request::parse(json!(1), &json!({"questions":[q.clone(),q]})).is_err());
        assert!(
            Request::parse(
                json!(1),
                &json!({"questions":[{"id":"q","header":"H","question":"Q","isSecret":true}]})
            )
            .is_err()
        );
        assert!(Request::parse(json!(1), &json!({"questions":[]})).is_err());
    }
}
