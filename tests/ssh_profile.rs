#[path = "../src/ssh_profile.rs"]
mod ssh_profile;

use ssh_profile::{Draft, Error, Profile, Source};

#[test]
fn profiles_match_the_vue_validation_and_argument_vectors() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ssh_profile_vue.json")).unwrap();
    for case in fixtures["profiles"].as_array().unwrap() {
        let profile: Profile = serde_json::from_value(case["input"].clone()).unwrap();
        match profile.checked() {
            Err(error) => assert_eq!(error.code(), case["error"].as_str().unwrap(), "{case}"),
            Ok(profile) => {
                assert!(case["error"].is_null(), "{case}");
                assert_eq!(profile.label, case["label"].as_str().unwrap(), "{case}");
                let mut expected: Vec<String> =
                    serde_json::from_value(case["args"].clone()).unwrap();
                expected.insert(expected.len() - 1, "--".into());
                assert_eq!(profile.argv().unwrap(), expected, "{case}");
            }
        }
    }
}

#[test]
fn pasted_endpoints_match_the_vue_parser_vectors() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ssh_profile_vue.json")).unwrap();
    for case in fixtures["inputs"].as_array().unwrap() {
        let raw = case["input"].as_str().unwrap();
        let parsed = ssh_profile::parse_input(raw);
        let expected = &case["expected"];
        if expected.is_null() {
            assert_eq!(parsed, None, "{raw}");
        } else {
            let parsed = parsed.unwrap();
            assert_eq!(parsed.host, expected["host"].as_str().unwrap(), "{raw}");
            assert_eq!(
                parsed.username.as_deref(),
                expected["username"].as_str(),
                "{raw}"
            );
            assert_eq!(
                parsed.port.map(u64::from),
                expected["port"].as_u64(),
                "{raw}"
            );
            assert_eq!(
                parsed.invalid_port,
                expected["invalidPort"].as_bool().unwrap_or(false),
                "{raw}"
            );
        }
    }
}

#[test]
fn editing_a_config_profile_pins_the_displayed_endpoint_and_drops_a_changed_alias() {
    let imported = Profile {
        host: "fe80::1%eth0".into(),
        config_host: "production".into(),
        source: Source::SshConfig,
        port: 2222,
        username: "deploy".into(),
        identity_file: "C:\\keys\\my key".into(),
        ..Default::default()
    }
    .checked()
    .unwrap();
    assert_eq!(imported.argv().unwrap(), ["--", "production"]);
    let mut draft = Draft::from_profile(&imported);
    let edited = draft.profile().unwrap();
    assert_eq!(edited.source, Source::Manual);
    assert_eq!(
        edited.argv().unwrap(),
        [
            "-o",
            "HostName=fe80::1%%eth0",
            "-p",
            "2222",
            "-l",
            "deploy",
            "-i",
            "C:\\keys\\my key",
            "--",
            "production"
        ]
    );
    draft.host = "other.example".into();
    draft.identity_file.clear();
    let changed = draft.profile().unwrap();
    assert_eq!(changed.config_host, "other.example");
    assert_eq!(
        changed.argv().unwrap(),
        ["-p", "2222", "-l", "deploy", "--", "other.example"]
    );
    let restored: Profile =
        serde_json::from_str(&serde_json::to_string(&changed).unwrap()).unwrap();
    assert_eq!(restored, changed);
}

#[test]
fn form_precedence_and_invalid_ports_never_silently_choose_another_endpoint() {
    let mut draft = Draft {
        host: "ssh://deploy@server:2222".into(),
        ..Default::default()
    };
    assert_eq!(draft.profile().unwrap().port, 2222);
    draft.port = "22".into();
    assert_eq!(draft.profile().unwrap().port, 2222);
    draft.port = "2022".into();
    draft.username = "explicit".into();
    let p = draft.profile().unwrap();
    assert_eq!((p.port, p.username.as_str()), (2022, "explicit"));
    for endpoint in [
        "server:0",
        "server:65536",
        "server:no",
        "ssh://server:65536",
        "[::1]:-1",
    ] {
        draft.host = endpoint.into();
        assert_eq!(draft.profile(), Err(Error::PortInvalid), "{endpoint}");
    }
    draft.host = "server".into();
    for raw in ["0", "65536", "1.5", "1e2", "-1", "+22"] {
        draft.port = raw.into();
        assert_eq!(draft.profile(), Err(Error::PortInvalid), "{raw}");
    }
}

#[test]
fn all_options_remain_separate_arguments_and_unsafe_values_are_refused_after_deserialization() {
    let profile = Profile {
        host: "server".into(),
        identity_file: "C:\\keys\\test key".into(),
        proxy_command: "cloudflared access ssh --hostname %h".into(),
        jump_host: "user@bastion:2222".into(),
        ..Default::default()
    };
    assert_eq!(
        profile.argv().unwrap(),
        [
            "-i",
            "C:\\keys\\test key",
            "-J",
            "user@bastion:2222",
            "-o",
            "ProxyCommand=cloudflared access ssh --hostname %h",
            "--",
            "server"
        ]
    );
    for value in ["-option", "host\ncommand", "a;b", "a b", "$(x)", "`x`"] {
        let mut p = profile.clone();
        p.host = value.into();
        let loaded: Profile = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert!(loaded.argv().is_err(), "{value}");
    }
}
