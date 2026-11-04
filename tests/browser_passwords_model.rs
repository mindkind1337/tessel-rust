#[path = "../src/browser_passwords_model.rs"]
mod browser_passwords_model;

use browser_passwords_model::{
    BatchProfileAttempt, BrowserSessionPartition, BulkAllowDecision, ConfirmationPrompt,
    CredentialActor, CredentialTargetFailure, CsvImportPreflight, DomFormEvaluationInput,
    DomInputDescriptor, DomPasswordReverifyInput, EntryMetadataChange, EscapeAction, FillCompleted,
    ImportCandidate, ImportOutcome, ImportPlan, ImportReportInput, MAX_CSV_COLUMNS,
    MAX_IMPORT_BATCH_ITEMS, MAX_IMPORT_PATH_UTF16, PAGE_SIZE, PageCredentialContext,
    PageShareDescriptor, PasswordsLang, PlannedVaultMutation, SecretSlot, SecretValidation,
    SelectedLoginFields, VaultEntryMetadata, VaultErrorCode, VaultLimits, VaultRowView,
    VerifiedPrivateHttpsOrigin, apply_entry_change, are_form_destinations_safe, build_dialog_view,
    decide_csv_import_file_access, decide_escape_action, decide_page_credential_access,
    decide_vault_management_access, evaluate_dom_login_form, evaluate_import_all_batch,
    export_steps_for_source, format_bulk_change_result, format_import_summary,
    is_allowed_local_import_file_path, is_browser_page_shared, is_safe_login_form_action,
    login_origin, needs_csv_help, no_username_placeholder, parse_password_csv_with,
    plan_bulk_allow_change, plan_import, plan_remove_confirmation, resolve_credential_target,
    reverify_dom_before_password_fill, row_allow_aria_label, row_remove_aria_label,
    sanitize_dom_fill_result, site_display_name, utf16_len, validate_secret_bounds,
    validate_username_bounds, vault_list_entries,
};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

const FIXTURE_JSON: &str = include_str!("fixtures/browser_passwords.json");

/// Opaque secret handle intentionally implementing NEITHER `Debug`, `Display`, `Clone`, NOR `Serialize`.
/// Proves at compile time that `parse_password_csv_with` and `plan_import` never require formatting
/// or serializing secrets.
struct OpaqueSecretHandle {
    slot: usize,
    utf16_len: usize,
    byte_len: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserPasswordsFixture {
    sources: Vec<FixtureSource>,
    constants: FixtureConstants,
    origin_vectors: Vec<OriginVector>,
    csv_vectors: Vec<CsvVector>,
    import_scenarios: Vec<ImportScenario>,
    path_vectors: Vec<PathVector>,
    page_boundary_vectors: Vec<PageBoundaryVector>,
    dom_form_vectors: Vec<DomFormVector>,
    sanitize_vectors: Vec<SanitizeVector>,
    ui_view_vectors: Vec<UiViewVector>,
    bulk_confirm_vectors: Vec<BulkConfirmVector>,
    summary_vectors: Vec<SummaryVector>,
    batch_summary_vectors: Vec<BatchSummaryVector>,
    export_step_vectors: Vec<ExportStepVector>,
    error_vectors: Vec<ErrorVector>,
}

#[derive(Debug, Deserialize)]
struct FixtureSource {
    file: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureConstants {
    page_size: usize,
    reference_max_entries: usize,
    reference_max_csv_bytes: usize,
    reference_max_username_utf16: usize,
    reference_max_password_utf16: usize,
}

#[derive(Debug, Deserialize)]
struct OriginVector {
    input: String,
    expected: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CsvExpectedRow {
    origin: Option<String>,
    username: String,
    secret_slot: usize,
    secret_utf16_len: usize,
    secret_byte_len: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CsvVector {
    name: String,
    csv: String,
    expected_ok: Option<Vec<CsvExpectedRow>>,
    expected_error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IncomingFixtureItem {
    origin: String,
    username_len_utf16: usize,
    username_sample: String,
    secret_slot: usize,
    secret_len_utf16: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportExpectedOk {
    imported: usize,
    added: usize,
    updated: usize,
    unchanged: usize,
    skipped: usize,
    entries: Vec<VaultEntryMetadata>,
    entry_secret_slots: BTreeMap<String, usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportScenario {
    name: String,
    seed: Vec<VaultEntryMetadata>,
    seed_slots: BTreeMap<String, usize>,
    incoming: Vec<IncomingFixtureItem>,
    expected_ok: Option<ImportExpectedOk>,
    expected_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PathVector {
    path: String,
    platform: String,
    allowed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageFixtureSpec {
    project: String,
    #[serde(default)]
    side: bool,
    #[serde(default)]
    opt_in: bool,
    #[serde(default)]
    destroyed: bool,
    partition: String,
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageBoundaryVector {
    name: String,
    op: String,
    trusted_sender: bool,
    joined_as_guest: bool,
    host_sharing_projects: Option<Vec<String>>,
    page: Option<PageFixtureSpec>,
    csv_path: Option<String>,
    credential_id: Option<String>,
    expected_ok: bool,
    expected_code: Option<String>,
    expected_account_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DomFieldFixture {
    #[serde(rename = "type")]
    field_type: String,
    #[serde(default)]
    autocomplete: String,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    read_only: bool,
    #[serde(default = "default_true")]
    visible: bool,
    #[serde(default = "default_true")]
    in_form: bool,
    #[serde(default)]
    owned_by_other_form: bool,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DomFormVector {
    name: String,
    entry_origin: String,
    location_origin: String,
    location_protocol: String,
    location_href: String,
    base_uri: Option<String>,
    form_action: Option<String>,
    #[serde(default = "default_true")]
    form_connected: bool,
    #[serde(default = "default_true")]
    form_in_same_document: bool,
    #[serde(default)]
    submit_form_actions: Vec<String>,
    mutate_origin_during_username_fill: Option<String>,
    #[serde(default)]
    reparent_password_form_during_username_fill: bool,
    fields: Vec<DomFieldFixture>,
    expected_ok: bool,
    expected_code: Option<String>,
    filled_username_index: Option<usize>,
    filled_password_index: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SanitizeVector {
    name: String,
    raw: serde_json::Value,
    expected_ok: bool,
    expected_code: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UiViewVector {
    name: String,
    entries: Vec<VaultEntryMetadata>,
    query: String,
    pages_shown: usize,
    lang: PasswordsLang,
    expected_total_rows: usize,
    expected_filtered_count: usize,
    expected_visible_rows: Vec<VaultRowView>,
    expected_remaining: usize,
    expected_allowed_count: usize,
    expected_count_label: String,
    expected_agents_label: String,
    expected_no_match_label: String,
    expected_more_label: String,
    expected_bulk_checked: bool,
    expected_bulk_indeterminate: bool,
    expected_bulk_disabled: bool,
    expected_bulk_label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BulkConfirmVector {
    name: String,
    entries: Vec<VaultEntryMetadata>,
    query: String,
    target_allow: bool,
    can_confirm: bool,
    lang: PasswordsLang,
    expected_kind: String,
    expected_target_ids: Vec<String>,
    expected_prompt: Option<ConfirmationPrompt>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SummaryResultSpec {
    ok: bool,
    code: Option<String>,
    imported: Option<usize>,
    added: Option<usize>,
    #[serde(default)]
    updated: usize,
    #[serde(default)]
    unchanged: usize,
    #[serde(default)]
    skipped: usize,
    #[serde(default)]
    app_bound_count: usize,
    #[serde(default)]
    protected_count: usize,
}

impl SummaryResultSpec {
    fn to_report_input(&self) -> ImportReportInput {
        ImportReportInput {
            ok: self.ok,
            error_code: self.code.as_deref().map(VaultErrorCode::from_wire_code),
            imported: self.imported,
            added: self.added,
            updated: self.updated,
            unchanged: self.unchanged,
            skipped: self.skipped,
            app_bound_count: self.app_bound_count,
            protected_count: self.protected_count,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SummaryVector {
    name: String,
    lang: PasswordsLang,
    result: SummaryResultSpec,
    expected_needs_csv: bool,
    expected_text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchProfileFixture {
    label: String,
    #[serde(default)]
    threw: bool,
    result: Option<SummaryResultSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExpectedBatchItem {
    index: usize,
    label: String,
    ok: bool,
    warn: bool,
    csv: bool,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchSummaryVector {
    name: String,
    lang: PasswordsLang,
    profiles: Vec<BatchProfileFixture>,
    expected_title: String,
    expected_items: Vec<ExpectedBatchItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportStepVector {
    browser_id: Option<String>,
    family: Option<String>,
    lang: PasswordsLang,
    expected: String,
}

#[derive(Debug, Deserialize)]
struct ErrorVector {
    code: String,
    lang: PasswordsLang,
    expected: String,
}

fn load_fixture() -> BrowserPasswordsFixture {
    serde_json::from_str(FIXTURE_JSON).expect("valid browser_passwords.json fixture")
}

#[test]
fn constants_and_limits_preserve_reference_and_native_vault_bounds() {
    let f = load_fixture();
    assert_eq!(f.sources.len(), 6);
    for src in &f.sources {
        assert!(!src.file.is_empty());
        assert_eq!(src.sha256.len(), 64);
    }

    assert_eq!(PAGE_SIZE, f.constants.page_size);
    assert_eq!(MAX_IMPORT_PATH_UTF16, 4096);

    let ref_limits = VaultLimits::TESSEL_REFERENCE;
    assert_eq!(ref_limits.max_entries, f.constants.reference_max_entries);
    assert_eq!(
        ref_limits.max_csv_bytes,
        f.constants.reference_max_csv_bytes
    );
    assert_eq!(ref_limits.max_csv_data_rows, 5000);
    assert_eq!(
        ref_limits.max_username_utf16,
        f.constants.reference_max_username_utf16
    );
    assert_eq!(
        ref_limits.max_password_utf16,
        f.constants.reference_max_password_utf16
    );
    assert_eq!(ref_limits.max_sealed_vault_bytes, 20 * 1024 * 1024);

    // Default limits must remain the conservative native `src/vault.rs` DPAPI bounds.
    let native = VaultLimits::default();
    assert_eq!(native, VaultLimits::NATIVE_VAULT);
    assert_eq!(native.max_entries, 5000);
    assert_eq!(native.max_csv_bytes, 5 * 1024 * 1024);
    assert_eq!(native.max_sealed_vault_bytes, 5 * 1024 * 1024);
    assert_eq!(native.max_username_bytes, 1024);
    assert_eq!(native.max_password_bytes, 8192);

    // Multi-byte UTF-8 username under 2000 UTF-16 code units but > 1024 UTF-8 bytes:
    // accepted by reference JS limits, rejected by conservative native byte limits.
    let multibyte_user = "é".repeat(600); // 600 UTF-16 units, 1200 UTF-8 bytes
    assert!(validate_username_bounds(&multibyte_user, &ref_limits));
    assert!(!validate_username_bounds(&multibyte_user, &native));
    assert!(validate_username_bounds(&"u".repeat(1024), &native));

    assert_eq!(validate_secret_bounds("", &native), SecretValidation::Empty);
    assert_eq!(
        validate_secret_bounds(&"s".repeat(8192), &native),
        SecretValidation::Valid
    );
    assert_eq!(
        validate_secret_bounds(&"s".repeat(8193), &native),
        SecretValidation::TooLong
    );

    assert_eq!(PasswordsLang::from_code("EN"), Some(PasswordsLang::En));
    assert_eq!(PasswordsLang::from_code(" fr "), Some(PasswordsLang::Fr));
    assert_eq!(PasswordsLang::from_code("de"), None);
    assert_eq!(PasswordsLang::default().as_str(), "en");
    assert_eq!(PasswordsLang::Fr.as_str(), "fr");
}

#[test]
fn origin_normalization_and_site_display_match_reference() {
    let f = load_fixture();
    for vec_case in &f.origin_vectors {
        let got = login_origin(&vec_case.input);
        assert_eq!(
            got, vec_case.expected,
            "login_origin mismatch for {:?}",
            vec_case.input
        );
        if let Some(origin) = &got {
            let site = site_display_name(origin);
            assert!(!site.starts_with("https://"));
            assert_eq!(format!("https://{site}"), *origin);
        }
    }
}

#[test]
fn csv_rfc4180_parser_matches_reference_without_storing_plaintext_dtos() {
    let f = load_fixture();
    let mut slot_by_secret: HashMap<String, usize> = HashMap::new();

    for case in &f.csv_vectors {
        let owned_big_csv;
        let csv_input: &str = if case.csv == "<SYNTHETIC_5001_ROWS>" {
            let mut buf = String::from("url,username,password\n");
            for idx in 0..5001 {
                buf.push_str(&format!("https://s{idx}.example.test,u,syn-p\n"));
            }
            owned_big_csv = buf;
            &owned_big_csv
        } else {
            &case.csv
        };

        let result =
            parse_password_csv_with(csv_input, &VaultLimits::TESSEL_REFERENCE, |pw_slice| {
                let next_slot = slot_by_secret.len();
                let slot = *slot_by_secret
                    .entry(pw_slice.to_owned())
                    .or_insert(next_slot);
                OpaqueSecretHandle {
                    slot,
                    utf16_len: utf16_len(pw_slice),
                    byte_len: pw_slice.len(),
                }
            });

        if let Some(expected_rows) = &case.expected_ok {
            let rows = result.unwrap_or_else(|e| {
                panic!("CSV vector {} expected Ok, got Err({:?})", case.name, e)
            });
            assert_eq!(rows.len(), expected_rows.len(), "vector {}", case.name);
            for (got, exp) in rows.iter().zip(expected_rows.iter()) {
                assert_eq!(got.origin, exp.origin, "vector {}", case.name);
                assert_eq!(got.username, exp.username, "vector {}", case.name);
                assert_eq!(got.secret.slot, exp.secret_slot, "vector {}", case.name);
                assert_eq!(
                    got.secret.utf16_len, exp.secret_utf16_len,
                    "vector {}",
                    case.name
                );
                assert_eq!(
                    got.secret.byte_len, exp.secret_byte_len,
                    "vector {}",
                    case.name
                );
            }
        } else {
            let expected_code = case
                .expected_error
                .as_deref()
                .expect("expected_error present");
            let err = result.err().unwrap_or_else(|| {
                panic!("CSV vector {} expected Err({expected_code})", case.name)
            });
            assert_eq!(err.as_str(), expected_code, "vector {}", case.name);
        }
    }

    // Native byte ceiling rejects oversized CSV buffers before parsing rows.
    let small_cap = VaultLimits {
        max_csv_bytes: 32,
        ..VaultLimits::NATIVE_VAULT
    };
    let err = parse_password_csv_with(
        "url,username,password\nhttps://example.test,alice,0123456789\n",
        &small_cap,
        |_| OpaqueSecretHandle {
            slot: 0,
            utf16_len: 0,
            byte_len: 0,
        },
    )
    .err()
    .expect("oversized CSV rejected");
    assert_eq!(err, VaultErrorCode::TooBig);
}

#[test]
fn import_reconciliation_matches_reference_and_revokes_allow_agents_on_secret_change() {
    let f = load_fixture();
    for scenario in &f.import_scenarios {
        let candidates: Vec<ImportCandidate<OpaqueSecretHandle>> = scenario
            .incoming
            .iter()
            .map(|inc| {
                let username = if inc.username_len_utf16 > 32 {
                    "u".repeat(inc.username_len_utf16)
                } else {
                    inc.username_sample.clone()
                };
                ImportCandidate {
                    origin: Some(inc.origin.clone()),
                    username,
                    secret: OpaqueSecretHandle {
                        slot: inc.secret_slot,
                        utf16_len: inc.secret_len_utf16,
                        byte_len: inc.secret_len_utf16,
                    },
                }
            })
            .collect();

        let mut next_uuid = 1usize;
        let plan_res = plan_import(
            &scenario.seed,
            candidates,
            &VaultLimits::TESSEL_REFERENCE,
            |sec| {
                if sec.utf16_len == 0 {
                    SecretValidation::Empty
                } else if sec.utf16_len > VaultLimits::TESSEL_REFERENCE.max_password_utf16 {
                    SecretValidation::TooLong
                } else {
                    SecretValidation::Valid
                }
            },
            |existing_or_candidate, incoming_sec| match existing_or_candidate {
                SecretSlot::Existing { entry_id } => {
                    scenario.seed_slots.get(entry_id).copied() == Some(incoming_sec.slot)
                }
                SecretSlot::Candidate(prev) => prev.slot == incoming_sec.slot,
            },
            || {
                let id = format!("00000000-0000-4000-8000-{next_uuid:012}");
                next_uuid += 1;
                id
            },
        );

        if let Some(exp) = &scenario.expected_ok {
            let plan = plan_res.unwrap_or_else(|e| {
                panic!("scenario {} expected Ok, got Err({:?})", scenario.name, e)
            });
            assert_eq!(
                plan.outcome(),
                ImportOutcome {
                    imported: exp.imported,
                    added: exp.added,
                    updated: exp.updated,
                    unchanged: exp.unchanged,
                    skipped: exp.skipped,
                },
                "scenario {}",
                scenario.name
            );
            assert_eq!(
                plan.write_required(),
                exp.imported > 0,
                "scenario {}",
                scenario.name
            );
            assert_eq!(plan.next_entries(), exp.entries.as_slice());

            let (outcome, next_entries, mutations) = plan.into_parts();
            assert_eq!(outcome.imported, exp.imported);
            assert_eq!(next_entries, exp.entries);

            let mut applied_slots = scenario.seed_slots.clone();
            for mutation in mutations {
                match mutation {
                    PlannedVaultMutation::Add {
                        id,
                        origin,
                        username,
                        allow_agents,
                        secret,
                    } => {
                        assert!(
                            !allow_agents,
                            "added logins must always start with allow_agents=false"
                        );
                        assert!(origin.starts_with("https://"));
                        assert!(username.len() <= 2000);
                        applied_slots.insert(id, secret.slot);
                    }
                    PlannedVaultMutation::Update {
                        id,
                        origin,
                        username,
                        allow_agents,
                        secret,
                    } => {
                        assert!(
                            !allow_agents,
                            "updated logins must always reset allow_agents=false"
                        );
                        assert!(origin.starts_with("https://"));
                        assert!(!username.is_empty());
                        assert!(applied_slots.contains_key(&id));
                        applied_slots.insert(id, secret.slot);
                    }
                }
            }
            assert_eq!(
                applied_slots, exp.entry_secret_slots,
                "scenario {}",
                scenario.name
            );
        } else {
            let exp_err = scenario.expected_error.as_deref().unwrap();
            let err = plan_res
                .err()
                .unwrap_or_else(|| panic!("scenario {} expected Err({exp_err})", scenario.name));
            assert_eq!(err.as_str(), exp_err);
        }
    }
}

#[test]
fn import_enforces_entry_capacity_and_corrupt_vault_ceiling() {
    let seed: Vec<VaultEntryMetadata> = (0..5000)
        .map(|i| VaultEntryMetadata {
            id: format!("id-{i}"),
            origin: format!("https://s{i}.example.test"),
            username: "alice".to_owned(),
            allow_agents: i == 0,
        })
        .collect();

    // Updating or leaving unchanged within 5000 entries succeeds without exceeding capacity.
    let ok_plan = plan_import(
        &seed,
        vec![
            ImportCandidate {
                origin: Some("https://s0.example.test".to_owned()),
                username: "alice".to_owned(),
                secret: OpaqueSecretHandle {
                    slot: 1,
                    utf16_len: 8,
                    byte_len: 8,
                },
            },
            ImportCandidate {
                origin: Some("https://s1.example.test".to_owned()),
                username: "alice".to_owned(),
                secret: OpaqueSecretHandle {
                    slot: 2,
                    utf16_len: 8,
                    byte_len: 8,
                },
            },
        ],
        &VaultLimits::NATIVE_VAULT,
        |_| SecretValidation::Valid,
        |slot, inc| matches!(slot, SecretSlot::Existing { entry_id: "id-0" } if inc.slot == 1),
        || "new-id".to_owned(),
    )
    .expect("in-place update at 5000 ceiling succeeds");
    assert_eq!(ok_plan.outcome().unchanged, 1);
    assert_eq!(ok_plan.outcome().updated, 1);
    assert!(ok_plan.next_entries()[0].allow_agents);
    assert!(!ok_plan.next_entries()[1].allow_agents);

    // Adding a 5001st distinct login fails closed with TooBig.
    let err = plan_import(
        &seed,
        vec![ImportCandidate {
            origin: Some("https://overflow.example.test".to_owned()),
            username: "alice".to_owned(),
            secret: OpaqueSecretHandle {
                slot: 9,
                utf16_len: 8,
                byte_len: 8,
            },
        }],
        &VaultLimits::NATIVE_VAULT,
        |_| SecretValidation::Valid,
        |_, _| false,
        || "id-5001".to_owned(),
    )
    .err()
    .expect("5001st entry must fail");
    assert_eq!(err, VaultErrorCode::TooBig);

    // Existing vault already over ceiling fails closed with VaultUnavailable.
    let mut corrupt_seed = seed;
    corrupt_seed.push(VaultEntryMetadata {
        id: "id-5000".to_owned(),
        origin: "https://extra.example.test".to_owned(),
        username: "alice".to_owned(),
        allow_agents: false,
    });
    let corrupt_err = plan_import(
        &corrupt_seed,
        Vec::<ImportCandidate<OpaqueSecretHandle>>::new(),
        &VaultLimits::NATIVE_VAULT,
        |_| SecretValidation::Valid,
        |_, _| true,
        || "unused".to_owned(),
    )
    .err()
    .expect("corrupt oversized vault rejected");
    assert_eq!(corrupt_err, VaultErrorCode::VaultUnavailable);
}

#[test]
fn local_import_path_and_csv_preflight_guards_match_reference() {
    let f = load_fixture();
    for pv in &f.path_vectors {
        let is_win = pv.platform == "win32";
        assert_eq!(
            is_allowed_local_import_file_path(&pv.path, is_win),
            pv.allowed,
            "path {:?} on {}",
            pv.path,
            pv.platform
        );
    }

    let over_4096 = format!("C:\\{}", "a".repeat(4095));
    assert!(!is_allowed_local_import_file_path(&over_4096, true));

    // Cancelled file dialog returns `Cancelled`.
    assert_eq!(
        decide_csv_import_file_access(
            &CsvImportPreflight {
                trusted_owner_sender: true,
                joined_as_guest_before_dialog: false,
                picked_path: None,
                joined_as_guest_after_dialog: false,
                is_windows: true,
                is_regular_file: true,
                file_size_bytes: 100,
            },
            &VaultLimits::NATIVE_VAULT,
        ),
        Err(VaultErrorCode::Cancelled)
    );

    // Joining a Live Share guest session while the file picker was open fails closed!
    assert_eq!(
        decide_csv_import_file_access(
            &CsvImportPreflight {
                trusted_owner_sender: true,
                joined_as_guest_before_dialog: false,
                picked_path: Some("C:\\exports\\passwords.csv"),
                joined_as_guest_after_dialog: true,
                is_windows: true,
                is_regular_file: true,
                file_size_bytes: 100,
            },
            &VaultLimits::NATIVE_VAULT,
        ),
        Err(VaultErrorCode::PrivatePageRequired)
    );

    // Symlink / non-regular file or file exceeding `max_csv_bytes` fails with `TooBig`.
    assert_eq!(
        decide_csv_import_file_access(
            &CsvImportPreflight {
                trusted_owner_sender: true,
                joined_as_guest_before_dialog: false,
                picked_path: Some("C:\\exports\\passwords.csv"),
                joined_as_guest_after_dialog: false,
                is_windows: true,
                is_regular_file: false,
                file_size_bytes: 100,
            },
            &VaultLimits::NATIVE_VAULT,
        ),
        Err(VaultErrorCode::TooBig)
    );
    assert_eq!(
        decide_csv_import_file_access(
            &CsvImportPreflight {
                trusted_owner_sender: true,
                joined_as_guest_before_dialog: false,
                picked_path: Some("C:\\exports\\passwords.csv"),
                joined_as_guest_after_dialog: false,
                is_windows: true,
                is_regular_file: true,
                file_size_bytes: (VaultLimits::NATIVE_VAULT.max_csv_bytes as u64) + 1,
            },
            &VaultLimits::NATIVE_VAULT,
        ),
        Err(VaultErrorCode::TooBig)
    );
    assert_eq!(
        decide_csv_import_file_access(
            &CsvImportPreflight {
                trusted_owner_sender: true,
                joined_as_guest_before_dialog: false,
                picked_path: Some("C:\\exports\\passwords.csv"),
                joined_as_guest_after_dialog: false,
                is_windows: true,
                is_regular_file: true,
                file_size_bytes: 1024,
            },
            &VaultLimits::NATIVE_VAULT,
        ),
        Ok(())
    );
}

#[test]
fn live_share_guest_vs_host_sharing_page_boundaries_match_reference() {
    let f = load_fixture();
    let seed = vec![
        VaultEntryMetadata {
            id: "cred-allowed".to_owned(),
            origin: "https://allowed.example.test".to_owned(),
            username: "agent-ok".to_owned(),
            allow_agents: true,
        },
        VaultEntryMetadata {
            id: "cred-same-site-disallowed".to_owned(),
            origin: "https://allowed.example.test".to_owned(),
            username: "human-only".to_owned(),
            allow_agents: false,
        },
        VaultEntryMetadata {
            id: "cred-denied-agents".to_owned(),
            origin: "https://private.example.test".to_owned(),
            username: "owner-user".to_owned(),
            allow_agents: false,
        },
    ];

    for case in &f.page_boundary_vectors {
        let host_projects: Option<Vec<&str>> = case
            .host_sharing_projects
            .as_ref()
            .map(|v| v.iter().map(String::as_str).collect());

        let map_partition = |p: &str| match p {
            "owner" => BrowserSessionPartition::OwnerPrivate,
            "agent" => BrowserSessionPartition::AgentIsolated,
            "guest" => BrowserSessionPartition::LiveShareGuestEphemeral,
            "view" => BrowserSessionPartition::ViewPreview,
            _ => BrowserSessionPartition::ExternalOrUnknown,
        };

        let outcome: Result<Option<Vec<String>>, VaultErrorCode> = match case.op.as_str() {
            "vault_list" => {
                decide_vault_management_access(case.trusted_sender, case.joined_as_guest).map(
                    |()| {
                        let listed = vault_list_entries(&seed, None, false);
                        Some(listed.into_iter().map(|e| e.id).collect())
                    },
                )
            }
            "vault_change" => {
                decide_vault_management_access(case.trusted_sender, case.joined_as_guest).and_then(
                    |()| {
                        apply_entry_change(
                            &seed,
                            "cred-allowed",
                            EntryMetadataChange::SetAllowAgents(false),
                        )
                        .map(|_| None)
                    },
                )
            }
            "import_csv_path" => decide_csv_import_file_access(
                &CsvImportPreflight {
                    trusted_owner_sender: case.trusted_sender,
                    joined_as_guest_before_dialog: case.joined_as_guest,
                    picked_path: case.csv_path.as_deref(),
                    joined_as_guest_after_dialog: case.joined_as_guest,
                    is_windows: true,
                    is_regular_file: true,
                    file_size_bytes: 64,
                },
                &VaultLimits::NATIVE_VAULT,
            )
            .map(|()| None),
            "user_fill" | "agent_list" | "agent_fill" => {
                let actor = if case.op == "user_fill" {
                    CredentialActor::OwnerUi {
                        trusted_sender: case.trusted_sender,
                    }
                } else {
                    CredentialActor::LocalAgent
                };
                let page_spec = case.page.as_ref().expect("page present");
                let shared = is_browser_page_shared(
                    host_projects.as_deref(),
                    PageShareDescriptor {
                        project_id: &page_spec.project,
                        is_side_pane: page_spec.side,
                        side_opt_in: page_spec.opt_in,
                    },
                );
                let ctx = PageCredentialContext {
                    actor,
                    joined_as_guest: case.joined_as_guest,
                    page_present_and_alive: !page_spec.destroyed,
                    page_shared_in_live_share: shared,
                    partition: map_partition(&page_spec.partition),
                    current_page_url: &page_spec.url,
                };
                decide_page_credential_access(&ctx).and_then(|verified_origin| {
                    if case.op == "agent_list" {
                        let accounts =
                            vault_list_entries(&seed, Some(verified_origin.as_str()), true);
                        Ok(Some(accounts.into_iter().map(|a| a.id).collect()))
                    } else {
                        let cred_id = case.credential_id.as_deref().unwrap_or("");
                        resolve_credential_target(&seed, cred_id, &verified_origin, actor)
                            .map(|_| None)
                            .map_err(CredentialTargetFailure::wire_code)
                    }
                })
            }
            other => panic!("unexpected op {other}"),
        };

        match outcome {
            Ok(account_ids) => {
                assert!(case.expected_ok, "case {} expected failure", case.name);
                if let Some(exp_ids) = &case.expected_account_ids {
                    assert_eq!(
                        account_ids.as_ref(),
                        Some(exp_ids),
                        "case {} account ids",
                        case.name
                    );
                }
            }
            Err(code) => {
                assert!(
                    !case.expected_ok,
                    "case {} expected Ok, got {:?}",
                    case.name, code
                );
                assert_eq!(
                    Some(code.as_str()),
                    case.expected_code.as_deref(),
                    "case {}",
                    case.name
                );
            }
        }
    }

    // Exercise ExternalOrUnknown partition, Delete change, and all CredentialTargetFailure variants.
    assert!(!BrowserSessionPartition::ExternalOrUnknown.allows_saved_credentials());
    let after_del = apply_entry_change(&seed, "cred-allowed", EntryMetadataChange::Delete)
        .expect("delete existing id");
    assert_eq!(after_del.len(), 2);
    assert_eq!(
        apply_entry_change(&seed, "missing-id", EntryMetadataChange::Delete),
        Err(VaultErrorCode::NotFound)
    );

    let verified = decide_page_credential_access(&PageCredentialContext {
        actor: CredentialActor::LocalAgent,
        joined_as_guest: false,
        page_present_and_alive: true,
        page_shared_in_live_share: false,
        partition: BrowserSessionPartition::AgentIsolated,
        current_page_url: "https://allowed.example.test/login",
    })
    .expect("verified https origin");
    assert_eq!(
        resolve_credential_target(&seed, "unknown", &verified, CredentialActor::LocalAgent),
        Err(CredentialTargetFailure::NotFound)
    );
    assert_eq!(
        resolve_credential_target(
            &seed,
            "cred-denied-agents",
            &verified,
            CredentialActor::LocalAgent
        ),
        Err(CredentialTargetFailure::OriginMismatch)
    );
    assert_eq!(
        resolve_credential_target(
            &seed,
            "cred-same-site-disallowed",
            &verified,
            CredentialActor::LocalAgent
        ),
        Err(CredentialTargetFailure::AgentAccessDisabled)
    );
}

#[test]
fn dom_login_form_validation_and_sanitization_match_reference() {
    let f = load_fixture();
    assert!(
        f.dom_form_vectors.len() >= 18,
        "expected differential DOM vectors including baseURI, formaction, reparenting, and form ownership"
    );
    for case in &f.dom_form_vectors {
        let has_form = case.form_action.is_some();
        let base_uri = case.base_uri.as_deref().unwrap_or(&case.location_href);
        let submit_refs: Vec<&str> = case
            .submit_form_actions
            .iter()
            .map(String::as_str)
            .collect();
        let descriptors: Vec<DomInputDescriptor<'_>> = case
            .fields
            .iter()
            .map(|fld| {
                let in_subtree = has_form && fld.in_form;
                DomInputDescriptor {
                    input_type: &fld.field_type,
                    autocomplete: &fld.autocomplete,
                    disabled: fld.disabled,
                    read_only: fld.read_only,
                    visible: fld.visible,
                    form_id: if !in_subtree {
                        None
                    } else if fld.owned_by_other_form {
                        Some(99)
                    } else {
                        Some(1)
                    },
                    ancestor_form_id: in_subtree.then_some(1),
                }
            })
            .collect();

        let eval_input = DomFormEvaluationInput {
            expected_origin: &case.entry_origin,
            is_top_frame: true,
            location_origin: &case.location_origin,
            location_protocol: &case.location_protocol,
            location_href: &case.location_href,
            document_base_uri: base_uri,
            form_connected: case.form_connected,
            form_in_same_document: case.form_in_same_document,
            form_action: case.form_action.as_deref(),
            submit_form_actions: &submit_refs,
            inputs: &descriptors,
        };

        let mut filled_username_idx: Option<usize> = None;
        let mut filled_password_idx: Option<usize> = None;

        let step_result: Result<SelectedLoginFields, VaultErrorCode> =
            evaluate_dom_login_form(&eval_input).and_then(|selected| {
                assert_eq!(selected.form_id, 1);
                let mut current_origin = case.location_origin.as_str();
                let mut current_pw_form_id = descriptors[selected.password_index].form_id;
                let mut current_form_action = case.form_action.as_deref();
                if let Some(u_idx) = selected.username_index {
                    filled_username_idx = Some(u_idx);
                    if let Some(mutated) = &case.mutate_origin_during_username_fill {
                        current_origin = mutated.as_str();
                    }
                    if case.reparent_password_form_during_username_fill {
                        current_pw_form_id = Some(99);
                        current_form_action = Some("https://attacker.example.test/collect");
                    }
                }
                let pw_field = &descriptors[selected.password_index];
                reverify_dom_before_password_fill(&DomPasswordReverifyInput {
                    expected_origin: &case.entry_origin,
                    expected_form_id: selected.form_id,
                    is_top_frame: true,
                    location_origin: current_origin,
                    location_protocol: &case.location_protocol,
                    location_href: &case.location_href,
                    document_base_uri: base_uri,
                    password_connected: true,
                    password_disabled: pw_field.disabled,
                    password_read_only: pw_field.read_only,
                    password_visible: pw_field.visible,
                    password_input_type: pw_field.input_type,
                    current_password_form_id: current_pw_form_id,
                    current_form_connected: case.form_connected,
                    current_form_in_same_document: case.form_in_same_document,
                    current_form_action,
                    current_submit_form_actions: &submit_refs,
                })?;
                filled_password_idx = Some(selected.password_index);
                Ok(selected)
            });

        assert_eq!(
            filled_username_idx, case.filled_username_index,
            "case {} username index",
            case.name
        );
        assert_eq!(
            filled_password_idx, case.filled_password_index,
            "case {} password index",
            case.name
        );

        match step_result {
            Ok(_) => assert!(case.expected_ok, "case {} expected error", case.name),
            Err(code) => {
                assert!(!case.expected_ok, "case {} expected ok", case.name);
                assert_eq!(
                    Some(code.as_str()),
                    case.expected_code.as_deref(),
                    "case {}",
                    case.name
                );
            }
        }
    }

    assert!(!is_safe_login_form_action(
        Some("http://login.example.test/downgrade"),
        "https://login.example.test/signin",
        "https://login.example.test/signin",
        "https://login.example.test"
    ));

    for sv in &f.sanitize_vectors {
        let ok_flag = sv.raw.get("ok").and_then(serde_json::Value::as_bool);
        let raw_code = sv.raw.get("code").and_then(serde_json::Value::as_str);
        let res = sanitize_dom_fill_result(ok_flag, raw_code);
        if sv.expected_ok {
            let completed = res.expect("expected FillCompleted");
            assert!(!completed.submitted);
            assert_eq!(completed, FillCompleted { submitted: false });
        } else {
            let err = res.expect_err("expected sanitized error");
            assert_eq!(
                Some(err.as_str()),
                sv.expected_code.as_deref(),
                "sanitize {}",
                sv.name
            );
        }
    }

    assert_eq!(
        VaultErrorCode::sanitize_ipc_exception_code(Some("locked")),
        VaultErrorCode::Locked
    );
    assert_eq!(
        VaultErrorCode::sanitize_ipc_exception_code(Some("unexpected-internal-stack")),
        VaultErrorCode::Failed
    );
    assert_eq!(
        VaultErrorCode::sanitize_ipc_exception_code(None),
        VaultErrorCode::Failed
    );
}

#[test]
fn dialog_view_sorting_search_pagination_and_escape_match_reference() {
    let f = load_fixture();
    for uv in &f.ui_view_vectors {
        let limit = PAGE_SIZE * uv.pages_shown;
        let view = build_dialog_view(&uv.entries, &uv.query, limit, false, uv.lang);
        assert_eq!(view.total_rows, uv.expected_total_rows, "{}", uv.name);
        assert_eq!(
            view.filtered_count, uv.expected_filtered_count,
            "{}",
            uv.name
        );
        assert_eq!(view.visible_rows, uv.expected_visible_rows, "{}", uv.name);
        assert_eq!(view.remaining, uv.expected_remaining, "{}", uv.name);
        assert_eq!(view.allowed_count, uv.expected_allowed_count, "{}", uv.name);
        assert_eq!(view.count_label, uv.expected_count_label, "{}", uv.name);
        assert_eq!(view.agents_label, uv.expected_agents_label, "{}", uv.name);
        assert_eq!(
            view.no_match_label, uv.expected_no_match_label,
            "{}",
            uv.name
        );
        assert_eq!(view.more_label, uv.expected_more_label, "{}", uv.name);
        assert_eq!(
            view.bulk_switch.checked, uv.expected_bulk_checked,
            "{}",
            uv.name
        );
        assert_eq!(
            view.bulk_switch.indeterminate, uv.expected_bulk_indeterminate,
            "{}",
            uv.name
        );
        assert_eq!(
            view.bulk_switch.disabled, uv.expected_bulk_disabled,
            "{}",
            uv.name
        );
        assert_eq!(
            view.bulk_switch.label, uv.expected_bulk_label,
            "{}",
            uv.name
        );
    }

    // 550-entry multi-page pagination (200 -> 400 -> 550) & busy state disabling bulk switch.
    let many: Vec<VaultEntryMetadata> = (0..550)
        .map(|i| VaultEntryMetadata {
            id: format!("m-{i}"),
            origin: format!("https://site-{i:03}.example.test"),
            username: format!("user-{i:03}"),
            allow_agents: i % 2 == 0,
        })
        .collect();
    let page1 = build_dialog_view(&many, "", PAGE_SIZE, false, PasswordsLang::En);
    assert_eq!(page1.visible_rows.len(), 200);
    assert_eq!(page1.visible_rows[0].id, "m-0");
    assert_eq!(
        page1.visible_rows[0].origin,
        "https://site-000.example.test"
    );
    assert_eq!(page1.visible_rows[0].site, "site-000.example.test");
    assert_eq!(page1.visible_rows[0].username, "user-000");
    assert!(page1.visible_rows[0].allow_agents);
    assert!(!page1.visible_rows[0].same);
    assert_eq!(page1.remaining, 350);
    assert_eq!(page1.more_label, "Show more (350 left)");
    assert!(page1.bulk_switch.indeterminate);
    assert!(!page1.bulk_switch.disabled);

    let page2 = build_dialog_view(&many, "", PAGE_SIZE * 2, true, PasswordsLang::Fr);
    assert_eq!(page2.visible_rows.len(), 400);
    assert_eq!(page2.remaining, 150);
    assert_eq!(page2.more_label, "Afficher plus (150 restants)");
    assert!(page2.bulk_switch.disabled);

    let page3 = build_dialog_view(&many, "", PAGE_SIZE * 3, false, PasswordsLang::En);
    assert_eq!(page3.visible_rows.len(), 550);
    assert_eq!(page3.remaining, 0);

    assert_eq!(decide_escape_action(""), EscapeAction::CloseModal);
    assert_eq!(decide_escape_action("bank"), EscapeAction::ClearSearchQuery);
}

#[test]
fn confirmations_aria_labels_summaries_export_steps_and_errors_match_reference() {
    let f = load_fixture();

    for bc in &f.bulk_confirm_vectors {
        let decision = plan_bulk_allow_change(
            &bc.entries,
            &bc.query,
            bc.target_allow,
            bc.can_confirm,
            bc.lang,
        );
        match bc.expected_kind.as_str() {
            "noop" => assert_eq!(decision, BulkAllowDecision::NoOp, "{}", bc.name),
            "refused_without_confirm" => assert_eq!(
                decision,
                BulkAllowDecision::RefusedWithoutConfirmation,
                "{}",
                bc.name
            ),
            "apply_immediately" => assert_eq!(
                decision,
                BulkAllowDecision::ApplyImmediately {
                    target_ids: bc.expected_target_ids.clone(),
                    allow_agents: false,
                },
                "{}",
                bc.name
            ),
            "ask_confirm" => assert_eq!(
                decision,
                BulkAllowDecision::AskConfirmation {
                    target_ids: bc.expected_target_ids.clone(),
                    prompt: bc.expected_prompt.clone().expect("prompt present"),
                },
                "{}",
                bc.name
            ),
            other => panic!("unexpected bulk kind {other}"),
        }
    }

    assert_eq!(
        plan_bulk_allow_change(&[], "", true, true, PasswordsLang::En),
        BulkAllowDecision::NoOp
    );

    assert_eq!(
        format_bulk_change_result(4, 0, PasswordsLang::En),
        ("4 accounts changed.".to_owned(), false)
    );
    assert_eq!(
        format_bulk_change_result(3, 1, PasswordsLang::Fr),
        ("3 modifiés, 1 n'ont pas pu l'être.".to_owned(), true)
    );

    assert_eq!(no_username_placeholder(PasswordsLang::En), "(no username)");
    assert_eq!(
        no_username_placeholder(PasswordsLang::Fr),
        "(sans nom d'utilisateur)"
    );

    let rm_en = plan_remove_confirmation("example.test", "", PasswordsLang::En);
    assert_eq!(rm_en.title, "Remove this login from Tessel?");
    assert_eq!(
        rm_en.text,
        "(no username) on example.test. It stays saved in your browser."
    );
    assert_eq!(rm_en.confirm_label, "Remove");
    assert!(rm_en.danger);

    let rm_fr = plan_remove_confirmation("example.test", "alice", PasswordsLang::Fr);
    assert_eq!(rm_fr.title, "Retirer cette connexion de Tessel ?");
    assert_eq!(
        rm_fr.text,
        "alice sur example.test. Elle reste enregistrée dans votre navigateur."
    );
    assert_eq!(rm_fr.confirm_label, "Retirer");

    assert_eq!(
        row_allow_aria_label("example.test", "", PasswordsLang::En),
        "Allow my local agents to use (no username) on example.test"
    );
    assert_eq!(
        row_remove_aria_label("example.test", "alice", PasswordsLang::Fr),
        "Retirer alice sur example.test de Tessel"
    );

    for sv in &f.summary_vectors {
        let input = sv.result.to_report_input();
        assert_eq!(needs_csv_help(&input), sv.expected_needs_csv, "{}", sv.name);
        assert_eq!(
            format_import_summary(&input, sv.lang),
            sv.expected_text,
            "{}",
            sv.name
        );
    }

    // Legacy `imported`-only fallback (when `added` is None).
    let legacy_report = ImportReportInput {
        ok: true,
        imported: Some(2),
        added: None,
        ..ImportReportInput::default()
    };
    assert_eq!(
        format_import_summary(&legacy_report, PasswordsLang::Fr),
        "2 importés"
    );

    for bv in &f.batch_summary_vectors {
        let attempts: Vec<BatchProfileAttempt> = bv
            .profiles
            .iter()
            .map(|p| BatchProfileAttempt {
                label: p.label.clone(),
                report: if p.threw {
                    ImportReportInput {
                        ok: false,
                        error_code: Some(VaultErrorCode::Failed),
                        ..ImportReportInput::default()
                    }
                } else {
                    p.result
                        .as_ref()
                        .expect("result present when !threw")
                        .to_report_input()
                },
            })
            .collect();
        let batch = evaluate_import_all_batch(&attempts, bv.lang);
        let expected_added: usize = attempts
            .iter()
            .filter(|a| a.report.ok)
            .map(|a| a.report.added.or(a.report.imported).unwrap_or(0))
            .sum();
        let expected_attention = bv.expected_items.iter().filter(|i| i.warn).count();
        assert_eq!(batch.total_added, expected_added, "{}", bv.name);
        assert_eq!(batch.attention_count, expected_attention, "{}", bv.name);
        assert_eq!(batch.title, bv.expected_title, "{}", bv.name);
        assert_eq!(batch.items.len(), bv.expected_items.len(), "{}", bv.name);
        for (got, exp) in batch.items.iter().zip(bv.expected_items.iter()) {
            assert_eq!(got.index, exp.index);
            assert_eq!(got.label, exp.label);
            assert_eq!(got.ok, exp.ok);
            assert_eq!(got.warn, exp.warn);
            assert_eq!(got.csv, exp.csv);
            assert_eq!(got.text, exp.text);
        }
    }

    for ev in &f.export_step_vectors {
        let got = export_steps_for_source(ev.browser_id.as_deref(), ev.family.as_deref(), ev.lang);
        assert_eq!(
            got, ev.expected,
            "export steps {:?}/{:?}/{:?}",
            ev.browser_id, ev.family, ev.lang
        );
    }

    for err_vec in &f.error_vectors {
        let code = VaultErrorCode::from_wire_code(&err_vec.code);
        assert_eq!(
            code.user_message(err_vec.lang),
            err_vec.expected,
            "error code {} ({:?})",
            err_vec.code,
            err_vec.lang
        );
    }
}

#[test]
fn passwordless_dtos_never_serialize_or_debug_secret_material() {
    let sentinel = "SYNTHETIC_SECRET_DO_NOT_LEAK_99182";
    let csv = format!("url,username,password\nhttps://vault.example.test/login,alice,{sentinel}\n");

    let candidates = parse_password_csv_with(&csv, &VaultLimits::NATIVE_VAULT, |pw| {
        assert_eq!(pw, sentinel);
        OpaqueSecretHandle {
            slot: 42,
            utf16_len: utf16_len(pw),
            byte_len: pw.len(),
        }
    })
    .expect("parsed synthetic CSV");

    let plan = plan_import(
        &[],
        candidates,
        &VaultLimits::NATIVE_VAULT,
        |_| SecretValidation::Valid,
        |_, _| false,
        || "entry-1".to_owned(),
    )
    .expect("planned import");

    let outcome_json = serde_json::to_string(&plan.outcome()).expect("serialize outcome");
    let entries_json = serde_json::to_string(plan.next_entries()).expect("serialize entries");
    let dialog_view =
        build_dialog_view(plan.next_entries(), "", PAGE_SIZE, false, PasswordsLang::En);
    let view_json = serde_json::to_string(&dialog_view).expect("serialize dialog view");
    let debug_blob = format!(
        "{:?} {:?} {:?}",
        plan.outcome(),
        plan.next_entries(),
        dialog_view
    );

    for surface in [&outcome_json, &entries_json, &view_json, &debug_blob] {
        assert!(!surface.contains(sentinel));
        assert!(!surface.contains("password"));
    }
}

#[test]
fn csv_streaming_bounds_columns_resists_comma_amplification_and_streams_per_row() {
    assert_eq!(MAX_CSV_COLUMNS, 128);

    // 1. Exactly MAX_CSV_COLUMNS (128) columns succeeds.
    let mut cols_128_header: Vec<String> = (0..(MAX_CSV_COLUMNS - 3))
        .map(|i| format!("extra_{i}"))
        .collect();
    cols_128_header.extend([
        "url".to_owned(),
        "username".to_owned(),
        "password".to_owned(),
    ]);
    assert_eq!(cols_128_header.len(), MAX_CSV_COLUMNS);

    let mut cols_128_row: Vec<String> = vec!["ignored".to_owned(); MAX_CSV_COLUMNS - 3];
    cols_128_row.extend([
        "https://wide.example.test/login".to_owned(),
        "alice".to_owned(),
        "syn-wide".to_owned(),
    ]);
    let wide_ok_csv = format!(
        "{}\n{}\n",
        cols_128_header.join(","),
        cols_128_row.join(",")
    );
    let wide_parsed = parse_password_csv_with(&wide_ok_csv, &VaultLimits::NATIVE_VAULT, |pw| {
        OpaqueSecretHandle {
            slot: 1,
            utf16_len: utf16_len(pw),
            byte_len: pw.len(),
        }
    })
    .expect("128-column CSV accepted");
    assert_eq!(wide_parsed.len(), 1);
    assert_eq!(
        wide_parsed[0].origin.as_deref(),
        Some("https://wide.example.test")
    );
    assert_eq!(wide_parsed[0].username, "alice");

    // 2. 129-column header or data row fails immediately with Format.
    let header_129 = format!("{},overflow\n", cols_128_header.join(","));
    assert_eq!(
        parse_password_csv_with(&header_129, &VaultLimits::NATIVE_VAULT, |_| ()).err(),
        Some(VaultErrorCode::Format)
    );
    let data_129 = format!(
        "url,username,password\nhttps://a.example.test,u,p,{}\n",
        vec!["x"; MAX_CSV_COLUMNS - 2].join(",")
    );
    assert_eq!(
        parse_password_csv_with(&data_129, &VaultLimits::NATIVE_VAULT, |_| ()).err(),
        Some(VaultErrorCode::Format)
    );

    // 3. Pathological 5 MiB comma-only payload aborts on the 128th comma without allocating millions of cells.
    let five_mib_commas = ",".repeat(VaultLimits::NATIVE_VAULT.max_csv_bytes);
    assert_eq!(
        parse_password_csv_with(&five_mib_commas, &VaultLimits::NATIVE_VAULT, |_| {
            panic!("map_secret must never run on comma bomb")
        })
        .err(),
        Some(VaultErrorCode::Format)
    );

    // 4. Pathological blank lines and empty-comma lines do not count toward data rows or amplify memory.
    let mut sparse = String::with_capacity(512 * 1024);
    for _ in 0..25_000 {
        sparse.push('\n');
    }
    sparse.push_str("url,username,password\n");
    for _ in 0..25_000 {
        sparse.push_str(",,,\r\n");
    }
    sparse.push_str("https://sparse.example.test,bob,syn-ok\n");
    let sparse_rows = parse_password_csv_with(&sparse, &VaultLimits::NATIVE_VAULT, |pw| pw.len())
        .expect("sparse CSV parsed");
    assert_eq!(sparse_rows.len(), 1);
    assert_eq!(sparse_rows[0].secret, 6);

    // 5. Headerless CSV whose first row contains credentials rejects on row 1 without calling `map_secret`.
    assert_eq!(
        parse_password_csv_with(
            "https://vault.example.test,alice,syn-secret-row1\n",
            &VaultLimits::NATIVE_VAULT,
            |_| panic!("map_secret must never run without valid header")
        )
        .err(),
        Some(VaultErrorCode::Format)
    );

    // 6. Behavioral proof of row-by-row streaming: `map_secret` runs immediately as rows 1 and 2
    // finish, BEFORE row 3's unclosed quote is encountered.
    let mut streamed_lengths = Vec::new();
    let streaming_err = parse_password_csv_with(
        "url,username,password\nhttps://r1.example.test,u1,abcd\nhttps://r2.example.test,u2,abcdef\nhttps://r3.example.test,\"unterminated,xyz\n",
        &VaultLimits::NATIVE_VAULT,
        |pw| {
            streamed_lengths.push(pw.len());
            pw.len()
        },
    )
    .err();
    assert_eq!(streaming_err, Some(VaultErrorCode::Format));
    assert_eq!(streamed_lengths, vec![4, 6]);
}

#[test]
fn verified_https_origin_and_secret_carriers_cannot_be_forged_or_serialized() {
    trait NotImplementedCheck<Marker> {
        fn verify() {}
    }
    impl<T: ?Sized> NotImplementedCheck<()> for T {}

    enum ForbiddenSerialize {}
    impl<T: ?Sized + serde::Serialize> NotImplementedCheck<ForbiddenSerialize> for T {}

    enum ForbiddenDeserialize {}
    impl<T: serde::de::DeserializeOwned> NotImplementedCheck<ForbiddenDeserialize> for T {}

    enum ForbiddenDefault {}
    impl<T: Default> NotImplementedCheck<ForbiddenDefault> for T {}

    // If any of these types ever implements `Serialize`, `DeserializeOwned`, or `Default`,
    // type inference for `_` becomes ambiguous and fails compilation immediately.
    <VerifiedPrivateHttpsOrigin as NotImplementedCheck<_>>::verify();
    <ImportCandidate<String> as NotImplementedCheck<_>>::verify();
    <SecretSlot<'_, String> as NotImplementedCheck<_>>::verify();
    <PlannedVaultMutation<String> as NotImplementedCheck<_>>::verify();
    <ImportPlan<String> as NotImplementedCheck<_>>::verify();

    // Runtime verification: only a live, non-guest, non-shared HTTPS owner/agent page yields a proof.
    let ok_token = decide_page_credential_access(&PageCredentialContext {
        actor: CredentialActor::OwnerUi {
            trusted_sender: true,
        },
        joined_as_guest: false,
        page_present_and_alive: true,
        page_shared_in_live_share: false,
        partition: BrowserSessionPartition::OwnerPrivate,
        current_page_url: "https://VAULT.EXAMPLE.TEST:443/login?next=1",
    })
    .expect("valid private HTTPS origin");
    assert_eq!(ok_token.as_str(), "https://vault.example.test");
}

#[test]
fn plan_import_streams_bounds_working_len_in_flight_and_rejects_id_collisions() {
    assert_eq!(MAX_IMPORT_BATCH_ITEMS, 20_000);

    // 1. Corrupt `existing` snapshots (empty ID or duplicate ID) fail closed with VaultUnavailable.
    assert_eq!(
        plan_import(
            &[VaultEntryMetadata {
                id: String::new(),
                origin: "https://site.example".to_owned(),
                username: "alice".to_owned(),
                allow_agents: false,
            }],
            Vec::<ImportCandidate<OpaqueSecretHandle>>::new(),
            &VaultLimits::NATIVE_VAULT,
            |_| SecretValidation::Valid,
            |_, _| true,
            || "unused".to_owned(),
        )
        .err(),
        Some(VaultErrorCode::VaultUnavailable)
    );
    assert_eq!(
        plan_import(
            &[
                VaultEntryMetadata {
                    id: "dup-id".to_owned(),
                    origin: "https://site1.example".to_owned(),
                    username: "alice".to_owned(),
                    allow_agents: false,
                },
                VaultEntryMetadata {
                    id: "dup-id".to_owned(),
                    origin: "https://site2.example".to_owned(),
                    username: "bob".to_owned(),
                    allow_agents: false,
                },
            ],
            Vec::<ImportCandidate<OpaqueSecretHandle>>::new(),
            &VaultLimits::NATIVE_VAULT,
            |_| SecretValidation::Valid,
            |_, _| true,
            || "unused".to_owned(),
        )
        .err(),
        Some(VaultErrorCode::VaultUnavailable)
    );

    // 2. Arbitrary unbounded iterator (`repeat_with`) of skipped or unchanged items is cut off
    // strictly at `MAX_IMPORT_BATCH_ITEMS + 1` with `TooBig`.
    let mut pulled_skipped = 0usize;
    let infinite_skipped = std::iter::repeat_with(|| {
        pulled_skipped += 1;
        ImportCandidate {
            origin: Some("http://insecure.example".to_owned()),
            username: "u".to_owned(),
            secret: OpaqueSecretHandle {
                slot: 1,
                utf16_len: 4,
                byte_len: 4,
            },
        }
    });
    assert_eq!(
        plan_import(
            &[],
            infinite_skipped,
            &VaultLimits::NATIVE_VAULT,
            |_| SecretValidation::Valid,
            |_, _| true,
            || "unused".to_owned(),
        )
        .err(),
        Some(VaultErrorCode::TooBig)
    );
    assert_eq!(pulled_skipped, MAX_IMPORT_BATCH_ITEMS + 1);

    // 3. In-flight `working.len()` cap stops immediately on the 5001st distinct entry without
    // pulling the rest of a 15_000-item iterator or calling `next_id` past `limits.max_entries`.
    let seed_4999: Vec<VaultEntryMetadata> = (0..4999)
        .map(|i| VaultEntryMetadata {
            id: format!("seed-{i}"),
            origin: format!("https://seed-{i}.example.test"),
            username: "u".to_owned(),
            allow_agents: false,
        })
        .collect();
    let mut pulled_distinct = 0usize;
    let mut next_id_calls = 0usize;
    let distinct_stream = (0..15_000).map(|i| {
        pulled_distinct += 1;
        ImportCandidate {
            origin: Some(format!("https://new-{i}.example.test")),
            username: "u".to_owned(),
            secret: OpaqueSecretHandle {
                slot: i,
                utf16_len: 8,
                byte_len: 8,
            },
        }
    });
    assert_eq!(
        plan_import(
            &seed_4999,
            distinct_stream,
            &VaultLimits::NATIVE_VAULT,
            |_| SecretValidation::Valid,
            |_, _| false,
            || {
                next_id_calls += 1;
                format!("gen-{next_id_calls}")
            },
        )
        .err(),
        Some(VaultErrorCode::TooBig)
    );
    assert_eq!(pulled_distinct, 2);
    assert_eq!(next_id_calls, 1);

    // 4. Empty or colliding IDs from `next_id` fail closed with `VaultErrorCode::Failed`.
    let one_existing = [VaultEntryMetadata {
        id: "existing-1".to_owned(),
        origin: "https://one.example.test".to_owned(),
        username: "alice".to_owned(),
        allow_agents: false,
    }];
    let mk_new = |n: usize| ImportCandidate {
        origin: Some(format!("https://fresh-{n}.example.test")),
        username: "alice".to_owned(),
        secret: OpaqueSecretHandle {
            slot: n,
            utf16_len: 6,
            byte_len: 6,
        },
    };
    assert_eq!(
        plan_import(
            &one_existing,
            [mk_new(1)],
            &VaultLimits::NATIVE_VAULT,
            |_| SecretValidation::Valid,
            |_, _| false,
            String::new,
        )
        .err(),
        Some(VaultErrorCode::Failed)
    );
    assert_eq!(
        plan_import(
            &one_existing,
            [mk_new(1)],
            &VaultLimits::NATIVE_VAULT,
            |_| SecretValidation::Valid,
            |_, _| false,
            || "existing-1".to_owned(),
        )
        .err(),
        Some(VaultErrorCode::Failed)
    );
    assert_eq!(
        plan_import(
            &one_existing,
            [mk_new(1), mk_new(2)],
            &VaultLimits::NATIVE_VAULT,
            |_| SecretValidation::Valid,
            |_, _| false,
            || "same-generated-id".to_owned(),
        )
        .err(),
        Some(VaultErrorCode::Failed)
    );
}

#[test]
fn dom_guard_blocks_form_reparenting_submit_formaction_and_subframes() {
    let inputs = [
        DomInputDescriptor {
            input_type: "text",
            autocomplete: "username",
            disabled: false,
            read_only: false,
            visible: true,
            form_id: Some(10),
            ancestor_form_id: Some(10),
        },
        DomInputDescriptor {
            input_type: "password",
            autocomplete: "current-password",
            disabled: false,
            read_only: false,
            visible: true,
            form_id: Some(10),
            ancestor_form_id: Some(10),
        },
    ];

    let base_eval = DomFormEvaluationInput {
        expected_origin: "https://site.example",
        is_top_frame: true,
        location_origin: "https://site.example",
        location_protocol: "https:",
        location_href: "https://site.example/login",
        document_base_uri: "https://site.example/login",
        form_connected: true,
        form_in_same_document: true,
        form_action: Some("https://site.example/login"),
        submit_form_actions: &[],
        inputs: &inputs,
    };

    // 1. Subframe (`window !== window.top`), disconnected form, or foreign-document form -> UnsafeForm.
    assert_eq!(
        evaluate_dom_login_form(&DomFormEvaluationInput {
            is_top_frame: false,
            ..base_eval
        }),
        Err(VaultErrorCode::UnsafeForm)
    );
    assert_eq!(
        evaluate_dom_login_form(&DomFormEvaluationInput {
            form_connected: false,
            ..base_eval
        }),
        Err(VaultErrorCode::UnsafeForm)
    );
    assert_eq!(
        evaluate_dom_login_form(&DomFormEvaluationInput {
            form_in_same_document: false,
            ..base_eval
        }),
        Err(VaultErrorCode::UnsafeForm)
    );

    // 2. `<base href="https://evil.example/phish/">` hijacks non-empty relative `action` and
    // `formaction`, whereas missing (`None`) or empty (`Some("")`) resolves to `location_href`.
    assert!(!is_safe_login_form_action(
        Some("/session"),
        "https://site.example/login",
        "https://evil.example/phish/",
        "https://site.example",
    ));
    assert!(is_safe_login_form_action(
        Some(""),
        "https://site.example/login",
        "https://evil.example/phish/",
        "https://site.example",
    ));
    assert!(is_safe_login_form_action(
        None,
        "https://site.example/login",
        "https://evil.example/phish/",
        "https://site.example",
    ));
    assert_eq!(
        evaluate_dom_login_form(&DomFormEvaluationInput {
            document_base_uri: "https://evil.example/phish/",
            form_action: Some("/session"),
            ..base_eval
        }),
        Err(VaultErrorCode::UnsafeForm)
    );
    assert_eq!(
        evaluate_dom_login_form(&DomFormEvaluationInput {
            document_base_uri: "https://evil.example/phish/",
            form_action: Some(""),
            submit_form_actions: &["step2"],
            ..base_eval
        }),
        Err(VaultErrorCode::UnsafeForm)
    );

    // 3. Static cross-origin `<button formaction="https://other.example/receive">` (inside `<form>`
    // or external tied via `form="f"`) is rejected before any field is touched.
    assert!(!are_form_destinations_safe(
        Some("https://site.example/login"),
        &["/login", "https://other.example/receive"],
        "https://site.example/login",
        "https://site.example/login",
        "https://site.example",
    ));
    assert_eq!(
        evaluate_dom_login_form(&DomFormEvaluationInput {
            submit_form_actions: &["https://other.example/receive"],
            ..base_eval
        }),
        Err(VaultErrorCode::UnsafeForm)
    );

    // 4. Both external `<input form="f">` (`ancestor_form_id: None, form_id: Some(10)`) and
    // internal `<input form="other">` (`ancestor_form_id: Some(10), form_id: Some(99)`) are ignored
    // for username selection, filling only the password input cleanly.
    let mixed_ownership_inputs = [
        DomInputDescriptor {
            input_type: "email",
            autocomplete: "username",
            disabled: false,
            read_only: false,
            visible: true,
            form_id: Some(10),
            ancestor_form_id: None,
        },
        DomInputDescriptor {
            input_type: "email",
            autocomplete: "username",
            disabled: false,
            read_only: false,
            visible: true,
            form_id: Some(99),
            ancestor_form_id: Some(10),
        },
        DomInputDescriptor {
            input_type: "password",
            autocomplete: "current-password",
            disabled: false,
            read_only: false,
            visible: true,
            form_id: Some(10),
            ancestor_form_id: Some(10),
        },
    ];
    let pw_only_selected = evaluate_dom_login_form(&DomFormEvaluationInput {
        inputs: &mixed_ownership_inputs,
        ..base_eval
    })
    .expect("mis-owned email inputs ignored");
    assert_eq!(pw_only_selected.form_id, 10);
    assert_eq!(pw_only_selected.username_index, None);
    assert_eq!(pw_only_selected.password_index, 2);

    // Same-origin relative and absolute `formaction` overrides succeed.
    let selected = evaluate_dom_login_form(&DomFormEvaluationInput {
        submit_form_actions: &["/login?step=2", "https://site.example/session"],
        ..base_eval
    })
    .expect("same-origin submit formactions allowed");
    assert_eq!(selected.form_id, 10);
    assert_eq!(selected.username_index, Some(0));
    assert_eq!(selected.password_index, 1);

    // 5. Reparenting `password.form`, detaching `form`, moving `form` to a foreign document,
    // injecting `<base href>`, or mutating `formaction` during `username` `input` event all abort
    // BEFORE setting `password.value`.
    let baseline_reverify = DomPasswordReverifyInput {
        expected_origin: "https://site.example",
        expected_form_id: selected.form_id,
        is_top_frame: true,
        location_origin: "https://site.example",
        location_protocol: "https:",
        location_href: "https://site.example/login",
        document_base_uri: "https://site.example/login",
        password_connected: true,
        password_disabled: false,
        password_read_only: false,
        password_visible: true,
        password_input_type: "password",
        current_password_form_id: Some(10),
        current_form_connected: true,
        current_form_in_same_document: true,
        current_form_action: Some("/login"),
        current_submit_form_actions: &["/login"],
    };
    assert_eq!(
        reverify_dom_before_password_fill(&baseline_reverify),
        Ok(())
    );

    assert_eq!(
        reverify_dom_before_password_fill(&DomPasswordReverifyInput {
            current_password_form_id: Some(99),
            ..baseline_reverify
        }),
        Err(VaultErrorCode::UnsafeForm)
    );
    assert_eq!(
        reverify_dom_before_password_fill(&DomPasswordReverifyInput {
            current_password_form_id: None,
            ..baseline_reverify
        }),
        Err(VaultErrorCode::UnsafeForm)
    );
    assert_eq!(
        reverify_dom_before_password_fill(&DomPasswordReverifyInput {
            current_form_connected: false,
            ..baseline_reverify
        }),
        Err(VaultErrorCode::UnsafeForm)
    );
    assert_eq!(
        reverify_dom_before_password_fill(&DomPasswordReverifyInput {
            current_form_in_same_document: false,
            ..baseline_reverify
        }),
        Err(VaultErrorCode::UnsafeForm)
    );
    assert_eq!(
        reverify_dom_before_password_fill(&DomPasswordReverifyInput {
            document_base_uri: "https://evil.example/phish/",
            ..baseline_reverify
        }),
        Err(VaultErrorCode::UnsafeForm)
    );
    assert_eq!(
        reverify_dom_before_password_fill(&DomPasswordReverifyInput {
            current_submit_form_actions: &["https://other.example/receive"],
            ..baseline_reverify
        }),
        Err(VaultErrorCode::UnsafeForm)
    );
}
