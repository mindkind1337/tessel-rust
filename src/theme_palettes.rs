//! Exact UI, window title-bar, and terminal colour palettes transcribed directly from:
//! - `src/renderer/src/style.css` (`:root` Classic tokens)
//! - `src/renderer/src/themes.css` (`:root[data-theme='warp']` tokens)
//! - `src/renderer/src/themes.js` (`THEMES`, `TERMINAL_THEMES`, `terminalTheme`)
//! - `src/shared/themePalettes.js` (`PALETTE_THEMES`, `paletteVars`, `titleBarColors`)
//! - `src/renderer/src/i18n/locales/fr/settings.json` (`settings.themes.*`)

use eframe::egui::Color32;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(test, allow(dead_code))] // Standalone palette fixtures also include this module.
pub enum FileIconStyle {
    #[default]
    Color,
    Mono,
    Off,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThemeId {
    #[default]
    #[serde(rename = "classic")]
    Classic,
    #[serde(rename = "warp")]
    Warp,
    #[serde(rename = "dracula")]
    Dracula,
    #[serde(rename = "nord")]
    Nord,
    #[serde(rename = "tokyo-night")]
    TokyoNight,
    #[serde(rename = "gruvbox")]
    Gruvbox,
    #[serde(rename = "solarized-dark")]
    SolarizedDark,
}

pub const THEMES: [ThemeId; 7] = [
    ThemeId::Classic,
    ThemeId::Warp,
    ThemeId::Dracula,
    ThemeId::Nord,
    ThemeId::TokyoNight,
    ThemeId::Gruvbox,
    ThemeId::SolarizedDark,
];

impl ThemeId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Warp => "warp",
            Self::Dracula => "dracula",
            Self::Nord => "nord",
            Self::TokyoNight => "tokyo-night",
            Self::Gruvbox => "gruvbox",
            Self::SolarizedDark => "solarized-dark",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "classic" => Some(Self::Classic),
            "warp" => Some(Self::Warp),
            "dracula" => Some(Self::Dracula),
            "nord" => Some(Self::Nord),
            "tokyo-night" => Some(Self::TokyoNight),
            "gruvbox" => Some(Self::Gruvbox),
            "solarized-dark" => Some(Self::SolarizedDark),
            _ => None,
        }
    }

    /// Theme display name in English or French (`themes.js` + `locales/fr/settings.json`).
    /// Named schemes (`Dracula`, `Nord`, `Tokyo Night`, `Gruvbox`, `Solarized Dark`) keep
    /// their proper name in every language.
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Classic, false) => "Classic",
            (Self::Classic, true) => "Classique",
            (Self::Warp, false) => "Warp-inspired",
            (Self::Warp, true) => "Inspiré de Warp",
            (Self::Dracula, _) => "Dracula",
            (Self::Nord, _) => "Nord",
            (Self::TokyoNight, _) => "Tokyo Night",
            (Self::Gruvbox, _) => "Gruvbox",
            (Self::SolarizedDark, _) => "Solarized Dark",
        }
    }

    pub const fn description(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Classic, false) => "Original Tessel appearance",
            (Self::Classic, true) => "L'apparence d'origine de Tessel",
            (Self::Warp, false) => "Graphite surfaces and soft green accents",
            (Self::Warp, true) => "Surfaces graphite et accents vert doux",
            (Self::Dracula, false) => "Dark purple with vivid colours",
            (Self::Dracula, true) => "Violet sombre aux couleurs vives",
            (Self::Nord, false) => "Arctic blue-grey, calm and low-contrast",
            (Self::Nord, true) => "Gris-bleu arctique, calme et peu contrasté",
            (Self::TokyoNight, false) => "Deep navy with neon accents",
            (Self::TokyoNight, true) => "Bleu nuit profond aux accents néon",
            (Self::Gruvbox, false) => "Warm retro browns and oranges",
            (Self::Gruvbox, true) => "Bruns et oranges rétro chaleureux",
            (Self::SolarizedDark, false) => "The classic teal, easy on the eyes",
            (Self::SolarizedDark, true) => "Le classique bleu-vert, reposant pour les yeux",
        }
    }

    pub const fn is_named_palette(self) -> bool {
        !matches!(self, Self::Classic | Self::Warp)
    }

    pub const fn spec(self) -> &'static ThemeSpec {
        match self {
            Self::Classic => &CLASSIC_SPEC,
            Self::Warp => &WARP_SPEC,
            Self::Dracula => &DRACULA_SPEC,
            Self::Nord => &NORD_SPEC,
            Self::TokyoNight => &TOKYO_NIGHT_SPEC,
            Self::Gruvbox => &GRUVBOX_SPEC,
            Self::SolarizedDark => &SOLARIZED_DARK_SPEC,
        }
    }
}

pub fn is_theme(id: &str) -> bool {
    ThemeId::from_id(id).is_some()
}

pub fn resolve_theme_id(id: &str) -> ThemeId {
    ThemeId::from_id(id).unwrap_or(ThemeId::Classic)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiPalette {
    pub chrome: Color32,
    pub surface: Color32,
    pub surface_2: Color32,
    pub surface_3: Color32,
    pub term: Color32,
    pub backdrop: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub accent: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub text: Color32,
    pub text_strong: Color32,
    pub text_dim: Color32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitleBarColors {
    pub color: Color32,
    pub symbol_color: Color32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnsiPalette {
    pub black: Color32,
    pub red: Color32,
    pub green: Color32,
    pub yellow: Color32,
    pub blue: Color32,
    pub magenta: Color32,
    pub cyan: Color32,
    pub white: Color32,
    pub bright_black: Color32,
    pub bright_red: Color32,
    pub bright_green: Color32,
    pub bright_yellow: Color32,
    pub bright_blue: Color32,
    pub bright_magenta: Color32,
    pub bright_cyan: Color32,
    pub bright_white: Color32,
}

impl AnsiPalette {
    pub const fn as_array(&self) -> [Color32; 16] {
        [
            self.black,
            self.red,
            self.green,
            self.yellow,
            self.blue,
            self.magenta,
            self.cyan,
            self.white,
            self.bright_black,
            self.bright_red,
            self.bright_green,
            self.bright_yellow,
            self.bright_blue,
            self.bright_magenta,
            self.bright_cyan,
            self.bright_white,
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalPalette {
    pub background: Color32,
    pub foreground: Color32,
    pub cursor: Color32,
    pub cursor_accent: Color32,
    pub selection_background: Color32,
    /// `None` for `Classic` and `Warp-inspired` so terminal programs retain default ANSI semantics.
    pub ansi: Option<AnsiPalette>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeSpec {
    pub id: ThemeId,
    pub ui: UiPalette,
    pub term: TerminalPalette,
    pub title_bar: TitleBarColors,
    pub radius_px: f32,
}

pub fn title_bar_colors(id: &str) -> TitleBarColors {
    resolve_theme_id(id).spec().title_bar
}

pub fn terminal_theme(id: &str) -> TerminalPalette {
    resolve_theme_id(id).spec().term
}

pub const CLASSIC_SPEC: ThemeSpec = ThemeSpec {
    id: ThemeId::Classic,
    ui: UiPalette {
        chrome: rgb(0x10, 0x12, 0x16),
        surface: rgb(0x18, 0x1b, 0x21),
        surface_2: rgb(0x1f, 0x23, 0x2b),
        surface_3: rgb(0x28, 0x2d, 0x37),
        term: rgb(0x15, 0x17, 0x1c),
        backdrop: rgb(0x0c, 0x0d, 0x10),
        border: rgb(0x25, 0x29, 0x32),
        border_strong: rgb(0x33, 0x39, 0x46),
        accent: rgb(0x6c, 0x9c, 0xff),
        warn: rgb(0xf5, 0xa5, 0x24),
        danger: rgb(0xe5, 0x48, 0x4d),
        text: rgb(0xd6, 0xd9, 0xdf),
        text_strong: rgb(0xf1, 0xf3, 0xf6),
        text_dim: rgb(0x8a, 0x92, 0xa0),
    },
    term: TerminalPalette {
        background: rgb(0x15, 0x17, 0x1c),
        foreground: rgb(0xd6, 0xd9, 0xdf),
        cursor: rgb(0xe8, 0xea, 0xee),
        cursor_accent: rgb(0x15, 0x17, 0x1c),
        selection_background: rgb(0x2f, 0x4a, 0x7a),
        ansi: None,
    },
    title_bar: TitleBarColors {
        color: rgb(0x10, 0x12, 0x16),
        symbol_color: rgb(0xd6, 0xd9, 0xdf),
    },
    radius_px: 7.0,
};

/// Warp-inspired green status/working colour (`--green` / `--ok` in `themes.css`).
pub const WARP_OK_GREEN: Color32 = rgb(0xa1, 0xc8, 0xa7);

pub const WARP_SPEC: ThemeSpec = ThemeSpec {
    id: ThemeId::Warp,
    ui: UiPalette {
        chrome: rgb(0x16, 0x19, 0x17),
        surface: rgb(0x20, 0x23, 0x21),
        surface_2: rgb(0x25, 0x2b, 0x27),
        surface_3: rgb(0x2c, 0x39, 0x30),
        term: rgb(0x19, 0x1b, 0x1a),
        backdrop: rgb(0x16, 0x19, 0x17),
        border: rgb(0x30, 0x36, 0x31),
        border_strong: rgb(0x47, 0x54, 0x4a),
        accent: rgb(0xae, 0xd5, 0xb2),
        warn: rgb(0xdb, 0xba, 0x87),
        danger: rgb(0xe5, 0xa6, 0xa0),
        text: rgb(0xdf, 0xe5, 0xdf),
        text_strong: rgb(0xef, 0xf3, 0xed),
        text_dim: rgb(0x93, 0x9e, 0x94),
    },
    term: TerminalPalette {
        background: rgb(0x19, 0x1b, 0x1a),
        foreground: rgb(0xdf, 0xe5, 0xdf),
        cursor: rgb(0xae, 0xd5, 0xb2),
        cursor_accent: rgb(0x19, 0x1b, 0x1a),
        selection_background: rgb(0x3b, 0x53, 0x42),
        ansi: None,
    },
    title_bar: TitleBarColors {
        color: rgb(0x16, 0x19, 0x17),
        symbol_color: rgb(0xdf, 0xe5, 0xdf),
    },
    radius_px: 5.0,
};

pub const DRACULA_SPEC: ThemeSpec = ThemeSpec {
    id: ThemeId::Dracula,
    ui: UiPalette {
        chrome: rgb(0x1e, 0x1f, 0x29),
        surface: rgb(0x21, 0x22, 0x2c),
        surface_2: rgb(0x28, 0x2a, 0x36),
        surface_3: rgb(0x34, 0x37, 0x46),
        term: rgb(0x28, 0x2a, 0x36),
        backdrop: rgb(0x1e, 0x1f, 0x29),
        border: rgb(0x34, 0x37, 0x46),
        border_strong: rgb(0x44, 0x47, 0x5a),
        accent: rgb(0xbd, 0x93, 0xf9),
        warn: rgb(0xff, 0xb8, 0x6c),
        danger: rgb(0xff, 0x55, 0x55),
        text: rgb(0xf8, 0xf8, 0xf2),
        text_strong: rgb(0xff, 0xff, 0xff),
        text_dim: rgb(0x8b, 0x8f, 0xa8),
    },
    term: TerminalPalette {
        background: rgb(0x28, 0x2a, 0x36),
        foreground: rgb(0xf8, 0xf8, 0xf2),
        cursor: rgb(0xf8, 0xf8, 0xf2),
        cursor_accent: rgb(0x28, 0x2a, 0x36),
        selection_background: rgb(0x44, 0x47, 0x5a),
        ansi: Some(AnsiPalette {
            black: rgb(0x21, 0x22, 0x2c),
            red: rgb(0xff, 0x55, 0x55),
            green: rgb(0x50, 0xfa, 0x7b),
            yellow: rgb(0xf1, 0xfa, 0x8c),
            blue: rgb(0xbd, 0x93, 0xf9),
            magenta: rgb(0xff, 0x79, 0xc6),
            cyan: rgb(0x8b, 0xe9, 0xfd),
            white: rgb(0xf8, 0xf8, 0xf2),
            bright_black: rgb(0x62, 0x72, 0xa4),
            bright_red: rgb(0xff, 0x6e, 0x6e),
            bright_green: rgb(0x69, 0xff, 0x94),
            bright_yellow: rgb(0xff, 0xff, 0xa5),
            bright_blue: rgb(0xd6, 0xac, 0xff),
            bright_magenta: rgb(0xff, 0x92, 0xdf),
            bright_cyan: rgb(0xa4, 0xff, 0xff),
            bright_white: rgb(0xff, 0xff, 0xff),
        }),
    },
    title_bar: TitleBarColors {
        color: rgb(0x1e, 0x1f, 0x29),
        symbol_color: rgb(0xf8, 0xf8, 0xf2),
    },
    radius_px: 7.0,
};

pub const NORD_SPEC: ThemeSpec = ThemeSpec {
    id: ThemeId::Nord,
    ui: UiPalette {
        chrome: rgb(0x24, 0x29, 0x33),
        surface: rgb(0x2b, 0x30, 0x3b),
        surface_2: rgb(0x2e, 0x34, 0x40),
        surface_3: rgb(0x3b, 0x42, 0x52),
        term: rgb(0x2e, 0x34, 0x40),
        backdrop: rgb(0x24, 0x29, 0x33),
        border: rgb(0x3b, 0x42, 0x52),
        border_strong: rgb(0x4c, 0x56, 0x6a),
        accent: rgb(0x88, 0xc0, 0xd0),
        warn: rgb(0xeb, 0xcb, 0x8b),
        danger: rgb(0xbf, 0x61, 0x6a),
        text: rgb(0xd8, 0xde, 0xe9),
        text_strong: rgb(0xec, 0xef, 0xf4),
        text_dim: rgb(0x8a, 0x93, 0xa6),
    },
    term: TerminalPalette {
        background: rgb(0x2e, 0x34, 0x40),
        foreground: rgb(0xd8, 0xde, 0xe9),
        cursor: rgb(0xd8, 0xde, 0xe9),
        cursor_accent: rgb(0x2e, 0x34, 0x40),
        selection_background: rgb(0x43, 0x4c, 0x5e),
        ansi: Some(AnsiPalette {
            black: rgb(0x3b, 0x42, 0x52),
            red: rgb(0xbf, 0x61, 0x6a),
            green: rgb(0xa3, 0xbe, 0x8c),
            yellow: rgb(0xeb, 0xcb, 0x8b),
            blue: rgb(0x81, 0xa1, 0xc1),
            magenta: rgb(0xb4, 0x8e, 0xad),
            cyan: rgb(0x88, 0xc0, 0xd0),
            white: rgb(0xe5, 0xe9, 0xf0),
            bright_black: rgb(0x4c, 0x56, 0x6a),
            bright_red: rgb(0xbf, 0x61, 0x6a),
            bright_green: rgb(0xa3, 0xbe, 0x8c),
            bright_yellow: rgb(0xeb, 0xcb, 0x8b),
            bright_blue: rgb(0x81, 0xa1, 0xc1),
            bright_magenta: rgb(0xb4, 0x8e, 0xad),
            bright_cyan: rgb(0x8f, 0xbc, 0xbb),
            bright_white: rgb(0xec, 0xef, 0xf4),
        }),
    },
    title_bar: TitleBarColors {
        color: rgb(0x24, 0x29, 0x33),
        symbol_color: rgb(0xd8, 0xde, 0xe9),
    },
    radius_px: 7.0,
};

pub const TOKYO_NIGHT_SPEC: ThemeSpec = ThemeSpec {
    id: ThemeId::TokyoNight,
    ui: UiPalette {
        chrome: rgb(0x16, 0x16, 0x1e),
        surface: rgb(0x1a, 0x1b, 0x26),
        surface_2: rgb(0x1f, 0x23, 0x35),
        surface_3: rgb(0x29, 0x2e, 0x42),
        term: rgb(0x1a, 0x1b, 0x26),
        backdrop: rgb(0x16, 0x16, 0x1e),
        border: rgb(0x29, 0x2e, 0x42),
        border_strong: rgb(0x3b, 0x42, 0x61),
        accent: rgb(0x7a, 0xa2, 0xf7),
        warn: rgb(0xe0, 0xaf, 0x68),
        danger: rgb(0xf7, 0x76, 0x8e),
        text: rgb(0xc0, 0xca, 0xf5),
        text_strong: rgb(0xe0, 0xe6, 0xff),
        text_dim: rgb(0x73, 0x7a, 0xa2),
    },
    term: TerminalPalette {
        background: rgb(0x1a, 0x1b, 0x26),
        foreground: rgb(0xc0, 0xca, 0xf5),
        cursor: rgb(0xc0, 0xca, 0xf5),
        cursor_accent: rgb(0x1a, 0x1b, 0x26),
        selection_background: rgb(0x33, 0x46, 0x7c),
        ansi: Some(AnsiPalette {
            black: rgb(0x15, 0x16, 0x1e),
            red: rgb(0xf7, 0x76, 0x8e),
            green: rgb(0x9e, 0xce, 0x6a),
            yellow: rgb(0xe0, 0xaf, 0x68),
            blue: rgb(0x7a, 0xa2, 0xf7),
            magenta: rgb(0xbb, 0x9a, 0xf7),
            cyan: rgb(0x7d, 0xcf, 0xff),
            white: rgb(0xa9, 0xb1, 0xd6),
            bright_black: rgb(0x41, 0x48, 0x68),
            bright_red: rgb(0xf7, 0x76, 0x8e),
            bright_green: rgb(0x9e, 0xce, 0x6a),
            bright_yellow: rgb(0xe0, 0xaf, 0x68),
            bright_blue: rgb(0x7a, 0xa2, 0xf7),
            bright_magenta: rgb(0xbb, 0x9a, 0xf7),
            bright_cyan: rgb(0x7d, 0xcf, 0xff),
            bright_white: rgb(0xc0, 0xca, 0xf5),
        }),
    },
    title_bar: TitleBarColors {
        color: rgb(0x16, 0x16, 0x1e),
        symbol_color: rgb(0xc0, 0xca, 0xf5),
    },
    radius_px: 7.0,
};

pub const GRUVBOX_SPEC: ThemeSpec = ThemeSpec {
    id: ThemeId::Gruvbox,
    ui: UiPalette {
        chrome: rgb(0x1d, 0x20, 0x21),
        surface: rgb(0x24, 0x24, 0x24),
        surface_2: rgb(0x28, 0x28, 0x28),
        surface_3: rgb(0x3c, 0x38, 0x36),
        term: rgb(0x28, 0x28, 0x28),
        backdrop: rgb(0x1d, 0x20, 0x21),
        border: rgb(0x3c, 0x38, 0x36),
        border_strong: rgb(0x50, 0x49, 0x45),
        accent: rgb(0xfa, 0xbd, 0x2f),
        warn: rgb(0xfe, 0x80, 0x19),
        danger: rgb(0xfb, 0x49, 0x34),
        text: rgb(0xeb, 0xdb, 0xb2),
        text_strong: rgb(0xfb, 0xf1, 0xc7),
        text_dim: rgb(0xa8, 0x99, 0x84),
    },
    term: TerminalPalette {
        background: rgb(0x28, 0x28, 0x28),
        foreground: rgb(0xeb, 0xdb, 0xb2),
        cursor: rgb(0xeb, 0xdb, 0xb2),
        cursor_accent: rgb(0x28, 0x28, 0x28),
        selection_background: rgb(0x50, 0x49, 0x45),
        ansi: Some(AnsiPalette {
            black: rgb(0x28, 0x28, 0x28),
            red: rgb(0xcc, 0x24, 0x1d),
            green: rgb(0x98, 0x97, 0x1a),
            yellow: rgb(0xd7, 0x99, 0x21),
            blue: rgb(0x45, 0x85, 0x88),
            magenta: rgb(0xb1, 0x62, 0x86),
            cyan: rgb(0x68, 0x9d, 0x6a),
            white: rgb(0xa8, 0x99, 0x84),
            bright_black: rgb(0x92, 0x83, 0x74),
            bright_red: rgb(0xfb, 0x49, 0x34),
            bright_green: rgb(0xb8, 0xbb, 0x26),
            bright_yellow: rgb(0xfa, 0xbd, 0x2f),
            bright_blue: rgb(0x83, 0xa5, 0x98),
            bright_magenta: rgb(0xd3, 0x86, 0x9b),
            bright_cyan: rgb(0x8e, 0xc0, 0x7c),
            bright_white: rgb(0xeb, 0xdb, 0xb2),
        }),
    },
    title_bar: TitleBarColors {
        color: rgb(0x1d, 0x20, 0x21),
        symbol_color: rgb(0xeb, 0xdb, 0xb2),
    },
    radius_px: 7.0,
};

pub const SOLARIZED_DARK_SPEC: ThemeSpec = ThemeSpec {
    id: ThemeId::SolarizedDark,
    ui: UiPalette {
        chrome: rgb(0x00, 0x21, 0x2b),
        surface: rgb(0x00, 0x28, 0x33),
        surface_2: rgb(0x00, 0x2b, 0x36),
        surface_3: rgb(0x07, 0x36, 0x42),
        term: rgb(0x00, 0x2b, 0x36),
        backdrop: rgb(0x00, 0x21, 0x2b),
        border: rgb(0x07, 0x36, 0x42),
        border_strong: rgb(0x1b, 0x4a, 0x55),
        accent: rgb(0x26, 0x8b, 0xd2),
        warn: rgb(0xb5, 0x89, 0x00),
        danger: rgb(0xdc, 0x32, 0x2f),
        text: rgb(0x93, 0xa1, 0xa1),
        text_strong: rgb(0xee, 0xe8, 0xd5),
        text_dim: rgb(0x65, 0x7b, 0x83),
    },
    term: TerminalPalette {
        background: rgb(0x00, 0x2b, 0x36),
        foreground: rgb(0x83, 0x94, 0x96),
        cursor: rgb(0x93, 0xa1, 0xa1),
        cursor_accent: rgb(0x00, 0x2b, 0x36),
        selection_background: rgb(0x07, 0x36, 0x42),
        ansi: Some(AnsiPalette {
            black: rgb(0x07, 0x36, 0x42),
            red: rgb(0xdc, 0x32, 0x2f),
            green: rgb(0x85, 0x99, 0x00),
            yellow: rgb(0xb5, 0x89, 0x00),
            blue: rgb(0x26, 0x8b, 0xd2),
            magenta: rgb(0xd3, 0x36, 0x82),
            cyan: rgb(0x2a, 0xa1, 0x98),
            white: rgb(0xee, 0xe8, 0xd5),
            bright_black: rgb(0x58, 0x6e, 0x75),
            bright_red: rgb(0xcb, 0x4b, 0x16),
            bright_green: rgb(0x85, 0x99, 0x00),
            bright_yellow: rgb(0xb5, 0x89, 0x00),
            bright_blue: rgb(0x83, 0x94, 0x96),
            bright_magenta: rgb(0x6c, 0x71, 0xc4),
            bright_cyan: rgb(0x93, 0xa1, 0xa1),
            bright_white: rgb(0xfd, 0xf6, 0xe3),
        }),
    },
    title_bar: TitleBarColors {
        color: rgb(0x00, 0x21, 0x2b),
        symbol_color: rgb(0x93, 0xa1, 0xa1),
    },
    radius_px: 7.0,
};
