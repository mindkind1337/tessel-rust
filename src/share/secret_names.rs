//! Pure secret-name policy matching the shipped Vue shareScope reference.
use std::sync::OnceLock;
pub(super) fn is_secret_name(name: &str) -> bool {
    let name = name.to_lowercase();
    if matches!(
        name.as_str(),
        ".npmrc"
            | ".pypirc"
            | ".netrc"
            | ".git-credentials"
            | ".aws"
            | ".ssh"
            | ".docker"
            | ".kube"
            | ".gnupg"
    ) {
        return true;
    }
    static EXAMPLE: OnceLock<regex::Regex> = OnceLock::new();
    let examples = EXAMPLE.get_or_init(|| {
        regex::Regex::new(r"^(\.env(\..+)?|.+\.env)\.(example|sample|template)$").unwrap()
    });
    if examples.is_match(&name) {
        return false;
    }
    static SECRET: OnceLock<regex::Regex> = OnceLock::new();
    SECRET.get_or_init(|| regex::Regex::new(r"(^\.env(\..+)?$|\.env$|\.(pem|key|p12|pfx|p8|ppk|jks|keystore|kdbx|tfvars)$|^id_(rsa|ed25519|ecdsa|dsa)|\.tfstate|^credentials.*\.json$|^service-account.*\.json$|^secrets?\..+$)").unwrap()).is_match(&name)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_match_shipped_vue_reference() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/secret_names_reference.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            assert_eq!(
                is_secret_name(name),
                case["secret"].as_bool().unwrap(),
                "{name:?}"
            );
        }
    }
}
