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
        start_row: 0,
        start_col: 0,
    })
}
pub(super) fn parse(bytes: &[u8], extension: &str) -> Result<Book, String> {
    let mut book = Book {
        sections: vec![],
        truncated: false,
    };
    if matches!(extension, "xlsx" | "xlsm" | "ods") {
        let mut workbook = calamine::open_workbook_auto_from_rs(Cursor::new(bytes))
            .map_err(|_| "Cannot decode spreadsheet")?;
        let names = workbook.sheet_names().to_vec();
        if names.len() > 200 {
            book.truncated = true;
        }
        let mut cells = 0;
        let mut text_bytes = 0;
        for name in names.into_iter().take(200) {
            let range = workbook
                .worksheet_range(&name)
                .map_err(|_| "Cannot read spreadsheet sheet")?;
            let (start_row, start_col) = range.start().unwrap_or((0, 0));
            let mut section = Section {
                title: name,
                paragraphs: vec![],
                rows: vec![],
                start_row,
                start_col,
            };
            if range.height() > 5000 || range.width() > 100 {
                book.truncated = true;
            }
            for row in range.rows().take(5000) {
                let mut values = Vec::new();
                for cell in row.iter().take(100) {
                    if cells >= 100000 || text_bytes >= TEXT_LIMIT {
                        book.truncated = true;
                        break;
                    }
                    let value = cell.to_string();
                    let value = if value.len() > 4096 {
                        book.truncated = true;
                        value.chars().take(1024).collect::<String>()
                    } else {
                        value
                    };
                    cells += 1;
                    text_bytes += value.len();
                    values.push(value);
                }
                if values.is_empty() {
                    break;
                }
                section.rows.push(values);
            }
            book.sections.push(section);
            if cells >= 100000 || text_bytes >= TEXT_LIMIT {
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
    if book.sections.is_empty() {
        return Err("No readable Office content found".into());
    }
    book.validate()?;
    Ok(book)
}
