use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

pub(crate) enum Block<'a> {
    Prose(&'a str),
    Code { language: String, text: String },
}

/// CommonMark determines fence length, indentation and where an unfinished
/// block ends while a reply is streaming. Keep untouched prose for file links.
pub(crate) fn message_blocks<'a>(role: &str, source: &'a str) -> Vec<Block<'a>> {
    if !super::citations::assistant_role(role)
        || !(source.contains("```") || source.contains("~~~"))
    {
        return vec![Block::Prose(source)];
    }
    let mut parts = Vec::new();
    let mut end = 0;
    let mut events = Parser::new(source).into_offset_iter();
    while let Some((event, span)) = events.next() {
        let Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) = event else {
            continue;
        };
        // The block layout supplies the vertical gap between paragraphs and code.
        // Retaining the separator as prose adds empty lines to the link hit area.
        let prose = source[end..span.start].trim_end_matches(['\r', '\n']);
        let prose = if end == 0 {
            prose
        } else {
            prose.trim_start_matches(['\r', '\n'])
        };
        if !prose.is_empty() {
            parts.push(Block::Prose(prose));
        }
        let language = info.split_whitespace().next().unwrap_or("");
        let mut text = String::new();
        for (event, _) in events.by_ref() {
            match event {
                Event::Text(chunk) => text.push_str(&chunk),
                Event::End(TagEnd::CodeBlock) => break,
                _ => {}
            }
        }
        parts.push(Block::Code {
            language: language_label(language).to_owned(),
            text,
        });
        end = span.end;
    }
    if parts.is_empty() {
        parts.push(Block::Prose(source));
    } else {
        let prose = source[end..].trim_start_matches(['\r', '\n']);
        if !prose.is_empty() {
            parts.push(Block::Prose(prose));
        }
    }
    parts
}

fn language_label(language: &str) -> &str {
    match language {
        "bash" => "Bash",
        "c" => "C",
        "cpp" => "C++",
        "css" => "CSS",
        "diff" => "Diff",
        "go" => "Go",
        "graphql" => "GraphQL",
        "html" => "HTML",
        "java" => "Java",
        "javascript" => "JavaScript",
        "json" => "JSON",
        "kotlin" => "Kotlin",
        "markdown" => "Markdown",
        "mermaid" => "Mermaid",
        "python" => "Python",
        "ruby" => "Ruby",
        "rust" => "Rust",
        "scss" => "SCSS",
        "shell" => "Shell",
        "sql" => "SQL",
        "swift" => "Swift",
        "typescript" => "TypeScript",
        "xml" => "XML",
        "yaml" => "YAML",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fences_preserve_code_and_streaming_boundaries() {
        let source = "Intro\n~~~~rust example\n  é\n```\n\n~~~~\nNext\n```xyz\n  open";
        let parts = message_blocks("Claude", source);
        let [
            Block::Prose(before),
            Block::Code {
                language: first,
                text: body,
            },
            Block::Prose(after),
            Block::Code {
                language: second,
                text: tail,
            },
        ] = parts.as_slice()
        else {
            panic!("unexpected segments")
        };
        assert_eq!(*before, "Intro");
        assert_eq!(first, "Rust");
        assert_eq!(body, "  é\n```\n\n");
        assert_eq!(*after, "Next");
        assert_eq!(second, "xyz");
        assert_eq!(tail, "  open");
        assert!(matches!(
            message_blocks("You", source).as_slice(),
            [Block::Prose(..)]
        ));
    }
}
