pub(crate) fn text_diff(name: &str, old: &str, current: &str) -> String {
    similar::TextDiff::configure()
        .timeout(std::time::Duration::from_millis(500))
        .diff_lines(old, current)
        .unified_diff()
        .header(&format!("a/{name}"), &format!("b/{name}"))
        .to_string()
}

/// Index lines once on the comparison worker so each renderer only lays out the visible rows.
#[derive(Debug)]
pub(crate) struct Patch {
    text: String,
    starts: Vec<usize>,
}
impl Patch {
    pub fn new(text: String) -> Self {
        let mut offset = 0;
        let starts = text
            .split_inclusive('\n')
            .map(|line| {
                let start = offset;
                offset += line.len();
                start
            })
            .collect();
        Self { text, starts }
    }
    pub fn line_count(&self) -> usize {
        self.starts.len()
    }
    pub fn line(&self, index: usize) -> &str {
        self.text[self.starts[index]
            ..self
                .starts
                .get(index + 1)
                .copied()
                .unwrap_or(self.text.len())]
            .trim_end_matches(['\r', '\n'])
    }
}
