mod account_manager;
mod auth_checker;
mod logger;
mod machine_id;
mod seamless;

use account_manager::{AccountInfo, AccountListResult, AccountManager, LogoutResult, SwitchAccountResult};
use auth_checker::{AuthCheckResult, AuthChecker, TokenInfo};
use chrono;
use machine_id::{BackupInfo, MachineIdRestorer, MachineIds, ResetResult, RestoreResult};
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use tauri::{Emitter, Manager};

// ============================================================================
// Proxy Settings and HTTP Client
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySettings {
    pub enabled: bool,
    pub proxy_type: String, // "http" or "socks5"
    pub host: String,
    pub port: u16,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            proxy_type: "http".to_string(),
            host: "127.0.0.1".to_string(),
            port: 7890,
        }
    }
}

fn get_proxy_settings_file_path() -> Result<PathBuf, String> {
    get_data_dir().map(|dir| dir.join("proxy_settings.json"))
}

pub fn load_proxy_settings() -> Result<ProxySettings, String> {
    let path = get_proxy_settings_file_path()?;
    if !path.exists() {
        return Ok(ProxySettings::default());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("读取代理设置失败: {}", e))?;
    serde_json::from_str(&content).map_err(|e| format!("解析代理设置失败: {}", e))
}

pub fn save_proxy_settings_to_file(settings: &ProxySettings) -> Result<(), String> {
    let path = get_proxy_settings_file_path()?;
    let json = serde_json::to_string_pretty(settings).map_err(|e| format!("序列化代理设置失败: {}", e))?;
    fs::write(&path, json).map_err(|e| format!("保存代理设置失败: {}", e))?;
    log_info!("[代理设置] 已保存: enabled={}, type={}, {}:{}", 
        settings.enabled, settings.proxy_type, settings.host, settings.port);
    Ok(())
}

// Global HTTP client with proxy support
static HTTP_CLIENT: OnceLock<RwLock<reqwest::Client>> = OnceLock::new();

fn build_http_client_with_proxy() -> reqwest::Client {
    let settings = load_proxy_settings().unwrap_or_default();
    
    let mut builder = reqwest::Client::builder()
        .pool_max_idle_per_host(10)
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10));
    
    if settings.enabled {
        let proxy_url = match settings.proxy_type.as_str() {
            "socks5" => format!("socks5://{}:{}", settings.host, settings.port),
            _ => format!("http://{}:{}", settings.host, settings.port),
        };
        
        match reqwest::Proxy::all(&proxy_url) {
            Ok(proxy) => {
                builder = builder.proxy(proxy);
                log_info!("[代理] 已启用: {}", proxy_url);
            }
            Err(e) => {
                log_error!("[代理] 设置失败: {}", e);
            }
        }
    }
    
    builder.build().unwrap_or_else(|e| {
        log_error!("[HTTP客户端] 构建失败: {}, 使用默认客户端", e);
        reqwest::Client::new()
    })
}

pub fn get_http_client() -> reqwest::Client {
    let lock = HTTP_CLIENT.get_or_init(|| {
        RwLock::new(build_http_client_with_proxy())
    });
    lock.read().unwrap().clone()
}

pub fn rebuild_http_client() {
    if let Some(lock) = HTTP_CLIENT.get() {
        if let Ok(mut client) = lock.write() {
            *client = build_http_client_with_proxy();
            log_info!("[HTTP客户端] 已重建");
        }
    } else {
        // Initialize if not yet created
        let _ = get_http_client();
    }
}

#[tauri::command]
async fn get_proxy_settings() -> Result<ProxySettings, String> {
    load_proxy_settings()
}

#[tauri::command]
async fn save_proxy_settings(settings: ProxySettings) -> Result<serde_json::Value, String> {
    save_proxy_settings_to_file(&settings)?;
    Ok(serde_json::json!({
        "success": true,
        "message": "代理设置已保存"
    }))
}

#[tauri::command]
async fn rebuild_http_client_cmd() -> Result<serde_json::Value, String> {
    rebuild_http_client();
    Ok(serde_json::json!({
        "success": true,
        "message": "HTTP客户端已重建"
    }))
}

// 日志宏现在在logger.rs中定义

// 获取应用目录的辅助函数
pub fn get_app_dir() -> Result<PathBuf, String> {
    let exe_path = env::current_exe().map_err(|e| format!("Failed to get exe path: {}", e))?;
    let app_dir = exe_path
        .parent()
        .ok_or("Failed to get parent directory")?
        .to_path_buf();
    Ok(app_dir)
}






#[tauri::command]
async fn get_available_backups() -> Result<Vec<BackupInfo>, String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    restorer
        .find_backups()
        .map_err(|e| format!("Failed to find backups: {}", e))
}

#[tauri::command]
async fn extract_backup_ids(backup_path: String) -> Result<MachineIds, String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    restorer
        .extract_ids_from_backup(&backup_path)
        .map_err(|e| format!("Failed to extract IDs from backup: {}", e))
}

#[tauri::command]
async fn delete_backup(backup_path: String) -> Result<serde_json::Value, String> {
    use std::fs;

    match fs::remove_file(&backup_path) {
        Ok(_) => {
            log_info!("✅ 成功删除备份文件: {}", backup_path);
            Ok(serde_json::json!({
                "success": true,
                "message": "备份文件删除成功"
            }))
        }
        Err(e) => {
            log_error!("❌ 删除备份文件失败: {}", e);
            Ok(serde_json::json!({
                "success": false,
                "message": format!("删除失败: {}", e)
            }))
        }
    }
}

#[tauri::command]
async fn restore_machine_ids(backup_path: String) -> Result<RestoreResult, String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    let mut details = Vec::new();
    let mut success = true;

    // Extract IDs from backup
    let ids = match restorer.extract_ids_from_backup(&backup_path) {
        Ok(ids) => {
            details.push("Successfully extracted IDs from backup".to_string());
            ids
        }
        Err(e) => {
            return Ok(RestoreResult {
                success: false,
                message: format!("Failed to extract IDs from backup: {}", e),
                details,
            });
        }
    };

    // Create backup of current state
    match restorer.create_backup() {
        Ok(backup_path) => {
            details.push(format!("Created backup at: {}", backup_path));
        }
        Err(e) => {
            details.push(format!("Warning: Failed to create backup: {}", e));
        }
    }

    // Update storage file
    if let Err(e) = restorer.update_storage_file(&ids) {
        success = false;
        details.push(format!("Failed to update storage file: {}", e));
    } else {
        details.push("Successfully updated storage.json".to_string());
    }

    // Update SQLite database (simplified version)
    match restorer.update_sqlite_db(&ids) {
        Ok(sqlite_results) => {
            details.extend(sqlite_results);
        }
        Err(e) => {
            details.push(format!("Warning: Failed to update SQLite database: {}", e));
        }
    }

    // Update machine ID file
    if let Err(e) = restorer.update_machine_id_file(&ids.dev_device_id) {
        details.push(format!("Warning: Failed to update machine ID file: {}", e));
    } else {
        details.push("Successfully updated machine ID file".to_string());
    }

    // Update system IDs
    match restorer.update_system_ids(&ids) {
        Ok(system_results) => {
            details.extend(system_results);
        }
        Err(e) => {
            details.push(format!("Warning: Failed to update system IDs: {}", e));
        }
    }

    let message = if success {
        "Machine IDs restored successfully".to_string()
    } else {
        "Machine ID restoration completed with some errors".to_string()
    };

    Ok(RestoreResult {
        success,
        message,
        details,
    })
}

#[tauri::command]
async fn get_cursor_paths() -> Result<(String, String), String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    Ok((
        restorer.db_path.to_string_lossy().to_string(),
        restorer.sqlite_path.to_string_lossy().to_string(),
    ))
}

#[tauri::command]
async fn check_cursor_installation() -> Result<bool, String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    Ok(restorer.db_path.exists() || restorer.sqlite_path.exists())
}

#[tauri::command]
async fn reset_machine_ids() -> Result<ResetResult, String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    restorer
        .reset_machine_ids()
        .map_err(|e| format!("Failed to reset machine IDs: {}", e))
}

#[tauri::command]
async fn complete_cursor_reset() -> Result<ResetResult, String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    restorer
        .complete_cursor_reset()
        .map_err(|e| format!("Failed to complete Cursor reset: {}", e))
}

/// 获取 cursor-updater 路径
fn get_cursor_updater_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        let local_app_data = std::env::var("LOCALAPPDATA")
            .map_err(|_| "无法获取 LOCALAPPDATA 路径".to_string())?;
        Ok(PathBuf::from(local_app_data).join("cursor-updater"))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(dirs::home_dir()
            .ok_or("无法获取用户主目录".to_string())?
            .join("Library")
            .join("Application Support")
            .join("cursor-updater"))
    }
    #[cfg(target_os = "linux")]
    {
        Ok(dirs::home_dir()
            .ok_or("无法获取用户主目录".to_string())?
            .join(".config")
            .join("cursor-updater"))
    }
}

/// 获取自动更新状态
#[tauri::command]
async fn get_auto_update_status() -> Result<serde_json::Value, String> {
    let updater_path = get_cursor_updater_path()?;
    let disabled = updater_path.exists() && updater_path.is_file();

    Ok(serde_json::json!({
        "disabled": disabled,
        "path": updater_path.to_string_lossy()
    }))
}

/// 禁用自动更新：删除 cursor-updater 目录，创建同名只读空文件
#[tauri::command]
async fn disable_auto_update() -> Result<serde_json::Value, String> {
    let updater_path = get_cursor_updater_path()?;

    // 如果已经是阻止文件，无需操作
    if updater_path.exists() && updater_path.is_file() {
        return Ok(serde_json::json!({ "success": true, "message": "自动更新已处于禁用状态" }));
    }

    // 如果是目录，先删除
    if updater_path.exists() && updater_path.is_dir() {
        fs::remove_dir_all(&updater_path)
            .map_err(|e| format!("删除 cursor-updater 目录失败: {}", e))?;
    }

    // 创建空文件
    fs::write(&updater_path, "")
        .map_err(|e| format!("创建阻止文件失败: {}", e))?;

    // Windows: 设置只读属性
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let _ = Command::new("attrib")
            .args(["+R", &updater_path.to_string_lossy()])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }

    // macOS/Linux: 设置只读权限
    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&updater_path, fs::Permissions::from_mode(0o444));
    }

    log_info!("✅ 已禁用 Cursor 自动更新: {:?}", updater_path);
    Ok(serde_json::json!({ "success": true, "message": "已禁用自动更新" }))
}

/// 恢复自动更新：删除阻止文件
#[tauri::command]
async fn enable_auto_update() -> Result<serde_json::Value, String> {
    let updater_path = get_cursor_updater_path()?;

    if updater_path.exists() && updater_path.is_file() {
        // 先去掉只读属性
        #[cfg(target_os = "windows")]
        {
            use std::process::Command;
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            let _ = Command::new("attrib")
                .args(["-R", &updater_path.to_string_lossy()])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        }
        #[cfg(not(target_os = "windows"))]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&updater_path, fs::Permissions::from_mode(0o755));
        }

        fs::remove_file(&updater_path)
            .map_err(|e| format!("删除阻止文件失败: {}", e))?;
    }

    log_info!("✅ 已恢复 Cursor 自动更新");
    Ok(serde_json::json!({ "success": true, "message": "已恢复自动更新" }))
}

/// 列出 Windows 系统中可同步的用户（仅 Windows）
#[tauri::command]
async fn list_windows_users() -> Result<serde_json::Value, String> {
    #[cfg(not(target_os = "windows"))]
    {
        return Ok(serde_json::json!({ "success": false, "message": "仅支持 Windows", "users": [] }));
    }

    #[cfg(target_os = "windows")]
    {
        let users_dir = PathBuf::from(r"C:\Users");
        if !users_dir.exists() {
            return Ok(serde_json::json!({ "success": false, "message": "未找到 C:\\Users 目录", "users": [] }));
        }

        let current_user = std::env::var("USERNAME").unwrap_or_default();
        let skip_names: std::collections::HashSet<&str> = [
            "Public", "Default", "Default User", "All Users", "desktop.ini",
        ].iter().copied().collect();

        let mut users = Vec::new();
        if let Ok(entries) = fs::read_dir(&users_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !entry.path().is_dir() || skip_names.contains(name.as_str()) || name == current_user {
                    continue;
                }
                let cursor_path = entry.path().join("AppData").join("Roaming").join("Cursor");
                users.push(serde_json::json!({
                    "username": name,
                    "has_cursor": cursor_path.exists(),
                    "cursor_path": cursor_path.to_string_lossy(),
                }));
            }
        }

        Ok(serde_json::json!({ "success": true, "users": users }))
    }
}

/// 同步当前 Cursor 账号到目标 Windows 用户
#[tauri::command]
async fn sync_account_to_user(_target_username: String) -> Result<serde_json::Value, String> {
    #[cfg(not(target_os = "windows"))]
    {
        return Ok(serde_json::json!({ "success": false, "message": "仅支持 Windows" }));
    }

    #[cfg(target_os = "windows")]
    {
        let mut details = Vec::new();

        // 1. 关闭所有 Cursor 进程
        log_info!("🔄 同步前关闭所有 Cursor 进程...");
        if AccountManager::is_cursor_running() {
            match AccountManager::force_close_cursor() {
                Ok(()) => details.push("已关闭 Cursor 进程".to_string()),
                Err(e) => details.push(format!("Warning: 关闭 Cursor 失败: {}", e)),
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }

        // 2. 提取当前用户的 token 和机器码
        let email = AccountManager::get_current_email()
            .ok_or("无法获取当前登录邮箱，请确保已在 Cursor 中登录")?;
        let token = AccountManager::get_current_token()
            .ok_or("无法获取当前 token")?;

        let restorer = MachineIdRestorer::new()
            .map_err(|e| format!("初始化失败: {}", e))?;
        let machine_ids = restorer.get_current_machine_ids()
            .map_err(|e| format!("获取机器码失败: {}", e))?
            .ok_or("当前无机器码信息")?;

        details.push(format!("提取账号: {}", email));

        // 3. 构建目标路径
        let target_cursor = PathBuf::from(format!(r"C:\Users\{}\AppData\Roaming\Cursor", target_username));
        let target_storage_dir = target_cursor.join("User").join("globalStorage");
        let target_storage_json = target_storage_dir.join("storage.json");
        let target_sqlite = target_storage_dir.join("state.vscdb");

        // 确保目录存在
        if let Err(e) = fs::create_dir_all(&target_storage_dir) {
            return Ok(serde_json::json!({ "success": false, "message": format!("创建目标目录失败: {}", e) }));
        }

        // 4. 写入 storage.json（机器码 + 认证字段）
        let storage_data = if target_storage_json.exists() {
            let content = fs::read_to_string(&target_storage_json).unwrap_or_else(|_| "{}".to_string());
            serde_json::from_str::<serde_json::Value>(&content).unwrap_or_else(|_| serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        if let Some(obj) = storage_data.as_object().cloned().as_mut() {
            obj.insert("telemetry.devDeviceId".into(), serde_json::Value::String(machine_ids.dev_device_id.clone()));
            obj.insert("telemetry.macMachineId".into(), serde_json::Value::String(machine_ids.mac_machine_id.clone()));
            obj.insert("telemetry.machineId".into(), serde_json::Value::String(machine_ids.machine_id.clone()));
            obj.insert("telemetry.sqmId".into(), serde_json::Value::String(machine_ids.sqm_id.clone()));
            obj.insert("storage.serviceMachineId".into(), serde_json::Value::String(machine_ids.service_machine_id.clone()));
            obj.insert("cursorAuth/cachedEmail".into(), serde_json::Value::String(email.clone()));
            obj.insert("cursorAuth/accessToken".into(), serde_json::Value::String(token.clone()));
            obj.insert("cursorAuth/refreshToken".into(), serde_json::Value::String(token.clone()));
            obj.insert("cursorAuth/cachedSignUpType".into(), serde_json::Value::String("Auth_0".to_string()));

            let content = serde_json::to_string_pretty(&serde_json::Value::Object(obj.clone()))
                .map_err(|e| format!("序列化失败: {}", e))?;
            fs::write(&target_storage_json, content)
                .map_err(|e| format!("写入 storage.json 失败: {}", e))?;
            details.push("已写入 storage.json".to_string());
        }

        // 5. 写入 state.vscdb
        if target_sqlite.exists() {
            match rusqlite::Connection::open(&target_sqlite) {
                Ok(conn) => {
                    let _ = conn.execute("BEGIN TRANSACTION", []);
                    let fields = vec![
                        ("cursorAuth/accessToken", token.as_str()),
                        ("cursorAuth/refreshToken", token.as_str()),
                        ("cursorAuth/cachedEmail", email.as_str()),
                        ("cursorAuth/cachedSignUpType", "Auth_0"),
                        ("cursor.email", email.as_str()),
                        ("cursor.accessToken", token.as_str()),
                        ("storage.serviceMachineId", machine_ids.service_machine_id.as_str()),
                    ];
                    for (key, value) in fields {
                        let exists: i64 = conn.query_row(
                            "SELECT COUNT(*) FROM ItemTable WHERE key = ?", [key], |row| row.get(0),
                        ).unwrap_or(0);
                        if exists > 0 {
                            let _ = conn.execute("UPDATE ItemTable SET value = ? WHERE key = ?", [value, key]);
                        } else {
                            let _ = conn.execute("INSERT INTO ItemTable (key, value) VALUES (?, ?)", [key, value]);
                        }
                    }
                    let _ = conn.execute("COMMIT", []);
                    details.push("已写入 state.vscdb".to_string());
                }
                Err(e) => details.push(format!("Warning: 打开 state.vscdb 失败: {}", e)),
            }
        } else {
            details.push("目标用户无 state.vscdb，跳过数据库写入".to_string());
        }

        // 6. 写入 machineId 文件
        let target_machine_id_file = PathBuf::from(format!(r"C:\Users\{}\AppData\Roaming\Cursor\machineId", target_username));
        if let Err(e) = fs::write(&target_machine_id_file, &machine_ids.dev_device_id) {
            details.push(format!("Warning: 写入 machineId 文件失败: {}", e));
        } else {
            details.push("已写入 machineId 文件".to_string());
        }

        log_info!("✅ 同步完成: {} -> {}", email, target_username);

        Ok(serde_json::json!({
            "success": true,
            "message": format!("已同步账号 {} 到用户 {}", email, target_username),
            "details": details
        }))
    }
}

#[tauri::command]
async fn get_log_file_path() -> Result<String, String> {
    if let Some(log_path) = logger::Logger::get_log_path() {
        Ok(log_path.to_string_lossy().to_string())
    } else {
        Err("Logger not initialized".to_string())
    }
}

#[tauri::command]
async fn get_log_config() -> Result<serde_json::Value, String> {
    let (max_size_mb, log_file_name) = logger::get_log_config();
    Ok(serde_json::json!({
        "max_size_mb": max_size_mb,
        "log_file_name": log_file_name,
        "log_file_path": logger::Logger::get_log_path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "Not initialized".to_string())
    }))
}

#[tauri::command]
async fn test_logging() -> Result<String, String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    restorer
        .test_logging()
        .map_err(|e| format!("Failed to test logging: {}", e))
}

#[tauri::command]
async fn debug_windows_cursor_paths() -> Result<Vec<String>, String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    restorer
        .debug_windows_cursor_paths()
        .map_err(|e| format!("Failed to debug Windows cursor paths: {}", e))
}

#[tauri::command]
async fn set_custom_cursor_path(path: String) -> Result<String, String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    restorer
        .set_custom_cursor_path(&path)
        .map_err(|e| format!("Failed to set custom cursor path: {}", e))
}

#[tauri::command]
async fn get_custom_cursor_path() -> Result<Option<String>, String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    Ok(restorer.get_custom_cursor_path())
}

#[tauri::command]
async fn clear_custom_cursor_path() -> Result<String, String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    restorer
        .clear_custom_cursor_path()
        .map_err(|e| format!("Failed to clear custom cursor path: {}", e))
}

#[tauri::command]
async fn open_log_file() -> Result<String, String> {
    // 使用新的日志系统获取日志文件路径
    let log_path = if let Some(path) = logger::Logger::get_log_path() {
        path
    } else {
        return Err("日志系统未初始化".to_string());
    };

    // 检查日志文件是否存在
    if !log_path.exists() {
        return Err("日志文件不存在，请先运行应用以生成日志".to_string());
    }

    let log_path_str = log_path.to_string_lossy().to_string();

    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        Command::new("cmd")
            .args(["/C", "start", "", &log_path_str])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Failed to open log file: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        Command::new("open")
            .arg(&log_path_str)
            .spawn()
            .map_err(|e| format!("Failed to open log file: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        Command::new("xdg-open")
            .arg(&log_path_str)
            .spawn()
            .map_err(|e| format!("Failed to open log file: {}", e))?;
    }

    Ok(format!("已打开日志文件: {}", log_path_str))
}

#[tauri::command]
async fn open_log_directory() -> Result<String, String> {
    // 使用新的日志系统获取日志文件路径
    let log_path = if let Some(path) = logger::Logger::get_log_path() {
        path
    } else {
        return Err("日志系统未初始化".to_string());
    };

    let log_dir = log_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let log_dir_str = log_dir.to_string_lossy().to_string();

    // 根据操作系统打开目录
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        Command::new("explorer")
            .arg(&log_dir_str)
            .spawn()
            .map_err(|e| format!("Failed to open log directory: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        Command::new("open")
            .arg(&log_dir_str)
            .spawn()
            .map_err(|e| format!("Failed to open log directory: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        Command::new("xdg-open")
            .arg(&log_dir_str)
            .spawn()
            .map_err(|e| format!("Failed to open log directory: {}", e))?;
    }

    Ok(format!("已打开日志目录: {}", log_dir_str))
}

#[tauri::command]
async fn get_current_machine_ids() -> Result<Option<MachineIds>, String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    restorer
        .get_current_machine_ids()
        .map_err(|e| format!("Failed to get current machine IDs: {}", e))
}

#[tauri::command]
async fn get_machine_id_file_content() -> Result<Option<String>, String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    restorer
        .get_machine_id_file_content()
        .map_err(|e| format!("Failed to get machine ID file content: {}", e))
}

#[tauri::command]
async fn get_backup_directory_info() -> Result<(String, Vec<String>), String> {
    let restorer =
        MachineIdRestorer::new().map_err(|e| format!("Failed to initialize restorer: {}", e))?;

    restorer
        .get_backup_directory_info()
        .map_err(|e| format!("Failed to get backup directory info: {}", e))
}

#[tauri::command]
async fn check_user_authorization(token: String) -> Result<AuthCheckResult, String> {
    AuthChecker::check_user_authorized(&token)
        .await
        .map_err(|e| format!("Failed to check user authorization: {}", e))
}

#[tauri::command]
async fn get_user_info(token: String) -> Result<AuthCheckResult, String> {
    AuthChecker::get_user_info(&token)
        .await
        .map_err(|e| format!("Failed to get user info: {}", e))
}

#[tauri::command]
async fn get_token_auto() -> Result<TokenInfo, String> {
    Ok(AuthChecker::get_token_auto())
}

#[tauri::command]
async fn debug_cursor_paths() -> Result<Vec<String>, String> {
    AuthChecker::debug_cursor_paths().map_err(|e| format!("Failed to debug cursor paths: {}", e))
}

// Account Management Commands
#[tauri::command]
async fn get_current_account() -> Result<Option<AccountInfo>, String> {
    AccountManager::get_current_account().map_err(|e| format!("{}", e))
}

#[tauri::command]
async fn get_account_list() -> Result<AccountListResult, String> {
    Ok(AccountManager::get_account_list())
}

#[tauri::command]
async fn add_account(
    email: String,
    token: String,
    refresh_token: Option<String>,
    workos_cursor_session_token: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<serde_json::Value, String> {
    match AccountManager::add_account(email.clone(), token, refresh_token, workos_cursor_session_token, tags) {
        Ok(updated) => {
            let msg = if updated {
                format!("账户 {} 已更新", email)
            } else {
                format!("账户 {} 添加成功", email)
            };
            Ok(serde_json::json!({ "success": true, "message": msg, "updated": updated }))
        }
        Err(e) => Ok(serde_json::json!({ "success": false, "message": format!("添加账户失败: {}", e) })),
    }
}

#[tauri::command]
async fn check_admin_privileges() -> Result<bool, String> {
    Ok(AccountManager::is_running_as_admin())
}

#[tauri::command]
async fn switch_account(email: String) -> Result<SwitchAccountResult, String> {
    let mut result = AccountManager::switch_account(email);
    if result.success {
        if let Ok(restorer) = MachineIdRestorer::new() {
            match restorer.update_product_json_checksums() {
                Ok(()) => result.details.push("product.json 校验和已更新".to_string()),
                Err(e) => result.details.push(format!("Warning: 校验和更新失败: {}", e)),
            }
        }
    }
    Ok(result)
}

#[tauri::command]
async fn switch_account_with_token(
    email: String,
    token: String,
    auth_type: Option<String>,
) -> Result<SwitchAccountResult, String> {
    let mut result = AccountManager::switch_account_with_token(email, token, auth_type);
    if result.success {
        if let Ok(restorer) = MachineIdRestorer::new() {
            match restorer.update_product_json_checksums() {
                Ok(()) => result.details.push("product.json 校验和已更新".to_string()),
                Err(e) => result.details.push(format!("Warning: 校验和更新失败: {}", e)),
            }
        }
    }
    Ok(result)
}

/// 切换账号（带机器码选项）
/// machine_id_option: "bound" | "new" | "none"
/// auto_restart: whether to force-close Cursor before switching
#[tauri::command]
async fn switch_account_with_options(
    email: String,
    machine_id_option: String,
    auto_restart: Option<bool>,
) -> Result<SwitchAccountResult, String> {
    let mut details = Vec::new();
    let should_restart = auto_restart.unwrap_or(true);

    // 加载账户
    let accounts = AccountManager::load_accounts()
        .map_err(|e| format!("加载账户失败: {}", e))?;
    let target = accounts.iter().find(|a| a.email == email)
        .ok_or_else(|| format!("未找到账户: {}", email))?;

    // 根据选项处理机器码
    match machine_id_option.as_str() {
        "bound" => {
            if let Some(ref ids) = target.machine_ids {
                log_info!("🔄 恢复账户绑定的机器码...");
                let restorer = MachineIdRestorer::new()
                    .map_err(|e| format!("初始化失败: {}", e))?;
                if let Err(e) = restorer.update_storage_file(ids) {
                    details.push(format!("Warning: 更新 storage.json 失败: {}", e));
                } else {
                    details.push("已恢复 storage.json 机器码".to_string());
                }
                match restorer.update_sqlite_db(ids) {
                    Ok(results) => details.extend(results),
                    Err(e) => details.push(format!("Warning: 更新 state.vscdb 失败: {}", e)),
                }
                if let Err(e) = restorer.update_machine_id_file(&ids.dev_device_id) {
                    details.push(format!("Warning: 更新 machineId 文件失败: {}", e));
                }
                match restorer.update_system_ids(ids) {
                    Ok(results) => details.extend(results),
                    Err(e) => details.push(format!("Warning: 更新系统 ID 失败: {}", e)),
                }
            } else {
                details.push("该账户无绑定机器码，跳过机器码恢复".to_string());
            }
        }
        "new" => {
            log_info!("🔄 生成新机器码...");
            let restorer = MachineIdRestorer::new()
                .map_err(|e| format!("初始化失败: {}", e))?;
            match restorer.reset_machine_ids() {
                Ok(result) => {
                    details.extend(result.details);
                    // 将新机器码保存到该账户
                    if let Ok(Some(new_ids)) = restorer.get_current_machine_ids() {
                        let mut updated_accounts = AccountManager::load_accounts()
                            .unwrap_or_default();
                        if let Some(acc) = updated_accounts.iter_mut().find(|a| a.email == email) {
                            acc.machine_ids = Some(new_ids);
                            let _ = AccountManager::save_accounts(&updated_accounts);
                            details.push("新机器码已绑定到该账户".to_string());
                        }
                    }
                }
                Err(e) => {
                    details.push(format!("重置机器码失败: {}", e));
                }
            }
        }
        _ => {
            details.push("不操作机器码".to_string());
        }
    }

    // 注入账号（email + token）
    let switch_result = AccountManager::switch_account_ex(email.clone(), should_restart);
    details.extend(switch_result.details);

    // 更新 product.json 校验和（inject_email_update_js 会修改 workbench.desktop.main.js，
    // 必须在所有文件修改完成后重新计算，否则 Cursor 会报完整性校验错误）
    if switch_result.success {
        match MachineIdRestorer::new() {
            Ok(restorer) => match restorer.update_product_json_checksums() {
                Ok(()) => details.push("product.json 校验和已更新".to_string()),
                Err(e) => {
                    log_warn!("[切换账号] 更新校验和失败: {}", e);
                    details.push(format!("Warning: 校验和更新失败: {}", e));
                }
            },
            Err(e) => details.push(format!("Warning: 初始化失败，无法更新校验和: {}", e)),
        }
    }

    Ok(SwitchAccountResult {
        success: switch_result.success,
        message: switch_result.message,
        details,
    })
}

#[tauri::command]
async fn edit_account(
    email: String,
    new_email: Option<String>,
    new_token: Option<String>,
    new_refresh_token: Option<String>,
    new_workos_cursor_session_token: Option<String>,
    new_stripe_url: Option<String>,
    new_username: Option<String>,
    new_tags: Option<Vec<String>>,
    new_machine_ids: Option<MachineIds>,
) -> Result<serde_json::Value, String> {
    log_info!("🔍 edit_account called for: {}", email);

    match AccountManager::edit_account(
        email.clone(),
        new_email,
        new_token,
        new_refresh_token,
        new_workos_cursor_session_token,
        new_stripe_url,
        new_username,
        new_tags,
        new_machine_ids,
    ) {
        Ok(()) => {
            log_info!("✅ [DEBUG] Account {} updated successfully", email);
            Ok(serde_json::json!({
                "success": true,
                "message": format!("Account {} updated successfully", email)
            }))
        }
        Err(e) => {
            log_error!("❌ [DEBUG] Failed to update account {}: {}", email, e);
            Ok(serde_json::json!({
                "success": false,
                "message": format!("Failed to update account: {}", e)
            }))
        }
    }
}

#[tauri::command]
async fn batch_update_tags(updates: Vec<(String, Vec<String>)>) -> Result<serde_json::Value, String> {
    log_info!("🔍 batch_update_tags called for {} accounts", updates.len());

    match AccountManager::batch_update_tags(updates) {
        Ok(result) => {
            log_info!(
                "✅ Batch update completed: {} updated, {} skipped, {} failed",
                result.success_count, result.skipped_count, result.fail_count
            );
            Ok(serde_json::json!({
                "success": result.fail_count == 0,
                "successCount": result.success_count,
                "failCount": result.fail_count,
                "skippedCount": result.skipped_count,
                "message": if result.fail_count == 0 {
                    format!("已更新 {} 个账户", result.success_count)
                } else {
                    format!("更新完成: 成功 {} 个, 跳过 {} 个, 失败 {} 个", 
                        result.success_count, result.skipped_count, result.fail_count)
                }
            }))
        }
        Err(e) => {
            log_error!("❌ Batch update failed: {}", e);
            Ok(serde_json::json!({
                "success": false,
                "successCount": 0,
                "failCount": 0,
                "skippedCount": 0,
                "message": format!("批量更新失败: {}", e)
            }))
        }
    }
}

#[tauri::command]
async fn remove_account(email: String) -> Result<serde_json::Value, String> {
    match AccountManager::remove_account(email.clone()) {
        Ok(()) => Ok(serde_json::json!({
            "success": true,
            "message": format!("Account {} removed successfully", email)
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("Failed to remove account: {}", e)
        })),
    }
}

#[tauri::command]
async fn logout_current_account() -> Result<LogoutResult, String> {
    Ok(AccountManager::logout_current_account())
}

#[tauri::command]
async fn export_accounts(export_path: String, selected_emails: Option<Vec<String>>) -> Result<serde_json::Value, String> {
    match AccountManager::export_accounts(export_path, selected_emails) {
        Ok(exported_path) => Ok(serde_json::json!({
            "success": true,
            "message": format!("账户导出成功: {}", exported_path),
            "exported_path": exported_path
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("导出失败: {}", e)
        })),
    }
}

/// Internal helper: Get access token from WorkOS session token using PKCE flow
async fn get_access_token_from_workos(workos_token: &str) -> Option<(String, Option<String>)> {
    use sha2::{Sha256, Digest};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    
    let client = get_http_client();
    
    // Generate PKCE values
    let verifier_bytes: [u8; 32] = rand::random();
    let verifier = URL_SAFE_NO_PAD.encode(&verifier_bytes);
    
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge_bytes = hasher.finalize();
    let challenge = URL_SAFE_NO_PAD.encode(&challenge_bytes);
    
    let uuid = uuid::Uuid::new_v4().to_string();
    
    log_info!("🔐 PKCE flow for WorkOS token: uuid={}", uuid);
    
    // Step 1: Call loginDeepCallbackControl
    let cookie_value = format!("WorkosCursorSessionToken={}", workos_token);
    let payload = serde_json::json!({
        "challenge": challenge,
        "uuid": uuid,
    });
    
    let auth_result = client
        .post("https://cursor.com/api/auth/loginDeepCallbackControl")
        .header("Cookie", &cookie_value)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;
    
    if let Err(e) = auth_result {
        log_error!("❌ PKCE step 1 failed: {}", e);
        return None;
    }
    
    let auth_response = auth_result.unwrap();
    if !auth_response.status().is_success() {
        log_error!("❌ PKCE step 1 HTTP error: {}", auth_response.status());
        return None;
    }
    
    // Step 2: Poll for result
    let poll_url = format!(
        "https://api2.cursor.sh/auth/poll?uuid={}&verifier={}",
        uuid, verifier
    );
    
    let poll_result = client
        .get(&poll_url)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;
    
    if let Err(e) = poll_result {
        log_error!("❌ PKCE step 2 failed: {}", e);
        return None;
    }
    
    let poll_response = poll_result.unwrap();
    if !poll_response.status().is_success() {
        log_error!("❌ PKCE step 2 HTTP error: {}", poll_response.status());
        return None;
    }
    
    let body = match poll_response.text().await {
        Ok(b) => b,
        Err(e) => {
            log_error!("❌ PKCE read body failed: {}", e);
            return None;
        }
    };
    
    let data: serde_json::Value = match serde_json::from_str(&body) {
        Ok(d) => d,
        Err(e) => {
            log_error!("❌ PKCE parse JSON failed: {}", e);
            return None;
        }
    };
    
    let access_token = data.get("accessToken").and_then(|v| v.as_str()).map(|s| s.to_string());
    let refresh_token = data.get("refreshToken").and_then(|v| v.as_str()).map(|s| s.to_string());
    
    if let Some(ref at) = access_token {
        log_info!("✅ PKCE success: got accessToken ({}...)", &at[..at.len().min(20)]);
        Some((at.clone(), refresh_token))
    } else {
        log_error!("❌ PKCE: no accessToken in response");
        None
    }
}

#[tauri::command]
async fn import_accounts(import_file_path: String) -> Result<serde_json::Value, String> {
    // Read and parse the JSON file
    let content = match std::fs::read_to_string(&import_file_path) {
        Ok(c) => c,
        Err(e) => return Ok(serde_json::json!({
            "success": false,
            "message": format!("读取文件失败: {}", e)
        })),
    };
    
    // Support both array and single object formats
    let accounts: Vec<serde_json::Value> = match serde_json::from_str::<Vec<serde_json::Value>>(&content) {
        Ok(a) => a,
        Err(_) => {
            // Try parsing as single object
            match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(obj) if obj.is_object() => vec![obj],
                _ => return Ok(serde_json::json!({
                    "success": false,
                    "message": "解析JSON失败: 应为数组或对象"
                })),
            }
        }
    };
    
    // Check if any accounts need processing (missing email or access_token but have workos_token)
    let mut needs_lookup = false;
    for account in &accounts {
        let email = account.get("email").and_then(|v| v.as_str()).unwrap_or("");
        let access_token = account.get("access_token")
            .or_else(|| account.get("token"))
            .or_else(|| account.get("accessToken"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let workos_token = account.get("workos_session_token")
            .or_else(|| account.get("workos_cursor_session_token"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !workos_token.is_empty() && (email.is_empty() || access_token.is_empty()) {
            needs_lookup = true;
            break;
        }
    }
    
    // If no lookup needed, use original import path
    if !needs_lookup {
        match AccountManager::import_accounts(import_file_path) {
            Ok(message) => return Ok(serde_json::json!({
                "success": true,
                "message": message
            })),
            Err(e) => return Ok(serde_json::json!({
                "success": false,
                "message": format!("导入失败: {}", e)
            })),
        }
    }
    
    // Process accounts that need email/token lookup
    let client = get_http_client();
    let mut processed_accounts: Vec<serde_json::Value> = Vec::new();
    let mut lookup_count = 0;
    let mut skip_count = 0;
    let mut dup_count = 0;
    let total = accounts.len();
    
    // Load existing accounts to check for duplicates before processing
    let existing_accounts = AccountManager::load_accounts().unwrap_or_default();
    let existing_workos_tokens: std::collections::HashSet<String> = existing_accounts
        .iter()
        .filter_map(|acc| acc.workos_cursor_session_token.clone())
        .filter(|t| !t.is_empty())
        .collect();
    let existing_emails: std::collections::HashSet<String> = existing_accounts
        .iter()
        .filter(|acc| !acc.email.is_empty())
        .map(|acc| acc.email.clone())
        .collect();
    
    for (idx, account) in accounts.into_iter().enumerate() {
        let email = account.get("email").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let access_token = account.get("access_token")
            .or_else(|| account.get("token"))
            .or_else(|| account.get("accessToken"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let workos_token = account.get("workos_session_token")
            .or_else(|| account.get("workos_cursor_session_token"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        
        // Skip if WorkOS token already exists in database
        if !workos_token.is_empty() && existing_workos_tokens.contains(&workos_token) {
            log_info!("⏭️ [{}/{}] 跳过重复 WorkOS token", idx + 1, total);
            dup_count += 1;
            continue;
        }
        
        // Skip if email already exists in database
        if !email.is_empty() && existing_emails.contains(&email) {
            log_info!("⏭️ [{}/{}] 跳过重复邮箱: {}", idx + 1, total, email);
            dup_count += 1;
            continue;
        }
        
        // If workos_session_token exists but email or access_token is missing
        if !workos_token.is_empty() && (email.is_empty() || access_token.is_empty()) {
            log_info!("📥 [{}/{}] Processing WorkOS token...", idx + 1, total);
            
            let mut new_account = account.clone();
            let obj = match new_account.as_object_mut() {
                Some(o) => o,
                None => {
                    skip_count += 1;
                    continue;
                }
            };
            
            // Step 1: Get email from /api/auth/me if missing
            if email.is_empty() {
                let cookie_value = format!("WorkosCursorSessionToken={}", workos_token);
                
                if let Ok(response) = client
                    .get("https://cursor.com/api/auth/me")
                    .header("Cookie", &cookie_value)
                    .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
                    .timeout(std::time::Duration::from_secs(15))
                    .send()
                    .await
                {
                    if response.status().as_u16() == 200 {
                        if let Ok(body) = response.text().await {
                            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&body) {
                                if let Some(fetched_email) = data.get("email").and_then(|v| v.as_str()) {
                                    obj.insert("email".to_string(), serde_json::json!(fetched_email));
                                    log_info!("✅ Found email: {}", fetched_email);
                                }
                            }
                        }
                    }
                }
                
                // Check if we got email
                if obj.get("email").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
                    log_warn!("⚠️ Failed to fetch email, skipping");
                    skip_count += 1;
                    continue;
                }
            }
            
            // Step 2: Get access token via PKCE if missing
            if access_token.is_empty() {
                log_info!("🔐 Getting access token via PKCE...");
                if let Some((at, rt)) = get_access_token_from_workos(&workos_token).await {
                    obj.insert("access_token".to_string(), serde_json::json!(at));
                    if let Some(refresh) = rt {
                        obj.insert("refresh_token".to_string(), serde_json::json!(refresh));
                    }
                    log_info!("✅ Got access token");
                } else {
                    log_warn!("⚠️ Failed to get access token, account will need manual refresh");
                }
            }
            
            processed_accounts.push(new_account);
            lookup_count += 1;
            continue;
        }
        
        processed_accounts.push(account);
    }
    
    if processed_accounts.is_empty() {
        let message = if dup_count > 0 {
            format!("所有 {} 个账户已存在，跳过导入", dup_count)
        } else if skip_count > 0 {
            format!("没有有效的账户可导入 (跳过 {} 个无效)", skip_count)
        } else {
            "没有有效的账户可导入".to_string()
        };
        return Ok(serde_json::json!({
            "success": dup_count > 0,
            "message": message
        }));
    }
    
    // Write processed accounts to temp file and import
    let temp_path = std::env::temp_dir().join("mycursor_import_temp.json");
    let temp_content = match serde_json::to_string_pretty(&processed_accounts) {
        Ok(c) => c,
        Err(e) => return Ok(serde_json::json!({
            "success": false,
            "message": format!("序列化失败: {}", e)
        })),
    };
    
    if let Err(e) = std::fs::write(&temp_path, temp_content) {
        return Ok(serde_json::json!({
            "success": false,
            "message": format!("写入临时文件失败: {}", e)
        }));
    }
    
    let result = match AccountManager::import_accounts(temp_path.to_string_lossy().to_string()) {
        Ok(message) => {
            let final_message = if lookup_count > 0 || skip_count > 0 || dup_count > 0 {
                format!("{} (自动处理 {} 个, 跳过 {} 个重复, {} 个无效)", message, lookup_count, dup_count, skip_count)
            } else {
                message
            };
            serde_json::json!({
                "success": true,
                "message": final_message
            })
        },
        Err(e) => serde_json::json!({
            "success": false,
            "message": format!("导入失败: {}", e)
        }),
    };
    
    let _ = std::fs::remove_file(&temp_path);
    Ok(result)
}

#[tauri::command]
async fn open_cancel_subscription_page(
    app: tauri::AppHandle,
    workos_cursor_session_token: String,
) -> Result<serde_json::Value, String> {
    log_info!("🔄 Opening cancel subscription page with WorkOS token...");

    let url = "https://cursor.com/dashboard?tab=billing";

    // 先尝试关闭已存在的窗口
    if let Some(existing_window) = app.get_webview_window("cancel_subscription") {
        log_info!("🔄 Closing existing cancel subscription window...");
        if let Err(e) = existing_window.close() {
            log_error!("❌ Failed to close existing window: {}", e);
        } else {
            log_info!("✅ Existing window closed successfully");
        }
        // 等待一小段时间确保窗口完全关闭
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // 创建新的 WebView 窗口（默认隐藏）
    let app_handle = app.clone();
    let webview_window = tauri::WebviewWindowBuilder::new(
        &app,
        "cancel_subscription",
        tauri::WebviewUrl::External(url.parse().unwrap()),
    )
    .title("Cursor - 取消订阅")
    .inner_size(1200.0, 800.0)
    .resizable(true)
    .initialization_script(&format!(
        r#"
        // 在页面加载前设置 Cookie
        document.cookie = 'WorkosCursorSessionToken={}; domain=.cursor.com; path=/; secure; samesite=none';
        console.log('Cookie injected via initialization script');
        
        // 可选：检查 Cookie 是否设置成功
        console.log('Current cookies:', document.cookie);
        "#,
        workos_cursor_session_token
    ))
    .on_page_load(move |_window, _payload| {
        // 在页面加载完成时注入 Cookie
        let cus_script = r#"
            function findAndClickCancelButton () {
            console.log('Current page URL:', window.location.href);

            const manBtn = document.querySelector('.dashboard-outline-button') || document.querySelector('.dashboard-outline-button-medium')
            if (manBtn) {
                console.log('找到了');
                manBtn.click();
                setTimeout(() => {
                manBtn.click();
                setTimeout(() => {
                    manBtn.click();
                }, 1000)
                }, 1000)
                setTimeout(() => {
                window.__TAURI_INTERNALS__.invoke('show_cancel_subscription_window');
                }, 1500)
            } else {
                if (location.href.includes('dashboard')) {
                window.__TAURI_INTERNALS__.invoke('cancel_subscription_failed');
                console.log('没找到按钮');
                }
            }
            }
            if (document.readyState === 'complete') {
            console.log('页面已经加载完成');
            setTimeout(() => {
                findAndClickCancelButton()
            }, 2500)
            } else {
            // 监听页面加载完成事件
            window.addEventListener('load', function () {
                console.log('window load 事件触发');
                setTimeout(() => {
                findAndClickCancelButton()
                }, 2500)
            });
            }
            "#;
        
        if let Err(e) = _window.eval(cus_script) {
            log_error!("❌ Failed to inject page load: {}", e);
        } else {
            log_info!("✅ Page load injected successfully on page load");
        }
    })
    .visible(true) // 默认隐藏窗口
    .build();

    match webview_window {
        Ok(window) => {
            // 添加窗口关闭事件监听器
            let app_handle_clone = app_handle.clone();
            window.on_window_event(move |event| {
                match event {
                    tauri::WindowEvent::CloseRequested { .. } => {
                        log_info!("🔄 Cancel subscription window close requested by user");
                        // 用户手动关闭窗口时，调用失败处理
                        let app_handle_clone = app_handle_clone.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = cancel_subscription_failed(app_handle_clone).await {
                                log_error!("❌ Failed to handle window close: {}", e);
                            }
                        });
                    }
                    tauri::WindowEvent::Destroyed => {
                        log_info!("🔄 Cancel subscription window destroyed");
                    }
                    _ => {}
                }
            });
            
            log_info!("✅ Successfully opened WebView window");
            Ok(serde_json::json!({
                "success": true,
                "message": "已打开取消订阅页面，正在自动登录..."
            }))
        }
        Err(e) => {
            log_error!("❌ Failed to create WebView window: {}", e);
            Ok(serde_json::json!({
                "success": false,
                "message": format!("无法打开内置浏览器: {}", e)
            }))
        }
    }
}

#[tauri::command]
async fn show_cancel_subscription_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("cancel_subscription") {
        // 延迟1500ms再显示窗口
        tokio::time::sleep(tokio::time::Duration::from_millis(2500)).await;

        window
            .show()
            .map_err(|e| format!("Failed to show window: {}", e))?;
        log_info!("✅ Cancel subscription window shown");

        // 发送事件通知前端操作成功
        if let Err(e) = app.emit("cancel-subscription-success", ()) {
            log_error!("❌ Failed to emit success event: {}", e);
        }
    }
    Ok(())
}

#[tauri::command]
async fn cancel_subscription_failed(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("cancel_subscription") {
        window
            .close()
            .map_err(|e| format!("Failed to close window: {}", e))?;
        log_error!("❌ Cancel subscription failed, window closed");

        // 发送事件通知前端操作失败
        if let Err(e) = app.emit("cancel-subscription-failed", ()) {
            log_error!("❌ Failed to emit failed event: {}", e);
        }
    }
    Ok(())
}

/// 查看绑卡/订阅信息：调用 Stripe Session API 获取管理链接，用内置浏览器打开
#[tauri::command]
async fn open_bind_card_info(
    app: tauri::AppHandle,
    access_token: String,
    workos_cursor_session_token: Option<String>,
) -> Result<serde_json::Value, String> {
    let cookie = match &workos_cursor_session_token {
        Some(wt) if !wt.is_empty() => format!("WorkosCursorSessionToken={}", wt),
        _ => {
            let token_part = if access_token.contains("%3A%3A") {
                access_token.split("%3A%3A").nth(1).unwrap_or(&access_token)
            } else if access_token.contains("::") {
                access_token.split("::").nth(1).unwrap_or(&access_token)
            } else {
                &access_token
            };
            format!("WorkosCursorSessionToken=user_01000000000000000000000000%3A%3A{}", token_part)
        }
    };

    log_info!("🔗 获取 Stripe 订阅管理链接...");

    let client = get_http_client();
    let resp = client
        .get("https://cursor.com/api/stripeSession")
        .header("Cookie", &cookie)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        log_error!("❌ Stripe Session API 返回 {}: {}", status, body);
        return Ok(serde_json::json!({ "success": false, "message": format!("获取绑卡信息失败 ({})", status) }));
    }

    let mut url = resp.text().await.map_err(|e| format!("读取响应失败: {}", e))?;
    url = url.trim().trim_matches('"').to_string();

    if url.is_empty() || !url.starts_with("https://") {
        return Ok(serde_json::json!({ "success": false, "message": "该账户暂无绑卡信息" }));
    }

    log_info!("✅ 获取到 Stripe 管理链接，用内置浏览器打开...");

    // 关闭已有窗口
    if let Some(w) = app.get_webview_window("bind_card_info") {
        let _ = w.close();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    match tauri::WebviewWindowBuilder::new(
        &app,
        "bind_card_info",
        tauri::WebviewUrl::External(url.parse().map_err(|e| format!("URL 解析失败: {}", e))?),
    )
    .title("绑卡/订阅信息")
    .inner_size(1200.0, 800.0)
    .resizable(true)
    .visible(true)
    .devtools(true)
    .build()
    {
        Ok(_) => Ok(serde_json::json!({ "success": true, "message": "已打开绑卡信息页面" })),
        Err(e) => Ok(serde_json::json!({ "success": false, "message": format!("打开窗口失败: {}", e) })),
    }
}


/// 删除 Cursor 账户：调用 Cursor 官方 delete-account API
///
/// 优先使用 workos_cursor_session_token；若为空则用占位 userId 拼接 access_token。
#[tauri::command]
async fn delete_cursor_account(
    access_token: String,
    workos_cursor_session_token: Option<String>,
) -> Result<serde_json::Value, String> {
    log_info!("🔄 开始调用 Cursor 删除账户 API...");

    let cookie = match &workos_cursor_session_token {
        Some(wt) if !wt.is_empty() => format!("WorkosCursorSessionToken={}", wt),
        _ => {
            let token_part = if access_token.contains("%3A%3A") {
                access_token.split("%3A%3A").nth(1).unwrap_or(&access_token)
            } else if access_token.contains("::") {
                access_token.split("::").nth(1).unwrap_or(&access_token)
            } else {
                &access_token
            };
            format!("WorkosCursorSessionToken=user_01000000000000000000000000%3A%3A{}", token_part)
        }
    };

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("Accept", "*/*".parse().unwrap());
    headers.insert("Accept-Encoding", "gzip, deflate, br, zstd".parse().unwrap());
    headers.insert("Accept-Language", "en,zh-CN;q=0.9,zh;q=0.8,eu;q=0.7".parse().unwrap());
    headers.insert("Content-Type", "application/json".parse().unwrap());
    headers.insert("Content-Length", "2".parse().unwrap());
    headers.insert("Origin", "https://cursor.com".parse().unwrap());
    headers.insert("Referer", "https://cursor.com/dashboard?tab=settings".parse().unwrap());
    headers.insert("Sec-CH-UA", "\"Not;A=Brand\";v=\"99\", \"Google Chrome\";v=\"139\", \"Chromium\";v=\"139\"".parse().unwrap());
    headers.insert("Sec-CH-UA-Arch", "\"x86\"".parse().unwrap());
    headers.insert("Sec-CH-UA-Bitness", "\"64\"".parse().unwrap());
    headers.insert("Sec-CH-UA-Mobile", "?0".parse().unwrap());
    headers.insert("Sec-CH-UA-Platform", "\"macOS\"".parse().unwrap());
    headers.insert("Sec-CH-UA-Platform-Version", "\"15.3.1\"".parse().unwrap());
    headers.insert("Sec-Fetch-Dest", "empty".parse().unwrap());
    headers.insert("Sec-Fetch-Mode", "cors".parse().unwrap());
    headers.insert("Sec-Fetch-Site", "same-origin".parse().unwrap());
    headers.insert("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36".parse().unwrap());
    headers.insert("Cookie", cookie.parse().map_err(|e| format!("Invalid cookie: {}", e))?);

    // 使用全局 HTTP 客户端
    let client = get_http_client();

    // 发送请求
    match client
        .post("https://cursor.com/api/dashboard/delete-account")
        .headers(headers)
        .body("{}")
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            let headers_map: std::collections::HashMap<String, String> = response
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();

            log_debug!("📥 API 响应状态: {}", status);
            log_debug!("📥 响应头: {:?}", headers_map);

            match response.text().await {
                Ok(body) => {
                    log_debug!("📥 响应体: {}", body);

                    Ok(serde_json::json!({
                        "success": status.is_success(),
                        "status": status.as_u16(),
                        "message": if status.is_success() {
                            format!("✅ 删除账户请求成功！状态码: {}, 响应: {}", status, body)
                        } else {
                            format!("❌ 删除账户失败！状态码: {}, 响应: {}", status, body)
                        },
                        "response_body": body,
                        "response_headers": headers_map
                    }))
                }
                Err(e) => {
                    log_error!("❌ 读取响应体失败: {}", e);
                    Ok(serde_json::json!({
                        "success": false,
                        "status": status.as_u16(),
                        "message": format!("❌ 读取响应失败: {}", e),
                        "response_headers": headers_map
                    }))
                }
            }
        }
        Err(e) => {
            log_error!("❌ 网络请求失败: {}", e);
            Ok(serde_json::json!({
                "success": false,
                "message": format!("❌ 网络请求失败: {}", e)
            }))
        }
    }
}

#[tauri::command]
async fn trigger_authorization_login(
    uuid: String,
    challenge: String,
    workos_cursor_session_token: String,
) -> Result<serde_json::Value, String> {
    use reqwest::header::{HeaderMap, HeaderValue};

    log_info!("🔄 开始调用 Cursor 授权登录 API...");
    log_debug!("🔍 [DEBUG] UUID: {}", uuid);
    log_debug!("🔍 [DEBUG] Challenge: {}", challenge);

    // 构建请求头
    let mut headers = HeaderMap::new();
    // headers.insert("Accept", HeaderValue::from_static("*/*"));
    // headers.insert(
    //     "Accept-Encoding",
    //     HeaderValue::from_static("gzip, deflate, br, zstd"),
    // );
    // headers.insert(
    //     "Accept-Language",
    //     HeaderValue::from_static("en,zh-CN;q=0.9,zh;q=0.8,eu;q=0.7"),
    // );
    // headers.insert("Content-Type", HeaderValue::from_static("application/json"));
    // headers.insert("Content-Length", HeaderValue::from_static("2"));
    // headers.insert("Origin", HeaderValue::from_static("https://cursor.com"));
    // headers.insert(
    //     "Referer",
    //     HeaderValue::from_str(&format!(
    //         "https://cursor.com/cn/loginDeepControl?challenge={}&uuid={}&mode=login",
    //         challenge, uuid
    //     ))
    //     .map_err(|e| format!("Invalid Referer header value: {}", e))?,
    // );
    // headers.insert(
    //     "Sec-CH-UA",
    //     HeaderValue::from_static(
    //         "\"Not;A=Brand\";v=\"99\", \"Google Chrome\";v=\"139\", \"Chromium\";v=\"139\"",
    //     ),
    // );
    // headers.insert("Sec-CH-UA-Arch", HeaderValue::from_static("\"x86\""));
    // headers.insert("Sec-CH-UA-Bitness", HeaderValue::from_static("\"64\""));
    // headers.insert("Sec-CH-UA-Mobile", HeaderValue::from_static("?0"));
    // headers.insert("Sec-CH-UA-Platform", HeaderValue::from_static("\"macOS\""));
    // headers.insert(
    //     "Sec-CH-UA-Platform-Version",
    //     HeaderValue::from_static("\"15.3.1\""),
    // );
    // headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("empty"));
    // headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("cors"));
    // headers.insert("Sec-Fetch-Site", HeaderValue::from_static("same-origin"));
    // headers.insert("User-Agent", HeaderValue::from_static("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36"));

    // 使用传入的 WorkosCursorSessionToken
    let cookie_value = format!("WorkosCursorSessionToken={}", workos_cursor_session_token);
    log_info!(
        "🔍 [DEBUG] Using WorkosCursorSessionToken: {}...",
        &workos_cursor_session_token[..workos_cursor_session_token.len().min(50)]
    );
    headers.insert(
        "Cookie",
        HeaderValue::from_str(&cookie_value).map_err(|e| format!("Invalid cookie value: {}", e))?,
    );

    // 使用全局 HTTP 客户端
    let client = get_http_client();

    let payload = serde_json::json!({
        "challenge": challenge,
        "uuid": uuid,
    });

    // 发送请求
    match client
        .post("https://cursor.com/api/auth/loginDeepCallbackControl")
        .headers(headers)
        .json(&payload)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            let headers_map: std::collections::HashMap<String, String> = response
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();

            log_debug!("📥 API 响应状态: {}", status);
            log_debug!("📥 响应头: {:?}", headers_map);

            match response.text().await {
                Ok(body) => {
                    log_debug!("📥 响应体: {}", body);

                    Ok(serde_json::json!({
                        "success": status.is_success(),
                        "status": status.as_u16(),
                        "message": if status.is_success() {
                            format!("✅ 授权登录请求成功！状态码: {}, 响应: {}", status, body)
                        } else {
                            format!("❌ 授权登录失败！状态码: {}, 响应: {}", status, body)
                        },
                        "response_body": body,
                        "response_headers": headers_map
                    }))
                }
                Err(e) => {
                    log_error!("❌ 读取响应体失败: {}", e);
                    Ok(serde_json::json!({
                        "success": false,
                        "status": status.as_u16(),
                        "message": format!("❌ 读取授权登录响应失败: {}", e),
                        "response_headers": headers_map
                    }))
                }
            }
        }
        Err(e) => {
            log_error!("❌ 网络请求授权登录失败: {}", e);
            Ok(serde_json::json!({
                "success": false,
                "message": format!("❌ 网络请求授权登录失败: {}", e)
            }))
        }
    }
}

#[tauri::command]
async fn trigger_authorization_login_poll(
    uuid: String,
    verifier: String,
) -> Result<serde_json::Value, String> {
    use reqwest::header::{HeaderMap, HeaderValue};

    log_info!("🔄 开始调用 Cursor 授权登录 Poll API...");
    log_debug!("🔍 [DEBUG] UUID: {}", uuid);
    log_debug!("🔍 [DEBUG] verifier: {}", verifier);

    // 构建请求头
    let mut headers = HeaderMap::new();
    headers.insert("Accept", HeaderValue::from_static("*/*"));
    headers.insert(
        "Accept-Encoding",
        HeaderValue::from_static("gzip, deflate, br, zstd"),
    );
    headers.insert(
        "Accept-Language",
        HeaderValue::from_static("en,zh-CN;q=0.9,zh;q=0.8,eu;q=0.7"),
    );
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));
    headers.insert("Content-Length", HeaderValue::from_static("2"));
    headers.insert("Origin", HeaderValue::from_static("https://cursor.com"));
    headers.insert(
        "Sec-CH-UA",
        HeaderValue::from_static(
            "\"Not;A=Brand\";v=\"99\", \"Google Chrome\";v=\"139\", \"Chromium\";v=\"139\"",
        ),
    );
    headers.insert("Sec-CH-UA-Arch", HeaderValue::from_static("\"x86\""));
    headers.insert("Sec-CH-UA-Bitness", HeaderValue::from_static("\"64\""));
    headers.insert("Sec-CH-UA-Mobile", HeaderValue::from_static("?0"));
    headers.insert("Sec-CH-UA-Platform", HeaderValue::from_static("\"macOS\""));
    headers.insert(
        "Sec-CH-UA-Platform-Version",
        HeaderValue::from_static("\"15.3.1\""),
    );
    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("empty"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("cors"));
    headers.insert("Sec-Fetch-Site", HeaderValue::from_static("same-origin"));
    headers.insert("User-Agent", HeaderValue::from_static("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36"));

    // 使用全局 HTTP 客户端
    let client = get_http_client();

    // 发送请求
    match client
        .get(&format!(
            "https://api2.cursor.sh/auth/poll?uuid={}&verifier={}",
            uuid, verifier
        ))
        .headers(headers)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            let headers_map: std::collections::HashMap<String, String> = response
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();

            log_debug!("📥 API 响应状态: {}", status);
            log_debug!("📥 响应头: {:?}", headers_map);

            match response.text().await {
                Ok(body) => {
                    log_debug!("📥 响应体: {}", body);

                    Ok(serde_json::json!({
                        "success": status.is_success(),
                        "status": status.as_u16(),
                        "message": if status.is_success() {
                            format!("✅ 授权登录Poll请求成功！状态码: {}, 响应: {}", status, body)
                        } else {
                            format!("❌ 授权登录Poll失败！状态码: {}, 响应: {}", status, body)
                        },
                        "response_body": body,
                        "response_headers": headers_map
                    }))
                }
                Err(e) => {
                    log_error!("❌ 读取响应体失败: {}", e);
                    Ok(serde_json::json!({
                        "success": false,
                        "status": status.as_u16(),
                        "message": format!("❌ 读取授权登录Poll响应失败: {}", e),
                        "response_headers": headers_map
                    }))
                }
            }
        }
        Err(e) => {
            log_error!("❌ 网络请求授权登录Poll失败: {}", e);
            Ok(serde_json::json!({
                "success": false,
                "message": format!("❌ 网络请求授权登录Poll失败: {}", e)
            }))
        }
    }
}

#[tauri::command]
async fn get_usage_for_period(
    token: String,
    start_date: u64,
    end_date: u64,
    team_id: i32,
) -> Result<serde_json::Value, String> {
    log_info!(
        "🔍 获取用量数据请求: token长度={}, start_date={}, end_date={}, team_id={}",
        token.len(),
        start_date,
        end_date,
        team_id
    );

    match AuthChecker::get_usage_for_period(&token, start_date, end_date, team_id).await {
        Ok(Some(usage_data)) => {
            log_info!("✅ 成功获取用量数据");
            Ok(serde_json::json!({
                "success": true,
                "message": "Successfully retrieved usage data",
                "data": usage_data
            }))
        }
        Ok(None) => {
            log_warn!("⚠️ 未找到用量数据");
            Ok(serde_json::json!({
                "success": false,
                "message": "No usage data found"
            }))
        }
        Err(e) => {
            log_error!("❌ 获取用量数据失败: {}", e);
            Ok(serde_json::json!({
                "success": false,
                "message": format!("Failed to get usage data: {}", e)
            }))
        }
    }
}

#[tauri::command]
async fn get_user_analytics(
    token: String,
    team_id: i32,
    user_id: i32,
    start_date: String,
    end_date: String,
) -> Result<serde_json::Value, String> {
    log_info!(
        "🔍 获取用户分析数据 - team_id: {}, user_id: {}, 时间范围: {} 到 {}",
        team_id,
        user_id,
        start_date,
        end_date
    );

    match AuthChecker::get_user_analytics(&token, team_id, user_id, &start_date, &end_date).await {
        Ok(Some(analytics_data)) => {
            log_info!("✅ 成功获取用户分析数据");
            Ok(serde_json::json!({
                "success": true,
                "message": "Successfully retrieved user analytics data",
                "data": analytics_data
            }))
        }
        Ok(None) => {
            log_warn!("⚠️ 未找到用户分析数据");
            Ok(serde_json::json!({
                "success": false,
                "message": "No user analytics data found"
            }))
        }
        Err(e) => {
            log_error!("❌ 获取用户分析数据失败: {}", e);
            Ok(serde_json::json!({
                "success": false,
                "message": format!("Failed to get user analytics data: {}", e)
            }))
        }
    }
}

#[tauri::command]
async fn get_usage_events(
    token: String,
    team_id: i32,
    start_date: String,
    end_date: String,
    page: i32,
    page_size: i32,
) -> Result<serde_json::Value, String> {
    log_info!(
        "🔍 获取使用事件数据 - team_id: {}, 时间范围: {} 到 {}, 页码: {}, 页大小: {}",
        team_id,
        start_date,
        end_date,
        page,
        page_size
    );

    match AuthChecker::get_usage_events(&token, team_id, &start_date, &end_date, page, page_size)
        .await
    {
        Ok(Some(events_data)) => {
            log_info!("✅ 成功获取使用事件数据");
            Ok(serde_json::json!({
                "success": true,
                "message": "Successfully retrieved usage events data",
                "data": events_data
            }))
        }
        Ok(None) => {
            log_warn!("⚠️ 未找到使用事件数据");
            Ok(serde_json::json!({
                "success": false,
                "message": "No usage events data found"
            }))
        }
        Err(e) => {
            log_error!("❌ 获取使用事件数据失败: {}", e);
            Ok(serde_json::json!({
                "success": false,
                "message": format!("Failed to get usage events data: {}", e)
            }))
        }
    }
}

// 简化版事件数据获取接口 - 用于前端图表展示
#[tauri::command]
async fn get_events_v2(
    token: String,
    team_id: String,
    start_date: String,
    end_date: String,
) -> Result<serde_json::Value, String> {
    log_info!(
        "🔍 [get_events_v2] 获取事件数据 - team_id: {}, 时间范围: {} 到 {}",
        team_id,
        start_date,
        end_date
    );

    // 转换 team_id 为整数，如果失败则使用 0
    let team_id_int = team_id.parse::<i32>().unwrap_or(0);

    // 将 ISO 时间或其他格式统一转换为毫秒时间戳字符串（Cursor 后端对该接口更稳定）
    fn to_millis_string(input: &str) -> String {
        // 已经是纯数字（毫秒）则直接返回
        if input.chars().all(|c| c.is_ascii_digit()) {
            return input.to_string();
        }
        // 尝试 RFC3339 / ISO8601 解析
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(input) {
            return dt.timestamp_millis().to_string();
        }
        // 尝试常见日期格式（YYYY-MM-DD）
        if let Ok(naive) = chrono::NaiveDate::parse_from_str(input, "%Y-%m-%d") {
            if let Some(dt) = naive.and_hms_opt(0, 0, 0) {
                // 将日期视为 UTC 零点
                let dt_utc = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc);
                return dt_utc.timestamp_millis().to_string();
            }
        }
        // 回退：原样返回
        input.to_string()
    }

    let start_param = to_millis_string(&start_date);
    let end_param = to_millis_string(&end_date);

    // 按候选页大小逐步尝试，避免后端 500 错误
    let page_sizes: [i32; 4] = [200, 100, 50, 20];

    for page_size in page_sizes {
        let mut current_page: i32 = 1;
        let mut all_events: Vec<serde_json::Value> = Vec::new();
        let total_count: usize;

        // 先拉取首页，判断该 page_size 是否可用
        match AuthChecker::get_usage_events(
            &token,
            team_id_int,
            &start_param,
            &end_param,
            current_page,
            page_size,
        )
        .await
        {
            Ok(Some(events_data)) => {
                total_count = events_data.total_usage_events_count as usize;
                // 收集第一页
                for ev in events_data.usage_events_display.into_iter() {
                    if let Ok(val) = serde_json::to_value(ev) {
                        all_events.push(val);
                    }
                }

                log_info!(
                    "✅ [get_events_v2] 首页获取成功，page_size={}, 本页 {} 条，总计 {} 条",
                    page_size,
                    all_events.len(),
                    total_count
                );

                // 持续分页拉取直到达到总数或页数据为空
                while all_events.len() < total_count {
                    current_page += 1;
                    match AuthChecker::get_usage_events(
                        &token,
                        team_id_int,
                        &start_param,
                        &end_param,
                        current_page,
                        page_size,
                    )
                    .await
                    {
                        Ok(Some(page_data)) => {
                            let mut fetched_in_page = 0usize;
                            for ev in page_data.usage_events_display.into_iter() {
                                if let Ok(val) = serde_json::to_value(ev) {
                                    all_events.push(val);
                                    fetched_in_page += 1;
                                }
                            }

                            log_info!(
                                "📄 [get_events_v2] 第 {} 页获取 {} 条，累计 {} / {}",
                                current_page,
                                fetched_in_page,
                                all_events.len(),
                                total_count
                            );

                            if fetched_in_page == 0 {
                                // 提前结束，避免死循环
                                break;
                            }
                        }
                        Ok(None) => {
                            log_warn!(
                                "⚠️ [get_events_v2] 第 {} 页无数据，提前结束",
                                current_page
                            );
                            break;
                        }
                        Err(e) => {
                            log_error!(
                                "❌ [get_events_v2] 第 {} 页获取失败: {}，尝试使用更小的 page_size",
                                current_page,
                                e
                            );
                            // 本 page_size 失败，跳到下一个候选
                            all_events.clear();
                            // total_count 会在下一次循环中重新初始化
                            break;
                        }
                    }
                }

                // 若该 page_size 成功拿到任何数据，则返回
                if !all_events.is_empty() {
                    let total = total_count.max(all_events.len());
                    return Ok(serde_json::json!({
                        "success": true,
                        "message": "Successfully retrieved events data",
                        "events": all_events,
                        "total": total
                    }));
                }
            }
            Ok(None) => {
                log_warn!(
                    "⚠️ [get_events_v2] 使用 page_size={} 拉取首页失败（无数据），尝试更小的页大小",
                    page_size
                );
                // 继续使用更小的 page_size
            }
            Err(e) => {
                log_error!(
                    "❌ [get_events_v2] 使用 page_size={} 拉取首页异常: {}",
                    page_size,
                    e
                );
                // 继续尝试更小的 page_size
            }
        }
    }

    // 所有候选页大小均失败
    Ok(serde_json::json!({
        "success": false,
        "message": "Failed to retrieve events data after retries",
        "events": [],
        "total": 0
    }))
}

#[tauri::command]
async fn get_saved_accounts() -> Result<Vec<serde_json::Value>, String> {
    // 获取已保存的账户列表功能暂时不可用
    match AccountManager::load_accounts() {
        Ok(accounts) => {
            // 将AccountInfo转换为serde_json::Value
            let json_accounts: Vec<serde_json::Value> = accounts
                .into_iter()
                .map(|account| serde_json::to_value(account).unwrap_or(serde_json::Value::Null))
                .collect();
            Ok(json_accounts)
        }
        Err(e) => Err(format!("获取保存的账户失败: {}", e)),
    }
}


// 获取应用版本
#[tauri::command]
async fn get_app_version(app: tauri::AppHandle) -> Result<String, String> {
    let package_info = app.package_info();
    Ok(package_info.version.to_string())
}

// 打开更新链接
#[tauri::command]
async fn open_update_url(url: String) -> Result<(), String> {
    use std::process::Command;

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        Command::new("cmd")
            .args(["/C", "start", &url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }

    Ok(())
}

/// 打开内置浏览器让用户手动登录，持续轮询 Cookie 直到获取 WorkosCursorSessionToken
#[tauri::command]
async fn open_login_for_session_token(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    log_info!("🚀 打开登录页面获取 SessionToken");

    let window_label = "login_session_token";

    // 关闭已有窗口
    if let Some(w) = app.get_webview_window(window_label) {
        let _ = w.close();
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    let app_handle = app.clone();

    let webview_window = tauri::WebviewWindowBuilder::new(
        &app,
        window_label,
        tauri::WebviewUrl::External("https://authenticator.cursor.sh/".parse().unwrap()),
    )
    .title("Cursor - 登录获取 SessionToken")
    .inner_size(1200.0, 800.0)
    .resizable(true)
    .visible(true)
    .incognito(true)
    .build();

    match webview_window {
        Ok(window) => {
            // 后台轮询 Cookie，每 2 秒检测一次
            let win = window.clone();
            tokio::spawn(async move {
                let urls_to_try = [
                    "https://authenticator.cursor.sh/",
                    "https://cursor.com/",
                    "https://www.cursor.com/",
                ];

                for _ in 0..150 {
                    // 最多轮询 5 分钟
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                    // 窗口已被用户关闭则退出
                    if win.is_visible().unwrap_or(false) == false && win.is_minimized().unwrap_or(false) == false {
                        // 额外判断：尝试获取标题，失败说明窗口已销毁
                        if win.title().is_err() {
                            log_info!("🔒 登录窗口已关闭，停止轮询");
                            return;
                        }
                    }

                    for url_str in &urls_to_try {
                        let url = match url_str.parse() {
                            Ok(u) => u,
                            Err(_) => continue,
                        };
                        if let Ok(cookies) = win.cookies_for_url(url) {
                            for cookie in cookies {
                                if cookie.name() == "WorkosCursorSessionToken" {
                                    let token = cookie.value().to_string();
                                    log_info!("✅ 获取到 SessionToken: {}...", &token[..token.len().min(30)]);

                                    let _ = app_handle.emit("session-token-obtained", serde_json::json!({ "token": token }));

                                    let _ = win.close();
                                    return;
                                }
                            }
                        }
                    }
                }

                log_warn!("⏰ 轮询 SessionToken 超时（5分钟）");
                let _ = win.close();
            });

            Ok(serde_json::json!({ "success": true, "message": "登录窗口已打开" }))
        }
        Err(e) => {
            log_error!("❌ 创建登录窗口失败: {}", e);
            Ok(serde_json::json!({ "success": false, "message": format!("打开登录窗口失败: {}", e) }))
        }
    }
}

#[tauri::command]
async fn auto_login_and_get_cookie(
    app: tauri::AppHandle,
    email: String,
    password: String,
    show_window: Option<bool>,
) -> Result<serde_json::Value, String> {
    log_info!("🚀 开始自动登录获取Cookie: {}", email);

    // 检查是否已经有同名窗口，如果有则关闭
    if let Some(existing_window) = app.get_webview_window("auto_login") {
        log_info!("🔄 关闭现有的自动登录窗口");
        if let Err(e) = existing_window.close() {
            log_error!("❌ Failed to close existing auto login window: {}", e);
        } else {
            log_info!("✅ Existing auto login window closed successfully");
        }
        // 等待一小段时间确保窗口完全关闭
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // 根据参数决定是否显示窗口
    let should_show_window = show_window.unwrap_or(false);
    log_info!("🖥️ 窗口显示设置: {}", if should_show_window { "显示" } else { "隐藏" });
    
    // 创建新的 WebView 窗口（根据配置显示/隐藏，启用无痕模式）
    let webview_window = tauri::WebviewWindowBuilder::new(
        &app,
        "auto_login",
        tauri::WebviewUrl::External("https://authenticator.cursor.sh/".parse().unwrap()),
    )
    .title("Cursor - 自动登录")
    .inner_size(1200.0, 800.0)
    .resizable(true)
    .visible(should_show_window) // 根据参数决定是否显示
    .incognito(true) // 启用无痕模式
    .on_page_load(move |window, _payload| {
        let email_clone = email.clone();
        let password_clone = password.clone();
        
        // 创建自动登录脚本
        let login_script = format!(
            r#"
            (function() {{
                console.log('自动登录脚本已注入');
                
                function performLogin() {{
                    console.log('开始执行登录流程');
                    console.log('Current page URL:', window.location.href);
                    console.log('Page title:', document.title);
                    
                    // 检查是否已经登录成功（在dashboard页面）
                    if (window.location.href.includes('/dashboard')) {{
                        console.log('检测到已经在dashboard页面，直接获取cookie');
                        window.__TAURI_INTERNALS__.invoke('check_login_cookies');
                        return;
                    }}
                    
                    // 等待页面完全加载
                    if (document.readyState !== 'complete') {{
                        console.log('页面未完全加载，等待中...');
                        return;
                    }}
                    
                    // 步骤1: 填写邮箱
                    setTimeout(() => {{
                        console.log('步骤1: 填写邮箱');
                        const emailInput = document.querySelector('.rt-reset .rt-TextFieldInput');
                        if (emailInput) {{
                            emailInput.value = '{}';
                            console.log('邮箱已填写:', emailInput.value);
                            
                            // 触发input事件以确保值被正确设置
                            emailInput.dispatchEvent(new Event('input', {{ bubbles: true }}));
                            emailInput.dispatchEvent(new Event('change', {{ bubbles: true }}));
                        }} else {{
                            console.error('未找到邮箱输入框');
                        }}
                    }}, 1000);
                    
                    // 步骤2: 点击第一个按钮（继续）
                    setTimeout(() => {{
                        console.log('步骤2: 点击继续按钮');
                        const firstButton = document.querySelector('.BrandedButton');
                        if (firstButton) {{
                            firstButton.click();
                            console.log('继续按钮已点击');
                        }} else {{
                            console.error('未找到继续按钮');
                        }}
                    }}, 2000);
                    
                    // 步骤3: 填写密码
                    setTimeout(() => {{
                        console.log('步骤3: 填写密码');
                        const passwordInput = document.querySelector('[name="password"]');
                        if (passwordInput) {{
                            passwordInput.value = '{}';
                            console.log('密码已填写');
                            
                            // 触发input事件以确保值被正确设置
                            passwordInput.dispatchEvent(new Event('input', {{ bubbles: true }}));
                            passwordInput.dispatchEvent(new Event('change', {{ bubbles: true }}));
                        }} else {{
                            console.error('未找到密码输入框');
                        }}
                    }}, 6000);
                    
                    // 步骤4: 点击登录按钮
                    setTimeout(() => {{
                        console.log('步骤4: 点击登录按钮');
                        const loginButton = document.querySelector('.BrandedButton');
                        if (loginButton) {{
                            loginButton.click();
                            console.log('登录按钮已点击');
                            
                            // 等待登录完成后检查cookie
                            setTimeout(() => {{
                                console.log('检查登录状态和cookie');
                                checkLoginSuccess();
                            }}, 3000);
                        }} else {{
                            console.error('未找到登录按钮');
                        }}
                    }}, 9000);
                }}
                
                function checkLoginSuccess() {{
                    console.log('检查登录是否成功');
                    console.log('当前URL:', window.location.href);
                    
                    // 检查是否登录成功（通过URL变化或页面元素判断）
                    if (window.location.href.includes('/dashboard')) {{
                        console.log('登录成功，通知Rust获取cookie');
                        
                        // 通知Rust后端登录成功，让Rust获取httpOnly cookie
                        // window.__TAURI_INTERNALS__.invoke('check_login_cookies');
                    }} else {{
                        console.log('登录可能未完成，继续检查...');
                        // 再次检查
                        setTimeout(() => {{
                            checkLoginSuccess();
                        }}, 2000);
                    }}
                }}
                
                // 监听URL变化（用于检测重定向）
                let lastUrl = location.href;
                new MutationObserver(() => {{
                    const url = location.href;
                    if (url !== lastUrl) {{
                        lastUrl = url;
                        console.log('检测到URL变化:', url);
                        // 如果重定向到dashboard，直接获取cookie
                        if (url.includes('dashboard') || url.includes('app')) {{
                            console.log('重定向到dashboard，获取cookie');
                            setTimeout(() => {{
                                // window.__TAURI_INTERNALS__.invoke('check_login_cookies');
                            }}, 1000);
                        }}
                    }}
                }}).observe(document, {{ subtree: true, childList: true }});

                // 检查页面加载状态
                if (document.readyState === 'complete') {{
                    console.log('页面已经加载完成，开始登录流程');
                    setTimeout(() => {{
                        performLogin();
                    }}, 1000);
                }} else {{
                    // 监听页面加载完成事件
                    window.addEventListener('load', function() {{
                        console.log('window load 事件触发，开始登录流程');
                        setTimeout(() => {{
                            performLogin();
                        }}, 1000);
                    }});
                }}
            }})();
            "#,
            email_clone, password_clone
        );

        if let Err(e) = window.eval(&login_script) {
            log_error!("❌ Failed to inject login script: {}", e);
        } else {
            log_info!("✅ Login script injected successfully");
        }
    })
    .build();

    match webview_window {
        Ok(_window) => {
            let message = if should_show_window {
                "自动登录窗口已打开，正在执行登录流程..."
            } else {
                "正在后台执行自动登录流程..."
            };
            log_info!("✅ Successfully created auto login WebView window ({})", if should_show_window { "visible" } else { "hidden" });
            
            Ok(serde_json::json!({
                "success": true,
                "message": message
            }))
        }
        Err(e) => {
            log_error!("❌ Failed to create auto login WebView window: {}", e);
            Ok(serde_json::json!({
                "success": false,
                "message": format!("无法打开自动登录窗口: {}", e)
            }))
        }
    }
}

#[tauri::command]
async fn verification_code_login(
    app: tauri::AppHandle,
    email: String,
    verification_code: Option<String>,
    show_window: Option<bool>,
) -> Result<serde_json::Value, String> {
    log_info!("🚀 开始验证码登录: {}", email);

    // 检查是否已经有同名窗口，如果有则关闭
    if let Some(existing_window) = app.get_webview_window("verification_code_login") {
        log_info!("🔄 关闭现有的验证码登录窗口");
        if let Err(e) = existing_window.close() {
            log_error!("❌ Failed to close existing verification code login window: {}", e);
        } else {
            log_info!("✅ Existing verification code login window closed successfully");
        }
        // 等待一小段时间确保窗口完全关闭
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // 根据参数决定是否显示窗口
    let should_show_window = show_window.unwrap_or(false);
    log_info!("🖥️ 窗口显示设置: {}", if should_show_window { "显示" } else { "隐藏" });
    
    // 创建新的 WebView 窗口（根据配置显示/隐藏，启用无痕模式）
    let webview_window = tauri::WebviewWindowBuilder::new(
        &app,
        "verification_code_login",
        tauri::WebviewUrl::External("https://authenticator.cursor.sh/".parse().unwrap()),
    )
    .title("Cursor - 验证码登录")
    .inner_size(1200.0, 800.0)
    .resizable(true)
    .visible(should_show_window) // 根据参数决定是否显示
    .incognito(true) // 启用无痕模式
    .on_page_load(move |window, _payload| {
        let email_clone = email.clone();
        let code_clone = verification_code.clone().unwrap_or_default();
        
        // 创建验证码登录脚本（先用自动登录的脚本，你后面修改）
        let login_script = format!(
            r#"
            (function() {{
                console.log('验证码登录脚本已注入');
                
                function performLogin() {{
                    console.log('开始执行验证码登录流程');
                    console.log('Current page URL:', window.location.href);
                    console.log('Page title:', document.title);
                    
                    // 检查是否已经登录成功（在dashboard页面）
                    if (window.location.href.includes('/dashboard')) {{
                        console.log('检测到已经在dashboard页面，直接获取cookie');
                        window.__TAURI_INTERNALS__.invoke('check_verification_login_cookies');
                        return;
                    }}
                    
                    // 等待页面完全加载
                    if (document.readyState !== 'complete') {{
                        console.log('页面未完全加载，等待中...');
                        return;
                    }}
                    
                    // TODO: 你需要修改这里的脚本来实现验证码登录
                    // 步骤1: 填写邮箱
                    setTimeout(() => {{
                        console.log('步骤1: 填写邮箱');
                        const emailInput = document.querySelector('.rt-reset .rt-TextFieldInput');
                        if (emailInput) {{
                            emailInput.value = '{}';
                            console.log('邮箱已填写:', emailInput.value);
                            
                            // 触发input事件以确保值被正确设置
                            emailInput.dispatchEvent(new Event('input', {{ bubbles: true }}));
                            emailInput.dispatchEvent(new Event('change', {{ bubbles: true }}));
                        }} else {{
                            console.error('未找到邮箱输入框');
                        }}
                    }}, 1000);
                    
                    // 步骤2: 点击第一个按钮（继续）
                    setTimeout(() => {{
                        console.log('步骤2: 点击继续按钮');
                        const firstButton = document.querySelector('.BrandedButton');
                        if (firstButton) {{
                            firstButton.click();
                            console.log('继续按钮已点击');
                        }} else {{
                            console.error('未找到继续按钮');
                        }}
                    }}, 2000);
                            
                     // 点击验证码登录
                     setTimeout(() => {{
                        console.log('步骤2: 点击继续按钮');
                        const firstButton2 = document.querySelector('.rt-Button.ak-AuthButton');

                        if (firstButton2) {{
                            firstButton2.click();
                            console.log('继续按钮已点击');
                        }} else {{
                            console.error('未找到继续按钮');
                        }}
                    }}, 6000);
                    
                    // // 步骤3: 填写验证码（这里需要修改）
                    // setTimeout(() => {{
                    //     console.log('步骤3: 填写验证码');
                    //     // TODO: 修改为验证码输入框的选择器
                    //     const codeInput = document.querySelector('[name="verification_code"]');
                    //     if (codeInput) {{
                    //         codeInput.value = '{}';
                    //         console.log('验证码已填写');
                            
                    //         // 触发input事件以确保值被正确设置
                    //         codeInput.dispatchEvent(new Event('input', {{ bubbles: true }}));
                    //         codeInput.dispatchEvent(new Event('change', {{ bubbles: true }}));
                    //     }} else {{
                    //         console.error('未找到验证码输入框');
                    //     }}
                    // }}, 6000);
                    
                    // // 步骤4: 点击登录按钮
                    // setTimeout(() => {{
                    //     console.log('步骤4: 点击登录按钮');
                    //     const loginButton = document.querySelector('.BrandedButton');
                    //     if (loginButton) {{
                    //         loginButton.click();
                    //         console.log('登录按钮已点击');
                            
                    //         // 等待登录完成后检查cookie
                    //         setTimeout(() => {{
                    //             console.log('检查登录状态和cookie');
                    //             checkLoginSuccess();
                    //         }}, 3000);
                    //     }} else {{
                    //         console.error('未找到登录按钮');
                    //     }}
                    // }}, 9000);
                }}
                
                function checkLoginSuccess() {{
                    console.log('检查登录是否成功');
                    console.log('当前URL:', window.location.href);
                    
                    // 检查是否登录成功（通过URL变化或页面元素判断）
                    if (window.location.href.includes('/dashboard')) {{
                        console.log('登录成功，通知Rust获取cookie');
                        // 通知Rust后端登录成功，让Rust获取httpOnly cookie
                        // window.__TAURI_INTERNALS__.invoke('check_verification_login_cookies');
                    }} else {{
                        console.log('登录可能未完成，继续检查...');
                        // 再次检查
                        setTimeout(() => {{
                            checkLoginSuccess();
                        }}, 2000);
                    }}
                }}
                
                // 监听URL变化（用于检测重定向）
                let lastUrl = location.href;
                new MutationObserver(() => {{
                    const url = location.href;
                    if (url !== lastUrl) {{
                        lastUrl = url;
                        console.log('检测到URL变化:', url);
                        // 如果重定向到dashboard，直接获取cookie
                        if (url.includes('dashboard') || url.includes('app')) {{
                            console.log('重定向到dashboard，获取cookie');
                            setTimeout(() => {{
                                // window.__TAURI_INTERNALS__.invoke('check_verification_login_cookies');
                            }}, 1000);
                        }}
                    }}
                }}).observe(document, {{ subtree: true, childList: true }});

                // 检查页面加载状态
                if (document.readyState === 'complete') {{
                    console.log('页面已经加载完成，开始登录流程');
                    setTimeout(() => {{
                        performLogin();
                    }}, 1000);
                }} else {{
                    // 监听页面加载完成事件
                    window.addEventListener('load', function() {{
                        console.log('window load 事件触发，开始登录流程');
                        setTimeout(() => {{
                            performLogin();
                        }}, 1000);
                    }});
                }}
            }})();
            "#,
            email_clone, code_clone
        );

        if let Err(e) = window.eval(&login_script) {
            log_error!("❌ Failed to inject verification code login script: {}", e);
        } else {
            log_info!("✅ Verification code login script injected successfully");
        }
    })
    .build();

    match webview_window {
        Ok(_window) => {
            let message = if should_show_window {
                "验证码登录窗口已打开，正在执行登录流程..."
            } else {
                "正在后台执行验证码登录流程..."
            };
            log_info!("✅ Successfully created verification code login WebView window ({})", if should_show_window { "visible" } else { "hidden" });
            
            Ok(serde_json::json!({
                "success": true,
                "message": message
            }))
        }
        Err(e) => {
            log_error!("❌ Failed to create verification code login WebView window: {}", e);
            Ok(serde_json::json!({
                "success": false,
                "message": format!("无法打开验证码登录窗口: {}", e)
            }))
        }
    }
}

#[tauri::command]
async fn check_verification_login_cookies(app: tauri::AppHandle) -> Result<(), String> {
    log_info!("🔍 开始检查验证码登录Cookie");
    
    if let Some(window) = app.get_webview_window("verification_code_login") {
        // 尝试多个可能的URL来获取cookie
        let urls_to_try = vec![
            "https://authenticator.cursor.sh/",
            "https://cursor.com/",
            "https://app.cursor.com/",
            "https://www.cursor.com/",
        ];
        
        for url_str in urls_to_try {
            log_info!("🔍 尝试从 {} 获取cookie", url_str);
            let url = url_str.parse().map_err(|e| format!("Invalid URL {}: {}", url_str, e))?;
        
            match window.cookies_for_url(url) {
                Ok(cookies) => {
                    log_info!("📋 从 {} 找到 {} 个cookie", url_str, cookies.len());
                    
                    // 查找 WorkosCursorSessionToken
                    for cookie in cookies {
                        log_info!("🍪 Cookie: {} = {}...", cookie.name(), &cookie.value()[..cookie.value().len().min(20)]);
                        
                        if cookie.name() == "WorkosCursorSessionToken" {
                            let token = cookie.value().to_string();
                            log_info!("✅ 找到 WorkosCursorSessionToken: {}...", &token[..token.len().min(50)]);
                            
                            // 发送事件到前端
                            let _ = app.emit("verification-login-cookie-found", serde_json::json!({
                                "WorkosCursorSessionToken": token
                            }));
                            
                            // 关闭窗口
                            if let Err(e) = window.close() {
                                log_error!("❌ 关闭验证码登录窗口失败: {}", e);
                            } else {
                                log_info!("✅ 验证码登录窗口已关闭");
                            }
                            
                            return Ok(());
                        }
                    }
                }
                Err(e) => {
                    log_error!("❌ 从 {} 获取cookie失败: {}", url_str, e);
                }
            }
        }
        
        log_error!("❌ 未找到 WorkosCursorSessionToken");
        Err("未找到登录Token".to_string())
    } else {
        log_error!("❌ 未找到验证码登录窗口");
        Err("验证码登录窗口不存在".to_string())
    }
}

#[tauri::command]
async fn check_login_cookies(app: tauri::AppHandle) -> Result<(), String> {
    log_info!("🔍 开始检查登录Cookie");
    
    if let Some(window) = app.get_webview_window("auto_login") {
        // 尝试多个可能的URL来获取cookie
        let urls_to_try = vec![
            "https://authenticator.cursor.sh/",
            "https://cursor.com/",
            "https://app.cursor.com/",
            "https://www.cursor.com/",
        ];
        
        for url_str in urls_to_try {
            log_info!("🔍 尝试从 {} 获取cookie", url_str);
            let url = url_str.parse().map_err(|e| format!("Invalid URL {}: {}", url_str, e))?;
        
            match window.cookies_for_url(url) {
                Ok(cookies) => {
                    log_info!("📋 从 {} 找到 {} 个cookie", url_str, cookies.len());
                    
                    // 查找 WorkosCursorSessionToken
                    for cookie in cookies {
                        log_info!("🍪 Cookie: {} = {}...", cookie.name(), &cookie.value()[..cookie.value().len().min(20)]);
                        
                        if cookie.name() == "WorkosCursorSessionToken" {
                            let token = cookie.value().to_string();
                            log_info!("🎉 在 {} 找到 WorkosCursorSessionToken: {}...", url_str, &token[..token.len().min(20)]);
                            
                            // 关闭自动登录窗口
                            if let Err(e) = window.close() {
                                log_error!("❌ Failed to close auto login window: {}", e);
                            } else {
                                log_info!("✅ Auto login window closed successfully");
                            }
                            
                            // 发送事件通知前端获取到了token
                            if let Err(e) = app.emit("auto-login-success", serde_json::json!({
                                "token": token
                            })) {
                                log_error!("❌ Failed to emit auto login success event: {}", e);
                            } else {
                                log_info!("✅ Auto login success event emitted");
                            }
                            
                            return Ok(());
                        }
                    }
                }
                Err(e) => {
                    log_error!("❌ 从 {} 获取cookie失败: {}", url_str, e);
                }
            }
        }
        
        // 如果所有URL都没找到目标cookie
        log_info!("⏳ 在所有URL中都未找到 WorkosCursorSessionToken");
        if let Err(e) = app.emit("auto-login-failed", serde_json::json!({
            "error": "未找到 WorkosCursorSessionToken cookie"
        })) {
            log_error!("❌ Failed to emit auto login failed event: {}", e);
        }
    } else {
        log_error!("❌ 未找到自动登录窗口");
        if let Err(e) = app.emit("auto-login-failed", serde_json::json!({
            "error": "未找到自动登录窗口"
        })) {
            log_error!("❌ Failed to emit auto login failed event: {}", e);
        }
    }
    
    Ok(())
}

#[tauri::command]
async fn auto_login_success(
    app: tauri::AppHandle,
    token: String,
) -> Result<(), String> {
    log_info!("🎉 自动登录成功，获取到Token: {}...", &token[..token.len().min(20)]);
    
    // 关闭自动登录窗口
    if let Some(window) = app.get_webview_window("auto_login") {
        if let Err(e) = window.close() {
            log_error!("❌ Failed to close auto login window: {}", e);
        } else {
            log_info!("✅ Auto login window closed successfully");
        }
    }
    
    // 发送事件通知前端获取到了token
    if let Err(e) = app.emit("auto-login-success", serde_json::json!({
        "token": token
    })) {
        log_error!("❌ Failed to emit auto login success event: {}", e);
    } else {
        log_info!("✅ Auto login success event emitted");
    }
    
    Ok(())
}

#[tauri::command]
async fn auto_login_failed(app: tauri::AppHandle, error: String) -> Result<(), String> {
    log_error!("❌ 自动登录失败: {}", error);
    
    // 关闭自动登录窗口
    if let Some(window) = app.get_webview_window("auto_login") {
        if let Err(e) = window.close() {
            log_error!("❌ Failed to close auto login window: {}", e);
        }
    }
    
    // 发送事件通知前端登录失败
    if let Err(e) = app.emit("auto-login-failed", serde_json::json!({
        "error": error
    })) {
        log_error!("❌ Failed to emit auto login failed event: {}", e);
    }
    
    Ok(())
}

/// 打开 Cursor 主页（内置浏览器）
///
/// 流程：先加载 cursor.com -> on_page_load 注入 Cookie 并跳转到 dashboard（仅一次）
#[tauri::command]
async fn open_cursor_dashboard(
    app: tauri::AppHandle,
    workos_cursor_session_token: String,
) -> Result<serde_json::Value, String> {
    if workos_cursor_session_token.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "缺少 WorkOS Session Token，无法登录 Cursor 主页"
        }));
    }

    log_info!("🔄 打开 Cursor 主页...");

    if let Some(w) = app.get_webview_window("cursor_dashboard") {
        let _ = w.close();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    let token = workos_cursor_session_token.clone();
    let injected = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let injected_clone = injected.clone();
    match tauri::WebviewWindowBuilder::new(
        &app,
        "cursor_dashboard",
        tauri::WebviewUrl::External("https://cursor.com".parse().unwrap()),
    )
    .title("Cursor - 主页")
    .inner_size(1200.0, 800.0)
    .resizable(true)
    .visible(true)
    .devtools(true)
    .on_page_load(move |webview, payload| {
        // 只在首次加载 cursor.com 域名时注入 Cookie 并跳转，后续导航不再干扰
        if injected_clone.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }

        let url = payload.url().to_string();
        if !url.contains("cursor.com") {
            return;
        }

        injected_clone.store(true, std::sync::atomic::Ordering::Relaxed);

        let script = format!(
            r#"(function(){{
                document.cookie="WorkosCursorSessionToken={}; domain=.cursor.com; path=/; secure; max-age=31536000";
                document.cookie="NEXT_LOCALE=zh-CN; domain=.cursor.com; path=/; max-age=31536000";
                window.location.href="https://cursor.com/dashboard";
            }})();"#,
            token
        );
        let _ = webview.eval(&script);
    })
    .build()
    {
        Ok(_) => Ok(serde_json::json!({ "success": true, "message": "已打开 Cursor 主页" })),
        Err(e) => Ok(serde_json::json!({ "success": false, "message": format!("打开失败: {}", e) })),
    }
}

#[tauri::command]
async fn show_auto_login_window(app: tauri::AppHandle) -> Result<(), String> {
    log_info!("🔍 Attempting to show auto login window");

    if let Some(window) = app.get_webview_window("auto_login") {
        window
            .show()
            .map_err(|e| format!("Failed to show auto login window: {}", e))?;
        log_info!("✅ Auto login window shown successfully");
    } else {
        log_error!("❌ Auto login window not found");
        return Err("Auto login window not found".to_string());
    }

    Ok(())
}


// ---------------------------------------------------------------------------
// 无感换号命令
// ---------------------------------------------------------------------------

#[tauri::command]
async fn start_seamless_server(port: u16) -> Result<serde_json::Value, String> {
    seamless::start_server(port)?;
    Ok(serde_json::json!({"success": true, "message": format!("服务器已启动，端口 {}", port)}))
}

#[tauri::command]
async fn stop_seamless_server() -> Result<serde_json::Value, String> {
    seamless::stop_server()?;
    Ok(serde_json::json!({"success": true, "message": "服务器已停止"}))
}

#[tauri::command]
async fn inject_seamless(port: u16) -> Result<serde_json::Value, String> {
    seamless::inject_seamless(port)
}

#[tauri::command]
async fn restore_seamless() -> Result<serde_json::Value, String> {
    seamless::restore_seamless()
}

#[tauri::command]
async fn get_seamless_status() -> Result<seamless::SeamlessStatus, String> {
    seamless::get_seamless_status()
}

#[tauri::command]
async fn save_sort_settings(field: String, order: String) -> Result<serde_json::Value, String> {
    seamless::save_sort_settings(&field, &order)?;
    Ok(serde_json::json!({"success": true, "field": field, "order": order}))
}

/// 启动 Cursor 应用 (以普通用户权限，即使 MyCursor 以管理员运行)
#[tauri::command]
async fn launch_cursor() -> Result<serde_json::Value, String> {
    use std::process::Command;

    #[cfg(target_os = "windows")]
    {
        // 优先从自定义路径查找，其次标准安装路径
        let mut paths: Vec<std::path::PathBuf> = Vec::new();

        if let Ok(restorer) = MachineIdRestorer::new() {
            if let Some(custom) = restorer.get_custom_cursor_path() {
                // 自定义路径就是安装根目录，Cursor.exe 直接在里面
                paths.push(std::path::PathBuf::from(&custom).join("Cursor.exe"));
            }
        }

        // 标准安装路径
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            paths.push(std::path::PathBuf::from(&local).join("Programs").join("Cursor").join("Cursor.exe"));
        }

        for path in &paths {
            if path.exists() {
                log_info!("[启动Cursor] 找到 Cursor: {:?}", path);
                // 使用 explorer.exe 启动，这样即使 MyCursor 以管理员运行，
                // Cursor 也会以普通用户权限启动
                match Command::new("explorer.exe")
                    .arg(path)
                    .spawn()
                {
                    Ok(_) => {
                        return Ok(serde_json::json!({
                            "success": true,
                            "message": format!("Cursor 已启动 (普通权限): {}", path.display())
                        }));
                    }
                    Err(e) => {
                        log_error!("[启动Cursor] 启动失败: {}", e);
                    }
                }
            }
        }

        Ok(serde_json::json!({
            "success": false,
            "message": "未找到 Cursor.exe，请检查自定义路径设置"
        }))
    }

    #[cfg(target_os = "macos")]
    {
        match Command::new("open").arg("-a").arg("Cursor").spawn() {
            Ok(_) => Ok(serde_json::json!({ "success": true, "message": "Cursor 已启动" })),
            Err(e) => Ok(serde_json::json!({ "success": false, "message": format!("无法启动 Cursor: {}", e) })),
        }
    }

    #[cfg(target_os = "linux")]
    {
        match Command::new("cursor").spawn() {
            Ok(_) => Ok(serde_json::json!({ "success": true, "message": "Cursor 已启动" })),
            Err(e) => Ok(serde_json::json!({ "success": false, "message": format!("无法启动 Cursor: {}", e) })),
        }
    }
}

/// 通过 session token 调用 /api/auth/me 获取用户详细信息
#[tauri::command]
async fn get_auth_me(session_token: String) -> Result<serde_json::Value, String> {
    let client = get_http_client();

    let cookie_value = format!("WorkosCursorSessionToken={}", session_token);

    let response = client
        .get("https://cursor.com/api/auth/me")
        .header("Cookie", &cookie_value)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("请求 /api/auth/me 失败: {}", e))?;

    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {}", e))?;

    if status != 200 {
        return Ok(serde_json::json!({
            "success": false,
            "message": format!("HTTP {}: {}", status, body)
        }));
    }

    let data: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("解析响应 JSON 失败: {}", e))?;

    log_info!("[auth/me] 获取用户信息成功: {}", data.get("email").and_then(|v| v.as_str()).unwrap_or("?"));

    Ok(serde_json::json!({
        "success": true,
        "data": data
    }))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 第二个实例启动时，聚焦到已有窗口
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|_app| {
            // 初始化日志系统
            if let Err(e) = logger::Logger::init() {
                eprintln!("Failed to initialize logger: {}", e);
            } else {
                log_info!("Application starting up...");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![

            get_available_backups,
            extract_backup_ids,
            delete_backup,
            restore_machine_ids,
            get_cursor_paths,
            check_cursor_installation,
            reset_machine_ids,
            complete_cursor_reset,
            get_auto_update_status,
            disable_auto_update,
            enable_auto_update,
            list_windows_users,
            sync_account_to_user,
            get_log_file_path,
            get_log_config,
            test_logging,
            debug_windows_cursor_paths,
            set_custom_cursor_path,
            get_custom_cursor_path,
            clear_custom_cursor_path,
            open_log_file,
            open_log_directory,
            get_current_machine_ids,
            get_machine_id_file_content,
            get_backup_directory_info,
            check_user_authorization,
            get_user_info,
            get_token_auto,
            debug_cursor_paths,
            get_current_account,
            get_account_list,
            add_account,
            edit_account,
            batch_update_tags,
            check_admin_privileges,
            switch_account,
            switch_account_with_token,
            switch_account_with_options,
            remove_account,
            logout_current_account,
            export_accounts,
            import_accounts,
            open_cancel_subscription_page,
            show_cancel_subscription_window,
            cancel_subscription_failed,
            open_bind_card_info,
            delete_cursor_account,
            trigger_authorization_login,
            trigger_authorization_login_poll,
            get_usage_for_period,
            get_user_analytics,
            get_usage_events,
            get_events_v2,
            get_saved_accounts,
            get_app_version,
            open_update_url,
            open_login_for_session_token,
            auto_login_and_get_cookie,
            check_login_cookies,
            auto_login_success,
            auto_login_failed,
            show_auto_login_window,
            open_cursor_dashboard,
            verification_code_login,
            check_verification_login_cookies,

            save_usage_data_cache,
            load_usage_data_cache,
            clear_usage_data,
            save_events_data_cache,
            load_events_data_cache,
            clear_events_data,
            save_account_cache,
            load_account_cache,
            clear_account_cache,
            get_preset_tags,
            save_preset_tags,
            refresh_single_account_info,
            refresh_all_accounts_info,
            get_auth_me,
            launch_cursor,
            start_seamless_server,
            stop_seamless_server,
            inject_seamless,
            restore_seamless,
            get_seamless_status,
            save_sort_settings,
            get_proxy_settings,
            save_proxy_settings,
            rebuild_http_client_cmd
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsageCache {
    email: String,
    token: String,
    start_date: String,
    end_date: String,
    data: serde_json::Value,
    saved_at: i64,
}

/// 获取数据目录
/// - 若设置环境变量 MYCURSOR_DATA_DIR，则使用该路径（开发测试用）
/// - Windows: exe 同级的 cursor_data/
/// - macOS/Linux: ~/.cursor_data/
pub fn get_data_dir() -> Result<PathBuf, String> {
    let data_dir = if let Ok(custom) = std::env::var("MYCURSOR_DATA_DIR") {
        // 不要在这里调用 log，因为 logger 会调用 get_data_dir 导致无限递归
        eprintln!("[DEBUG] 使用自定义数据目录: {}", custom);
        PathBuf::from(custom)
    } else if cfg!(target_os = "windows") {
        get_app_dir()?.join("cursor_data")
    } else {
        dirs::home_dir()
            .ok_or("无法获取用户主目录".to_string())?
            .join(".cursor_data")
    };
    if let Err(e) = fs::create_dir_all(&data_dir) {
        log_error!("创建数据目录失败: {:?}, 错误: {}", data_dir, e);
    }
    Ok(data_dir)
}

// 获取用量数据文件路径
fn get_usage_data_file_path() -> Result<PathBuf, String> {
    let data_dir = get_data_dir()?;
    Ok(data_dir.join("usage_data.json"))
}

// 获取账户数据文件路径
fn get_account_data_file_path() -> Result<PathBuf, String> {
    let data_dir = get_data_dir()?;
    Ok(data_dir.join("account_cache.json"))
}

// 获取事件数据文件路径
fn get_events_data_file_path() -> Result<PathBuf, String> {
    let data_dir = get_data_dir()?;
    Ok(data_dir.join("events_data.json"))
}

// 保存用量数据缓存
#[tauri::command]
async fn save_usage_data_cache(cache_data: String) -> Result<serde_json::Value, String> {
    let cache: UsageCache = serde_json::from_str(&cache_data)
        .map_err(|e| format!("解析缓存数据失败: {}", e))?;
    
    let data_path = get_usage_data_file_path()?;
    
    // 读取现有数据
    let mut all_caches: Vec<UsageCache> = if data_path.exists() {
        let content = fs::read_to_string(&data_path)
            .map_err(|e| format!("读取数据文件失败: {}", e))?;
        serde_json::from_str(&content).unwrap_or_else(|_| Vec::new())
    } else {
        Vec::new()
    };
    
    // 更新或添加当前账户的缓存
    if let Some(index) = all_caches.iter().position(|c| c.email == cache.email) {
        all_caches[index] = cache;
    } else {
        all_caches.push(cache);
    }
    
    // 保存到文件
    let content = serde_json::to_string_pretty(&all_caches)
        .map_err(|e| format!("序列化数据失败: {}", e))?;
    
    fs::write(&data_path, content)
        .map_err(|e| format!("写入数据文件失败: {}", e))?;
    
    log_info!("用量数据已保存到: {:?}", data_path);
    
    Ok(serde_json::json!({
        "success": true,
        "message": "数据保存成功"
    }))
}

// 读取用量数据缓存
#[tauri::command]
async fn load_usage_data_cache(email: String) -> Result<serde_json::Value, String> {
    let data_path = get_usage_data_file_path()?;
    
    if !data_path.exists() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "没有缓存数据"
        }));
    }
    
    let content = fs::read_to_string(&data_path)
        .map_err(|e| format!("读取数据文件失败: {}", e))?;
    let all_caches: Vec<UsageCache> = serde_json::from_str(&content)
        .map_err(|e| format!("解析数据文件失败: {}", e))?;
    
    if let Some(cache) = all_caches.iter().find(|c| c.email == email) {
        Ok(serde_json::json!({
            "success": true,
            "data": cache
        }))
    } else {
        Ok(serde_json::json!({
            "success": false,
            "message": "未找到该账户的缓存数据"
        }))
    }
}

// 清除所有用量数据
#[tauri::command]
async fn clear_usage_data() -> Result<serde_json::Value, String> {
    let data_path = get_usage_data_file_path()?;
    
    if data_path.exists() {
        fs::remove_file(&data_path)
            .map_err(|e| format!("删除数据文件失败: {}", e))?;
        log_info!("用量数据已清除: {:?}", data_path);
    }
    
    Ok(serde_json::json!({
        "success": true,
        "message": "数据清除成功"
    }))
}

// 保存事件数据缓存
#[tauri::command]
async fn save_events_data_cache(events_data: String) -> Result<serde_json::Value, String> {
    let data_path = get_events_data_file_path()?;
    
    // 直接保存JSON数据（已经是JSON字符串）
    fs::write(&data_path, &events_data)
        .map_err(|e| format!("写入事件数据文件失败: {}", e))?;
    
    log_info!("事件数据已保存到: {:?}", data_path);
    
    Ok(serde_json::json!({
        "success": true,
        "message": "事件数据保存成功"
    }))
}

// 读取事件数据缓存
#[tauri::command]
async fn load_events_data_cache() -> Result<serde_json::Value, String> {
    let data_path = get_events_data_file_path()?;
    
    if !data_path.exists() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "没有事件数据缓存"
        }));
    }
    
    let content = fs::read_to_string(&data_path)
        .map_err(|e| format!("读取事件数据文件失败: {}", e))?;
    
    Ok(serde_json::json!({
        "success": true,
        "data": content
    }))
}

// 清除所有事件数据
#[tauri::command]
async fn clear_events_data() -> Result<serde_json::Value, String> {
    let data_path = get_events_data_file_path()?;
    
    if data_path.exists() {
        fs::remove_file(&data_path)
            .map_err(|e| format!("删除事件数据文件失败: {}", e))?;
        log_info!("事件数据已清除: {:?}", data_path);
    }
    
    Ok(serde_json::json!({
        "success": true,
        "message": "事件数据清除成功"
    }))
}

// 保存账户缓存数据
#[tauri::command]
async fn save_account_cache(accounts_json: String) -> Result<serde_json::Value, String> {
    let data_path = get_account_data_file_path()?;
    
    fs::write(&data_path, accounts_json)
        .map_err(|e| format!("写入账户缓存失败: {}", e))?;
    
    log_info!("账户数据已保存到: {:?}", data_path);
    
    Ok(serde_json::json!({
        "success": true,
        "message": "账户数据保存成功"
    }))
}

// 加载账户缓存数据
#[tauri::command]
async fn load_account_cache() -> Result<serde_json::Value, String> {
    let data_path = get_account_data_file_path()?;
    
    if !data_path.exists() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "没有缓存数据"
        }));
    }
    
    let content = fs::read_to_string(&data_path)
        .map_err(|e| format!("读取账户缓存失败: {}", e))?;
    
    Ok(serde_json::json!({
        "success": true,
        "data": content
    }))
}

// 清除账户缓存
#[tauri::command]
async fn clear_account_cache() -> Result<serde_json::Value, String> {
    let data_path = get_account_data_file_path()?;
    
    if data_path.exists() {
        fs::remove_file(&data_path)
            .map_err(|e| format!("删除账户缓存失败: {}", e))?;
        log_info!("账户缓存已清除: {:?}", data_path);
    }
    
    Ok(serde_json::json!({
        "success": true,
        "message": "账户缓存清除成功"
    }))
}

// 获取预设标签文件路径
fn get_preset_tags_file_path() -> Result<PathBuf, String> {
    let data_dir = get_data_dir()?;
    Ok(data_dir.join("preset_tags.json"))
}

// 获取预设标签列表
#[tauri::command]
async fn get_preset_tags() -> Result<serde_json::Value, String> {
    let data_path = get_preset_tags_file_path()?;

    if !data_path.exists() {
        return Ok(serde_json::json!({
            "success": true,
            "tags": []
        }));
    }

    let content = fs::read_to_string(&data_path)
        .map_err(|e| format!("读取预设标签失败: {}", e))?;

    let tags: Vec<String> = serde_json::from_str(&content)
        .unwrap_or_default();

    Ok(serde_json::json!({
        "success": true,
        "tags": tags
    }))
}

// 保存预设标签列表
#[tauri::command]
async fn save_preset_tags(tags: Vec<String>) -> Result<serde_json::Value, String> {
    let data_path = get_preset_tags_file_path()?;

    let json = serde_json::to_string_pretty(&tags)
        .map_err(|e| format!("序列化预设标签失败: {}", e))?;

    fs::write(&data_path, json)
        .map_err(|e| format!("保存预设标签失败: {}", e))?;

    log_info!("预设标签已保存: {:?}, 共 {} 个", data_path, tags.len());

    Ok(serde_json::json!({
        "success": true,
        "message": "预设标签保存成功"
    }))
}

// 刷新单个账户信息（不使用缓存，直接从API获取）
#[tauri::command]
async fn refresh_single_account_info(token: String) -> Result<serde_json::Value, String> {
    match AuthChecker::get_user_info(&token).await {
        Ok(result) => Ok(serde_json::json!(result)),
        Err(e) => Err(format!("获取账户信息失败: {}", e)),
    }
}

// 刷新所有账户信息
#[tauri::command]
async fn refresh_all_accounts_info(tokens: Vec<String>) -> Result<serde_json::Value, String> {
    let mut results = Vec::new();
    
    for token in tokens {
        match AuthChecker::get_user_info(&token).await {
            Ok(result) => {
                results.push(serde_json::json!({
                    "token": token,
                    "success": result.success,
                    "user_info": result.user_info
                }));
            }
            Err(e) => {
                results.push(serde_json::json!({
                    "token": token,
                    "success": false,
                    "error": e.to_string()
                }));
            }
        }
    }
    
    Ok(serde_json::json!({
        "success": true,
        "results": results
    }))
}

