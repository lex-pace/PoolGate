use crate::services::pool_management;
use crate::AppState;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

#[derive(serde::Serialize, Clone, Debug)]
pub struct AgentAppInfo {
    pub app_id: String,
    pub name: String,
    pub installed: bool,
    pub executable: Option<String>,
    pub config_paths: Vec<String>,
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct AgentAppPreview {
    pub app: AgentAppInfo,
    pub group_id: String,
    pub affected_paths: Vec<String>,
    pub backup_root: String,
    pub warnings: Vec<String>,
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct AgentAppLaunchResult {
    pub app_id: String,
    pub group_id: String,
    pub snapshot_id: String,
    pub config_path: String,
    pub backup_path: String,
    pub launched: bool,
}

struct AppOperationGuard<'a> {
    state: &'a AppState,
    app_id: String,
}

impl<'a> AppOperationGuard<'a> {
    fn acquire(state: &'a AppState, app_id: &str) -> Result<Self, String> {
        let mut operations = state
            .agent_app_operations
            .lock()
            .map_err(|error| error.to_string())?;
        if !operations.insert(app_id.to_string()) {
            return Err("APP_OPERATION_IN_PROGRESS: 该应用正在执行配置或恢复操作".into());
        }
        Ok(Self {
            state,
            app_id: app_id.to_string(),
        })
    }
}

impl Drop for AppOperationGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut operations) = self.state.agent_app_operations.lock() {
            operations.remove(&self.app_id);
        }
    }
}

pub fn validate_route_pool(state: &AppState, group_id: &str) -> Result<(), String> {
    let group = state
        .db
        .groups
        .get_by_id(&state.db.conn, group_id)?
        .ok_or_else(|| "POOL_NOT_FOUND: 路由池不存在".to_string())?;
    if group.enabled == Some(false) {
        return Err("POOL_DISABLED: 路由池已停用".into());
    }
    let models = state
        .db
        .groups
        .get_model_resources(&state.db.conn, group_id)?;
    let legacy_accounts = state.db.groups.get_account_ids(&state.db.conn, group_id)?;
    if models.is_empty() && legacy_accounts.is_empty() {
        return Err("POOL_EMPTY: 请先为路由池添加模型资源".into());
    }
    Ok(())
}

fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "APP_HOME_MISSING: 无法解析用户主目录".to_string())
}

fn executable_for(app_id: &str) -> Result<&'static str, String> {
    match app_id {
        "codex" => Ok("codex"),
        "claude_code" => Ok("claude"),
        "opencode" => Ok("opencode"),
        _ => Err("APP_UNSUPPORTED: 不支持的 Agent 应用".into()),
    }
}

fn name_for(app_id: &str) -> &'static str {
    match app_id {
        "codex" => "Codex",
        "claude_code" => "Claude Code",
        "opencode" => "OpenCode",
        _ => "Unknown",
    }
}

fn config_path_for(app_id: &str) -> Result<PathBuf, String> {
    let home = home_dir()?;
    match app_id {
        "codex" => Ok(home.join(".codex").join("config.toml")),
        "claude_code" => Ok(home.join(".claude").join("settings.json")),
        "opencode" => Ok(home.join(".config").join("opencode").join("opencode.json")),
        _ => Err("APP_UNSUPPORTED: 不支持的 Agent 应用".into()),
    }
}

fn find_executable(app_id: &str) -> Option<String> {
    let binary = executable_for(app_id).ok()?;
    Command::new("/usr/bin/which")
        .arg(binary)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn detect_apps() -> Result<Vec<AgentAppInfo>, String> {
    ["codex", "claude_code", "opencode"]
        .into_iter()
        .map(|app_id| {
            let path = config_path_for(app_id)?;
            let executable = find_executable(app_id);
            Ok(AgentAppInfo {
                app_id: app_id.to_string(),
                name: name_for(app_id).to_string(),
                installed: executable.is_some(),
                executable,
                config_paths: vec![path.to_string_lossy().to_string()],
            })
        })
        .collect()
}

pub fn preview(
    app_id: &str,
    group_id: &str,
    backup_root: &Path,
) -> Result<AgentAppPreview, String> {
    let app = detect_apps()?
        .into_iter()
        .find(|app| app.app_id == app_id)
        .ok_or_else(|| "APP_UNSUPPORTED: 不支持的 Agent 应用".to_string())?;
    Ok(AgentAppPreview {
        affected_paths: app.config_paths.clone(),
        backup_root: backup_root.join(app_id).to_string_lossy().to_string(),
        warnings: vec![
            "将先创建可恢复备份，再原子改写配置文件。".into(),
            "如果应用已在运行，请先退出，避免它覆盖新配置。".into(),
            "PoolGate 不会把路由池 Key 写入数据库或备份元数据。".into(),
        ],
        app,
        group_id: group_id.to_string(),
    })
}

fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn render_config(
    app_id: &str,
    original: &[u8],
    base_url: &str,
    raw_key: &str,
) -> Result<Vec<u8>, String> {
    match app_id {
        "codex" => {
            let text = String::from_utf8_lossy(original);
            let mut value: toml::Value = if text.trim().is_empty() {
                toml::Value::Table(toml::map::Map::new())
            } else {
                toml::from_str(&text).map_err(|error| {
                    format!("APP_CONFIG_INVALID: Codex TOML 无法解析: {}", error)
                })?
            };
            let table = value
                .as_table_mut()
                .ok_or_else(|| "APP_CONFIG_INVALID: Codex 配置不是 TOML table".to_string())?;
            table.insert(
                "model_provider".into(),
                toml::Value::String("poolgate".into()),
            );
            let providers = table
                .entry("model_providers")
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let providers = providers
                .as_table_mut()
                .ok_or_else(|| "APP_CONFIG_INVALID: model_providers 结构无效".to_string())?;
            let mut poolgate = toml::map::Map::new();
            poolgate.insert("name".into(), toml::Value::String("PoolGate".into()));
            poolgate.insert("requires_openai_auth".into(), toml::Value::Boolean(false));
            poolgate.insert(
                "base_url".into(),
                toml::Value::String(format!("{}/v1", base_url.trim_end_matches('/'))),
            );
            poolgate.insert(
                "env_key".into(),
                toml::Value::String("POOLGATE_API_KEY".into()),
            );
            poolgate.insert("wire_api".into(), toml::Value::String("responses".into()));
            providers.insert("poolgate".into(), toml::Value::Table(poolgate));
            toml::to_string_pretty(&value)
                .map(|value| value.into_bytes())
                .map_err(|error| error.to_string())
        }
        "claude_code" => {
            let mut value: serde_json::Value = if original.iter().all(u8::is_ascii_whitespace) {
                serde_json::json!({})
            } else {
                serde_json::from_slice(original).map_err(|error| {
                    format!("APP_CONFIG_INVALID: Claude Code JSON 无法解析: {}", error)
                })?
            };
            let object = value.as_object_mut().ok_or_else(|| {
                "APP_CONFIG_INVALID: Claude Code 配置不是 JSON object".to_string()
            })?;
            let env = object.entry("env").or_insert_with(|| serde_json::json!({}));
            let env = env
                .as_object_mut()
                .ok_or_else(|| "APP_CONFIG_INVALID: Claude Code env 结构无效".to_string())?;
            env.insert("ANTHROPIC_BASE_URL".into(), base_url.into());
            env.insert("ANTHROPIC_AUTH_TOKEN".into(), raw_key.into());
            serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())
        }
        "opencode" => {
            let mut value: serde_json::Value = if original.iter().all(u8::is_ascii_whitespace) {
                serde_json::json!({})
            } else {
                serde_json::from_slice(original).map_err(|error| {
                    format!("APP_CONFIG_INVALID: OpenCode JSON 无法解析: {}", error)
                })?
            };
            let object = value
                .as_object_mut()
                .ok_or_else(|| "APP_CONFIG_INVALID: OpenCode 配置不是 JSON object".to_string())?;
            let provider = object
                .entry("provider")
                .or_insert_with(|| serde_json::json!({}));
            let provider = provider
                .as_object_mut()
                .ok_or_else(|| "APP_CONFIG_INVALID: OpenCode provider 结构无效".to_string())?;
            provider.insert("poolgate".into(), serde_json::json!({
                "npm": "@ai-sdk/openai-compatible",
                "name": "PoolGate",
                "options": { "baseURL": format!("{}/v1", base_url.trim_end_matches('/')), "apiKey": raw_key }
            }));
            serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())
        }
        _ => Err("APP_UNSUPPORTED: 不支持的 Agent 应用".into()),
    }
}

#[cfg(test)]
mod config_tests {
    use super::render_config;

    #[test]
    fn codex_uses_versioned_responses_base_url() {
        let rendered = render_config(
            "codex",
            b"model = \"gpt-test\"\n",
            "http://127.0.0.1:9800",
            "pg_live_test",
        )
        .unwrap();
        let text = String::from_utf8(rendered).unwrap();
        assert!(text.contains("base_url = \"http://127.0.0.1:9800/v1\""));
        assert!(text.contains("wire_api = \"responses\""));
        assert!(text.contains("requires_openai_auth = false"));
    }

    #[test]
    fn claude_uses_unversioned_anthropic_base_url() {
        let rendered = render_config(
            "claude_code",
            b"{}",
            "http://127.0.0.1:9800",
            "pg_live_test",
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&rendered).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:9800");
        assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], "pg_live_test");
    }

    #[test]
    fn opencode_uses_versioned_openai_base_url() {
        let rendered =
            render_config("opencode", b"{}", "http://127.0.0.1:9800", "pg_live_test").unwrap();
        let value: serde_json::Value = serde_json::from_slice(&rendered).unwrap();
        assert_eq!(
            value["provider"]["poolgate"]["options"]["baseURL"],
            "http://127.0.0.1:9800/v1"
        );
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "APP_CONFIG_PATH_INVALID: 配置路径无父目录".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("APP_CONFIG_DIR_FAILED: {}", error))?;
    let temp_path = parent.join(format!(".poolgate-{}.tmp", Uuid::new_v4().simple()));
    let mut file = fs::File::create(&temp_path)
        .map_err(|error| format!("APP_CONFIG_WRITE_FAILED: {}", error))?;
    file.write_all(bytes)
        .map_err(|error| format!("APP_CONFIG_WRITE_FAILED: {}", error))?;
    file.sync_all()
        .map_err(|error| format!("APP_CONFIG_SYNC_FAILED: {}", error))?;
    fs::rename(&temp_path, path).map_err(|error| format!("APP_CONFIG_RENAME_FAILED: {}", error))
}

pub fn configure_and_launch(
    state: &AppState,
    app_id: &str,
    group_id: &str,
    base_url: &str,
    backup_root: &Path,
    confirmed: bool,
    working_directory: Option<&Path>,
) -> Result<AgentAppLaunchResult, String> {
    if !confirmed {
        return Err("APP_CONFIRM_REQUIRED: 必须确认备份与改写路径后才能继续".into());
    }
    let _operation = AppOperationGuard::acquire(state, app_id)?;
    validate_route_pool(state, group_id)?;
    let executable = find_executable(app_id)
        .ok_or_else(|| "APP_NOT_INSTALLED: 未检测到应用可执行文件".to_string())?;
    let raw_key = pool_management::managed_pool_secret(state, group_id)?;
    let config_path = config_path_for(app_id)?;
    let original = fs::read(&config_path).unwrap_or_default();
    let managed = render_config(app_id, &original, base_url, &raw_key)?;
    let snapshot_id = Uuid::new_v4().to_string();
    let snapshot_dir = backup_root.join(app_id).join(&snapshot_id);
    fs::create_dir_all(&snapshot_dir)
        .map_err(|error| format!("APP_BACKUP_DIR_FAILED: {}", error))?;
    let backup_path = snapshot_dir.join(config_path.file_name().unwrap_or_default());
    fs::write(&backup_path, &original).map_err(|error| format!("APP_BACKUP_FAILED: {}", error))?;
    let backed_up =
        fs::read(&backup_path).map_err(|error| format!("APP_BACKUP_VERIFY_FAILED: {}", error))?;
    if hash_bytes(&backed_up) != hash_bytes(&original) {
        return Err("APP_BACKUP_VERIFY_FAILED: 备份校验不一致".into());
    }
    atomic_write(&config_path, &managed)?;
    {
        let conn = state.db.conn.lock().map_err(|error| error.to_string())?;
        conn.execute(
            "INSERT INTO agent_app_snapshots (id, app_id, group_id, config_path, backup_path, original_hash, managed_hash, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active')",
            rusqlite::params![
                snapshot_id,
                app_id,
                group_id,
                config_path.to_string_lossy(),
                backup_path.to_string_lossy(),
                hash_bytes(&original),
                hash_bytes(&managed)
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    let mut command = Command::new(&executable);
    command.env("POOLGATE_API_KEY", &raw_key);
    command.env("OPENAI_API_KEY", &raw_key);
    command.env("ANTHROPIC_AUTH_TOKEN", &raw_key);
    command.env("OPENAI_BASE_URL", base_url);
    command.env("ANTHROPIC_BASE_URL", base_url);
    if let Some(directory) = working_directory {
        command.current_dir(directory);
    }
    let launched = command.spawn().is_ok();
    Ok(AgentAppLaunchResult {
        app_id: app_id.into(),
        group_id: group_id.into(),
        snapshot_id,
        config_path: config_path.to_string_lossy().to_string(),
        backup_path: backup_path.to_string_lossy().to_string(),
        launched,
    })
}

pub fn restore(state: &AppState, snapshot_id: &str, force: bool) -> Result<(), String> {
    let (app_id, config_path, backup_path, managed_hash): (String, String, String, String) = {
        let conn = state.db.conn.lock().map_err(|error| error.to_string())?;
        conn.query_row(
            "SELECT app_id, config_path, backup_path, managed_hash FROM agent_app_snapshots WHERE id=?1 AND status='active'",
            rusqlite::params![snapshot_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| "APP_SNAPSHOT_NOT_FOUND: 找不到可恢复的配置快照".to_string())?
    };
    let _operation = AppOperationGuard::acquire(state, &app_id)?;
    let current = fs::read(&config_path).unwrap_or_default();
    if !force && hash_bytes(&current) != managed_hash {
        return Err("APP_CONFIG_CHANGED: 配置在 PoolGate 改写后又被修改，拒绝静默覆盖".into());
    }
    let backup =
        fs::read(&backup_path).map_err(|error| format!("APP_BACKUP_READ_FAILED: {}", error))?;
    atomic_write(Path::new(&config_path), &backup)?;
    let conn = state.db.conn.lock().map_err(|error| error.to_string())?;
    conn.execute(
        "UPDATE agent_app_snapshots SET status='restored', restored_at=CURRENT_TIMESTAMP WHERE id=?1",
        rusqlite::params![snapshot_id],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}
