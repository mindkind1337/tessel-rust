//! Exercise the vendored style extension through its public element and input APIs.
use gpui::{
    AppContext, Context, Entity, InteractiveElement, ParentElement, Render, Styled, TestAppContext,
    TextRun, Window, div, px,
};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, text_area};
use std::sync::Arc;

fn run(len: usize) -> TextRun {
    TextRun {
        len,
        font: gpui::font("Arial"),
        color: gpui::rgb_to_hsla(gpui::rgb(0xffffff)),
        ..Default::default()
    }
}

#[test]
fn styled_input_rejects_partial_unicode_overflow_and_incomplete_runs() {
    let valid = |source: &str, runs: Vec<TextRun>| {
        text_area("validation")
            .styled_runs(source.to_owned().into(), runs.into())
            .is_ok()
    };
    assert!(valid("é😀x", vec![run(2), run(4), run(1)]));
    assert!(!valid("é😀x", vec![run(1), run(6)]));
    assert!(!valid("é😀x", vec![run(2), run(4)]));
    assert!(!valid("é😀x", vec![run(8)]));
    assert!(!valid("é😀x", vec![run(0), run(7)]));
    assert!(!valid("x", vec![run(1), run(usize::MAX)]));
    assert!(valid("", vec![]));
}

struct Fixture {
    input: Entity<EditableTextState>,
    spacing: f32,
}
impl Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        let mut style = run(2);
        style.letter_spacing = Some(px(self.spacing));
        div().size_full().flex().items_start().child(
            text_area("styled-input")
                .debug_selector(|| "styled-input".to_owned())
                .state(self.input.downgrade())
                .styled_runs("ab".into(), Arc::from([style]))
                .unwrap()
                .font_family("Arial")
                .text_size(px(16.))
                .w_auto()
                .flex_shrink_0()
                .h(px(100.)),
        )
    }
}

#[gpui::test]
fn styled_input_refreshes_layout_for_styles_and_ignores_stale_same_length_source(
    cx: &mut TestAppContext,
) {
    let view = cx.add_window(|_, cx| Fixture {
        input: cx.new(|cx| EditableTextState::new(StringStorage::from("ab"), cx)),
        spacing: 0.,
    });
    let draw_width = |cx: &mut TestAppContext| {
        cx.refresh().unwrap();
        cx.run_until_parked();
        gpui::VisualTestContext::from_window(view.into(), cx)
            .debug_bounds("styled-input")
            .unwrap()
            .size
            .width
    };
    let plain = draw_width(cx);
    view.update(cx, |fixture, _, cx| {
        fixture.spacing = 10.;
        cx.notify();
    })
    .unwrap();
    let spaced = draw_width(cx);
    assert!(
        spaced > plain,
        "style-only update must reshape the document"
    );
    view.update(cx, |fixture, _, cx| {
        fixture
            .input
            .update(cx, |input, cx| input.emplace("cd", cx))
    })
    .unwrap();
    let stale = draw_width(cx);
    assert!(
        stale < spaced,
        "same-length replacement must not retain old style runs"
    );
}
