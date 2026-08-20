//! Multi-source account import for PoolGate.
//!
//! Import sources are normalized into a single credential model before preview
//! or persistence. Execution always reparses the original source so masked
//! preview data is never treated as trusted credentials.

use crate::db::accounts::{insert_account, Account};

use crate::services::credentials::CredentialPayload;
use crate::services::keychain;
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use uuid::Uuid;

struct ImportedSecretsGuard {
    refs: Vec<String>,
    committed: bool,
}

/// Stable error categories embedded in error strings so the UI can map them
/// to precise, human-readable messages. Keep the prefix format stable — the
/// frontend (`importErrorMessage`) matches on these tokens.
pub const ERR_UNSUPPORTED_FORMAT: &str = "UNSUPPORTED_FORMAT";
pub const ERR_MISSING_FIELD: &str = "MISSING_FIELD";
pub const ERR_INVALID_VALUE: &str = "INVALID_VALUE";
pub const ERR_UNREADABLE_FILE: &str = "UNREADABLE_FILE";
pub const ERR_NO_SOURCE: &str = "NO_SOURCE";

/// Build a categorized error message: `[CATEGORY] human message`.
pub(crate) fn import_error(category: &str, message: impl std::fmt::Display) -> String {
    format!("[{}] {}", category, message)
}

impl ImportedSecretsGuard {
    fn new() -> Self {
        Self {
            refs: Vec::new(),
            committed: false,
        }
    }

    fn track(&mut self, account_id: &str) {
        self.refs.push(keychain::account_secret_ref(account_id));
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for ImportedSecretsGuard {
    fn drop(&mut self) {
        if !self.committed {
            for secret_ref in &self.refs {
                if let Err(error) = keychain::delete_secret(secret_ref) {
                    tracing::warn!(
                        "Failed to compensate imported credential ref={}: {}",
                        secret_ref,
                        error
                    );
                }
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct ImportSourceRequest {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub source_name: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    /// Optional provider context supplied by the provider picker. It is merged
    /// into otherwise ambiguous pasted tokens and batch files before parsing.
    #[serde(default)]
    pub provider_hint: Option<ImportProviderHint>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct ImportProviderHint {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    /// User-selected multi-protocol set for this upstream. When present it
    /// overrides the single `protocol` derivation (e.g. MiMo speaks both
    /// Responses and Anthropic Messages).
    #[serde(default)]
    pub protocols: Option<Vec<String>>,
    /// Protocol-specific upstream Base URLs keyed by canonical protocol.
    #[serde(default)]
    pub base_urls: Option<HashMap<String, String>>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub credential_mode: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportOptions {
    #[serde(default = "default_true")]
    pub auto_create_providers: bool,
    #[serde(default = "default_true")]
    pub skip_duplicates: bool,
    #[serde(default)]
    pub import_adapter_required: bool,
    /// If provided, only accounts whose full SHA-256 fingerprint is in this
    /// list will be persisted during execution. Used after an import-health
    /// check where the user selected a subset of accounts.
    #[serde(default)]
    pub selected_fingerprints: Option<Vec<String>>,
    /// Optional per-account model selection keyed by the full credential
    /// fingerprint. When present, only these models are persisted for that
    /// account; accounts without model metadata are unaffected.
    #[serde(default)]
    pub selected_models: Option<HashMap<String, Vec<String>>>,
    /// Tags applied to every imported account in addition to the tags parsed
    /// from the source.
    #[serde(default)]
    pub default_tags: Option<Vec<String>>,
    /// Conflict resolution when an account with the same credential
    /// fingerprint already exists: `"skip"` (default, keeps existing),
    /// `"overwrite"` (replace credential + metadata), or `"merge"` (fill in
    /// missing fields only, keep existing values otherwise).
    #[serde(default)]
    pub on_conflict: Option<String>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            auto_create_providers: true,
            skip_duplicates: true,
            import_adapter_required: true,
            selected_fingerprints: None,
            selected_models: None,
            default_tags: None,
            on_conflict: None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportPreview {
    pub format: String,
    pub source_count: usize,
    pub accounts: Vec<ImportPreviewAccount>,
    pub auto_created_providers: Vec<ImportProviderPreview>,
    pub warnings: Vec<String>,
    pub skipped: Vec<String>,
    pub summary: ImportSummary,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportPreviewAccount {
    pub index: usize,
    pub name: String,
    pub email: Option<String>,
    pub provider_name: String,
    pub protocol: String,
    pub credential_type: String,
    pub source_format: String,
    pub masked_credential: String,
    pub external_account_id: Option<String>,
    pub expires_at: Option<String>,
    pub models: Vec<String>,
    pub tags: Vec<String>,
    pub fingerprint: String,
    pub action: String,
    pub routable: bool,
    pub adapter: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportProviderPreview {
    pub name: String,
    pub protocol: String,
    pub base_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct ImportSummary {
    pub total: usize,
    pub ready: usize,
    pub duplicates: usize,
    pub adapter_required: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportResult {
    pub imported: usize,
    pub updated: usize,
    pub skipped: usize,
    pub duplicates: usize,
    pub adapter_required: usize,
    pub created_providers: usize,
    pub account_ids: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ImportFormat {
    Sub2Api,
    Cpa,
    Cockpit,
    CodexAuth,
    EchoBird,
    GenericJson,
    Csv,
    ApiKeyText,
    Yaml,
    Toml,
}

impl ImportFormat {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Sub2Api => "sub2api",
            Self::Cpa => "cpa",
            Self::Cockpit => "cockpit",
            Self::CodexAuth => "codex_auth",
            Self::EchoBird => "echobird",
            Self::GenericJson => "json",
            Self::Csv => "csv",
            Self::ApiKeyText => "api_key_text",
            Self::Yaml => "yaml",
            Self::Toml => "toml",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedAccount {
    pub(crate) name: String,
    pub(crate) email: Option<String>,
    pub(crate) provider_name: String,
    pub(crate) provider_type: String,
    pub(crate) protocol: String,
    pub(crate) protocols: Vec<String>,
    pub(crate) base_url: String,
    pub(crate) base_urls: HashMap<String, String>,
    pub(crate) credential_type: String,
    pub(crate) credential: CredentialPayload,
    pub(crate) source_format: String,
    pub(crate) external_account_id: Option<String>,
    pub(crate) expires_at: Option<String>,
    pub(crate) models: Vec<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) metadata: serde_json::Value,
    pub(crate) fingerprint: String,
    pub(crate) routable: bool,
    pub(crate) adapter: String,
    pub(crate) warning: Option<String>,
}

#[derive(Default)]
pub(crate) struct ParseBatch {
    pub(crate) accounts: Vec<NormalizedAccount>,
    pub(crate) warnings: Vec<String>,
    pub(crate) skipped: Vec<String>,
    pub(crate) formats: Vec<String>,
    pub(crate) source_count: usize,
}

fn default_true() -> bool {
    true
}

pub fn preview_request(
    conn: &Mutex<Connection>,
    request: &ImportSourceRequest,
) -> Result<ImportPreview, String> {
    let batch = parse_request(request)?;
    let existing_fingerprints = load_existing_fingerprints(conn)?;
    let existing_providers = load_existing_provider_names(conn)?;
    let mut seen = HashSet::new();
    let mut summary = ImportSummary {
        total: batch.accounts.len(),
        skipped: batch.skipped.len(),
        ..ImportSummary::default()
    };
    let mut providers = HashMap::<String, ImportProviderPreview>::new();

    let accounts = batch
        .accounts
        .iter()
        .enumerate()
        .map(|(index, account)| {
            let duplicate = existing_fingerprints.contains(&account.fingerprint)
                || !seen.insert(account.fingerprint.clone());
            let action = if duplicate {
                summary.duplicates += 1;
                "duplicate"
            } else if !account.routable {
                summary.adapter_required += 1;
                "adapter_required"
            } else {
                summary.ready += 1;
                "create"
            };

            if !existing_providers.contains(&account.provider_name.to_lowercase()) {
                providers
                    .entry(account.provider_name.to_lowercase())
                    .or_insert_with(|| ImportProviderPreview {
                        name: account.provider_name.clone(),
                        protocol: account.protocol.clone(),
                        base_url: account.base_url.clone(),
                    });
            }

            ImportPreviewAccount {
                index,
                name: account.name.clone(),
                email: account.email.clone(),
                provider_name: account.provider_name.clone(),
                protocol: account.protocol.clone(),
                credential_type: account.credential_type.clone(),
                source_format: account.source_format.clone(),
                masked_credential: mask_secret(&primary_secret(account)),
                external_account_id: account.external_account_id.clone(),
                expires_at: account.expires_at.clone(),
                models: account.models.clone(),
                tags: account.tags.clone(),
                fingerprint: short_fingerprint(&account.fingerprint),
                action: action.to_string(),
                routable: account.routable,
                adapter: account.adapter.clone(),
                warning: account.warning.clone(),
            }
        })
        .collect();

    let format = if batch.formats.iter().collect::<HashSet<_>>().len() > 1 {
        "mixed".to_string()
    } else {
        batch
            .formats
            .first()
            .cloned()
            .unwrap_or_else(|| "unknown".to_string())
    };

    Ok(ImportPreview {
        format,
        source_count: batch.source_count,
        accounts,
        auto_created_providers: providers.into_values().collect(),
        warnings: batch.warnings,
        skipped: batch.skipped,
        summary,
    })
}

pub fn execute_request(
    conn: &Mutex<Connection>,
    request: &ImportSourceRequest,
    options: &ImportOptions,
) -> Result<ImportResult, String> {
    let batch = parse_request(request)?;
    let mut conn = conn.lock().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut secret_guard = ImportedSecretsGuard::new();
    let mut result = ImportResult {
        imported: 0,
        updated: 0,
        skipped: batch.skipped.len(),
        duplicates: 0,
        adapter_required: 0,
        created_providers: 0,
        account_ids: Vec::new(),
        errors: Vec::new(),
    };
    let mut seen = HashSet::new();
    // `on_conflict` supersedes the legacy `skip_duplicates` flag. Defaults to
    // "skip" so existing behaviour is preserved when the option is absent.
    let conflict_mode = options.on_conflict.as_deref().unwrap_or("skip");

    for mut normalized in batch.accounts {
        if let Some(ref selected) = options.selected_fingerprints {
            if !selected.contains(&normalized.fingerprint) {
                result.skipped += 1;
                continue;
            }
        }

        if let Some(selected_models) = options
            .selected_models
            .as_ref()
            .and_then(|models| models.get(&normalized.fingerprint))
        {
            normalized.models = selected_models.clone();
        }

        if !seen.insert(normalized.fingerprint.clone()) {
            result.duplicates += 1;
            result.skipped += 1;
            continue;
        }

        let existing_id: Option<String> = tx
            .query_row(
                "SELECT id FROM accounts WHERE credential_fingerprint=?1 LIMIT 1",
                rusqlite::params![normalized.fingerprint],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(existing_id) = existing_id {
            match conflict_mode {
                "overwrite" => {
                    update_account_in_transaction(&tx, &existing_id, &normalized)
                        .map_err(|e| format!("{}: {}", normalized.name, e))?;
                    result.updated += 1;
                    if !normalized.routable {
                        result.adapter_required += 1;
                    }
                    result.account_ids.push(existing_id);
                    continue;
                }
                "merge" => {
                    merge_account_in_transaction(&tx, &existing_id, &normalized)
                        .map_err(|e| format!("{}: {}", normalized.name, e))?;
                    result.updated += 1;
                    if !normalized.routable {
                        result.adapter_required += 1;
                    }
                    result.account_ids.push(existing_id);
                    continue;
                }
                _ => {
                    result.duplicates += 1;
                    result.skipped += 1;
                    continue;
                }
            }
        }

        if !normalized.routable && !options.import_adapter_required {
            result.adapter_required += 1;
            result.skipped += 1;
            continue;
        }

        let explicitly_selected_no_models = options
            .selected_models
            .as_ref()
            .and_then(|models| models.get(&normalized.fingerprint))
            .map(|models| models.is_empty())
            .unwrap_or(false);
        if explicitly_selected_no_models {
            result.skipped += 1;
            result
                .errors
                .push(format!("{}: 未选择任何模型，已跳过该账号", normalized.name));
            continue;
        }

        let (provider_id, created) =
            resolve_provider_in_transaction(&tx, &normalized, options.auto_create_providers)?;
        if created {
            result.created_providers += 1;
        }

        let credential_data = serde_json::to_string(&normalized.credential)
            .map_err(|e| format!("Cannot encode credential: {}", e))?;
        let account_id = format!("acct_{}", Uuid::new_v4().simple());
        let status = if normalized.routable {
            "active"
        } else {
            "disabled"
        };
        let health_status = if normalized.routable {
            "unchecked"
        } else {
            "adapter_required"
        };
        let mut tags = normalized.tags.clone();
        if let Some(ref default_tags) = options.default_tags {
            for tag in default_tags {
                if !tags.contains(tag) {
                    tags.push(tag.clone());
                }
            }
        }
        let account = Account {
            id: account_id.clone(),
            provider_id: Some(provider_id),
            name: Some(normalized.name.clone()),
            api_key: primary_secret(&normalized),
            models: Some(serde_json::to_string(&normalized.models).unwrap_or_else(|_| "[]".into())),
            quota_limit: None,
            quota_used: Some(0.0),
            status: Some(status.into()),
            health_status: Some(health_status.into()),
            health_code: None,
            health_msg: normalized.warning.clone(),
            health_latency: None,
            health_check_at: None,
            priority: Some(0),
            tags: Some(serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into())),
            last_used_at: None,
            created_at: None,
            credential_type: Some(normalized.credential_type.clone()),
            credential_data: Some(credential_data),
            source_format: Some(normalized.source_format.clone()),
            external_account_id: normalized.external_account_id.clone(),
            email: normalized.email.clone(),
            expires_at: normalized.expires_at.clone(),
            metadata: Some(normalized.metadata.to_string()),
            credential_fingerprint: Some(normalized.fingerprint.clone()),
            protocols: Some(
                serde_json::to_string(&normalized.protocols).unwrap_or_else(|_| "[]".into()),
            ),
            route_takeover: Some(1),
            plan_type: None,
            quota_windows: None,
            quota_refreshed_at: None,
            quota_error: None,
            token_refreshed_at: None,
            secret_ref: None,
        };

        match insert_account(&tx, &account) {
            Ok(()) => {
                secret_guard.track(&account_id);
                if let Some(plan_type) = normalized
                    .metadata
                    .get("plan_type")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.trim().is_empty())
                {
                    tx.execute(
                        "INSERT INTO account_usage (account_id, provider, plan_type, quota_windows) VALUES (?1, ?2, ?3, '[]') \
                         ON CONFLICT(account_id) DO UPDATE SET provider=excluded.provider, plan_type=excluded.plan_type",
                        rusqlite::params![account_id, normalized.provider_type, plan_type],
                    )
                    .map_err(|e| e.to_string())?;
                }
                result.imported += 1;
                if !normalized.routable {
                    result.adapter_required += 1;
                }
                result.account_ids.push(account_id);
            }
            Err(error) => result
                .errors
                .push(format!("{}: {}", normalized.name, error)),
        }
    }

    tx.commit().map_err(|e| e.to_string())?;
    secret_guard.commit();
    Ok(result)
}

/// Replace an existing account's credential and metadata (conflict mode
/// `overwrite`). Credentials always go through the OS vault; the SQLite row
/// keeps only the opaque `secret_ref`.
fn update_account_in_transaction(
    tx: &Connection,
    account_id: &str,
    normalized: &NormalizedAccount,
) -> Result<(), String> {
    let secret_ref = keychain::account_secret_ref(account_id);
    let credential_data =
        serde_json::to_string(&normalized.credential).map_err(|e| e.to_string())?;
    keychain::store_verified(&secret_ref, credential_data.as_bytes())?;

    let status = if normalized.routable {
        "active"
    } else {
        "disabled"
    };
    let health_status = if normalized.routable {
        "unchecked"
    } else {
        "adapter_required"
    };
    let models = serde_json::to_string(&normalized.models).unwrap_or_else(|_| "[]".into());
    let tags = serde_json::to_string(&normalized.tags).unwrap_or_else(|_| "[]".into());
    let protocols = serde_json::to_string(&normalized.protocols).unwrap_or_else(|_| "[]".into());
    tx.execute(
        "UPDATE accounts SET name=?1, api_key='', models=?2, status=?3, health_status=?4, \
         health_msg=?5, tags=?6, credential_type=?7, credential_data=NULL, secret_ref=?8, \
         source_format=?9, external_account_id=?10, email=?11, expires_at=?12, metadata=?13, \
         credential_fingerprint=?14, protocols=?15 WHERE id=?16",
        rusqlite::params![
            normalized.name,
            models,
            status,
            health_status,
            normalized.warning,
            tags,
            normalized.credential_type,
            secret_ref,
            normalized.source_format,
            normalized.external_account_id,
            normalized.email,
            normalized.expires_at,
            normalized.metadata.to_string(),
            normalized.fingerprint,
            protocols,
            account_id,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Merge an existing account with the imported one (conflict mode `merge`):
/// credential fields are filled in only when missing, and the DB row keeps its
/// current status. Existing values always win so a re-import never clobbers
/// live state.
fn merge_account_in_transaction(
    tx: &Connection,
    account_id: &str,
    normalized: &NormalizedAccount,
) -> Result<(), String> {
    let secret_ref = keychain::account_secret_ref(account_id);
    let stored: Option<(Option<String>, Option<String>, String)> = tx
        .query_row(
            "SELECT secret_ref, credential_data, api_key FROM accounts WHERE id=?1",
            rusqlite::params![account_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;

    // Read the existing payload (vault → legacy column → api_key column).
    let existing_payload: CredentialPayload = match stored {
        Some((Some(ref_name), _, _)) => {
            let bytes = keychain::get_secret(&ref_name)?;
            serde_json::from_slice::<CredentialPayload>(&bytes)
                .map_err(|e| format!("Stored credential payload is invalid: {}", e))?
        }
        Some((_, Some(data), _)) => serde_json::from_str::<CredentialPayload>(&data)
            .map_err(|e| format!("Stored credential payload is invalid: {}", e))?,
        Some((_, _, api_key)) if !api_key.trim().is_empty() => CredentialPayload {
            api_key: Some(api_key),
            ..CredentialPayload::default()
        },
        _ => CredentialPayload::default(),
    };

    let mut merged = existing_payload;
    let incoming = &normalized.credential;
    if merged.api_key.is_none() {
        merged.api_key = incoming.api_key.clone();
    }
    if merged.access_token.is_none() {
        merged.access_token = incoming.access_token.clone();
    }
    if merged.refresh_token.is_none() {
        merged.refresh_token = incoming.refresh_token.clone();
    }
    if merged.id_token.is_none() {
        merged.id_token = incoming.id_token.clone();
    }
    if merged.session_token.is_none() {
        merged.session_token = incoming.session_token.clone();
    }
    if merged.expires_at.is_none() {
        merged.expires_at = incoming.expires_at.clone();
    }
    if merged.metadata.is_none() {
        merged.metadata = incoming.metadata.clone();
    } else if let (Some(existing_meta), Some(incoming_meta)) =
        (merged.metadata.as_mut(), incoming.metadata.as_ref())
    {
        // Key-level merge: fill missing keys only (includes client_id / client_secret).
        if let (Some(existing_obj), Some(incoming_obj)) =
            (existing_meta.as_object_mut(), incoming_meta.as_object())
        {
            for (key, value) in incoming_obj {
                existing_obj
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
        }
    }

    let credential_data =
        serde_json::to_string(&merged).map_err(|e| format!("Cannot encode credential: {}", e))?;
    keychain::store_verified(&secret_ref, credential_data.as_bytes())?;

    let models = serde_json::to_string(&normalized.models).unwrap_or_else(|_| "[]".into());
    tx.execute(
        "UPDATE accounts SET api_key='', credential_data=NULL, secret_ref=?1, \
         models=CASE WHEN models IS NULL OR models='[]' THEN ?2 ELSE models END, \
         email=COALESCE(email, ?3), external_account_id=COALESCE(external_account_id, ?4), \
         expires_at=COALESCE(expires_at, ?5), source_format=COALESCE(source_format, ?6), \
         credential_fingerprint=COALESCE(credential_fingerprint, ?7) WHERE id=?8",
        rusqlite::params![
            secret_ref,
            models,
            normalized.email,
            normalized.external_account_id,
            normalized.expires_at,
            normalized.source_format,
            normalized.fingerprint,
            account_id,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn detect_format_from_path(path: &str) -> Result<String, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| import_error(ERR_UNREADABLE_FILE, format!("无法读取文件: {}", e)))?;
    Ok(detect_format(&content, Some(path)).as_str().to_string())
}

pub fn detect_format_name(content: &str, source_name: Option<&str>) -> String {
    detect_format(content, source_name).as_str().to_string()
}

pub(crate) fn parse_request(request: &ImportSourceRequest) -> Result<ParseBatch, String> {
    let mut sources = Vec::<(String, String)>::new();
    if let Some(content) = request
        .content
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        sources.push((
            request
                .source_name
                .clone()
                .unwrap_or_else(|| "pasted-content".into()),
            content.clone(),
        ));
    }
    for path in &request.paths {
        let content = std::fs::read_to_string(path).map_err(|e| {
            import_error(ERR_UNREADABLE_FILE, format!("无法读取 '{}': {}", path, e))
        })?;
        sources.push((path.clone(), content));
    }
    if sources.is_empty() {
        return Err(import_error(ERR_NO_SOURCE, "未提供任何导入内容或文件"));
    }

    let mut batch = ParseBatch::default();
    batch.source_count = sources.len();
    for (source_name, content) in sources {
        let format = detect_format(&content, Some(&source_name));
        batch.formats.push(format.as_str().to_string());
        parse_source(
            &content,
            &source_name,
            &format,
            request.provider_hint.as_ref(),
            &mut batch,
        )?;
    }
    Ok(batch)
}

fn detect_format(content: &str, source_name: Option<&str>) -> ImportFormat {
    let lower_name = source_name.unwrap_or_default().to_lowercase();
    if lower_name.ends_with(".csv") {
        return ImportFormat::Csv;
    }
    let trimmed = content.trim();

    // Explicit YAML / TOML file extensions take precedence over content
    // sniffing (Cockpit-tools configs are commonly `.yaml`, Codex CLI uses
    // `config.toml`). Unparseable files degrade to key-text handling.
    if lower_name.ends_with(".yaml") || lower_name.ends_with(".yml") {
        return if serde_yaml::from_str::<serde_yaml::Value>(trimmed)
            .map(|value| value.is_mapping() || value.is_sequence())
            .unwrap_or(false)
        {
            ImportFormat::Yaml
        } else {
            ImportFormat::ApiKeyText
        };
    }
    if lower_name.ends_with(".toml") {
        return if toml::from_str::<toml::Value>(trimmed).is_ok() {
            ImportFormat::Toml
        } else {
            ImportFormat::ApiKeyText
        };
    }

    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        // Not JSON: fall back to YAML then TOML before treating the content as
        // plain key text. YAML accepts almost any text as a scalar, so only
        // structured documents (mappings / sequences) count as YAML — this
        // keeps one-key-per-line API key text on the key-text path.
        if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(trimmed) {
            if value.is_mapping() || value.is_sequence() {
                return ImportFormat::Yaml;
            }
        }
        if toml::from_str::<toml::Value>(trimmed).is_ok() {
            return ImportFormat::Toml;
        }
        if trimmed
            .lines()
            .next()
            .map(|line| line.to_lowercase().contains("api_key"))
            .unwrap_or(false)
        {
            return ImportFormat::Csv;
        }
        return ImportFormat::ApiKeyText;
    };

    if lower_name.contains("sub2api") {
        return ImportFormat::Sub2Api;
    }
    if lower_name.contains("cpa") || lower_name.contains("cli-proxy") {
        return ImportFormat::Cpa;
    }
    if lower_name.contains("cockpit") || lower_name.contains("antigravity_cockpit") {
        return ImportFormat::Cockpit;
    }
    if lower_name.contains("echobird") || lower_name.contains(".echobird") {
        return ImportFormat::EchoBird;
    }

    let object = value.as_object();
    if let Some(object) = object {
        if object.contains_key("auth_mode")
            || object.contains_key("OPENAI_API_KEY")
            || object.contains_key("tokens")
            || object.contains_key("authJson")
        {
            return ImportFormat::CodexAuth;
        }
        if object
            .get("accounts")
            .and_then(|accounts| accounts.as_array())
            .map(|accounts| {
                accounts.iter().any(|account| {
                    account.get("credentials").is_some() || account.get("platform").is_some()
                })
            })
            .unwrap_or(false)
        {
            return ImportFormat::Sub2Api;
        }
        if object.get("type").and_then(|value| value.as_str()) == Some("codex") {
            if object.contains_key("session_token") {
                return ImportFormat::Cpa;
            }
            return ImportFormat::Cockpit;
        }
        if object.contains_key("providers") && object.contains_key("accounts") {
            return ImportFormat::Cpa;
        }
        if object.contains_key("data") {
            return ImportFormat::Cockpit;
        }
    }
    ImportFormat::GenericJson
}

fn parse_source(
    content: &str,
    source_name: &str,
    format: &ImportFormat,
    provider_hint: Option<&ImportProviderHint>,
    batch: &mut ParseBatch,
) -> Result<(), String> {
    match format {
        ImportFormat::Csv => parse_csv(content, source_name, format, provider_hint, batch),
        ImportFormat::ApiKeyText
            if provider_hint.and_then(|hint| hint.credential_mode.as_deref()) == Some("token") =>
        {
            parse_token_text(content, source_name, format, provider_hint.unwrap(), batch)
        }
        ImportFormat::ApiKeyText => {
            parse_api_key_text(content, source_name, format, provider_hint, batch)
        }
        ImportFormat::Yaml | ImportFormat::Toml => {
            let value = parse_structured_content(content, source_name)?;
            parse_json_value(&value, source_name, format, provider_hint, batch)
        }
        _ => {
            let value: serde_json::Value = serde_json::from_str(content).map_err(|error| {
                import_error(
                    ERR_INVALID_VALUE,
                    format!("'{}' 不是合法的 JSON：{}", source_name, error),
                )
            })?;
            parse_json_value(&value, source_name, format, provider_hint, batch)
        }
    }
}

/// Parse structured configuration content (JSON → YAML → TOML) into a JSON
/// value so the shared normalization pipeline can consume any of the three
/// dialects uniformly. Returns a categorized error when nothing parses.
fn parse_structured_content(content: &str, source_name: &str) -> Result<serde_json::Value, String> {
    let trimmed = content.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Ok(value);
    }
    // YAML accepts almost any text as a scalar, so only structured documents
    // (mappings / sequences) count — otherwise TOML or key-text content would
    // be swallowed as a YAML scalar string.
    if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(trimmed) {
        if value.is_mapping() || value.is_sequence() {
            return serde_json::to_value(value).map_err(|error| {
                import_error(
                    ERR_INVALID_VALUE,
                    format!("YAML 转换失败 in '{}': {}", source_name, error),
                )
            });
        }
    }
    if let Ok(value) = toml::from_str::<toml::Value>(trimmed) {
        return serde_json::to_value(value).map_err(|error| {
            import_error(
                ERR_INVALID_VALUE,
                format!("TOML 转换失败 in '{}': {}", source_name, error),
            )
        });
    }
    Err(import_error(
        ERR_UNSUPPORTED_FORMAT,
        format!("'{}' 内容既不是合法的 JSON、YAML 也不是 TOML", source_name),
    ))
}

fn parse_json_value(
    value: &serde_json::Value,
    source_name: &str,
    format: &ImportFormat,
    provider_hint: Option<&ImportProviderHint>,
    batch: &mut ParseBatch,
) -> Result<(), String> {
    if parse_cockpit_full_backup(value, source_name, format, batch)? {
        return Ok(());
    }
    if parse_echobird_model_catalog(value, source_name, format, batch)? {
        return Ok(());
    }

    let mut values = Vec::new();
    match value {
        serde_json::Value::Array(items) => values.extend(items.iter()),
        serde_json::Value::Object(object) => {
            if let Some(accounts) = object.get("accounts").and_then(|value| value.as_array()) {
                values.extend(accounts.iter());
            } else if let Some(data) = object.get("data") {
                match data {
                    serde_json::Value::Array(items) => values.extend(items.iter()),
                    serde_json::Value::Object(_) => values.push(data),
                    _ => batch
                        .skipped
                        .push(format!("{}: data is not an account object", source_name)),
                }
            } else {
                values.push(value);
            }
        }
        _ => return Err(format!("{} does not contain account JSON", source_name)),
    }

    for (index, item) in values.into_iter().enumerate() {
        let enriched;
        let item = if let (Some(hint), Some(object)) = (provider_hint, item.as_object()) {
            let mut object = object.clone();
            object
                .entry("provider_name")
                .or_insert_with(|| hint.name.clone().into());
            object
                .entry("provider")
                .or_insert_with(|| hint.id.clone().into());
            object
                .entry("protocol")
                .or_insert_with(|| hint.protocol.clone().into());
            object
                .entry("base_url")
                .or_insert_with(|| hint.base_url.clone().into());
            if let Some(base_urls) = hint.base_urls.as_ref() {
                object
                    .entry("base_urls")
                    .or_insert_with(|| serde_json::json!(base_urls));
            }
            object
                .entry("models")
                .or_insert_with(|| serde_json::json!(hint.models));
            let mut merged_tags = string_vec(object.get("tags"));
            for tag in &hint.tags {
                if !merged_tags.contains(tag) {
                    merged_tags.push(tag.clone());
                }
            }
            object.insert("tags".into(), serde_json::json!(merged_tags));
            if (hint.id == "codex" || hint.id == "openai")
                && !object.contains_key("api_key")
                && !object.contains_key("OPENAI_API_KEY")
            {
                object.entry("type").or_insert_with(|| "codex".into());
            }
            enriched = serde_json::Value::Object(object);
            &enriched
        } else {
            item
        };
        match normalize_account(
            item,
            source_name,
            format,
            provider_hint.and_then(|hint| hint.protocols.clone()),
        ) {
            Ok(account) => batch.accounts.push(account),
            Err(error) => batch
                .skipped
                .push(format!("{} #{}: {}", source_name, index + 1, error)),
        }
    }
    Ok(())
}

/// Cockpit Tools full backups keep account arrays under
/// `accounts.platforms.<platform>.exported_data`. Flatten those entries and
/// enrich them with a provider identity before sending them through the shared
/// normalization pipeline.
fn parse_cockpit_full_backup(
    value: &serde_json::Value,
    source_name: &str,
    format: &ImportFormat,
    batch: &mut ParseBatch,
) -> Result<bool, String> {
    let Some(platforms) = value
        .get("accounts")
        .and_then(|accounts| accounts.get("platforms"))
        .and_then(|platforms| platforms.as_object())
    else {
        return Ok(false);
    };

    let mut found = false;
    for (platform, block) in platforms {
        let Some(accounts) = block.get("exported_data").and_then(|data| data.as_array()) else {
            continue;
        };
        for (index, account) in accounts.iter().enumerate() {
            let Some(raw) = account.as_object() else {
                batch.skipped.push(format!(
                    "{} {} #{}: account entry is not an object",
                    source_name,
                    platform,
                    index + 1
                ));
                continue;
            };
            let mut object = raw.clone();
            object
                .entry("provider")
                .or_insert_with(|| serde_json::Value::String(platform.clone()));
            object
                .entry("provider_name")
                .or_insert_with(|| serde_json::Value::String(cockpit_platform_name(platform)));
            if platform == "codex" {
                object.entry("type").or_insert_with(|| "codex".into());
                if let Some(api_key) = object.remove("openai_api_key") {
                    object.entry("api_key").or_insert(api_key);
                }
                if let Some(base_url) = object.remove("api_base_url") {
                    object.entry("base_url").or_insert(base_url);
                }
                if let Some(models) = object.get("api_model_catalog").cloned() {
                    object.entry("models").or_insert(models);
                }
            }
            let normalized_value = serde_json::Value::Object(object);
            match normalize_account(&normalized_value, source_name, format, None) {
                Ok(account) => {
                    batch.accounts.push(account);
                    found = true;
                }
                Err(error) => batch.skipped.push(format!(
                    "{} {} #{}: {}",
                    source_name,
                    platform,
                    index + 1,
                    error
                )),
            }
        }
    }
    Ok(found)
}

fn cockpit_platform_name(platform: &str) -> String {
    match platform {
        "codex" => "OpenAI Codex".into(),
        "claude_manager" | "claude" => "Anthropic Claude".into(),
        "antigravity" | "antigravity_ide" => "Google Antigravity".into(),
        "kiro" => "Kiro".into(),
        "cursor" => "Cursor".into(),
        "grok" => "xAI Grok".into(),
        "codebuddy" => "CodeBuddy".into(),
        "codebuddy_cn" => "CodeBuddy CN".into(),
        "qoder" => "Qoder".into(),
        "zcode" => "ZCode".into(),
        "trae" | "trae_solo" => "Trae".into(),
        "trae_cn" | "trae_solo_cn" => "Trae CN".into(),
        "workbuddy" => "WorkBuddy".into(),
        _ => platform.replace(['_', '-'], " "),
    }
}

/// EchoBird stores cloud models as an array in `~/.echobird/config/models.json`.
/// It also writes single-model tool configs (`codex.json`, `grok.json`, etc.).
/// Both forms are normalized into one account per credential/model tuple.
fn parse_echobird_model_catalog(
    value: &serde_json::Value,
    source_name: &str,
    format: &ImportFormat,
    batch: &mut ParseBatch,
) -> Result<bool, String> {
    if *format != ImportFormat::EchoBird {
        return Ok(false);
    }
    let candidates: Vec<&serde_json::Value> = match value {
        serde_json::Value::Array(items) => items.iter().collect(),
        serde_json::Value::Object(object)
            if object.contains_key("apiKey")
                && (object.contains_key("modelId") || object.contains_key("actualModel")) =>
        {
            vec![value]
        }
        _ => return Ok(false),
    };

    let mut found = false;
    for (index, item) in candidates.into_iter().enumerate() {
        let Some(raw) = item.as_object() else {
            continue;
        };
        let api_key = string_at(raw, &["apiKey", "api_key"]);
        let base_url = string_at(raw, &["baseUrl", "base_url"]);
        if api_key.is_none() || base_url.is_none() {
            continue;
        }
        let model = string_at(raw, &["modelId", "actualModel"]);
        let provider_name = string_at(raw, &["name", "modelName", "providerId"])
            .unwrap_or_else(|| "EchoBird Cloud Model".into());
        let mut object = raw.clone();
        object.insert("provider_name".into(), provider_name.clone().into());
        object.insert("provider".into(), "echobird".into());
        object.insert(
            "name".into(),
            format!("{} (EchoBird)", provider_name).into(),
        );
        object.insert("api_key".into(), api_key.unwrap().into());
        object.insert("base_url".into(), base_url.unwrap().into());
        object.insert("protocol".into(), "chat".into());
        if let Some(anthropic_url) =
            string_at(raw, &["anthropicUrl", "anthropic_url"]).filter(|url| !url.is_empty())
        {
            object.insert("protocols".into(), serde_json::json!(["chat", "anthropic"]));
            object.insert(
                "base_urls".into(),
                serde_json::json!({
                    "chat": string_at(raw, &["baseUrl", "base_url"]).unwrap_or_default(),
                    "anthropic": anthropic_url,
                }),
            );
        }
        if let Some(model) = model.filter(|model| !model.is_empty()) {
            object.insert("models".into(), serde_json::json!([model]));
        }
        object.insert("tags".into(), serde_json::json!(["echobird", "local_scan"]));
        let normalized_value = serde_json::Value::Object(object);
        match normalize_account(&normalized_value, source_name, format, None) {
            Ok(account) => {
                batch.accounts.push(account);
                found = true;
            }
            Err(error) => batch.skipped.push(format!(
                "{} EchoBird #{}: {}",
                source_name,
                index + 1,
                error
            )),
        }
    }
    Ok(found)
}

fn normalize_account(
    value: &serde_json::Value,
    source_name: &str,
    format: &ImportFormat,
    override_protocols: Option<Vec<String>>,
) -> Result<NormalizedAccount, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "account entry is not an object".to_string())?;
    let credentials = object
        .get("credentials")
        .and_then(|value| value.as_object());
    let tokens = object.get("tokens").and_then(|value| value.as_object());
    // Codex CLI `config.toml` keeps tokens under `[auth]`.
    let auth = object.get("auth").and_then(|value| value.as_object());
    // Cockpit-tools style: `oauth: { access_token, refresh_token, client_id, ... }`.
    let oauth = object.get("oauth").and_then(|value| value.as_object());
    // Cockpit "Codex Tools" (`accounts.json`) wraps a Codex CLI `auth.json`
    // inside `authJson`, with tokens under `authJson.tokens`.
    let auth_json = object.get("authJson").and_then(|value| value.as_object());
    let auth_tokens = auth_json.and_then(|m| m.get("tokens").and_then(|v| v.as_object()));
    let source_kind = string_at(object, &["sourceKind", "source_kind"]).map(|v| v.to_lowercase());

    let mut api_key = string_at(object, &["api_key", "key", "OPENAI_API_KEY"])
        .or_else(|| {
            credentials.and_then(|map| string_at(map, &["api_key", "key", "OPENAI_API_KEY"]))
        })
        .or_else(|| auth_json.and_then(|map| string_at(map, &["OPENAI_API_KEY"])));
    let access_token = string_at(object, &["access_token", "accessToken"])
        .or_else(|| credentials.and_then(|map| string_at(map, &["access_token", "accessToken"])))
        .or_else(|| tokens.and_then(|map| string_at(map, &["access_token", "accessToken"])))
        .or_else(|| auth.and_then(|map| string_at(map, &["access_token", "accessToken"])))
        .or_else(|| oauth.and_then(|map| string_at(map, &["access_token", "accessToken"])))
        .or_else(|| auth_tokens.and_then(|map| string_at(map, &["access_token", "accessToken"])));
    let refresh_token = string_at(object, &["refresh_token", "refreshToken"])
        .or_else(|| credentials.and_then(|map| string_at(map, &["refresh_token", "refreshToken"])))
        .or_else(|| tokens.and_then(|map| string_at(map, &["refresh_token", "refreshToken"])))
        .or_else(|| auth.and_then(|map| string_at(map, &["refresh_token", "refreshToken"])))
        .or_else(|| oauth.and_then(|map| string_at(map, &["refresh_token", "refreshToken"])))
        .or_else(|| auth_tokens.and_then(|map| string_at(map, &["refresh_token", "refreshToken"])));
    let id_token = string_at(object, &["id_token", "idToken"])
        .or_else(|| credentials.and_then(|map| string_at(map, &["id_token", "idToken"])))
        .or_else(|| tokens.and_then(|map| string_at(map, &["id_token", "idToken"])))
        .or_else(|| auth.and_then(|map| string_at(map, &["id_token", "idToken"])))
        .or_else(|| oauth.and_then(|map| string_at(map, &["id_token", "idToken"])))
        .or_else(|| auth_tokens.and_then(|map| string_at(map, &["id_token", "idToken"])));
    let session_token = string_at(object, &["session_token", "sessionToken"])
        .or_else(|| credentials.and_then(|map| string_at(map, &["session_token", "sessionToken"])))
        .or_else(|| auth.and_then(|map| string_at(map, &["session_token", "sessionToken"])))
        .or_else(|| oauth.and_then(|map| string_at(map, &["session_token", "sessionToken"])))
        .or_else(|| auth_tokens.and_then(|map| string_at(map, &["session_token", "sessionToken"])));

    if api_key.is_none() && access_token.is_none() && refresh_token.is_none() {
        return Err(import_error(
            ERR_MISSING_FIELD,
            "缺少 API Key 或 OAuth 令牌（api_key / access_token / refresh_token）",
        ));
    }

    let raw_type = string_at(object, &["type", "account_type", "auth_mode"])
        .unwrap_or_default()
        .to_lowercase();
    let raw_provider = string_at(object, &["provider_name", "provider", "platform"])
        .unwrap_or_else(|| "openai".into());
    let provider_key = provider_identity_key(&raw_provider);
    let is_codex = raw_type == "codex"
        || string_at(object, &["auth_mode"])
            .map(|value| value.to_lowercase() == "chatgpt")
            .unwrap_or(false)
        || source_kind.as_deref() == Some("chatgpt")
        || auth_json.is_some()
        || object.get("agent_identity").is_some()
        || object.get("agentIdentity").is_some();
    let raw_protocol =
        string_at(object, &["protocol"]).unwrap_or_else(|| match provider_key.as_str() {
            "anthropic" | "claude" => "anthropic".into(),
            "gemini" | "google" | "googleantigravity" | "antigravity" => "gemini".into(),
            "codex" => "responses".into(),
            _ => "chat".into(),
        });
    let protocol = canonical_protocol(&raw_protocol, is_codex);
    let protocols = override_protocols
        .or_else(|| object.get("protocols").map(|value| string_vec(Some(value))))
        .map(|values| canonical_protocols(values, is_codex))
        .filter(|values| !values.is_empty())
        .unwrap_or_else(|| vec![protocol.clone()]);
    let default_url = if provider_key.as_str() == "antigravity" {
        crate::services::antigravity_adapter::ANTIGRAVITY_BASE_URL.to_string()
    } else {
        default_base_url(&protocol)
    };
    // Store the raw URL as-is; normalization only happens at routing/request time.
    let mut base_url = string_at(object, &["base_url", "api_base_url", "apiBaseUrl"])
        .or_else(|| {
            credentials.and_then(|map| string_at(map, &["base_url", "api_base_url", "apiBaseUrl"]))
        })
        .or_else(|| string_at(object, &["apiBaseUrl"]))
        .unwrap_or(default_url)
        .trim()
        .to_string();
    let mut base_urls = object
        .get("base_urls")
        .and_then(|value| serde_json::from_value::<HashMap<String, String>>(value.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|(key, value)| {
            let proto = canonical_protocol(&key, is_codex);
            (proto.clone(), value.trim().to_string())
        })
        .filter(|(_, value)| !value.is_empty())
        .collect::<HashMap<_, _>>();
    base_urls
        .entry(protocol.clone())
        .or_insert_with(|| base_url.clone());
    let is_relay = source_kind.as_deref() == Some("relay");
    let provider_name =
        string_at(object, &["provider_name"]).unwrap_or_else(|| match provider_key.as_str() {
            "anthropic" | "claude" => "Anthropic".into(),
            "gemini" | "google" => "Google Gemini".into(),
            "codex" => "OpenAI Codex".into(),
            "openai" if raw_type == "codex" => "OpenAI Codex".into(),
            "openai" if is_relay => {
                // Cockpit "Codex Tools" relay accounts (apiBaseUrl + apiKey) —
                // fall back to the user-visible label or raw provider string.
                string_at(object, &["label"]).unwrap_or_else(|| raw_provider.clone())
            }
            "openai" => "OpenAI".into(),
            _ => raw_provider.clone(),
        });

    let is_upstream_gateway = api_key.is_some()
        && string_at(
            object,
            &["base_url", "api_base_url", "apiBaseUrl", "apiBaseUrl"],
        )
        .is_some()
        && !is_official_base_url(&base_url);
    let mut credential_type = if is_upstream_gateway {
        "upstream_key"
    } else if api_key.is_some() {
        "api_key"
    } else if is_codex {
        "codex_oauth"
    } else if access_token.is_some() {
        "oauth"
    } else {
        "token"
    }
    .to_string();

    // Relay override: sourceKind=relay must always be routed via the
    // provider's API key (often a third-party token-plan service).
    if is_relay {
        if let Some(api) = string_at(object, &["apiKey"])
            .or_else(|| auth_json.and_then(|map| string_at(map, &["OPENAI_API_KEY"])))
            .filter(|value| !value.is_empty())
        {
            api_key = Some(api);
            if let Some(url) = string_at(object, &["apiBaseUrl"]).filter(|v| !v.is_empty()) {
                base_url = url.trim().to_string();
            }
            credential_type = "upstream_key".to_string();
        }
    }

    let routable = true;
    let adapter = if credential_type == "codex_oauth" {
        "codex_responses"
    } else if credential_type == "upstream_key" {
        "openai_compatible_upstream"
    } else {
        protocol.as_str()
    }
    .to_string();
    let warning = (credential_type == "codex_oauth")
        .then(|| "Codex OAuth 仅限本机本人账号私有使用；禁止共享、公开代理或号池轮询".to_string());

    let email = string_at(object, &["email", "user_email", "principalId"])
        .or_else(|| credentials.and_then(|map| string_at(map, &["email", "user_email"])))
        .or_else(|| infer_email_from_filename(source_name));
    let external_account_id = string_at(object, &["account_id", "accountId", "chatgpt_account_id"])
        .or_else(|| {
            credentials
                .and_then(|map| string_at(map, &["account_id", "accountId", "chatgpt_account_id"]))
        })
        .or_else(|| tokens.and_then(|map| string_at(map, &["account_id", "accountId"])))
        .or_else(|| oauth.and_then(|map| string_at(map, &["account_id", "accountId"])))
        .or_else(|| auth_tokens.and_then(|map| string_at(map, &["account_id", "accountId"])));
    let expires_at = string_at(object, &["expires_at", "expired", "expiry", "expiresAt"])
        .or_else(|| {
            credentials
                .and_then(|map| string_at(map, &["expires_at", "expired", "expiry", "expiresAt"]))
        })
        .or_else(|| {
            oauth.and_then(|map| string_at(map, &["expires_at", "expired", "expiry", "expiresAt"]))
        })
        .or_else(|| {
            auth_tokens
                .and_then(|map| string_at(map, &["expires_at", "expired", "expiry", "expiresAt"]))
        })
        .or_else(|| auth_json.and_then(|map| string_at(map, &["last_refresh"])));
    let mut models = string_vec(object.get("models"));
    for key in ["model", "modelName", "model_id", "modelId", "actualModel"] {
        if let Some(model) = string_at(object, &[key]).filter(|model| !model.is_empty()) {
            if !models.contains(&model) {
                models.push(model);
            }
        }
    }
    if let Some(catalog) = object.get("api_model_catalog") {
        match catalog {
            serde_json::Value::Array(items) => {
                for item in items {
                    let model = item
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| {
                            item.as_object()
                                .and_then(|map| string_at(map, &["id", "name", "model"]))
                        })
                        .filter(|model| !model.is_empty());
                    if let Some(model) = model.filter(|model| !models.contains(model)) {
                        models.push(model);
                    }
                }
            }
            serde_json::Value::Object(map) => {
                for model in map.keys() {
                    if !models.contains(model) {
                        models.push(model.clone());
                    }
                }
            }
            _ => {}
        }
    }
    let mut tags = string_vec(object.get("tags"));
    if !tags.iter().any(|tag| tag == format.as_str()) {
        tags.push(format.as_str().to_string());
    }
    let name = string_at(object, &["name", "display_name", "label"])
        .or_else(|| email.clone())
        .or_else(|| {
            external_account_id
                .as_ref()
                .map(|id| format!("{}-{}", provider_name, short_value(id)))
        })
        .unwrap_or_else(|| format!("{} account", provider_name));
    let agent_identity = object
        .get("agent_identity")
        .or_else(|| object.get("agentIdentity"))
        .cloned();
    // OAuth client credentials may appear at the top level, under a nested
    // `credentials` block, or under an `oauth` object (Cockpit-tools style).
    // They are stored inside `CredentialPayload.metadata` only, which lives in
    // the OS vault — never in the plaintext `accounts.metadata` column — so
    // token refresh can reuse the same client identity used by the source tool.
    let client_id = string_at(object, &["client_id", "clientId"])
        .or_else(|| credentials.and_then(|map| string_at(map, &["client_id", "clientId"])))
        .or_else(|| auth.and_then(|map| string_at(map, &["client_id", "clientId"])))
        .or_else(|| oauth.and_then(|map| string_at(map, &["client_id", "clientId"])))
        .or_else(|| auth_json.and_then(|map| string_at(map, &["client_id", "clientId"])));
    let client_secret = string_at(object, &["client_secret", "clientSecret"])
        .or_else(|| credentials.and_then(|map| string_at(map, &["client_secret", "clientSecret"])))
        .or_else(|| auth.and_then(|map| string_at(map, &["client_secret", "clientSecret"])))
        .or_else(|| oauth.and_then(|map| string_at(map, &["client_secret", "clientSecret"])));
    // Plaintext (DB) metadata: non-sensitive bookkeeping only.
    let metadata = serde_json::json!({
        "source_name": source_name,
        "adapter": adapter,
        "raw_type": raw_type,
        "plan_type": string_at(object, &["plan_type", "planType"]),
    });
    // Vault metadata: same fields plus OAuth client credentials (secret).
    let mut credential_metadata = metadata.clone();
    if let Some(value) = client_id {
        credential_metadata["client_id"] = serde_json::json!(value);
    }
    if let Some(value) = client_secret {
        credential_metadata["client_secret"] = serde_json::json!(value);
    }
    let credential = CredentialPayload {
        api_key,
        access_token,
        refresh_token,
        id_token,
        session_token,
        account_id: external_account_id.clone(),
        expires_at: expires_at.clone(),
        token_type: string_at(object, &["token_type", "tokenType"]),
        base_url: Some(base_url.clone()),
        agent_identity,
        metadata: Some(credential_metadata),
    };
    let secret = credential
        .api_key
        .as_ref()
        .or(credential.access_token.as_ref())
        .or(credential.refresh_token.as_ref())
        .ok_or_else(|| "credential has no usable secret".to_string())?;
    let fingerprint = credential_fingerprint(&provider_name, secret);

    Ok(NormalizedAccount {
        name,
        email,
        provider_name,
        provider_type: if is_codex {
            "codex".into()
        } else if provider_key.as_str() == "antigravity" {
            // Keep Antigravity distinct from plain Gemini so the adapter can
            // route to the Cloud Code v1internal upstream.
            "antigravity".into()
        } else {
            protocol.clone()
        },
        protocol: protocol.clone(),
        protocols,
        base_url,
        base_urls,
        credential_type,
        credential,
        source_format: format.as_str().to_string(),
        external_account_id,
        expires_at,
        models,
        tags,
        metadata,
        fingerprint,
        routable,
        adapter,
        warning,
    })
}

fn parse_token_text(
    content: &str,
    source_name: &str,
    format: &ImportFormat,
    hint: &ImportProviderHint,
    batch: &mut ParseBatch,
) -> Result<(), String> {
    for (index, line) in content.lines().enumerate() {
        let token = line.trim().trim_matches(',');
        if token.is_empty() || token.starts_with('#') {
            continue;
        }
        if token.starts_with("http://") || token.starts_with("https://") {
            batch.skipped.push(format!(
                "{} #{}: URL 不能作为 Token",
                source_name,
                index + 1
            ));
            continue;
        }
        let is_refresh = token.starts_with("rt_")
            || token.to_lowercase().starts_with("refresh_token=")
            || token.to_lowercase().starts_with("refresh-token=");
        let token = token
            .split_once('=')
            .map(|(_, value)| value.trim())
            .unwrap_or(token);
        let mut value = serde_json::json!({
            "name": format!("{} account", hint.name),
            "provider_name": hint.name,
            "provider": hint.id,
            "protocol": hint.protocol,
            "base_url": hint.base_url,
            "base_urls": hint.base_urls,
            "models": hint.models,
            "tags": hint.tags,
        });
        if hint.id == "codex" || hint.id == "openai" {
            value["type"] = serde_json::Value::String("codex".into());
        }
        value[if is_refresh {
            "refresh_token"
        } else {
            "access_token"
        }] = serde_json::Value::String(token.to_string());
        match normalize_account(&value, source_name, format, hint.protocols.clone()) {
            Ok(account) => batch.accounts.push(account),
            Err(error) => batch
                .skipped
                .push(format!("{} #{}: {}", source_name, index + 1, error)),
        }
    }
    Ok(())
}

fn parse_api_key_text(
    content: &str,
    source_name: &str,
    format: &ImportFormat,
    provider_hint: Option<&ImportProviderHint>,
    batch: &mut ParseBatch,
) -> Result<(), String> {
    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim().trim_matches(',');
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (name, key) = trimmed
            .split_once('=')
            .map(|(name, key)| (Some(name.trim().to_string()), key.trim().to_string()))
            .unwrap_or((None, trimmed.to_string()));
        if key.starts_with("http://") || key.starts_with("https://") {
            batch.skipped.push(format!(
                "{} #{}: URL 不能作为 API Key",
                source_name,
                index + 1
            ));
            continue;
        }
        let mut value = serde_json::json!({"name": name, "api_key": key});
        if let Some(hint) = provider_hint {
            value["provider_name"] = hint.name.clone().into();
            value["provider"] = hint.id.clone().into();
            value["protocol"] = hint.protocol.clone().into();
            value["base_url"] = hint.base_url.clone().into();
            if let Some(base_urls) = hint.base_urls.as_ref() {
                value["base_urls"] = serde_json::json!(base_urls);
            }
            value["models"] = serde_json::json!(hint.models);
            value["tags"] = serde_json::json!(hint.tags);
        }
        match normalize_account(
            &value,
            source_name,
            format,
            provider_hint.and_then(|hint| hint.protocols.clone()),
        ) {
            Ok(account) => batch.accounts.push(account),
            Err(error) => batch
                .skipped
                .push(format!("{} #{}: {}", source_name, index + 1, error)),
        }
    }
    Ok(())
}

fn parse_csv(
    content: &str,
    source_name: &str,
    format: &ImportFormat,
    provider_hint: Option<&ImportProviderHint>,
    batch: &mut ParseBatch,
) -> Result<(), String> {
    let mut lines = content.lines().filter(|line| !line.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| format!("{} is empty", source_name))?;
    let headers: Vec<String> = split_csv_line(header)
        .into_iter()
        .map(|value| value.to_lowercase())
        .collect();
    for (row_index, line) in lines.enumerate() {
        let fields = split_csv_line(line);
        let mut object = serde_json::Map::new();
        for (index, header) in headers.iter().enumerate() {
            if let Some(value) = fields.get(index).filter(|value| !value.is_empty()) {
                object.insert(header.clone(), serde_json::Value::String(value.clone()));
            }
        }
        if let Some(hint) = provider_hint {
            object
                .entry("provider_name")
                .or_insert_with(|| hint.name.clone().into());
            object
                .entry("provider")
                .or_insert_with(|| hint.id.clone().into());
            object
                .entry("protocol")
                .or_insert_with(|| hint.protocol.clone().into());
            object
                .entry("base_url")
                .or_insert_with(|| hint.base_url.clone().into());
            if let Some(base_urls) = hint.base_urls.as_ref() {
                object
                    .entry("base_urls")
                    .or_insert_with(|| serde_json::json!(base_urls));
            }
            object
                .entry("models")
                .or_insert_with(|| serde_json::json!(hint.models));
            let mut merged_tags = string_vec(object.get("tags"));
            for tag in &hint.tags {
                if !merged_tags.contains(tag) {
                    merged_tags.push(tag.clone());
                }
            }
            object.insert("tags".into(), serde_json::json!(merged_tags));
            if (hint.id == "codex" || hint.id == "openai")
                && !object.contains_key("api_key")
                && !object.contains_key("OPENAI_API_KEY")
            {
                object.entry("type").or_insert_with(|| "codex".into());
            }
        }
        match normalize_account(
            &serde_json::Value::Object(object),
            source_name,
            format,
            provider_hint.and_then(|hint| hint.protocols.clone()),
        ) {
            Ok(account) => batch.accounts.push(account),
            Err(error) => {
                batch
                    .skipped
                    .push(format!("{} row {}: {}", source_name, row_index + 2, error))
            }
        }
    }
    Ok(())
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in line.chars() {
        match character {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    fields.push(current.trim().to_string());
    fields
}

fn resolve_provider_in_transaction(
    conn: &Connection,
    account: &NormalizedAccount,
    auto_create: bool,
) -> Result<(String, bool), String> {
    let provider_key = provider_identity_key(&account.provider_name);
    let is_official = is_official_identity_key(&provider_key);
    let mut stmt = conn
        .prepare("SELECT id, name, protocols, base_urls, models FROM providers")
        .map_err(|e| e.to_string())?;
    let existing = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .find(|(id, name, _, base_urls, _)| {
            let same_key = provider_identity_key(name) == provider_key;
            if !same_key {
                return false;
            }
            // Known official upstreams (openai/anthropic/google/antigravity/xai)
            // are intended to be a single shared row per upstream. Custom /
            // user-defined providers must only be reused when BOTH name and
            // Base URL match — otherwise distinct custom providers that happen
            // to share a generic name ("custom", "自定义", empty default…)
            // would be collapsed into one row and have their Base URL / API
            // key rewritten for every attached account.
            if is_official {
                return true;
            }
            let existing_url = base_urls
                .as_deref()
                .and_then(|raw| serde_json::from_str::<HashMap<String, String>>(raw).ok())
                .and_then(|urls| urls.values().next().cloned())
                .unwrap_or_default();
            let incoming_url = account
                .base_urls
                .values()
                .next()
                .cloned()
                .unwrap_or_else(|| account.base_url.clone());
            // Preserve the historical "don't overwrite a configured URL with an
            // empty/default one" behaviour: an empty incoming URL is treated as
            // a match (so we reuse + fill the row) rather than spawning a dup.
            let same_url = incoming_url.is_empty()
                || existing_url.is_empty()
                || same_base_url(&existing_url, &incoming_url);
            // Guard against the degenerate case of two custom providers with
            // the exact same name AND Base URL but distinct credentials — that
            // is handled downstream via credential fingerprinting, not here.
            let _ = id;
            name.trim()
                .eq_ignore_ascii_case(account.provider_name.trim())
                && same_url
        });
    drop(stmt);
    if let Some((id, _name, existing_protocols, existing_base_urls, existing_models)) = existing {
        let mut protocols = crate::proxy::router::parse_protocols(existing_protocols.as_deref());
        for protocol in &account.protocols {
            if !protocols.contains(protocol) {
                protocols.push(protocol.clone());
            }
        }
        let mut base_urls = existing_base_urls
            .as_deref()
            .and_then(|raw| serde_json::from_str::<HashMap<String, String>>(raw).ok())
            .unwrap_or_default();
        base_urls.extend(account.base_urls.clone());
        let mut models = existing_models
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
            .unwrap_or_default();
        for model in &account.models {
            if !models
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(model))
            {
                models.push(model.clone());
            }
        }
        // Only update base_url if existing is empty or if new one is non-empty and different.
        // This prevents overwriting a user-configured URL with a default/empty one.
        let existing_base_url = existing_base_urls
            .as_deref()
            .and_then(|raw| serde_json::from_str::<HashMap<String, String>>(raw).ok())
            .and_then(|urls| urls.values().next().cloned())
            .unwrap_or_default();
        let new_base_url = if !account.base_url.is_empty() && account.base_url != existing_base_url
        {
            account.base_url.clone()
        } else {
            existing_base_url
        };
        conn.execute(
            "UPDATE providers SET type=?1, protocol=?2, protocols=?3, base_url=?4, base_urls=?5, models=?6 WHERE id=?7",
            rusqlite::params![
                account.provider_type,
                account.protocol,
                serde_json::to_string(&protocols).unwrap_or_else(|_| "[]".into()),
                new_base_url,
                serde_json::to_string(&base_urls).unwrap_or_else(|_| "{}".into()),
                serde_json::to_string(&models).unwrap_or_else(|_| "[]".into()),
                id,
            ],
        )
        .map_err(|e| e.to_string())?;
        return Ok((id, false));
    }
    if !auto_create {
        return Err(format!(
            "Provider '{}' does not exist",
            account.provider_name
        ));
    }

    let id = format!("prov_{}", Uuid::new_v4().simple());
    let protocols_json = serde_json::to_string(&account.protocols).unwrap_or_else(|_| "[]".into());
    let base_urls_json = serde_json::to_string(&account.base_urls).unwrap_or_else(|_| "{}".into());
    conn.execute(
        "INSERT INTO providers (id, name, type, base_url, base_urls, protocol, protocols, route_takeover, models, timeout_ms, priority, enabled) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, 30000, 0, 1)",
        rusqlite::params![
            id,
            account.provider_name,
            account.provider_type,
            account.base_url,
            base_urls_json,
            account.protocol,
            protocols_json,
            serde_json::to_string(&account.models).unwrap_or_else(|_| "[]".into())
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok((id, true))
}

pub(crate) fn load_existing_fingerprints(
    conn: &Mutex<Connection>,
) -> Result<HashSet<String>, String> {
    let conn = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT credential_fingerprint FROM accounts WHERE credential_fingerprint IS NOT NULL",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<HashSet<_>, _>>()
        .map_err(|e| e.to_string())
}

pub(crate) fn load_existing_provider_names(
    conn: &Mutex<Connection>,
) -> Result<HashSet<String>, String> {
    let conn = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT lower(name) FROM providers")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<HashSet<_>, _>>()
        .map_err(|e| e.to_string())
}

fn string_at(map: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key).and_then(|value| match value {
            serde_json::Value::String(value) if !value.trim().is_empty() => {
                Some(value.trim().to_string())
            }
            serde_json::Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
    })
}

fn string_vec(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(|value| value.as_str().map(|value| value.trim().to_string()))
            .filter(|value| !value.is_empty())
            .collect(),
        Some(serde_json::Value::String(value)) => value
            .split([',', ';'])
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn infer_email_from_filename(source_name: &str) -> Option<String> {
    let filename = std::path::Path::new(source_name)
        .file_stem()?
        .to_string_lossy();
    let decoded = filename.replace("_at_", "@").replace("_AT_", "@");
    decoded.contains('@').then(|| decoded.to_string())
}

fn canonical_protocol(raw: &str, is_codex: bool) -> String {
    if is_codex {
        return "responses".into();
    }
    match raw.trim().to_lowercase().as_str() {
        "responses" | "response" | "openai_responses" => "responses".into(),
        "anthropic" | "messages" | "anthropic_messages" | "claude" => "anthropic".into(),
        "gemini" | "google" | "google_gemini" => "gemini".into(),
        "chat" | "openai" | "chat_completions" | "openai_chat" => "chat".into(),
        _ => "chat".into(),
    }
}

fn canonical_protocols(values: Vec<String>, is_codex: bool) -> Vec<String> {
    let mut protocols = Vec::new();
    for value in values {
        let protocol = canonical_protocol(&value, is_codex);
        if !protocols.contains(&protocol) {
            protocols.push(protocol);
        }
    }
    protocols
}

fn default_base_url(protocol: &str) -> String {
    match protocol.to_lowercase().as_str() {
        "anthropic" => "https://api.anthropic.com".into(),
        "gemini" => "https://generativelanguage.googleapis.com".into(),
        "antigravity" => crate::services::antigravity_adapter::ANTIGRAVITY_BASE_URL.into(),
        _ => "https://api.openai.com".into(),
    }
}

fn is_official_base_url(base_url: &str) -> bool {
    [
        "api.openai.com",
        "api.anthropic.com",
        "generativelanguage.googleapis.com",
    ]
    .iter()
    .any(|host| base_url.contains(host))
}

pub(crate) fn primary_secret(account: &NormalizedAccount) -> String {
    account
        .credential
        .api_key
        .clone()
        .or_else(|| account.credential.access_token.clone())
        .or_else(|| account.credential.refresh_token.clone())
        .unwrap_or_default()
}

fn credential_fingerprint(provider: &str, secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider.to_lowercase().as_bytes());
    hasher.update(b"\0");
    hasher.update(secret.trim().as_bytes());
    hex::encode(hasher.finalize())
}

pub(crate) fn short_fingerprint(value: &str) -> String {
    value.chars().take(12).collect()
}

fn short_value(value: &str) -> String {
    value.chars().take(8).collect()
}

pub(crate) fn mask_secret(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.is_empty() {
        return "未提供".into();
    }
    if chars.len() <= 8 {
        return "••••••••".into();
    }
    let prefix: String = chars.iter().take(3).collect();
    let suffix: String = chars.iter().rev().take(4).rev().collect();
    format!("{}••••••{}", prefix, suffix)
}

/// A configuration file discovered by `scan_agent_configs`. Shown to the user
/// before any import so they can pick which sources to load. `path` can be fed
/// straight back into `ImportSourceRequest.paths`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredConfig {
    pub path: String,
    /// Origin tool: `codex`, `codex_tools`, `cockpit`, `echobird`, etc.
    pub kind: String,
    /// Result of `detect_format` (e.g. `codex_auth`, `echobird`, `toml`).
    pub format: String,
    pub size: u64,
    pub modified_at: Option<String>,
    /// Accounts parsed from this source. Secrets are always masked.
    pub accounts: Vec<DiscoveredAccount>,
    /// Deduplicated model names found in the parsed accounts.
    pub models: Vec<String>,
    /// Non-fatal parse diagnostics for entries that could not be imported.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredAccount {
    pub name: String,
    pub email: Option<String>,
    pub provider_name: String,
    pub credential_type: String,
    pub masked_credential: String,
    pub models: Vec<String>,
    /// Full credential fingerprint used for account-level selection during sync.
    pub fingerprint: String,
    pub routable: bool,
    pub warning: Option<String>,
    /// Origin applications merged across every duplicate occurrence found on disk.
    pub source_apps: Vec<String>,
    /// Internal source paths needed to reparse the selected account on import.
    /// The UI deliberately does not display these paths.
    pub source_paths: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredModelResource {
    /// Stable selection key: SHA-256(provider identity + normalized model name).
    pub key: String,
    pub provider_name: String,
    pub model: String,
    pub source_apps: Vec<String>,
    pub account_fingerprints: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentConfigScanResult {
    pub accounts: Vec<DiscoveredAccount>,
    pub model_resources: Vec<DiscoveredModelResource>,
    /// Internal source paths required for secure reparse during import.
    pub source_paths: Vec<String>,
    pub warnings: Vec<String>,
}

/// Resolve the user's home directory across macOS / Linux / Windows without
/// relying on the deprecated `std::env::home_dir`.
fn user_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(std::path::PathBuf::from))
}

/// Scan well-known locations for Cockpit Tools, Codex Tools / Codex CLI and
/// EchoBird credentials. File-level discoveries are collapsed into two business
/// collections: credential-deduplicated accounts and provider/model-deduplicated
/// model resources. Source paths stay internal and are only returned so import
/// can securely reparse the original files. This pass is strictly read-only.
pub fn scan_agent_configs() -> AgentConfigScanResult {
    let mut found = Vec::<DiscoveredConfig>::new();
    let mut seen = HashSet::new();

    let mut candidates = Vec::<(std::path::PathBuf, &str)>::new();
    if let Some(home) = user_home() {
        candidates.push((home.join(".codex").join("auth.json"), "codex"));
        candidates.push((home.join(".codex").join("config.toml"), "codex"));
        candidates.push((home.join(".codex"), "codex"));
        candidates.push((home.join(".config").join("cockpit"), "cockpit"));
        candidates.push((home.join(".cockpit"), "cockpit"));
        candidates.push((home.join(".config").join("cli-proxy"), "cpa"));
        candidates.push((home.join(".config").join("sub2api"), "sub2api"));
        // Cockpit Tools stores the live account manifests and OAuth-bearing
        // account details under this hidden directory on macOS.
        candidates.push((
            home.join(".antigravity_cockpit").join("accounts"),
            "cockpit",
        ));
        candidates.push((
            home.join(".antigravity_cockpit").join("codex_accounts"),
            "cockpit",
        ));
        candidates.push((
            home.join(".antigravity_cockpit").join("claude_accounts"),
            "cockpit",
        ));
        candidates.push((
            home.join(".antigravity_cockpit").join("accounts.json"),
            "cockpit",
        ));
        candidates.push((
            home.join(".antigravity_cockpit")
                .join("codex_accounts.json"),
            "cockpit",
        ));
        candidates.push((
            home.join(".antigravity_cockpit")
                .join("claude_accounts.json"),
            "cockpit",
        ));
        if let Some(latest_backup) = newest_matching_file(
            &home.join(".antigravity_cockpit").join("backups"),
            "cockpit_auto_backup_full_",
            "json",
        ) {
            candidates.push((latest_backup, "cockpit"));
        }
        // EchoBird cloud model and OAuth configuration.
        candidates.push((
            home.join(".echobird").join("config").join("models.json"),
            "echobird",
        ));
        candidates.push((home.join(".echobird").join("codex.json"), "echobird"));
        candidates.push((home.join(".echobird").join("grok.json"), "echobird"));
        candidates.push((
            home.join(".echobird").join("codex-auth.bak.json"),
            "echobird",
        ));
        // macOS application support locations.
        let app_support = home.join("Library").join("Application Support");
        candidates.push((
            app_support
                .join("com.carry.codex-tools")
                .join("accounts.json"),
            "codex_tools",
        ));
        candidates.push((app_support.join("com.carry.codex-tools"), "codex_tools"));
        candidates.push((app_support.join("com.jlcodes.cockpit-tools"), "cockpit"));
        candidates.push((app_support.join("cockpit-tools"), "cockpit"));
        candidates.push((app_support.join("com.echobird.ai"), "echobird"));
    }

    for (path, kind) in candidates {
        if path.is_file() {
            push_discovered(&mut found, &mut seen, &path, kind);
        } else if path.is_dir() {
            // One level of recursion over config directories; only structured
            // text formats are considered importable.
            if let Ok(entries) = std::fs::read_dir(&path) {
                let mut files: Vec<_> = entries
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .filter(|path| {
                        path.is_file()
                            && path
                                .extension()
                                .map(|ext| {
                                    matches!(
                                        ext.to_str().unwrap_or_default(),
                                        "json" | "yaml" | "yml" | "toml"
                                    )
                                })
                                .unwrap_or(false)
                    })
                    .collect();
                files.sort();
                for file in files {
                    push_discovered(&mut found, &mut seen, &file, kind);
                }
            }
        }
    }
    collapse_discovered_configs(found)
}

fn collapse_discovered_configs(configs: Vec<DiscoveredConfig>) -> AgentConfigScanResult {
    let mut account_map = HashMap::<String, DiscoveredAccount>::new();
    let mut model_map = HashMap::<String, DiscoveredModelResource>::new();
    let mut source_paths = Vec::new();
    let mut warnings = Vec::new();

    for config in configs {
        let source_app = scan_kind_label(&config.kind).to_string();
        if !source_paths.contains(&config.path) {
            source_paths.push(config.path.clone());
        }
        warnings.extend(config.warnings);
        for account in config.accounts {
            let fingerprint = account.fingerprint.clone();
            let entry = account_map
                .entry(fingerprint.clone())
                .or_insert_with(|| account.clone());
            if !entry.source_apps.contains(&source_app) {
                entry.source_apps.push(source_app.clone());
            }
            if !entry.source_paths.contains(&config.path) {
                entry.source_paths.push(config.path.clone());
            }
            for model in &account.models {
                if !entry.models.contains(model) {
                    entry.models.push(model.clone());
                }
                let key = model_resource_key(&account.provider_name, model);
                let resource =
                    model_map
                        .entry(key.clone())
                        .or_insert_with(|| DiscoveredModelResource {
                            key,
                            provider_name: account.provider_name.clone(),
                            model: model.clone(),
                            source_apps: Vec::new(),
                            account_fingerprints: Vec::new(),
                        });
                if !resource.source_apps.contains(&source_app) {
                    resource.source_apps.push(source_app.clone());
                }
                if !resource.account_fingerprints.contains(&fingerprint) {
                    resource.account_fingerprints.push(fingerprint.clone());
                }
            }
        }
    }

    let mut accounts = account_map.into_values().collect::<Vec<_>>();
    accounts.sort_by(|left, right| {
        left.provider_name
            .to_lowercase()
            .cmp(&right.provider_name.to_lowercase())
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    let mut model_resources = model_map.into_values().collect::<Vec<_>>();
    model_resources.sort_by(|left, right| {
        left.provider_name
            .to_lowercase()
            .cmp(&right.provider_name.to_lowercase())
            .then_with(|| left.model.to_lowercase().cmp(&right.model.to_lowercase()))
    });
    source_paths.sort();
    source_paths.dedup();
    warnings.sort();
    warnings.dedup();

    AgentConfigScanResult {
        accounts,
        model_resources,
        source_paths,
        warnings,
    }
}

fn scan_kind_label(kind: &str) -> &'static str {
    match kind {
        "codex" => "Codex CLI",
        "codex_tools" => "Codex Tools",
        "cockpit" => "Cockpit Tools",
        "echobird" => "EchoBird",
        "cpa" => "CLI Proxy",
        "sub2api" => "Sub2API",
        _ => "其他",
    }
}

fn provider_identity_key(provider_name: &str) -> String {
    let normalized = provider_name
        .trim()
        .to_lowercase()
        .replace([' ', '-', '_'], "");
    match normalized.as_str() {
        "openai" | "codex" | "openaicodex" | "chatgpt" => "openai".into(),
        "anthropic" | "claude" | "anthropicclaude" => "anthropic".into(),
        "google" | "gemini" | "googlegemini" => "google".into(),
        "googleantigravity" | "antigravity" | "antigravityide" => "antigravity".into(),
        "xai" | "grok" | "xaigrok" => "xai".into(),
        _ => normalized,
    }
}

/// Whether `provider_identity_key` resolved to a *known official* upstream
/// (openai/anthropic/google/antigravity/xai). Anything else is treated as a
/// user-defined custom/relay provider and must NOT be collapsed across rows —
/// otherwise several custom providers sharing a generic name (e.g. "custom",
/// "自定义", or an empty default) get merged into one row whose Base URL/API
/// key is then rewritten for every attached account.
fn is_official_identity_key(key: &str) -> bool {
    matches!(
        key,
        "openai" | "anthropic" | "google" | "antigravity" | "xai"
    )
}

/// Compare two Base URLs ignoring trailing slashes and trivial whitespace, so
/// `https://x/v1` and `https://x/v1/` are treated as the same upstream.
fn same_base_url(a: &str, b: &str) -> bool {
    let na = a.trim().trim_end_matches('/');
    let nb = b.trim().trim_end_matches('/');
    na.eq_ignore_ascii_case(nb)
}

fn model_resource_key(provider_name: &str, model: &str) -> String {
    let normalized = format!(
        "{}\n{}",
        provider_identity_key(provider_name),
        model.trim().to_lowercase()
    );
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    format!("model_{:x}", hasher.finalize())
}

fn newest_matching_file(
    directory: &std::path::Path,
    prefix: &str,
    extension: &str,
) -> Option<std::path::PathBuf> {
    let mut files = std::fs::read_dir(directory)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.starts_with(prefix))
                    .unwrap_or(false)
                && path.extension().and_then(|value| value.to_str()) == Some(extension)
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    });
    files.pop()
}

fn push_discovered(
    found: &mut Vec<DiscoveredConfig>,
    seen: &mut HashSet<String>,
    path: &std::path::Path,
    kind: &str,
) {
    let path_str = path.to_string_lossy().to_string();
    if !seen.insert(path_str.clone()) {
        return;
    }
    let meta = std::fs::metadata(path).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let modified_at = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(duration.as_secs() as i64, 0)
        })
        .map(|dt| dt.to_rfc3339());
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let detected_format = if kind == "echobird" {
        ImportFormat::EchoBird
    } else {
        detect_format(&content, Some(&path_str))
    };
    let format = detected_format.as_str().to_string();
    let mut parsed = ParseBatch::default();
    let parse_result = parse_source(&content, &path_str, &detected_format, None, &mut parsed);
    let mut models = Vec::new();
    let accounts = parsed
        .accounts
        .into_iter()
        .map(|account| {
            for model in &account.models {
                if !models.contains(model) {
                    models.push(model.clone());
                }
            }
            let masked_credential = mask_secret(&primary_secret(&account));
            DiscoveredAccount {
                name: account.name,
                email: account.email,
                provider_name: account.provider_name,
                credential_type: account.credential_type,
                masked_credential,
                models: account.models,
                fingerprint: account.fingerprint,
                routable: account.routable,
                warning: account.warning,
                source_apps: vec![scan_kind_label(kind).to_string()],
                source_paths: vec![path_str.clone()],
            }
        })
        .collect();
    let mut warnings = parsed.skipped;
    warnings.extend(parsed.warnings);
    if let Err(error) = parse_result {
        warnings.push(error);
    }
    found.push(DiscoveredConfig {
        path: path_str,
        kind: kind.to_string(),
        format,
        size,
        modified_at,
        accounts,
        models,
        warnings,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../migrations/001_initial.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../../migrations/002_account_credentials.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../../migrations/003_account_usage.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../../migrations/004_protocol_multi.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../../migrations/005_protocol_canonical.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../../migrations/006_secure_credentials.sql"))
            .unwrap();
        conn.execute_batch(include_str!(
            "../../migrations/007_provider_protocol_base_urls.sql"
        ))
        .unwrap();
        Mutex::new(conn)
    }

    #[test]
    fn previews_codex_auth_without_leaking_tokens() {
        let db = test_db();
        let request = ImportSourceRequest {
            content: Some(r#"{
                "auth_mode":"chatgpt",
                "tokens":{"access_token":"secret-access-token","refresh_token":"secret-refresh-token","account_id":"acct-1"},
                "agent_identity":{"email":"dev@example.com"}
            }"#.into()),
            source_name: Some("auth.json".into()),
            paths: vec![],
            provider_hint: None,
        };
        let preview = preview_request(&db, &request).unwrap();
        assert_eq!(preview.format, "codex_auth");
        assert_eq!(preview.accounts[0].credential_type, "codex_oauth");
        assert!(!serde_json::to_string(&preview)
            .unwrap()
            .contains("secret-access-token"));
        assert_eq!(preview.summary.adapter_required, 0);
        assert_eq!(preview.summary.ready, 1);
        assert!(preview.accounts[0].routable);
    }

    #[test]
    fn parses_sub2api_nested_credentials() {
        let db = test_db();
        let request = ImportSourceRequest {
            content: Some(r#"{"accounts":[{"platform":"openai","type":"oauth","credentials":{"access_token":"oauth-secret","chatgpt_account_id":"chat-1"}}]}"#.into()),
            source_name: Some("sub2api.json".into()),
            paths: vec![],
            provider_hint: None,
        };
        let preview = preview_request(&db, &request).unwrap();
        assert_eq!(preview.format, "sub2api");
        assert_eq!(preview.accounts.len(), 1);
        assert_eq!(preview.accounts[0].credential_type, "oauth");
    }

    #[test]
    fn imports_upstream_gateway_and_detects_duplicate() {
        let db = test_db();
        let request = ImportSourceRequest {
            content: Some(r#"{"name":"CPA gateway","provider":"openai","base_url":"http://127.0.0.1:8317","api_key":"gateway-secret","models":["gpt-5"]}"#.into()),
            source_name: Some("cpa.json".into()),
            paths: vec![],
            provider_hint: None,
        };
        let first = execute_request(&db, &request, &ImportOptions::default()).unwrap();
        assert_eq!(first.imported, 1, "import errors: {:?}", first.errors);
        let preview = preview_request(&db, &request).unwrap();
        assert_eq!(preview.summary.duplicates, 1);
        let second = execute_request(&db, &request, &ImportOptions::default()).unwrap();
        assert_eq!(second.duplicates, 1);
    }

    #[test]
    fn parses_api_key_lines_and_skips_urls() {
        let db = test_db();
        let request = ImportSourceRequest {
            content: Some("main=sk-123456789\nhttps://api.example.com\nbackup=sk-987654321".into()),
            source_name: Some("keys.txt".into()),
            paths: vec![],
            provider_hint: None,
        };
        let preview = preview_request(&db, &request).unwrap();
        assert_eq!(preview.accounts.len(), 2);
        assert_eq!(preview.skipped.len(), 1);
    }

    #[test]
    fn canonicalizes_legacy_protocols_and_preserves_multi_protocol_hint() {
        assert_eq!(canonical_protocol("openai", false), "chat");
        assert_eq!(canonical_protocol("codex", true), "responses");
        assert_eq!(canonical_protocol("messages", false), "anthropic");
        assert_eq!(canonical_protocol("google", false), "gemini");
        assert_eq!(
            canonical_protocols(
                vec!["openai".into(), "anthropic".into(), "openai".into()],
                false
            ),
            vec!["chat", "anthropic"]
        );

        let value = serde_json::json!({
            "provider_name": "MiMo",
            "provider": "mimo",
            "protocol": "chat",
            "protocols": ["chat", "anthropic"],
            "api_key": "mimo-secret",
            "models": ["mimo"]
        });
        let account =
            normalize_account(&value, "mimo.json", &ImportFormat::GenericJson, None).unwrap();
        assert_eq!(account.protocol, "chat");
        assert_eq!(account.protocols, vec!["chat", "anthropic"]);
    }

    #[test]
    fn provider_hint_merges_resource_category_with_existing_json_tags() {
        let db = test_db();
        let request = ImportSourceRequest {
            content: Some(
                serde_json::json!({
                    "name": "Tagged DeepSeek",
                    "api_key": "deepseek-secret",
                    "tags": ["team-a"]
                })
                .to_string(),
            ),
            source_name: Some("tagged.json".into()),
            paths: vec![],
            provider_hint: Some(ImportProviderHint {
                id: "deepseek".into(),
                name: "DeepSeek".into(),
                protocol: "chat".into(),
                base_url: "https://api.deepseek.com".into(),
                protocols: Some(vec!["chat".into(), "anthropic".into()]),
                base_urls: None,
                models: vec!["deepseek-chat".into()],
                tags: vec!["deepseek".into(), "resource_category:api_key".into()],
                credential_mode: Some("apikey".into()),
            }),
        };

        let preview = preview_request(&db, &request).unwrap();
        assert_eq!(preview.accounts.len(), 1);
        assert!(preview.accounts[0].tags.contains(&"team-a".to_string()));
        assert!(preview.accounts[0].tags.contains(&"deepseek".to_string()));
        assert!(preview.accounts[0]
            .tags
            .contains(&"resource_category:api_key".to_string()));
    }

    #[test]
    fn imports_one_multi_protocol_provider_with_protocol_urls() {
        let db = test_db();
        let request = ImportSourceRequest {
            content: Some("deepseek-secret".into()),
            source_name: Some("keys.txt".into()),
            paths: vec![],
            provider_hint: Some(ImportProviderHint {
                id: "deepseek".into(),
                name: "DeepSeek".into(),
                protocol: "anthropic".into(),
                base_url: "https://api.deepseek.com/anthropic".into(),
                protocols: Some(vec!["anthropic".into(), "chat".into()]),
                base_urls: Some(HashMap::from([
                    (
                        "anthropic".into(),
                        "https://api.deepseek.com/anthropic".into(),
                    ),
                    ("chat".into(), "https://api.deepseek.com".into()),
                ])),
                models: vec!["deepseek-chat".into()],
                tags: vec!["deepseek".into()],
                credential_mode: Some("apikey".into()),
            }),
        };

        let result = execute_request(&db, &request, &ImportOptions::default()).unwrap();
        assert_eq!(result.imported, 1, "import errors: {:?}", result.errors);
        assert_eq!(result.created_providers, 1);

        let conn = db.lock().unwrap();
        let (count, protocols, base_urls): (i64, String, String) = conn
            .query_row(
                "SELECT COUNT(*), protocols, base_urls FROM providers WHERE name='DeepSeek'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&protocols).unwrap(),
            vec!["anthropic", "chat"]
        );
        let urls = serde_json::from_str::<HashMap<String, String>>(&base_urls).unwrap();
        assert_eq!(urls["chat"], "https://api.deepseek.com");
        assert_eq!(urls["anthropic"], "https://api.deepseek.com/anthropic");
    }

    #[test]
    fn provider_hint_treats_bare_refresh_token_as_codex_oauth() {
        let db = test_db();
        let request = ImportSourceRequest {
            content: Some("refresh_token=rt_test_secret".into()),
            source_name: Some("pasted.json".into()),
            paths: vec![],
            provider_hint: Some(ImportProviderHint {
                id: "codex".into(),
                name: "Codex".into(),
                protocol: "responses".into(),
                base_url: "https://chatgpt.com/backend-api/codex".into(),
                models: vec!["codex".into()],
                tags: vec!["official".into()],
                protocols: Some(vec!["responses".into()]),
                base_urls: None,
                credential_mode: Some("token".into()),
            }),
        };
        let preview = preview_request(&db, &request).unwrap();
        assert_eq!(preview.accounts.len(), 1);
        assert_eq!(preview.accounts[0].provider_name, "Codex");
        assert_eq!(preview.accounts[0].credential_type, "codex_oauth");
    }

    #[test]
    fn parses_yaml_with_oauth_client_fields() {
        let request = ImportSourceRequest {
            content: Some(
                "type: codex\naccess_token: yaml-access\noauth:\n  refresh_token: yaml-refresh\n  client_id: app_yaml_client\n  client_secret: sec_yaml_client_secret\naccount_id: yaml-acct-1\n"
                    .into(),
            ),
            source_name: Some("cockpit.yaml".into()),
            paths: vec![],
            provider_hint: None,
        };
        let batch = parse_request(&request).unwrap();
        assert_eq!(batch.formats[0], "yaml");
        assert_eq!(batch.accounts.len(), 1);
        let account = &batch.accounts[0];
        assert_eq!(account.credential_type, "codex_oauth");
        let meta = account.credential.metadata.as_ref().unwrap();
        assert_eq!(meta["client_id"], "app_yaml_client");
        assert_eq!(meta["client_secret"], "sec_yaml_client_secret");
        assert_eq!(
            account.credential.refresh_token.as_deref(),
            Some("yaml-refresh")
        );
        // Sensitive client_secret must never leak into the plaintext DB metadata.
        assert!(account.metadata.get("client_secret").is_none());
        assert!(account.metadata.get("client_id").is_none());
    }

    #[test]
    fn parses_toml_codex_config() {
        let request = ImportSourceRequest {
            content: Some(
                "type = \"codex\"\n[auth]\naccess_token = \"toml-access\"\nrefresh_token = \"toml-refresh\"\n".into(),
            ),
            source_name: Some("config.toml".into()),
            paths: vec![],
            provider_hint: None,
        };
        let batch = parse_request(&request).unwrap();
        assert_eq!(batch.formats[0], "toml");
        assert_eq!(batch.accounts.len(), 1);
        assert_eq!(
            batch.accounts[0].credential.access_token.as_deref(),
            Some("toml-access")
        );
    }

    #[test]
    fn on_conflict_overwrite_replaces_existing_account() {
        // Conflict resolution persists credentials through the shared vault.
        let _vault = crate::services::keychain::test_vault_serial_guard();
        let db = test_db();
        let base = |name: &str, expires: &str| ImportSourceRequest {
            content: Some(format!(
                r#"{{"name":"{name}","type":"codex","access_token":"same-secret","expires_at":"{expires}"}}"#
            )),
            source_name: Some("auth.json".into()),
            paths: vec![],
            provider_hint: None,
        };
        let first = execute_request(
            &db,
            &base("old-name", "2026-01-01T00:00:00Z"),
            &ImportOptions::default(),
        )
        .unwrap();
        assert_eq!(first.imported, 1);

        // Same credential fingerprint, different display name + expiry.
        let options = ImportOptions {
            on_conflict: Some("overwrite".into()),
            ..ImportOptions::default()
        };
        let second =
            execute_request(&db, &base("new-name", "2027-01-01T00:00:00Z"), &options).unwrap();
        assert_eq!(second.updated, 1);
        assert_eq!(second.imported, 0);
        let conn = db.lock().unwrap();
        let fingerprint: String = conn
            .query_row(
                "SELECT credential_fingerprint FROM accounts WHERE id=?1",
                rusqlite::params![first.account_ids[0]],
                |row| row.get(0),
            )
            .unwrap();
        let (name, expires): (String, Option<String>) = conn
            .query_row(
                "SELECT name, expires_at FROM accounts WHERE credential_fingerprint=?1",
                rusqlite::params![fingerprint],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(name, "new-name");
        assert_eq!(expires.as_deref(), Some("2027-01-01T00:00:00Z"));
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn on_conflict_merge_fills_missing_credential_fields() {
        // Conflict resolution persists credentials through the shared vault.
        let _vault = crate::services::keychain::test_vault_serial_guard();
        let db = test_db();
        let request = |content: &str| ImportSourceRequest {
            content: Some(content.into()),
            source_name: Some("auth.json".into()),
            paths: vec![],
            provider_hint: None,
        };
        // First import: access_token only.
        let first = execute_request(
            &db,
            &request(r#"{"type":"codex","access_token":"merge-secret"}"#),
            &ImportOptions::default(),
        )
        .unwrap();
        assert_eq!(first.imported, 1);

        // Merge import: same access_token (same fingerprint) + refresh_token.
        let options = ImportOptions {
            on_conflict: Some("merge".into()),
            ..ImportOptions::default()
        };
        let second = execute_request(
            &db,
            &request(r#"{"type":"codex","access_token":"merge-secret","refresh_token":"merged-refresh"}"#),
            &options,
        )
        .unwrap();
        assert_eq!(second.updated, 1);

        // The refresh token must now be persisted inside the vault payload.
        let conn = db.lock().unwrap();
        let fingerprint: String = conn
            .query_row(
                "SELECT credential_fingerprint FROM accounts WHERE id=?1",
                rusqlite::params![first.account_ids[0]],
                |row| row.get(0),
            )
            .unwrap();
        let secret_ref: String = conn
            .query_row(
                "SELECT secret_ref FROM accounts WHERE credential_fingerprint=?1",
                rusqlite::params![fingerprint],
                |row| row.get(0),
            )
            .unwrap();
        let payload: CredentialPayload =
            serde_json::from_slice(&keychain::get_secret(&secret_ref).unwrap()).unwrap();
        assert_eq!(payload.refresh_token.as_deref(), Some("merged-refresh"));
        assert_eq!(payload.access_token.as_deref(), Some("merge-secret"));
    }

    #[test]
    fn parses_cockpit_full_backup_platform_accounts() {
        let request = ImportSourceRequest {
            content: Some(
                serde_json::json!({
                    "accounts": {
                        "platforms": {
                            "codex": {
                                "exported_data": [{
                                    "email": "codex@example.com",
                                    "auth_mode": "chatgpt",
                                    "tokens": {
                                        "access_token": "cockpit-access",
                                        "refresh_token": "cockpit-refresh",
                                        "account_id": "chatgpt-account"
                                    },
                                    "api_model_catalog": ["gpt-5.4", {"id": "gpt-5.3-codex"}]
                                }]
                            },
                            "kiro": {
                                "exported_data": [{
                                    "email": "kiro@example.com",
                                    "access_token": "kiro-access",
                                    "refresh_token": "kiro-refresh"
                                }]
                            }
                        }
                    }
                })
                .to_string(),
            ),
            source_name: Some("cockpit_auto_backup_full.json".into()),
            paths: vec![],
            provider_hint: None,
        };
        let batch = parse_request(&request).unwrap();
        assert_eq!(batch.accounts.len(), 2);
        assert_eq!(batch.accounts[0].credential_type, "codex_oauth");
        assert_eq!(batch.accounts[0].models, vec!["gpt-5.4", "gpt-5.3-codex"]);
        assert_eq!(batch.accounts[1].provider_name, "Kiro");
    }

    #[test]
    fn parses_echobird_cloud_models_and_model_names() {
        let request = ImportSourceRequest {
            content: Some(
                serde_json::json!([{
                    "internalId": "m-1",
                    "name": "Xiaomi MiMo",
                    "modelId": "mimo-v2.5-pro",
                    "baseUrl": "https://example.com/v1",
                    "anthropicUrl": "https://example.com/anthropic",
                    "apiKey": "echobird-secret"
                }])
                .to_string(),
            ),
            source_name: Some("/Users/test/.echobird/config/models.json".into()),
            paths: vec![],
            provider_hint: None,
        };
        let batch = parse_request(&request).unwrap();
        assert_eq!(batch.formats[0], "echobird");
        assert_eq!(batch.accounts.len(), 1);
        assert_eq!(batch.accounts[0].provider_name, "Xiaomi MiMo");
        assert_eq!(batch.accounts[0].credential_type, "upstream_key");
        assert_eq!(batch.accounts[0].models, vec!["mimo-v2.5-pro"]);
        assert_eq!(batch.accounts[0].protocols, vec!["chat", "anthropic"]);
    }

    #[test]
    fn selected_account_filter_applies_before_conflict_resolution() {
        let _vault = crate::services::keychain::test_vault_serial_guard();
        let db = test_db();
        let content = serde_json::json!([
            {"name":"first","api_key":"first-secret"},
            {"name":"second","api_key":"second-secret"}
        ])
        .to_string();
        let request = ImportSourceRequest {
            content: Some(content),
            source_name: Some("accounts.json".into()),
            paths: vec![],
            provider_hint: None,
        };
        let batch = parse_request(&request).unwrap();
        let selected = batch.accounts[1].fingerprint.clone();
        let result = execute_request(
            &db,
            &request,
            &ImportOptions {
                selected_fingerprints: Some(vec![selected]),
                ..ImportOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(result.skipped, 1);
        let conn = db.lock().unwrap();
        let name: String = conn
            .query_row("SELECT name FROM accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "second");
    }

    #[test]
    fn collapse_scan_results_deduplicates_accounts_and_model_resources() {
        let account = |name: &str, models: Vec<&str>, source_app: &str, source_path: &str| {
            DiscoveredAccount {
                name: name.into(),
                email: Some("same@example.com".into()),
                provider_name: "OpenAI Codex".into(),
                credential_type: "codex_oauth".into(),
                masked_credential: "abc••••••wxyz".into(),
                models: models.into_iter().map(str::to_string).collect(),
                fingerprint: "same-fingerprint".into(),
                routable: false,
                warning: None,
                source_apps: vec![source_app.into()],
                source_paths: vec![source_path.into()],
            }
        };
        let configs = vec![
            DiscoveredConfig {
                path: "/tmp/codex.json".into(),
                kind: "codex_tools".into(),
                format: "json".into(),
                size: 1,
                modified_at: None,
                accounts: vec![account(
                    "Codex A",
                    vec!["gpt-5.4", "gpt-5.3-codex"],
                    "Codex Tools",
                    "/tmp/codex.json",
                )],
                models: vec!["gpt-5.4".into(), "gpt-5.3-codex".into()],
                warnings: vec![],
            },
            DiscoveredConfig {
                path: "/tmp/cockpit.json".into(),
                kind: "cockpit".into(),
                format: "cockpit".into(),
                size: 1,
                modified_at: None,
                accounts: vec![account(
                    "Codex duplicate",
                    vec!["gpt-5.4"],
                    "Cockpit Tools",
                    "/tmp/cockpit.json",
                )],
                models: vec!["gpt-5.4".into()],
                warnings: vec![],
            },
        ];

        let result = collapse_discovered_configs(configs);
        assert_eq!(result.accounts.len(), 1);
        assert_eq!(result.accounts[0].source_apps.len(), 2);
        assert_eq!(result.accounts[0].source_paths.len(), 2);
        assert_eq!(result.model_resources.len(), 2);
        let gpt54 = result
            .model_resources
            .iter()
            .find(|resource| resource.model == "gpt-5.4")
            .unwrap();
        assert_eq!(gpt54.source_apps.len(), 2);
        assert_eq!(gpt54.account_fingerprints, vec!["same-fingerprint"]);
    }

    #[test]
    fn imports_selected_codex_oauth_without_models() {
        let db = test_db();
        let request = ImportSourceRequest {
            content: Some(
                serde_json::json!({
                    "accounts": [{
                        "id": "oauth-account",
                        "label": "oauth@example.com",
                        "email": "oauth@example.com",
                        "sourceKind": "chatgpt",
                        "authJson": {
                            "auth_mode": "chatgpt",
                            "tokens": {
                                "access_token": "oauth-access",
                                "refresh_token": "oauth-refresh",
                                "account_id": "oauth-account"
                            }
                        }
                    }]
                })
                .to_string(),
            ),
            source_name: Some("accounts.json".into()),
            paths: vec![],
            provider_hint: None,
        };
        let batch = parse_request(&request).unwrap();
        let fingerprint = batch.accounts[0].fingerprint.clone();
        assert_eq!(batch.accounts[0].credential_type, "codex_oauth");
        assert!(batch.accounts[0].models.is_empty());

        let result = execute_request(
            &db,
            &request,
            &ImportOptions {
                selected_fingerprints: Some(vec![fingerprint]),
                import_adapter_required: true,
                ..ImportOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(result.adapter_required, 0);
        assert_eq!(result.account_ids.len(), 1);

        let conn = db.lock().unwrap();
        let (credential_type, status, health_status, models): (String, String, String, String) =
            conn.query_row(
                "SELECT credential_type, status, health_status, models FROM accounts",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(credential_type, "codex_oauth");
        assert_eq!(status, "active");
        assert_eq!(health_status, "unchecked");
        assert_eq!(models, "[]");
    }

    #[test]
    fn parses_cockpit_codex_tools_accounts_json() {
        // Real-shape fixture from Cockpit "Codex Tools" `accounts.json`:
        // tokens live inside `authJson.tokens`, and relay accounts expose
        // `apiKey` + `apiBaseUrl` at the top level.
        let request = ImportSourceRequest {
            content: Some(
                r#"{
                  "version": 1,
                  "accounts": [
                    {
                      "id": "uuid-oauth-1",
                      "label": "ivenkral@gmail.com",
                      "email": "ivenkral@gmail.com",
                      "sourceKind": "chatgpt",
                      "accountId": "8a58c864-a41a-4b5a-8829-65c1fb56b62f",
                      "authJson": {
                        "auth_mode": "chatgpt",
                        "tokens": {
                          "access_token": "at-fake",
                          "refresh_token": "rt-fake",
                          "id_token": "id-fake",
                          "account_id": "8a58c864-a41a-4b5a-8829-65c1fb56b62f"
                        }
                      }
                    },
                    {
                      "id": "uuid-relay-1",
                      "label": "XiaoMi MiMo",
                      "email": null,
                      "sourceKind": "relay",
                      "authJson": { "auth_mode": "apikey", "OPENAI_API_KEY": "tp-fake" },
                      "apiBaseUrl": "https://token-plan-cn.xiaomimimo.com/v1",
                      "apiKey": "tp-fake"
                    }
                  ]
                }"#
                .into(),
            ),
            source_name: Some("accounts.json".into()),
            paths: vec![],
            provider_hint: None,
        };
        let batch = parse_request(&request).unwrap();
        assert_eq!(batch.accounts.len(), 2, "skipped: {:?}", batch.skipped);

        let oauth = &batch.accounts[0];
        assert_eq!(oauth.email.as_deref(), Some("ivenkral@gmail.com"));
        assert_eq!(oauth.credential.access_token.as_deref(), Some("at-fake"));
        assert_eq!(oauth.credential.refresh_token.as_deref(), Some("rt-fake"));
        assert_eq!(oauth.credential.id_token.as_deref(), Some("id-fake"));
        assert_eq!(
            oauth.external_account_id.as_deref(),
            Some("8a58c864-a41a-4b5a-8829-65c1fb56b62f")
        );

        let relay = &batch.accounts[1];
        assert_eq!(relay.credential_type, "upstream_key");
        assert_eq!(relay.credential.api_key.as_deref(), Some("tp-fake"));
        // Base URL is stored as imported (a trailing `/v1` is fine): the
        // shared URL builder composes `/v1/chat/completions` at request time
        // and de-duplicates the version segment, so both the raw versioned
        // form and the bare origin resolve to the same upstream.
        assert_eq!(relay.base_url, "https://token-plan-cn.xiaomimimo.com/v1");
        assert_eq!(relay.provider_name, "XiaoMi MiMo");
    }
}
