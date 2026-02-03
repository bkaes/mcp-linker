use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::command;

/// Special identifier for global MCP config on Windows native (applies to all projects)
pub const GLOBAL_WINDOWS_ID: &str = "Global (Windows)";
/// Special identifier for global MCP config on WSL (applies to all projects)
pub const GLOBAL_WSL_ID: &str = "Global (WSL)";
/// Legacy identifier - kept for backwards compatibility
pub const GLOBAL_PROJECT_ID: &str = "Global";

// ~/.claude.json structure:
//   - Root "mcpServers": {} = user-scope servers (available everywhere)
//   - "projects": { "/path": { "mcpServers": {} } } = local-scope servers (per-project)
// Server format example:
// {'sentry': {'type': 'http', 'url': 'https://mcp.sentry.dev/mcp'},
//  'airtable': {'type': 'stdio', 'command': 'npx', 'args': ['-y', 'airtable-mcp-server'], 'env': {'AIRTABLE_API_KEY': 'YOUR_KEY'}}}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClaudeCodeServer {
    pub name: String,
    pub r#type: String, // "http", "sse", "stdio"
    pub url: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClaudeCodeResponse {
    pub success: bool,
    pub message: String,
}

/// List all MCP servers configured in Claude Code
/// If working_dir is "Global", reads from ~/.claude.json root mcpServers (user-scope)
/// Otherwise reads from ~/.claude.json projects[working_dir].mcpServers (local-scope)
#[command]
pub async fn claude_mcp_list(working_dir: String) -> Result<Vec<ClaudeCodeServer>, String> {
    let mut servers = Vec::new();
    let claude_config_path = get_claude_config_path(Some(working_dir.clone()))?;

    if !claude_config_path.exists() {
        return Ok(Vec::new());
    }

    let config_content = fs::read_to_string(&claude_config_path)
        .map_err(|e| format!("Failed to read Claude config: {}", e))?;

    let config: serde_json::Value = serde_json::from_str(&config_content)
        .map_err(|e| format!("Failed to parse Claude config: {}", e))?;

    if is_global_config(&working_dir) {
        // Read from root-level mcpServers (user-scope config)
        if let Some(mcp_servers) = config.get("mcpServers") {
            if let Some(servers_obj) = mcp_servers.as_object() {
                for (name, server_config) in servers_obj {
                    if let Ok(server) = parse_server_config(name, server_config) {
                        servers.push(server);
                    }
                }
            }
        }
    } else {
        // Read from per-project config (local-scope)
        if let Some(projects) = config.get("projects") {
            if let Some(project_config) = projects.get(&working_dir) {
                if let Some(mcp_servers) = project_config.get("mcpServers") {
                    if let Some(servers_obj) = mcp_servers.as_object() {
                        for (name, server_config) in servers_obj {
                            if let Ok(server) = parse_server_config(name, server_config) {
                                servers.push(server);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(servers)
}

/// Get details for a specific MCP server
#[command]
pub async fn claude_mcp_get(name: String, working_dir: String) -> Result<ClaudeCodeServer, String> {
    let servers = claude_mcp_list(working_dir).await?;

    servers
        .into_iter()
        .find(|server| server.name == name)
        .ok_or_else(|| format!("Server '{}' not found", name))
}

/// Add a new MCP server to Claude Code
/// If working_dir is "Global", writes to ~/.claude.json root mcpServers (user-scope)
/// Otherwise writes to ~/.claude.json projects[working_dir].mcpServers (local-scope)
#[command]
pub async fn claude_mcp_add(
    request: ClaudeCodeServer,
    working_dir: String,
) -> Result<ClaudeCodeResponse, String> {
    println!("[claude_mcp_add] called: name={}, type={}, working_dir={}", request.name, request.r#type, working_dir);
    let server_json = server_to_json(&request)?;
    println!("[claude_mcp_add] server_json: {}", server_json);
    let claude_config_path = get_claude_config_path(Some(working_dir.clone()))?;
    println!("[claude_mcp_add] config_path: {:?}", claude_config_path);

    // Create backup if config file exists
    let backup_path = if claude_config_path.exists() {
        Some(create_backup(&claude_config_path)?)
    } else {
        None
    };

    // Read existing config or create new one
    let mut config: serde_json::Value = if claude_config_path.exists() {
        let config_content = fs::read_to_string(&claude_config_path)
            .map_err(|e| format!("Failed to read Claude config: {}", e))?;
        serde_json::from_str(&config_content)
            .map_err(|e| format!("Failed to parse Claude config: {}", e))?
    } else {
        serde_json::json!({})
    };

    if is_global_config(&working_dir) {
        // Write to root-level mcpServers (user-scope)
        if !config["mcpServers"].is_object() {
            config["mcpServers"] = serde_json::json!({});
        }
        config["mcpServers"][&request.name] = server_json;
    } else {
        // Write to per-project config (local-scope)
        if !config["projects"].is_object() {
            config["projects"] = serde_json::json!({});
        }
        if !config["projects"][&working_dir].is_object() {
            config["projects"][&working_dir] = serde_json::json!({"mcpServers": {}});
        }
        if !config["projects"][&working_dir]["mcpServers"].is_object() {
            config["projects"][&working_dir]["mcpServers"] = serde_json::json!({});
        }
        config["projects"][&working_dir]["mcpServers"][&request.name] = server_json;
    }

    // Write back to file
    println!("[claude_mcp_add] Writing config to {:?}", claude_config_path);
    let config_str = serde_json::to_string_pretty(&config).unwrap();
    println!("[claude_mcp_add] Config content length: {} bytes", config_str.len());
    if let Err(e) = fs::write(&claude_config_path, &config_str) {
        eprintln!("[claude_mcp_add] Failed to write config: {}", e);
        if let Some(backup_path) = &backup_path {
            let _ = restore_backup(&claude_config_path, backup_path);
        }
        return Err(format!("Failed to write Claude config: {}", e));
    }
    println!("[claude_mcp_add] Config written successfully");

    // Clean up backup file on success
    if let Some(backup_path) = backup_path {
        let _ = fs::remove_file(backup_path);
    }

    let scope = if is_global_config(&working_dir) { "user" } else { "project" };
    Ok(ClaudeCodeResponse {
        success: true,
        message: format!("Server '{}' added to {} config successfully", request.name, scope),
    })
}

/// Remove an MCP server from Claude Code
/// If working_dir is "Global", removes from ~/.claude.json root mcpServers (user-scope)
/// Otherwise removes from ~/.claude.json projects[working_dir].mcpServers (local-scope)
#[command]
pub async fn claude_mcp_remove(
    name: String,
    working_dir: String,
) -> Result<ClaudeCodeResponse, String> {
    let claude_config_path = get_claude_config_path(Some(working_dir.clone()))?;

    if !claude_config_path.exists() {
        return Err("Claude config file not found".to_string());
    }

    // Create backup before making changes
    let backup_path = create_backup(&claude_config_path)?;

    let config_content = fs::read_to_string(&claude_config_path)
        .map_err(|e| format!("Failed to read Claude config: {}", e))?;

    let mut config: serde_json::Value = serde_json::from_str(&config_content)
        .map_err(|e| format!("Failed to parse Claude config: {}", e))?;

    let mut found = false;

    if is_global_config(&working_dir) {
        // Remove from root-level mcpServers (user-scope)
        if let Some(mcp_servers) = config.get_mut("mcpServers") {
            if let Some(servers_obj) = mcp_servers.as_object_mut() {
                if servers_obj.remove(&name).is_some() {
                    found = true;
                }
            }
        }
    } else {
        // Remove from per-project config (local-scope)
        if let Some(projects) = config.get_mut("projects") {
            if let Some(project) = projects.get_mut(&working_dir) {
                if let Some(mcp_servers) = project.get_mut("mcpServers") {
                    if let Some(servers_obj) = mcp_servers.as_object_mut() {
                        if servers_obj.remove(&name).is_some() {
                            found = true;
                        }
                    }
                }
            }
        }
    }

    if found {
        // Write back to file
        if let Err(e) = fs::write(
            &claude_config_path,
            serde_json::to_string_pretty(&config).unwrap(),
        ) {
            let _ = restore_backup(&claude_config_path, &backup_path);
            return Err(format!("Failed to write Claude config: {}", e));
        }

        let _ = fs::remove_file(backup_path);

        let scope = if is_global_config(&working_dir) { "user" } else { "project" };
        Ok(ClaudeCodeResponse {
            success: true,
            message: format!("Server '{}' removed from {} config successfully", name, scope),
        })
    } else {
        let _ = fs::remove_file(backup_path);
        let scope = if is_global_config(&working_dir) { "user" } else { "project" };
        Err(format!("Server '{}' not found in {} config", name, scope))
    }
}

/// Helper to check if a config file has root-level mcpServers
fn config_has_global_mcp_servers(config_path: &Path) -> bool {
    if !config_path.exists() {
        return false;
    }
    if let Ok(content) = fs::read_to_string(config_path) {
        if let Ok(config) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(mcp_servers) = config.get("mcpServers") {
                return mcp_servers.is_object() && !mcp_servers.as_object().unwrap().is_empty();
            }
        }
    }
    false
}

/// Helper to get project paths from a config file
fn get_projects_from_config(config_path: &Path) -> Vec<String> {
    let mut project_paths = Vec::new();
    if !config_path.exists() {
        return project_paths;
    }
    if let Ok(content) = fs::read_to_string(config_path) {
        if let Ok(config) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(projects_obj) = config.get("projects") {
                if let Some(projects_map) = projects_obj.as_object() {
                    for project_name in projects_map.keys() {
                        project_paths.push(project_name.clone());
                    }
                }
            }
        }
    }
    project_paths
}

/// List all projects configured in Claude Code
/// Returns global configs first (Windows and/or WSL), followed by sorted project paths
#[command]
pub async fn claude_list_projects() -> Result<Vec<String>, String> {
    let mut projects = Vec::new();
    let mut all_project_paths = std::collections::HashSet::new();

    // Always show "Global (Windows)" - users can add servers even if file doesn't exist yet
    if get_windows_config_path().is_ok() {
        projects.push(GLOBAL_WINDOWS_ID.to_string());
    }

    // Check Windows native config for project paths
    if let Ok(windows_path) = get_windows_config_path() {
        if windows_path.exists() {
            for p in get_projects_from_config(&windows_path) {
                all_project_paths.insert(p);
            }
        }
    }

    // Check WSL config (Windows only) - show if config file exists
    #[cfg(target_os = "windows")]
    {
        if let Some(wsl_path) = get_wsl_config_path() {
            // Add "Global (WSL)" since we found a WSL config
            projects.push(GLOBAL_WSL_ID.to_string());
            // Collect project paths (may overlap with Windows paths)
            for p in get_projects_from_config(&wsl_path) {
                all_project_paths.insert(p);
            }
        }
    }

    // Sort and add project paths
    let mut sorted_paths: Vec<String> = all_project_paths.into_iter().collect();
    sorted_paths.sort();
    projects.extend(sorted_paths);

    Ok(projects)
}

/// Check if Claude Code CLI is available
#[command]
pub async fn check_claude_cli_available() -> Result<bool, String> {
    let output = Command::new("claude").args(&["--version"]).output();

    match output {
        Ok(output) => Ok(output.status.success()),
        Err(_) => Ok(false),
    }
}

#[tauri::command]
pub fn check_claude_config_exists() -> Result<bool, String> {
    // Check native path first
    let home_dir = dirs::home_dir().ok_or("Unable to find home directory")?;
    let path = home_dir.join(".claude.json");
    if path.exists() {
        return Ok(true);
    }

    // On Windows, also check WSL paths
    #[cfg(target_os = "windows")]
    {
        if find_wsl_claude_config(".claude.json").is_some() {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Get the native Windows config path
fn get_windows_config_path() -> Result<PathBuf, String> {
    let home_dir = dirs::home_dir().ok_or("Unable to find home directory")?;
    Ok(home_dir.join(".claude.json"))
}

/// Get the WSL config path if it exists
#[cfg(target_os = "windows")]
fn get_wsl_config_path() -> Option<PathBuf> {
    find_wsl_claude_config(".claude.json")
}

#[cfg(not(target_os = "windows"))]
fn get_wsl_config_path() -> Option<PathBuf> {
    None
}

/// Check if a path looks like a Linux/Unix path (starts with /)
fn is_linux_path(path: &str) -> bool {
    path.starts_with('/')
}

fn get_claude_config_path(working_dir: Option<String>) -> Result<PathBuf, String> {
    let working_dir = working_dir.as_deref().unwrap_or("");

    // If explicitly requesting Windows global, return Windows path
    if is_windows_global(working_dir) {
        return get_windows_config_path();
    }

    // If explicitly requesting WSL global, return WSL path
    if is_wsl_global(working_dir) {
        #[cfg(target_os = "windows")]
        {
            if let Some(wsl_path) = get_wsl_config_path() {
                return Ok(wsl_path);
            }
            return Err("WSL Claude config not found".to_string());
        }
        #[cfg(not(target_os = "windows"))]
        {
            return Err("WSL is only available on Windows".to_string());
        }
    }

    // For project paths, determine config based on path format
    #[cfg(target_os = "windows")]
    if !working_dir.is_empty() && !is_global_config(working_dir) {
        // Linux-style paths (e.g., /home/user/project) -> WSL config
        if is_linux_path(working_dir) {
            if let Some(wsl_path) = get_wsl_config_path() {
                return Ok(wsl_path);
            }
            // Fall through to Windows config if WSL not available
        } else {
            // Windows-style paths -> Windows config
            return get_windows_config_path();
        }
    }

    // For legacy "Global" or fallback, use existing logic:
    // First try Windows native path
    let native_path = get_windows_config_path()?;
    if native_path.exists() {
        return Ok(native_path);
    }

    // On Windows, also check WSL paths if native doesn't exist
    #[cfg(target_os = "windows")]
    {
        if let Some(wsl_path) = get_wsl_config_path() {
            return Ok(wsl_path);
        }
    }

    // Return native path even if it doesn't exist (for creation)
    Ok(native_path)
}

/// On Windows, attempt to find Claude config in WSL
#[cfg(target_os = "windows")]
fn find_wsl_claude_config(filename: &str) -> Option<PathBuf> {
    // Common WSL distro names to check
    let distros = ["Ubuntu", "Ubuntu-22.04", "Ubuntu-24.04", "Ubuntu-20.04", "Debian", "kali-linux", "openSUSE-Leap-15.5"];

    // Try to get WSL username by checking common paths
    for distro in &distros {
        let wsl_base = PathBuf::from(format!(r"\\wsl$\{}", distro));
        if !wsl_base.exists() {
            continue;
        }

        // Check /home/* directories for .claude.json
        let home_dir = wsl_base.join("home");
        if let Ok(entries) = fs::read_dir(&home_dir) {
            for entry in entries.flatten() {
                let user_home = entry.path();
                let config_path = user_home.join(filename);
                if config_path.exists() {
                    return Some(config_path);
                }
            }
        }
    }

    None
}

/// Check if a working_dir refers to any global config (Windows or WSL)
fn is_global_config(working_dir: &str) -> bool {
    working_dir == GLOBAL_PROJECT_ID
        || working_dir == GLOBAL_WINDOWS_ID
        || working_dir == GLOBAL_WSL_ID
}

/// Check if a working_dir refers to the Windows global config
fn is_windows_global(working_dir: &str) -> bool {
    working_dir == GLOBAL_WINDOWS_ID
}

/// Check if a working_dir refers to the WSL global config
fn is_wsl_global(working_dir: &str) -> bool {
    working_dir == GLOBAL_WSL_ID
}

fn parse_server_config(name: &str, config: &serde_json::Value) -> Result<ClaudeCodeServer, String> {
    let server_type = config
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("stdio")
        .to_string();

    let url = config
        .get("url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let command = config
        .get("command")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let args = config.get("args").and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_string())
            .collect()
    });

    let env = config.get("env").and_then(|v| v.as_object()).map(|obj| {
        obj.iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect()
    });

    Ok(ClaudeCodeServer {
        name: name.to_string(),
        r#type: server_type,
        url,
        command,
        args,
        env,
    })
}

fn server_to_json(server: &ClaudeCodeServer) -> Result<serde_json::Value, String> {
    let mut json = serde_json::json!({
        "type": server.r#type
    });

    if let Some(url) = &server.url {
        json["url"] = serde_json::Value::String(url.clone());
    }

    if let Some(command) = &server.command {
        json["command"] = serde_json::Value::String(command.clone());
    }

    if let Some(args) = &server.args {
        json["args"] = serde_json::Value::Array(
            args.iter()
                .map(|arg| serde_json::Value::String(arg.clone()))
                .collect(),
        );
    }

    if let Some(env) = &server.env {
        json["env"] = serde_json::Value::Object(
            env.iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect(),
        );
    }

    Ok(json)
}

fn create_backup(config_path: &PathBuf) -> Result<PathBuf, String> {
    if !config_path.exists() {
        return Err("Config file does not exist".to_string());
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let backup_path = config_path.with_extension(format!("json.backup.{}", timestamp));

    fs::copy(config_path, &backup_path).map_err(|e| format!("Failed to create backup: {}", e))?;

    Ok(backup_path)
}

fn restore_backup(config_path: &PathBuf, backup_path: &PathBuf) -> Result<(), String> {
    if !backup_path.exists() {
        return Err("Backup file does not exist".to_string());
    }

    fs::copy(backup_path, config_path).map_err(|e| format!("Failed to restore backup: {}", e))?;

    Ok(())
}
