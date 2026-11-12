//! Incremental line-token cache for native text editing. Never changes source bytes.
use super::theme::{GpuiThemeTokens, mono_font_family};
use crate::{
    syntax::{SYNTAX_MAX_BYTES, SyntaxLang, SyntaxPalette, TokenKind, tokenize_spans},
    theme_palettes::ThemeId,
};
use gpui::{SharedString, TextRun};
use std::sync::Arc;

struct Line {
    text: Arc<str>,
    utf16_len: usize,
    tokens: Arc<[(usize, usize, TokenKind)]>,
}

#[derive(Clone)]
pub(crate) struct StyledSource {
    pub source: SharedString,
    pub runs: Arc<[TextRun]>,
    pub utf16_lines: Arc<[usize]>,
}

#[derive(Default)]
pub(crate) struct Cache {
    lines: Vec<Line>,
    lang: Option<SyntaxLang>,
    key: Option<(ThemeId, bool, bool, Vec<usize>)>,
    output: Option<StyledSource>,
}

impl Cache {
    pub fn update(
        &mut self,
        text: &str,
        lang: SyntaxLang,
        theme: ThemeId,
        dark: bool,
        diff: bool,
        matches: &[usize],
    ) -> StyledSource {
        let lang = if text.len() > SYNTAX_MAX_BYTES {
            SyntaxLang::Plain
        } else {
            lang
        };
        let match_set: std::collections::BTreeSet<_> = matches.iter().copied().collect();
        let key = (theme, dark, diff, match_set.iter().copied().collect());
        if self.lang == Some(lang)
            && self.key.as_ref() == Some(&key)
            && let Some(output) = &self.output
            && output.source.as_ref() == text
        {
            return output.clone();
        }
        let pieces: Vec<_> = text.split_inclusive('\n').collect();
        let mut prefix = 0;
        let mut suffix = 0;
        if self.lang == Some(lang) {
            while prefix < pieces.len()
                && prefix < self.lines.len()
                && pieces[prefix] == self.lines[prefix].text.as_ref()
            {
                prefix += 1;
            }
            while suffix < pieces.len() - prefix
                && suffix < self.lines.len() - prefix
                && pieces[pieces.len() - suffix - 1]
                    == self.lines[self.lines.len() - suffix - 1].text.as_ref()
            {
                suffix += 1;
            }
        }
        let changed = pieces[prefix..pieces.len() - suffix]
            .iter()
            .map(|line| Line {
                text: Arc::from(*line),
                utf16_len: line.encode_utf16().count(),
                tokens: tokenize_spans(line, lang).into(),
            })
            .collect::<Vec<_>>();
        self.lines
            .splice(prefix..self.lines.len() - suffix, changed);
        self.lang = Some(lang);
        self.key = Some(key);

        let palette = SyntaxPalette::for_theme_mode(theme, dark);
        let colors = GpuiThemeTokens::from_theme(theme);
        let rgba = |color: eframe::egui::Color32| {
            gpui::rgb_to_hsla(gpui::rgb(
                (u32::from(color.r()) << 16) | (u32::from(color.g()) << 8) | u32::from(color.b()),
            ))
        };
        let plain = rgba(palette.plain);
        let font = gpui::font(mono_font_family());
        let mut runs: Vec<TextRun> = Vec::new();
        let mut utf16_lines = vec![0];
        let mut offset = 0;
        for (index, line) in self.lines.iter().enumerate() {
            let diff_color = if diff && line.text.starts_with('+') && !line.text.starts_with("+++")
            {
                Some(colors.ok_green)
            } else if diff && line.text.starts_with('-') && !line.text.starts_with("---") {
                Some(colors.danger)
            } else if diff && line.text.starts_with("@@") {
                Some(colors.accent)
            } else {
                None
            };
            let background = if diff {
                diff_color
                    .map(|color| gpui::rgb_to_hsla(super::theme::rgba_with_alpha(color, 0.14)))
            } else {
                match_set
                    .contains(&index)
                    .then(|| gpui::rgb_to_hsla(super::theme::rgba_with_alpha(colors.warn, 0.18)))
            };
            let mut push = |len: usize, token: Option<TokenKind>| {
                if len == 0 {
                    return;
                }
                let mut face = font.clone();
                if token == Some(TokenKind::Comment) && !diff {
                    face = face.italic();
                }
                let mut run = TextRun {
                    len,
                    font: face,
                    color: diff_color.map(gpui::rgb_to_hsla).unwrap_or_else(|| {
                        if diff {
                            plain
                        } else {
                            token.map(|kind| rgba(palette.color(kind))).unwrap_or(plain)
                        }
                    }),
                    background_color: background,
                    ..Default::default()
                };
                if let Some(previous) = runs.last_mut() {
                    let previous_len = previous.len;
                    run.len = previous_len;
                    if *previous == run {
                        previous.len += len;
                        return;
                    }
                    run.len = len;
                }
                runs.push(run);
            };
            let mut cursor = 0;
            for &(start, end, token) in line.tokens.iter() {
                if start < cursor
                    || end <= start
                    || end > line.text.len()
                    || !line.text.is_char_boundary(start)
                    || !line.text.is_char_boundary(end)
                {
                    continue;
                }
                push(start - cursor, None);
                push(end - start, Some(token));
                cursor = end;
            }
            push(line.text.len() - cursor, None);
            offset += line.utf16_len;
            if line.text.ends_with('\n') {
                utf16_lines.push(offset);
            }
        }
        let output = StyledSource {
            source: text.to_owned().into(),
            runs: runs.into(),
            utf16_lines: utf16_lines.into(),
        };
        self.output = Some(output.clone());
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_editor_styles_reuse_prefix_suffix_after_line_edits() {
        let mut cache = Cache::default();
        let update = |cache: &mut Cache, text: &str| {
            cache.update(text, SyntaxLang::Rust, ThemeId::default(), true, false, &[])
        };
        update(&mut cache, "let a = 1;\nlet b = 2;\n// end\n");
        let prefix = cache.lines[0].tokens.clone();
        let suffix = cache.lines[2].tokens.clone();
        for text in [
            "let a = 1;\nlet b = 3;\n// end\n",
            "let a = 1;\nlet b = 3;\nlet c = 4;\n// end\n",
            "let a = 1;\n// end\n",
        ] {
            let styled = update(&mut cache, text);
            assert!(Arc::ptr_eq(&prefix, &cache.lines[0].tokens));
            assert!(Arc::ptr_eq(&suffix, &cache.lines.last().unwrap().tokens));
            assert_eq!(
                styled.runs.iter().map(|run| run.len).sum::<usize>(),
                text.len()
            );
            assert!(
                gpui_ce_elements::editable_text::text_area("line-edit")
                    .styled_runs(styled.source, styled.runs)
                    .is_ok()
            );
        }
    }

    #[test]
    fn native_editor_styles_preserve_unicode_crlf_and_reuse_unchanged_lines() {
        let mut cache = Cache::default();
        let first = cache.update(
            "let a = \"é😀\";\r\n// end\n",
            SyntaxLang::Rust,
            ThemeId::default(),
            true,
            false,
            &[],
        );
        assert_eq!(
            first.runs.iter().map(|run| run.len).sum::<usize>(),
            first.source.len()
        );
        assert_eq!(&*first.utf16_lines, &[0, 16, 23]);
        let suffix = cache.lines[1].tokens.clone();
        let same = cache.update(
            &first.source,
            SyntaxLang::Rust,
            ThemeId::default(),
            true,
            false,
            &[],
        );
        assert!(Arc::ptr_eq(&first.runs, &same.runs));
        let next = cache.update(
            "let b = 1;\r\n// end\n",
            SyntaxLang::Rust,
            ThemeId::default(),
            true,
            false,
            &[],
        );
        assert!(Arc::ptr_eq(&suffix, &cache.lines[1].tokens));
        assert!(!Arc::ptr_eq(&first.runs, &next.runs));
        assert!(
            gpui_ce_elements::editable_text::text_area("syntax-test")
                .styled_runs(next.source, next.runs)
                .is_ok()
        );
    }

    #[test]
    fn native_editor_styles_keep_large_source_complete_and_refresh_theme() {
        let mut cache = Cache::default();
        let text = "é\n".repeat(SYNTAX_MAX_BYTES / 2);
        let plain = cache.update(
            &text,
            SyntaxLang::Rust,
            ThemeId::default(),
            true,
            false,
            &[],
        );
        assert_eq!(plain.source.as_ref(), text);
        assert_eq!(plain.runs.len(), 1);
        assert_eq!(plain.runs[0].len, text.len());
        let recolored = cache.update(
            &text,
            SyntaxLang::Rust,
            ThemeId::default(),
            false,
            false,
            &[],
        );
        assert!(!Arc::ptr_eq(&plain.runs, &recolored.runs));
        assert_eq!(cache.lang, Some(SyntaxLang::Plain));
    }
}
