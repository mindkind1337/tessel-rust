//! Runtime-local launch capabilities. Network arguments never establish project authority.
use crate::team_mcp_stdio::{BackendError, Operation};
use crate::team_mcp_wire::{decode, encode};
use crate::teams_relay::{
    SignedTeamRequest, TeamAuthRegistry, canonical_json, hex_decode_32, hex_encode, hmac_sha256,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

const REQUEST_DOMAIN: &str = "tessel-team-mcp-request-v1";
const RESPONSE_DOMAIN: &str = "tessel-team-mcp-response-v1";
const MAX_BINDINGS: usize = 128;
const MAX_AGE_MS: u64 = 30_000;
const MAX_SKEW_MS: u64 = 5_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub pane: u64,
    pub project: u64,
    pub cwd: PathBuf,
    pub generation: u64,
}

pub(crate) struct Credential {
    pane: u64,
    generation: u64,
    secret: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TeamCredential")
            .field("pane", &self.pane)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WireOperation {
    List,
    Call { name: String, arguments: Value },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestBody {
    domain: String,
    generation: u64,
    operation: WireOperation,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyBody {
    domain: String,
    generation: u64,
    request_nonce: String,
    request_digest: String,
    result: Result<Value, BackendError>,
}

pub(crate) struct VerifiedRequest {
    pub binding: Binding,
    pub operation: Operation,
    nonce: String,
    digest: String,
}

#[derive(Default)]
pub(crate) struct Authority {
    auth: TeamAuthRegistry,
    bindings: HashMap<u64, Binding>,
    next_generation: u64,
}

fn fresh(timestamp: u64, now: u64) -> Result<(), String> {
    if timestamp > now.saturating_add(MAX_SKEW_MS) || now.saturating_sub(timestamp) > MAX_AGE_MS {
        return Err("Team packet timestamp is outside the accepted window".into());
    }
    Ok(())
}

fn digest(request: &SignedTeamRequest) -> Result<String, String> {
    Ok(hex_encode(&Sha256::digest(encode(request)?)))
}

fn signed_bytes(request: &SignedTeamRequest) -> Vec<u8> {
    canonical_json(
        &json!({"pane_id":request.pane_id,"timestamp_ms":request.timestamp_ms,
        "nonce":request.nonce,"payload":request.payload}),
    )
    .into_bytes()
}

impl Credential {
    pub(crate) fn request(
        &self,
        operation: Operation,
        now: u64,
    ) -> Result<SignedTeamRequest, String> {
        let mut nonce = [0; 32];
        getrandom::fill(&mut nonce)
            .map_err(|_| "Secure randomness unavailable; no team request was sent")?;
        self.request_with_nonce(operation, now, hex_encode(&nonce))
    }

    fn request_with_nonce(
        &self,
        operation: Operation,
        now: u64,
        nonce: String,
    ) -> Result<SignedTeamRequest, String> {
        let operation = match operation {
            Operation::List => WireOperation::List,
            Operation::Call { name, arguments } => WireOperation::Call { name, arguments },
        };
        let body = RequestBody {
            domain: REQUEST_DOMAIN.into(),
            generation: self.generation,
            operation,
        };
        let payload = decode(&encode(&body)?)?;
        let mut request = SignedTeamRequest {
            pane_id: self.pane,
            timestamp_ms: now,
            nonce,
            payload,
            signature_hex: String::new(),
        };
        request.signature_hex = hex_encode(&hmac_sha256(&self.secret[..], &signed_bytes(&request)));
        encode(&request)?;
        Ok(request)
    }

    pub(crate) fn response(
        &self,
        request: &SignedTeamRequest,
        response: &SignedTeamRequest,
        now: u64,
    ) -> Result<Result<Value, BackendError>, String> {
        encode(response)?;
        fresh(response.timestamp_ms, now)?;
        if response.pane_id != self.pane || response.nonce != request.nonce {
            return Err("Team response does not match the request".into());
        }
        let mac =
            hex_decode_32(&response.signature_hex).ok_or("Malformed team response signature")?;
        let expected = hmac_sha256(&self.secret[..], &signed_bytes(response));
        if expected.ct_eq(&mac).unwrap_u8() != 1 {
            return Err("Unauthenticated team response".into());
        }
        let body: ReplyBody = decode(&encode(&response.payload)?)?;
        if body.domain != RESPONSE_DOMAIN
            || body.generation != self.generation
            || body.request_nonce != request.nonce
            || body.request_digest != digest(request)?
        {
            return Err("Team response binding mismatch".into());
        }
        Ok(body.result)
    }
}

impl Authority {
    pub(crate) fn issue(
        &mut self,
        pane: u64,
        project: u64,
        cwd: &Path,
    ) -> Result<Credential, String> {
        // A failed replacement must not retain an earlier launch's authority.
        self.revoke(pane);
        if !cwd.is_absolute() {
            return Err("Team capability requires an absolute trusted working directory".into());
        }
        if self.bindings.len() >= MAX_BINDINGS {
            return Err("Too many team capabilities".into());
        }
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or("Team launch generation exhausted")?;
        let secret_hex = Zeroizing::new(self.auth.ensure_pane_secret_hex(pane)?);
        let secret = Zeroizing::new(hex_decode_32(&secret_hex).ok_or("Invalid issued team key")?);
        let generation = self.next_generation;
        self.bindings.insert(
            pane,
            Binding {
                pane,
                project,
                cwd: cwd.to_path_buf(),
                generation,
            },
        );
        Ok(Credential {
            pane,
            generation,
            secret,
        })
    }

    pub(crate) fn revoke(&mut self, pane: u64) {
        self.bindings.remove(&pane);
        self.auth.revoke_pane(pane);
    }

    pub(crate) fn revoke_all(&mut self) {
        for pane in std::mem::take(&mut self.bindings).into_keys() {
            self.auth.revoke_pane(pane);
        }
    }

    pub(crate) fn current(&self, binding: &Binding) -> bool {
        self.bindings.get(&binding.pane) == Some(binding)
    }

    pub(crate) fn bindings(&self) -> Vec<Binding> {
        self.bindings.values().cloned().collect()
    }

    pub(crate) fn binding(&self, pane: u64) -> Option<Binding> {
        self.bindings.get(&pane).cloned()
    }

    pub(crate) fn authenticate(
        &mut self,
        request: &SignedTeamRequest,
        now: u64,
    ) -> Result<VerifiedRequest, String> {
        encode(request)?;
        fresh(request.timestamp_ms, now)?;
        if request.nonce.len() != 64 || hex_decode_32(&request.nonce).is_none() {
            return Err("Invalid team transport nonce".into());
        }
        let body: RequestBody = decode(&encode(&request.payload)?)?;
        let binding = self
            .bindings
            .get(&request.pane_id)
            .ok_or("Unknown team capability")?;
        if body.domain != REQUEST_DOMAIN || body.generation != binding.generation {
            return Err("Team launch binding mismatch".into());
        }
        let operation = match body.operation {
            WireOperation::List => Operation::List,
            WireOperation::Call { name, arguments } => {
                if name.is_empty() || name.len() > 128 || !arguments.is_object() {
                    return Err("Invalid team operation".into());
                }
                Operation::Call { name, arguments }
            }
        };
        self.auth.verify_request(request, now)?;
        Ok(VerifiedRequest {
            binding: binding.clone(),
            operation,
            nonce: request.nonce.clone(),
            digest: digest(request)?,
        })
    }

    pub(crate) fn response(
        &self,
        request: &VerifiedRequest,
        result: Result<Value, BackendError>,
        now: u64,
    ) -> Result<SignedTeamRequest, String> {
        if !self.current(&request.binding) {
            return Err("Team capability was revoked".into());
        }
        let mut body = ReplyBody {
            domain: RESPONSE_DOMAIN.into(),
            generation: request.binding.generation,
            request_nonce: request.nonce.clone(),
            request_digest: request.digest.clone(),
            result,
        };
        // Leave headroom for the signed outer envelope before materializing Value.
        let body_bytes = match encode(&body) {
            Ok(bytes) if bytes.len() <= crate::team_mcp_wire::MAX_BYTES - 512 => bytes,
            _ => {
                body.result = Err(BackendError::Failed("Team result exceeds limit; operation may already have completed. Do not retry automatically.".into()));
                encode(&body)?
            }
        };
        let payload = decode(&body_bytes)?;
        let response =
            self.auth
                .sign_request(request.binding.pane, now, &request.nonce, &payload)?;
        encode(&response)?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(credential: &Credential, byte: u8, now: u64) -> SignedTeamRequest {
        credential
            .request_with_nonce(
                Operation::Call {
                    name: "team_inbox".into(),
                    arguments: json!({"me":"other-pane","project":999,"cwd":"untrusted"}),
                },
                now,
                hex_encode(&[byte; 32]),
            )
            .unwrap()
    }

    #[test]
    fn authenticated_scope_comes_only_from_the_issued_binding_and_replays_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let mut authority = Authority::default();
        let key = authority.issue(1, 2, root.path()).unwrap();
        let request = request(&key, 1, 100_000);
        let verified = authority.authenticate(&request, 100_000).unwrap();
        assert_eq!(verified.binding.pane, 1);
        assert_eq!(verified.binding.project, 2);
        assert_eq!(verified.binding.cwd, root.path());
        assert!(authority.authenticate(&request, 100_001).is_err());
        let reply = authority
            .response(&verified, Ok(json!({"messages":[]})), 100_001)
            .unwrap();
        assert_eq!(
            key.response(&request, &reply, 100_002).unwrap().unwrap(),
            json!({"messages":[]})
        );
        assert!(
            authority.authenticate(&reply, 100_002).is_err(),
            "response domain cannot be a request"
        );
        let secret_hex = hex_encode(&key.secret[..]);
        assert!(!format!("{key:?}").contains(&secret_hex));
    }

    #[test]
    fn relaunch_revocation_entropy_failure_and_generation_exhaustion_remove_old_authority() {
        let root = tempfile::tempdir().unwrap();
        let mut authority = Authority::default();
        let first = authority.issue(1, 2, root.path()).unwrap();
        let old_request = request(&first, 1, 100_000);
        let old_verified = authority.authenticate(&old_request, 100_000).unwrap();
        let replacement = authority.issue(1, 3, root.path()).unwrap();
        assert!(replacement.generation > first.generation);
        assert!(!authority.current(&old_verified.binding));
        assert!(
            authority
                .response(&old_verified, Ok(json!({})), 100_001)
                .is_err()
        );
        assert!(
            authority
                .authenticate(&request(&first, 2, 100_000), 100_000)
                .is_err()
        );
        let current = authority
            .authenticate(&request(&replacement, 3, 100_000), 100_000)
            .unwrap();
        assert_eq!(current.binding.project, 3);
        authority.auth.force_entropy_error = true;
        assert!(authority.issue(1, 4, root.path()).is_err());
        assert!(!authority.current(&current.binding));
        assert!(
            authority
                .authenticate(&request(&replacement, 4, 100_000), 100_000)
                .is_err()
        );
        authority.auth.force_entropy_error = false;
        let last = authority.issue(1, 2, root.path()).unwrap();
        authority.next_generation = u64::MAX;
        assert!(authority.issue(1, 2, root.path()).is_err());
        assert!(
            authority
                .authenticate(&request(&last, 5, 100_000), 100_000)
                .is_err()
        );
        assert!(authority.bindings.is_empty());
    }

    #[test]
    fn signatures_domains_request_digests_and_freshness_bind_both_directions() {
        let root = tempfile::tempdir().unwrap();
        let mut authority = Authority::default();
        let key = authority.issue(1, 2, root.path()).unwrap();
        let other_key = authority.issue(2, 2, root.path()).unwrap();
        let original = request(&key, 1, 100_000);
        let mut altered = original.clone();
        altered.payload["operation"]["arguments"]["me"] = json!("changed");
        assert!(authority.authenticate(&altered, 100_000).is_err());
        altered = original.clone();
        altered.pane_id = other_key.pane;
        assert!(authority.authenticate(&altered, 100_000).is_err());
        let verified = authority.authenticate(&original, 100_000).unwrap();
        let reply = authority
            .response(&verified, Err(BackendError::UnknownTool), 100_000)
            .unwrap();
        assert!(matches!(
            key.response(&original, &reply, 100_000).unwrap(),
            Err(BackendError::UnknownTool)
        ));
        assert!(
            key.response(&original, &original, 100_000).is_err(),
            "request reflection is not a response"
        );
        assert!(other_key.response(&original, &reply, 100_000).is_err());
        assert!(
            key.response(&request(&key, 2, 100_000), &reply, 100_000)
                .is_err()
        );
        let mut payload = reply.payload.clone();
        payload["request_digest"] = json!("00".repeat(32));
        let wrong_digest = authority
            .auth
            .sign_request(1, 100_000, &original.nonce, &payload)
            .unwrap();
        assert!(key.response(&original, &wrong_digest, 100_000).is_err());
        let mut forged = reply.clone();
        forged.signature_hex = "00".repeat(32);
        assert!(key.response(&original, &forged, 100_000).is_err());
        assert!(
            key.response(&original, &reply, 100_000 + MAX_AGE_MS + 1)
                .is_err()
        );
        assert!(
            authority
                .authenticate(&request(&key, 3, 100_000), 100_000 + MAX_AGE_MS + 1)
                .is_err()
        );
        assert!(
            authority
                .authenticate(&request(&key, 4, 100_000 + MAX_SKEW_MS + 1), 100_000)
                .is_err()
        );
        let mut body: RequestBody = decode(&encode(&original.payload).unwrap()).unwrap();
        body.generation += 1;
        let payload = decode(&encode(&body).unwrap()).unwrap();
        let wrong_generation = authority
            .auth
            .sign_request(1, 100_000, &hex_encode(&[5; 32]), &payload)
            .unwrap();
        assert!(authority.authenticate(&wrong_generation, 100_000).is_err());
    }

    #[test]
    fn binding_capacity_and_oversized_results_fail_without_displacing_existing_bindings() {
        let root = tempfile::tempdir().unwrap();
        let mut authority = Authority::default();
        let key = authority.issue(1, 2, root.path()).unwrap();
        for pane in 2..=MAX_BINDINGS as u64 {
            authority.issue(pane, 2, root.path()).unwrap();
        }
        assert!(authority.issue(999, 2, root.path()).is_err());
        let request = request(&key, 1, 100_000);
        let verified = authority.authenticate(&request, 100_000).unwrap();
        let reply = authority
            .response(
                &verified,
                Ok(json!("x".repeat(crate::team_mcp_wire::MAX_BYTES))),
                100_000,
            )
            .unwrap();
        assert!(encode(&reply).unwrap().len() < 1024);
        let result = key.response(&request, &reply, 100_000).unwrap();
        assert!(
            matches!(result,Err(BackendError::Failed(message)) if message.contains("may already have completed"))
        );
        authority.revoke_all();
        assert!(authority.bindings.is_empty());
        assert!(!authority.current(&verified.binding));
    }
}
