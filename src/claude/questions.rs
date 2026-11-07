use crate::i18n::tr;
use eframe::egui;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Choice {
    label: String,
    #[serde(default)]
    description: String,
    #[serde(skip)]
    selected: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Question {
    question: String,
    header: String,
    #[serde(default)]
    multi_select: bool,
    #[serde(default)]
    is_secret: bool,
    options: Vec<Choice>,
    #[serde(skip)]
    other: String,
}
pub(super) struct Questions {
    pub id: String,
    original: Value,
    questions: Vec<Question>,
}
impl Questions {
    pub fn views(&self) -> Vec<crate::chat::QuestionView> {
        self.questions
            .iter()
            .enumerate()
            .map(|(idx, q)| crate::chat::QuestionView {
                id: format!("claude-q-{idx}"),
                header: q.header.clone(),
                question: q.question.clone(),
                multi_select: q.multi_select,
                is_other: true,
                options: q
                    .options
                    .iter()
                    .map(|o| (o.label.clone(), o.description.clone(), o.selected))
                    .collect(),
                text: q.other.clone(),
            })
            .collect()
    }

    pub fn select_option(&mut self, q_idx: usize, opt_idx: usize) {
        let Some(q) = self.questions.get_mut(q_idx) else {
            return;
        };
        if q.multi_select {
            if let Some(opt) = q.options.get_mut(opt_idx) {
                opt.selected = !opt.selected;
            }
        } else {
            for (i, opt) in q.options.iter_mut().enumerate() {
                opt.selected = i == opt_idx;
            }
            q.other.clear();
        }
    }

    pub fn set_text(&mut self, q_idx: usize, text: String) {
        let Some(q) = self.questions.get_mut(q_idx) else {
            return;
        };
        q.other = text.chars().take(4096).collect();
        if !q.multi_select && !q.other.trim().is_empty() {
            for opt in &mut q.options {
                opt.selected = false;
            }
        }
    }

    pub fn can_submit(&self) -> bool {
        self.answer().is_some()
    }

    pub fn build_answer(&self) -> Option<Value> {
        self.answer()
    }

    pub fn parse(id: &str, input: &Value) -> Option<Self> {
        let questions: Vec<Question> = serde_json::from_value(input["questions"].clone()).ok()?;
        if questions.is_empty() || questions.len() > 4 || input.to_string().len() > 65536 {
            return None;
        }
        let mut names = std::collections::HashSet::new();
        for q in &questions {
            if q.is_secret
                || q.question.is_empty()
                || q.question.len() > 4096
                || q.header.len() > 128
                || !names.insert(&q.question)
                || q.options.is_empty()
                || q.options.len() > 10
                || q.options.iter().any(|o| {
                    o.label.is_empty() || o.label.len() > 512 || o.description.len() > 4096
                })
            {
                return None;
            }
        }
        Some(Self {
            id: id.into(),
            original: input.clone(),
            questions,
        })
    }
    fn answer(&self) -> Option<Value> {
        let mut answers = serde_json::Map::new();
        for q in &self.questions {
            let mut values: Vec<&str> = q
                .options
                .iter()
                .filter(|o| o.selected)
                .map(|o| o.label.as_str())
                .collect();
            let other = q.other.trim();
            if other.len() > 16384 {
                return None;
            }
            if !other.is_empty() {
                values.push(other);
            }
            if values.is_empty() || (!q.multi_select && values.len() != 1) {
                return None;
            }
            answers.insert(q.question.clone(), json!(values.join(", ")));
        }
        let mut updated = self.original.clone();
        updated["answers"] = Value::Object(answers);
        Some(json!({"behavior":"allow","updatedInput":updated}))
    }
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Value> {
        let mut response = None;
        ui.group(|ui| {
            ui.strong(tr("Questions from the agent"));
            egui::ScrollArea::vertical()
                .id_salt("claude-questions")
                .max_height(300.0)
                .show(ui, |ui| {
                    for (index, q) in self.questions.iter_mut().enumerate() {
                        ui.push_id(index, |ui| {
                            ui.strong(&q.header);
                            ui.label(&q.question);
                            let mut selected = None;
                            for (i, option) in q.options.iter_mut().enumerate() {
                                if q.multi_select {
                                    ui.checkbox(&mut option.selected, &option.label)
                                        .on_hover_text(&option.description);
                                } else if ui
                                    .radio(option.selected, &option.label)
                                    .on_hover_text(&option.description)
                                    .clicked()
                                {
                                    selected = Some(i);
                                }
                            }
                            if let Some(i) = selected {
                                for (j, o) in q.options.iter_mut().enumerate() {
                                    o.selected = j == i;
                                }
                                q.other.clear();
                            }
                            ui.label(tr("Your answer"));
                            if ui
                                .add(
                                    egui::TextEdit::multiline(&mut q.other)
                                        .desired_rows(2)
                                        .char_limit(4096),
                                )
                                .changed()
                                && !q.multi_select
                            {
                                for o in &mut q.options {
                                    o.selected = false;
                                }
                            }
                            ui.separator();
                        });
                    }
                });
            ui.horizontal(|ui| {
                let answer = self.answer();
                if ui
                    .add_enabled(answer.is_some(), egui::Button::new(tr("Send answers")))
                    .clicked()
                {
                    response = answer;
                }
                if ui.button(tr("Skip questions")).clicked() {
                    response = Some(
                        json!({"behavior":"deny","message":"The owner skipped these questions."}),
                    );
                }
            });
        });
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> Value {
        json!({"questions":[{"question":"Which format?","header":"Format","multiSelect":true,"options":[{"label":"PDF"},{"label":"HTML"}]}]})
    }
    #[test]
    fn explicit_multi_choice_and_free_text_preserve_original_input() {
        let original = input();
        let mut request = Questions::parse("q", &original).unwrap();
        assert!(request.answer().is_none());
        request.questions[0].options[0].selected = true;
        request.questions[0].options[1].selected = true;
        request.questions[0].other = "Also text".into();
        let answer = request.answer().unwrap();
        assert_eq!(answer["updatedInput"]["questions"], original["questions"]);
        assert_eq!(
            answer["updatedInput"]["answers"]["Which format?"],
            "PDF, HTML, Also text"
        );
        request.questions[0].multi_select = false;
        assert!(request.answer().is_none());
    }
    #[test]
    fn duplicate_secret_and_oversized_questions_are_rejected() {
        let mut value = input();
        value["questions"][0]["isSecret"] = json!(true);
        assert!(Questions::parse("q", &value).is_none());
        let mut value = input();
        let first = value["questions"][0].clone();
        value["questions"].as_array_mut().unwrap().push(first);
        assert!(Questions::parse("q", &value).is_none());
        assert!(Questions::parse("q", &json!({"questions":[]})).is_none());
    }
}
