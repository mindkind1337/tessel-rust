use gpui::{App, Div, ParentElement, Rgba, Styled, div, px};
use std::borrow::Cow;

pub(super) fn register(app: &App) {
    let _ = app.text_system().add_fonts(vec![Cow::Borrowed(
        include_bytes!("../../resources/codicons/codicon.ttf").as_slice(),
    )]);
}

// Monaco SymbolKind order, with the property glyph as its fallback.
const KINDS: &[(char, &str, &str)] = &[
    ('\u{eb60}', "file", "fichier"),
    ('\u{ea8b}', "module", "module"),
    ('\u{ea8b}', "namespace", "espace de noms"),
    ('\u{ea8b}', "package", "package"),
    ('\u{eb5b}', "class", "classe"),
    ('\u{ea8c}', "method", "méthode"),
    ('\u{eb65}', "property", "propriété"),
    ('\u{eb5f}', "field", "champ"),
    ('\u{ea8c}', "constructor", "constructeur"),
    ('\u{ea95}', "enumeration", "énumération"),
    ('\u{eb61}', "interface", "interface"),
    ('\u{ea8c}', "function", "fonction"),
    ('\u{ea88}', "variable", "variable"),
    ('\u{eb5d}', "constant", "constante"),
    ('\u{eb8d}', "string", "chaîne"),
    ('\u{ea90}', "number", "nombre"),
    ('\u{ea8f}', "boolean", "booléen"),
    ('\u{ea8a}', "array", "tableau"),
    ('\u{ea8b}', "object", "objet"),
    ('\u{ea93}', "key", "clé"),
    ('\u{ea8f}', "null", "null"),
    ('\u{eb5e}', "enumeration member", "membre d'énumération"),
    ('\u{ea91}', "struct", "struct"),
    ('\u{ea86}', "event", "événement"),
    ('\u{eb64}', "operator", "opérateur"),
    ('\u{ea92}', "type parameter", "paramètre de type"),
];

pub(super) fn symbol(kind: u8, french: bool, foreground: Rgba) -> (Div, &'static str) {
    let &(glyph, en, fr) = KINDS.get(usize::from(kind)).unwrap_or(&KINDS[6]);
    let color = match kind {
        4 | 9 | 23 => gpui::rgb(0xee9d28),
        5 | 8 | 11 => gpui::rgb(0xb180d7),
        7 | 10 | 12 | 21 => gpui::rgb(0x75beff),
        _ => foreground,
    };
    let icon = div()
        .size(px(16.))
        .flex_shrink_0()
        .font_family("codicon")
        .text_size(px(16.))
        .line_height(px(16.))
        .text_color(color)
        .child(glyph.to_string());
    (icon, if french { fr } else { en })
}
