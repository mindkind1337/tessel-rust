#[path = "../src/share/guest_text.rs"]
mod guest_text;

#[test]
fn synthetic_guest_text_matches_reference_vectors() {
    #[derive(serde::Deserialize)]
    struct Case {
        input: String,
        roots: Vec<String>,
        max: usize,
        expected: String,
    }
    #[derive(serde::Deserialize)]
    struct Fixture {
        cases: Vec<Case>,
    }
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/share_guest_text_reference.json")).unwrap();
    let mut failures = Vec::new();
    for (index, case) in fixture.cases.iter().enumerate() {
        let roots: Vec<_> = case.roots.iter().map(String::as_str).collect();
        let actual = guest_text::guest_text(&case.input, &roots, case.max);
        if actual != case.expected {
            failures.push((index, actual, case.expected.clone()));
        }
    }
    assert!(failures.is_empty(), "reference differences: {failures:#?}");
}

#[test]
fn guest_titles_are_bounded_unicode_safe_and_strip_spoofing_controls() {
    let huge = "secret=\"".to_owned() + &"synthetic value ".repeat(80_000);
    let output = guest_text::task_title(&huge, "C:/Proj/app", 300);
    assert_eq!(output, "secret=\"••••\"");
    let output = guest_text::task_title(
        "left\u{202e}right\u{feff}\nnext\u{2028}end",
        "C:/Proj/app",
        300,
    );
    assert_eq!(output, "leftright next end");
    assert_eq!(guest_text::guest_text("😀😀😀😀", &[], 3), "😀😀…");
    assert_eq!(guest_text::guest_text("secret=x", &[], 300), "secret=••••");
    assert_eq!(guest_text::guest_text("/private/data", &["/"], 300), "…");
    assert_eq!(
        guest_text::guest_text("C:/private/data", &["C:/"], 300),
        "…"
    );
    assert_eq!(guest_text::guest_text("", &[], 0), "");
    assert_eq!(
        guest_text::guest_text(r"C:\Proj\app\src\a.rs", &[r"\\?\C:\Proj\app"], 300),
        "src/a.rs"
    );
}

#[test]
fn adversarial_titles_hide_encoded_credentials_and_ambiguous_paths() {
    for input in [
        "https://user:hello%20world@host/page",
        "https://user%3Ahello%20world@host/page",
        "https://user%253Ahello%2520world@host/page",
    ] {
        assert_eq!(
            guest_text::task_title(input, "C:/Proj/app", 300),
            "https://••••@host/page"
        );
    }
    for input in [
        r#""C:\Users\jean's file\secret.txt""#,
        r"x1:////srv/share/f",
        r"voir \Users\jean\.ssh\id_rsa",
    ] {
        let output = guest_text::task_title(input, "C:/Proj/app", 300);
        assert!(
            !output.contains("secret") && !output.contains("jean") && !output.contains("srv"),
            "{output}"
        );
    }
    assert_eq!(
        guest_text::task_title(r"a\nb 50\100", "C:/Proj/app", 300),
        r"a\nb 50\100"
    );
    assert_eq!(
        guest_text::task_title(r"\d\w \s*\(", "C:/Proj/app", 300),
        r"\d\w \s*\("
    );
    for input in [
        r"\Documents and Settings\jean\secret",
        r"\AppData\private\secret",
    ] {
        assert_eq!(guest_text::task_title(input, "C:/Proj/app", 300), "…");
    }
    assert_eq!(
        guest_text::task_title("ftp://a:p%40ss@h", "C:/Proj/app", 300),
        "…"
    );
    assert_eq!(
        guest_text::task_title(r"\d+\w*\s", "C:/Proj/app", 300),
        r"\d+\w*\s"
    );
    let prefix = format!("C:/private/{} ", "x".repeat(250));
    let title = format!(
        "{prefix}{} sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789",
        "x".repeat(540 - prefix.len())
    );
    let output = guest_text::task_title(&title, "C:/Proj/app", 300);
    assert!(
        !output.contains("sk-ant") && !output.contains("api03"),
        "{output}"
    );
}

#[test]
fn guest_input_is_plain_text_without_decoding_or_masking_the_authors_words() {
    assert_eq!(
        guest_text::clean_task_input(" \x1b[31mhello\x1b[0m\u{202e}\nworld "),
        Some("hello world".into())
    );
    assert_eq!(
        guest_text::clean_task_input("100%20 token=fixture"),
        Some("100%20 token=fixture".into())
    );
    assert!(guest_text::clean_task_input("\u{202e}\u{feff}\x1b[0m").is_none());
    assert!(guest_text::clean_task_input(&"a".repeat(301)).is_none());
}

#[test]
fn encoded_named_values_do_not_expose_words_after_decoding() {
    for (input, expected) in [
        ("password=hello%20world", "password=••••"),
        ("password=hello%2520world", "password=••••"),
        ("password=%22hello world%22", "password=\"••••\""),
        ("password=%2522hello world%2522", "password=\"••••\""),
        ("token=%27hello world%27", "token='••••'"),
    ] {
        assert_eq!(
            guest_text::task_title(input, "C:/Proj/app", 300),
            expected,
            "{input}"
        );
    }
}
