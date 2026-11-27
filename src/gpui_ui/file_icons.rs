use super::theme::GpuiThemeTokens;
use crate::theme_palettes::FileIconStyle;
use gpui::{App, IntoElement, ParentElement, Rgba, Styled, div, prelude::FluentBuilder, px};
use serde::Deserialize;
use std::{borrow::Cow, collections::HashMap, sync::OnceLock};

const FONT: &[u8] = include_bytes!("../../resources/file-icons/seti.ttf");

pub(super) fn register(app: &App) {
    let _ = app.text_system().add_fonts(vec![Cow::Borrowed(FONT)]);
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    file: String,
    icons: HashMap<String, (u32, Option<String>, Option<String>)>,
    file_names: HashMap<String, String>,
    file_extensions: HashMap<String, String>,
    language_by_name: HashMap<String, String>,
    language_by_extension: HashMap<String, String>,
    language_ids: HashMap<String, String>,
}
fn catalog() -> &'static Catalog {
    static DATA: OnceLock<Catalog> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../../resources/file-icons/seti.json"))
            .expect("bundled Seti icon catalog")
    })
}
impl Catalog {
    fn id(&self, path: &str) -> &str {
        let name = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_lowercase();
        let allow_extensions = name.encode_utf16().count() <= 255;
        let extensions = || {
            name.match_indices('.')
                .filter(|_| allow_extensions)
                .map(|(i, _)| &name[i + 1..])
        };
        self.file_names
            .get(&name)
            .or_else(|| extensions().find_map(|ext| self.file_extensions.get(ext)))
            .or_else(|| {
                self.language_by_name
                    .get(&name)
                    .or_else(|| extensions().find_map(|ext| self.language_by_extension.get(ext)))
                    .and_then(|language| self.language_ids.get(language))
            })
            .filter(|id| self.icons.contains_key(*id))
            .unwrap_or(&self.file)
    }
}

pub(super) fn render(path: &str, tokens: &GpuiThemeTokens, fallback: Rgba) -> impl IntoElement {
    let slot = div()
        .size(px(16.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center();
    if tokens.file_icons == FileIconStyle::Off {
        return slot.hidden();
    }
    let catalog = catalog();
    let (code, dark, light) = &catalog.icons[catalog.id(path)];
    let background = tokens.id.spec().term.background;
    let luminance = 299 * u32::from(background.r())
        + 587 * u32::from(background.g())
        + 114 * u32::from(background.b());
    let color = if luminance >= 128000 { light } else { dark };
    let mono = tokens.file_icons == FileIconStyle::Mono;
    let tint = color
        .as_deref()
        .filter(|_| !mono)
        .and_then(|hex| u32::from_str_radix(hex.trim_start_matches('#'), 16).ok())
        .map(gpui::rgb)
        .unwrap_or(fallback);
    slot.font_family("seti")
        .text_size(px(18.))
        .line_height(px(18.))
        .text_color(tint)
        .when(mono, |el| el.opacity(0.85))
        .child(char::from_u32(*code).expect("valid Seti glyph").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seti_catalog_matches_filename_extension_and_language_precedence() {
        let c = catalog();
        for (path, id) in [
            ("C:/demo/tsconfig.json", &c.file_names["tsconfig.json"]),
            ("VITE.CONFIG.TS", &c.file_names["vite.config.ts"]),
            ("demo/app.spec.ts", &c.file_extensions["spec.ts"]),
            ("APP.VUE", &c.language_ids["vue"]),
            ("example.py", &c.language_ids["python"]),
            ("unknown.zzzz", &c.file),
        ] {
            assert_eq!(c.id(path), id);
        }
        assert_eq!(c.id(&format!("{}.vue", "x".repeat(253))), c.file);
        for id in c
            .file_names
            .values()
            .chain(c.file_extensions.values())
            .chain(c.language_ids.values())
        {
            assert!(c.icons.contains_key(id), "{id}");
        }
    }

    #[cfg(all(windows, feature = "visual-test"))]
    #[gpui::test]
    fn seti_glyphs_render_in_native_tabs_and_modes_persist(test: &mut gpui::TestAppContext) {
        use crate::{app::gpui_bridge::GpuiAction, files::Document};
        let platform = gpui_platform::current_platform(true);
        let cx = &mut gpui::TestAppContext::build_with_text_system(
            test.dispatcher.clone(),
            None,
            platform.text_system(),
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.rs");
        std::fs::write(&path, "hello").unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root =
                super::super::TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app
                .install_editor_input_fixture(vec![Document::open(dir.path(), &path).unwrap()], 0);
            let ts = cx.text_system();
            assert!(
                ts.all_font_names()
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case("seti"))
            );
            let font = ts.resolve_font(&gpui::font("seti"));
            for (id, (code, _, _)) in &catalog().icons {
                let bounds = ts
                    .typographic_bounds(font, px(18.), char::from_u32(*code).unwrap())
                    .unwrap();
                assert!(
                    bounds.size.width > px(0.) && bounds.size.height > px(0.),
                    "{id}"
                );
            }
            root
        });
        let width = |cx: &mut gpui::TestAppContext| {
            cx.refresh().unwrap();
            gpui::VisualTestContext::from_window(view.into(), cx)
                .debug_bounds("view-tab-1")
                .unwrap()
                .size
                .width
        };
        let colored = width(cx);
        for (style, value) in [(FileIconStyle::Off, "off"), (FileIconStyle::Mono, "mono")] {
            view.update(cx, |root, _, cx| {
                root.dispatch(GpuiAction::SetFileIcons(style), cx)
            })
            .unwrap();
            let actual = width(cx);
            if style == FileIconStyle::Off {
                assert!(colored >= actual + px(16.));
            } else {
                assert_eq!(actual, colored);
            }
            let saved: serde_json::Value = serde_json::from_slice(
                &std::fs::read(dir.path().join("state/workspace.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(saved["file_icons"], value);
        }
    }
}
