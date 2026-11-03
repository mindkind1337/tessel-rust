//! Uses the installed Windows UI typeface without redistributing font files.
pub fn install(ctx: &eframe::egui::Context) {
    #[cfg(windows)]
    {
        use eframe::egui::{FontData, FontDefinitions, FontFamily};
        use std::{io::Read, path::PathBuf};
        let Some(windows) = std::env::var_os("WINDIR") else {
            return;
        };
        let path = PathBuf::from(windows).join("Fonts").join("segoeui.ttf");
        let Ok(file) = std::fs::File::open(path) else {
            return;
        };
        const MAX_FONT_BYTES: u64 = 8 * 1024 * 1024;
        let Ok(metadata) = file.metadata() else {
            return;
        };
        if metadata.len() == 0 || metadata.len() > MAX_FONT_BYTES {
            return;
        }
        let mut bytes = Vec::new();
        if file
            .take(MAX_FONT_BYTES + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.is_empty()
            || bytes.len() > MAX_FONT_BYTES as usize
        {
            return;
        }
        let mut fonts = FontDefinitions::default();
        fonts.font_data.insert(
            "windows-segoe-ui".into(),
            FontData::from_owned(bytes).into(),
        );
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "windows-segoe-ui".into());
        ctx.set_fonts(fonts);
    }
    #[cfg(not(windows))]
    let _ = ctx;
}
