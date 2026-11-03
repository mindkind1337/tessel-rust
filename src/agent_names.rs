//! Native counterpart of src/shared/agentNames.js: persistent human addresses.
use std::collections::HashSet;

pub const AGENT_NAMES: &[&str] = &[
    "Ada",
    "Bohr",
    "Curie",
    "Darwin",
    "Euler",
    "Fermi",
    "Gauss",
    "Hopper",
    "Kepler",
    "Lovelace",
    "Noether",
    "Turing",
    "Archimedes",
    "Babbage",
    "Bell",
    "Bose",
    "Boyle",
    "Brahe",
    "Carson",
    "Cavendish",
    "Chadwick",
    "Clarke",
    "Copernicus",
    "Crick",
    "Dalton",
    "Dirac",
    "Edison",
    "Einstein",
    "Euclid",
    "Faraday",
    "Feynman",
    "Fibonacci",
    "Fleming",
    "Franklin",
    "Galileo",
    "Goodall",
    "Halley",
    "Hamilton",
    "Hardy",
    "Hawking",
    "Herschel",
    "Hilbert",
    "Hubble",
    "Hypatia",
    "Joule",
    "Katherine",
    "Lagrange",
    "Lamarck",
    "Laplace",
    "Leibniz",
    "Lise",
    "Lorentz",
    "Maxwell",
    "Mendel",
    "Newton",
    "Nobel",
    "Ohm",
    "Pascal",
    "Pasteur",
    "Planck",
    "Poincare",
    "Ramanujan",
    "Rosalind",
    "Rutherford",
    "Sagan",
    "Shannon",
    "Tesla",
    "Vera",
    "Volta",
    "Watt",
    "Wu",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedPane {
    pub id: u64,
    pub number: u64,
    pub named: bool,
    pub name: Option<String>,
}

pub(crate) fn js_space(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

pub(crate) fn js_trim(value: &str) -> &str {
    value.trim_matches(js_space)
}

pub fn name_key(value: &str) -> String {
    js_trim(value).to_lowercase()
}

pub fn valid_agent_name(value: &str) -> bool {
    let trimmed = js_trim(value);
    !trimmed.is_empty() && trimmed.encode_utf16().count() <= 60
        && !value.chars().any(|c| matches!(c, '\u{0000}'..='\u{001f}' | '\u{007f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '<' | '>' | '/' | '\\'))
        && !trimmed.starts_with(['#', '@'])
        && !trimmed.bytes().all(|b| b.is_ascii_digit())
        && !matches!(name_key(value).as_str(), "team" | "all" | "tessel")
}

pub fn next_agent_name(used: &HashSet<String>) -> String {
    for suffix in 1..=used.len() + 1 {
        for base in AGENT_NAMES {
            let candidate = if suffix == 1 {
                (*base).to_owned()
            } else {
                format!("{base} {suffix}")
            };
            if !used.contains(&name_key(&candidate)) {
                return candidate;
            }
        }
    }
    unreachable!("more candidate names than used keys")
}

pub fn ensure_agent_names(panes: &mut [NamedPane]) {
    let mut used = HashSet::new();
    let mut missing = Vec::new();
    for (i, pane) in panes.iter_mut().enumerate() {
        if !pane.named {
            pane.name = None;
            continue;
        }
        match pane.name.as_deref() {
            Some(name) if valid_agent_name(name) && used.insert(name_key(name)) => {}
            _ => missing.push(i),
        }
    }
    for i in missing {
        let name = next_agent_name(&used);
        used.insert(name_key(&name));
        panes[i].name = Some(name);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameError {
    Invalid,
    Taken,
}

pub fn rename_agent_name(panes: &mut [NamedPane], id: u64, name: &str) -> Result<(), RenameError> {
    if !valid_agent_name(name) || !panes.iter().any(|p| p.id == id && p.named) {
        return Err(RenameError::Invalid);
    }
    let key = name_key(name);
    if panes
        .iter()
        .any(|p| p.id != id && p.named && p.name.as_deref().is_some_and(|n| name_key(n) == key))
    {
        return Err(RenameError::Taken);
    }
    panes
        .iter_mut()
        .find(|p| p.id == id && p.named)
        .unwrap()
        .name = Some(js_trim(name).into());
    Ok(())
}

pub fn resolve_agent_address<'a>(panes: &'a [NamedPane], value: &str) -> Option<&'a NamedPane> {
    let address = js_trim(value);
    let legacy = address
        .strip_prefix('#')
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
    let mut matches = panes.iter().filter(|p| match legacy {
        Some(number) => number.parse::<u64>().ok() == Some(p.number),
        None => name_key(p.name.as_deref().unwrap_or_default()) == name_key(address),
    });
    let first = matches.next()?;
    if matches.next().is_some() {
        None
    } else {
        Some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_rules_preserve_custom_names_fill_gaps_and_reject_ambiguous_addresses() {
        let mut panes = vec![
            NamedPane {
                id: 1,
                number: 1,
                named: true,
                name: Some("  Custom Name  ".into()),
            },
            NamedPane {
                id: 2,
                number: 2,
                named: true,
                name: Some("custom name".into()),
            },
            NamedPane {
                id: 3,
                number: 3,
                named: false,
                name: Some("removed".into()),
            },
            NamedPane {
                id: 4,
                number: 4,
                named: true,
                name: None,
            },
        ];
        ensure_agent_names(&mut panes);
        assert_eq!(panes[0].name.as_deref(), Some("  Custom Name  "));
        assert_eq!(panes[1].name.as_deref(), Some("Ada"));
        assert!(panes[2].name.is_none());
        assert_eq!(panes[3].name.as_deref(), Some("Bohr"));
        assert_eq!(resolve_agent_address(&panes, "#02").unwrap().id, 2);
        assert_eq!(resolve_agent_address(&panes, " ADA ").unwrap().id, 2);
        assert_eq!(
            rename_agent_name(&mut panes, 4, "ada"),
            Err(RenameError::Taken)
        );
        assert_eq!(
            rename_agent_name(&mut panes, 3, "valid"),
            Err(RenameError::Invalid)
        );
        rename_agent_name(&mut panes, 4, "  Hopper  ").unwrap();
        assert_eq!(panes[3].name.as_deref(), Some("Hopper"));
        panes[3].name = Some("Ada".into());
        assert!(resolve_agent_address(&panes, "ada").is_none());
        let used: HashSet<_> = AGENT_NAMES.iter().map(|s| name_key(s)).collect();
        assert_eq!(next_agent_name(&used), "Ada 2");
    }
    #[test]
    fn validation_matches_source_utf16_limits_reserved_names_and_bidi_controls() {
        for bad in [
            "",
            "  ",
            "#1",
            "@team",
            "123",
            "TeAm",
            "ALL",
            "tessel",
            "one/two",
            "one\\two",
            "<name>",
            "bad\nline",
            "bad\u{202e}name",
            "bad\u{2066}name",
        ] {
            assert!(!valid_agent_name(bad), "{bad:?}");
        }
        assert!(valid_agent_name(&"x".repeat(60)));
        assert!(!valid_agent_name(&"x".repeat(61)));
        assert!(valid_agent_name(&"\u{1f680}".repeat(30)));
        assert!(!valid_agent_name(&"\u{1f680}".repeat(31)));
        assert_eq!(name_key("\u{feff} Ada \u{feff}"), "ada");
        assert!(valid_agent_name("Agent 2"));
    }
    #[test]
    fn differential_reference_cases_match_the_vue_source() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/agent_names.json")).unwrap();
        let names: Vec<_> = fixture["names"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(AGENT_NAMES, names);
        for case in fixture["cases"].as_array().unwrap() {
            let value = case["value"].as_str().unwrap();
            assert_eq!(
                valid_agent_name(value),
                case["valid"].as_bool().unwrap(),
                "{value:?}"
            );
            assert_eq!(name_key(value), case["key"].as_str().unwrap(), "{value:?}");
        }
    }
}
