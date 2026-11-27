use super::{Book, Paragraph, Section};
use calamine::Reader;
use quick_xml::{Reader as Xml, events::Event};
use std::{
    collections::HashMap,
    io::{Cursor, Read},
};
const TEXT_LIMIT: usize = 2 * 1024 * 1024;
fn archive(bytes: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>, String> {
    let z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "Invalid Office archive")?;
    if z.len() > 20000 {
        return Err("Office archive has too many entries".into());
    }
    Ok(z)
}
fn xml(z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<String, String> {
    let f = z
        .by_name(name)
        .map_err(|_| "Office document is missing required content")?;
    if f.size() > 8 * 1024 * 1024 {
        return Err("Office XML exceeds 8 MiB".into());
    }
    let mut bytes = Vec::new();
    f.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read Office content")?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Office XML exceeds 8 MiB".into());
    }
    let source = String::from_utf8(bytes).map_err(|_| "Office XML must be UTF-8")?;
    validate_xml(&source)?;
    Ok(source)
}
fn validate_xml(source: &str) -> Result<(), String> {
    let mut reader = Xml::from_str(source);
    let mut depth = 0usize;
    let mut roots = 0usize;
    for _ in 0..500_000 {
        let event = reader.read_event().map_err(|_| "Invalid Office XML")?;
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                if depth == 0 {
                    roots += 1;
                }
                if !empty {
                    depth += 1;
                }
                if depth > 128 || roots > 1 {
                    return Err("Invalid Office XML structure".into());
                }
                for attribute in e.attributes() {
                    attribute.map_err(|_| "Invalid Office XML attribute")?;
                }
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).ok_or("Invalid Office XML structure")?;
            }
            Event::DocType(_) => return Err("Office XML DTDs are refused".into()),
            Event::Eof => {
                return if depth == 0 && roots == 1 {
                    Ok(())
                } else {
                    Err("Incomplete Office XML".into())
                };
            }
            Event::Text(e) if depth == 0 && !e.as_ref().trim().is_empty() => {
                return Err("Text outside Office XML root".into());
            }
            _ => {}
        }
    }
    Err("Office XML complexity limit reached".into())
}
fn attribute(e: &quick_xml::events::BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
}
fn text_page(source: &str, title: String, odf: bool) -> Result<Section, String> {
    let mut reader = Xml::from_str(source);
    let mut text = String::new();
    let mut paragraphs = Vec::new();
    let mut capture = 0usize;
    let mut heading = false;
    let mut total = 0usize;
    let mut events = 0usize;
    loop {
        events += 1;
        if events > 500000 {
            return Err("Office XML complexity limit reached".into());
        }
        let event = reader.read_event().map_err(|_| "Invalid Office XML")?;
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let n = e.name();
                let n = n.as_ref();
                if matches!(n, "w:p" | "a:p" | "text:p" | "text:h") {
                    if !text.trim().is_empty() {
                        paragraphs.push(Paragraph {
                            heading,
                            text: std::mem::take(&mut text),
                        });
                    }
                    heading = n == "text:h";
                }
                if !empty && (matches!(n, "w:t" | "a:t") || odf && matches!(n, "text:p" | "text:h"))
                {
                    capture += 1;
                }
                if n == "w:pStyle"
                    && attribute(&e, "w:val")
                        .is_some_and(|v| v.to_lowercase().starts_with("heading"))
                {
                    heading = true;
                }
                if matches!(n, "w:tab" | "text:tab") {
                    text.push('\t');
                    total += 1;
                }
                if matches!(n, "w:br" | "a:br" | "text:line-break") {
                    text.push('\n');
                    total += 1;
                }
                if n == "text:s" {
                    let count = attribute(&e, "text:c")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(1000);
                    text.extend(std::iter::repeat_n(' ', count));
                    total += count;
                }
            }
            Event::End(e) => {
                let n = e.name();
                let n = n.as_ref();
                if matches!(n, "w:t" | "a:t") || odf && matches!(n, "text:p" | "text:h") {
                    capture = capture.saturating_sub(1);
                }
                if matches!(n, "w:p" | "a:p" | "text:p" | "text:h") && !text.trim().is_empty() {
                    paragraphs.push(Paragraph {
                        heading,
                        text: std::mem::take(&mut text),
                    });
                    heading = false;
                }
            }
            Event::Text(e) if capture > 0 => {
                let decoded = e.xml10_content();
                text.push_str(&decoded);
                total += decoded.len();
            }
            Event::GeneralRef(e) if capture > 0 => {
                let decoded = e.xml10_content();
                let entity = format!("&{decoded};");
                let value = quick_xml::escape::unescape(&entity)
                    .map_err(|_| "Unsupported Office entity")?;
                text.push_str(&value);
                total += value.len();
            }
            Event::CData(e) if capture > 0 => {
                let decoded = e.xml10_content();
                text.push_str(&decoded);
                total += decoded.len();
            }
            Event::DocType(_) => return Err("Office XML declarations with DTDs are refused".into()),
            Event::Eof => break,
            _ => {}
        }
        if total > TEXT_LIMIT || text.len() > TEXT_LIMIT || paragraphs.len() > 20000 {
            return Err("Office text preview limit reached".into());
        }
    }
    if !text.trim().is_empty() {
        paragraphs.push(Paragraph { heading, text });
    }
    Ok(Section {
        title,
        paragraphs,
        rows: vec![],
        omitted_rows: 0,
        omitted_cols: 0,
        start_row: 0,
        start_col: 0,
    })
}
// Excel shows fifteen significant digits. Use ordinary decimal notation in
// the same exponent range as JavaScript's Number.toString in the Vue reader.
fn number_text(value: f64) -> String {
    if !value.is_finite() {
        return String::new();
    }
    if value == 0. {
        return "0".into();
    }
    let decimal = format!("{value:.14e}");
    let (mantissa, exponent) = decimal.split_once('e').expect("scientific notation");
    let exponent: i32 = exponent.parse().expect("decimal exponent");
    if (-6..21).contains(&exponent) {
        decimal
            .parse::<f64>()
            .expect("formatted number")
            .to_string()
    } else {
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        format!("{mantissa}e{exponent:+}")
    }
}

fn sheet_cell_text(cell: &calamine::Data, date1904: bool, ods: bool) -> String {
    use calamine::Data;
    match cell {
        Data::DateTime(date) => {
            let serial = date.as_f64();
            let epoch_offset = if date1904 { 24107. } else { 25569. };
            let millis = (serial - epoch_offset) * 86_400_000.;
            if !millis.is_finite() {
                return serial.to_string();
            }
            let Some(utc) = chrono::DateTime::from_timestamp_millis((millis + 0.5).floor() as i64)
            else {
                return serial.to_string();
            };
            let time = utc.format("%H:%M:%S").to_string();
            if (0.0..1.).contains(&serial) {
                time
            } else if time == "00:00:00" {
                utc.format("%Y-%m-%d").to_string()
            } else {
                format!("{} {time}", utc.format("%Y-%m-%d"))
            }
        }
        Data::DateTimeIso(value) if ods => value.replace('T', " "),
        Data::Bool(true) => "TRUE".into(),
        Data::Bool(false) => "FALSE".into(),
        Data::Float(value) => number_text(*value),
        Data::Int(value) => number_text(*value as f64),
        other => other.to_string(),
    }
}
pub(super) fn parse(bytes: &[u8], extension: &str) -> Result<Book, String> {
    let mut book = Book {
        sections: vec![],
        truncated: false,
    };
    if super::is_workbook(extension) {
        let mut workbook = calamine::open_workbook_auto_from_rs(Cursor::new(bytes))
            .map_err(|_| "Cannot decode spreadsheet")?;
        let date1904 = matches!(&workbook, calamine::Sheets::Xlsx(xlsx) if xlsx.has_1904_epoch());
        let names = workbook.sheet_names().to_vec();
        if names.len() > 200 {
            book.truncated = true;
        }
        let mut cells = 0;
        let mut text_bytes = 0;
        let sheet_count = names.len().min(200);
        for name in names.into_iter().take(200) {
            let range = workbook
                .worksheet_range(&name)
                .map_err(|_| "Cannot read spreadsheet sheet")?;
            let (start_row, start_col) = range.start().unwrap_or((0, 0));
            use crate::rich_previews::{MAX_SHEET_COLS, MAX_SHEET_ROWS};
            let (height, width) = range
                .end()
                .map_or((0, 0), |(r, c)| (r as usize + 1, c as usize + 1));
            let mut section = Section {
                title: name,
                paragraphs: vec![],
                rows: vec![],
                start_row: 0,
                start_col: 0,
                omitted_rows: height.saturating_sub(MAX_SHEET_ROWS),
                omitted_cols: width.saturating_sub(MAX_SHEET_COLS),
            };
            // Store blank positions only up to the last displayed value in each row.
            // A sparse sheet beginning at D5 still displays A1 through C4.
            let start_row = start_row as usize;
            let start_col = start_col as usize;
            'rows: for (r, row) in range
                .rows()
                .take(MAX_SHEET_ROWS.saturating_sub(start_row))
                .enumerate()
            {
                for (c, cell) in row
                    .iter()
                    .take(MAX_SHEET_COLS.saturating_sub(start_col))
                    .enumerate()
                {
                    if cell == &calamine::Data::Empty {
                        continue;
                    }
                    let mut value = sheet_cell_text(cell, date1904, extension == "ods");
                    if value.is_empty() {
                        continue;
                    }
                    if value.len() > 4096 {
                        book.truncated = true;
                        value = value.chars().take(1024).collect();
                    }
                    let row_index = start_row + r;
                    let col_index = start_col + c;
                    let prev = section.rows.get(row_index).map_or(0, Vec::len);
                    let added = col_index + 1 - prev;
                    if cells + added > 100000 || text_bytes + value.len() > TEXT_LIMIT {
                        book.truncated = true;
                        break 'rows;
                    }
                    if row_index >= section.rows.len() {
                        section.rows.resize_with(row_index + 1, Vec::new);
                    }
                    let cells_in_row = &mut section.rows[row_index];
                    cells_in_row.resize(col_index, String::new());
                    cells_in_row.push(value);
                    cells += added;
                    text_bytes += cells_in_row[col_index].len();
                }
            }
            book.sections.push(section);
            if cells >= 100000 || text_bytes >= TEXT_LIMIT {
                book.truncated |= book.sections.len() < sheet_count;
                break;
            }
        }
    } else {
        let mut zip = archive(bytes)?;
        match extension {
            "docx" => book.sections.push(text_page(
                &xml(&mut zip, "word/document.xml")?,
                "Document".into(),
                false,
            )?),
            "odt" => book.sections.push(text_page(
                &xml(&mut zip, "content.xml")?,
                "Document".into(),
                true,
            )?),
            "pptx" => {
                let relationships = xml(&mut zip, "ppt/_rels/presentation.xml.rels")?;
                let mut reader = Xml::from_str(&relationships);
                let mut targets = HashMap::new();
                loop {
                    match reader
                        .read_event()
                        .map_err(|_| "Invalid slide relationships")?
                    {
                        Event::Empty(e) | Event::Start(e)
                            if e.local_name().as_ref() == "Relationship" =>
                        {
                            if attribute(&e, "TargetMode").as_deref() != Some("External")
                                && let (Some(id), Some(target)) =
                                    (attribute(&e, "Id"), attribute(&e, "Target"))
                            {
                                targets.insert(id, target);
                            }
                        }
                        Event::DocType(_) => return Err("DTD refused".into()),
                        Event::Eof => break,
                        _ => {}
                    }
                }
                let presentation = xml(&mut zip, "ppt/presentation.xml")?;
                let mut reader = Xml::from_str(&presentation);
                loop {
                    match reader.read_event().map_err(|_| "Invalid presentation")? {
                        Event::Empty(e) | Event::Start(e) if e.local_name().as_ref() == "sldId" => {
                            if book.sections.len() == 200 {
                                book.truncated = true;
                                break;
                            }
                            let target = attribute(&e, "r:id")
                                .and_then(|id| targets.get(&id))
                                .ok_or("Missing slide relationship")?;
                            let path = if target.starts_with("/ppt/slides/") {
                                target.trim_start_matches('/').to_string()
                            } else {
                                format!("ppt/{target}")
                            };
                            if !path.starts_with("ppt/slides/")
                                || path.contains("..")
                                || path.contains('\\')
                            {
                                return Err("Unsupported slide path".into());
                            }
                            let title = format!("Slide {}", book.sections.len() + 1);
                            book.sections
                                .push(text_page(&xml(&mut zip, &path)?, title, false)?);
                        }
                        Event::DocType(_) => return Err("DTD refused".into()),
                        Event::Eof => break,
                        _ => {}
                    }
                }
            }
            "odp" => {
                let content = xml(&mut zip, "content.xml")?;
                let mut reader = Xml::from_str(&content);
                let mut start = None;
                let mut title = String::new();
                let mut offset = 0;
                loop {
                    match reader
                        .read_event()
                        .map_err(|_| "Invalid OpenDocument presentation")?
                    {
                        Event::Start(e) if e.name().as_ref() == "draw:page" => {
                            start = Some(offset);
                            title = attribute(&e, "draw:name")
                                .unwrap_or_else(|| format!("Slide {}", book.sections.len() + 1));
                        }
                        Event::End(e) if e.name().as_ref() == "draw:page" => {
                            if let Some(start) = start.take() {
                                let end = reader.buffer_position() as usize;
                                book.sections.push(text_page(
                                    &content[start..end],
                                    title.clone(),
                                    true,
                                )?);
                                if book.sections.len() == 200 {
                                    book.truncated = true;
                                    break;
                                }
                            }
                        }
                        Event::DocType(_) => return Err("DTD refused".into()),
                        Event::Eof => break,
                        _ => {}
                    }
                    offset = reader.buffer_position() as usize;
                }
            }
            _ => return Err("Unsupported Office format".into()),
        }
    }
    if book.sections.is_empty() && !super::is_workbook(extension) {
        return Err("No readable Office content found".into());
    }
    book.validate(extension)?;
    Ok(book)
}
