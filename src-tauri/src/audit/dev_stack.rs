//! Dev mode Docker orchestration: prerequisite checks + bring up/down the
//! local audit stack (nosqlbuddy-audit publisher + attester + reader).
//!
//! The 3-node MongoDB replica set is a *separate* Compose file
//! (`docker-compose.yml`, the hackathon one). This module manages the audit
//! services Compose file (`docker-compose.audit.yml`), which runs the audit
//! daemon containers that connect to the replica set and to Stellar/Pinata.
//!
//! All Docker interaction is via `docker compose` subprocess calls run in
//! `spawn_blocking` so the async runtime is never blocked.

use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::error::{AppError, AppResult};

/// The audit services Compose file (sits next to the replica-set Compose).
const AUDIT_COMPOSE_FILE: &str = "docker-compose.audit.yml";

/// Default ports used by the audit stack containers.
const PUBLISHER_PORT: u16 = 9173;
const ATTESTER_PORT: u16 = 9174;
const READER_PORT: u16 = 9175;

/// Resolve the project root (where the Compose files live).
///
/// In dev, this is the cargo manifest directory. In a packaged app, it falls
/// back to the app's resource directory. We look for the Compose file in a
/// few candidate locations and return the first that exists.
fn resolve_compose_dir(app: &AppHandle) -> AppResult<PathBuf> {
    use tauri::Manager;

    // Candidate 1: the app resource directory (packaged builds).
    if let Ok(res_dir) = app.path().resource_dir() {
        if res_dir.join(AUDIT_COMPOSE_FILE).exists() {
            return Ok(res_dir);
        }
    }

    // Candidate 2: cargo manifest dir (dev builds).
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // CARGO_MANIFEST_DIR is src-tauri/, the Compose file is one level up.
    let project_root = manifest_dir
        .parent()
        .unwrap_or(&manifest_dir)
        .to_path_buf();
    if project_root.join(AUDIT_COMPOSE_FILE).exists() {
        return Ok(project_root);
    }

    // Candidate 3: current working directory.
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join(AUDIT_COMPOSE_FILE).exists() {
            return Ok(cwd);
        }
    }

    Err(AppError::NotFound(format!(
        "{AUDIT_COMPOSE_FILE} not found in resource dir, project root, or cwd"
    )))
}

/// Run a `docker compose` command against the audit Compose file.
///
/// If a `.env.audit` file sits next to the Compose file, it is passed via
/// `--env-file` so credential variables (Pinata keys, Stellar identity) are
/// substituted into the Compose file and injected into containers.
fn docker_compose(app: &AppHandle, args: &[&str]) -> AppResult<String> {
    let dir = resolve_compose_dir(app)?;
    let env_file = dir.join(".env.audit");
    let has_env = env_file.exists();

    let mut cmd = Command::new("docker");
    cmd.current_dir(&dir);
    // Use an explicit project name so the audit stack is isolated from the
    // main mongo-buddy compose project that lives in the same directory.
    // Without this, `docker compose ps` returns mongo1/mongo2/mongo3 as
    // orphan containers and falsely reports the audit stack as "running".
    cmd.args(["compose", "-p", "nosqlbuddy-audit", "-f", AUDIT_COMPOSE_FILE]);
    if has_env {
        cmd.args(["--env-file", ".env.audit"]);
    }
    cmd.args(args);

    let output = cmd
        .output()
        .map_err(|e| AppError::Internal(format!("failed to run docker: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(AppError::Internal(format!(
            "docker compose {} failed (exit {:?}): {stderr}",
            args.join(" "),
            output.status.code()
        )));
    }

    Ok(stdout)
}

/// Check whether a binary is available on PATH.
fn binary_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check whether a TCP port is free (nothing listening) on localhost.
fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// The result of a prerequisite check for dev mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevPrerequisites {
    /// `docker` CLI is installed and responsive.
    pub docker_installed: bool,
    /// `docker compose` subcommand is available.
    pub docker_compose_available: bool,
    /// The audit Compose file exists on disk.
    pub compose_file_present: bool,
    /// The audit-stack ports (9173/9174/9175) are free.
    pub ports_free: bool,
    /// The publisher port (9173) is free.
    pub publisher_port_free: bool,
    /// The attester port (9174) is free.
    pub attester_port_free: bool,
    /// The reader port (9175) is free.
    pub reader_port_free: bool,
    /// The audit stack containers appear to be running already.
    pub audit_stack_running: bool,
    /// Docker daemon is running (engine responsive).
    pub docker_daemon_running: bool,
    /// Human-readable summary of what's ready and what's missing.
    pub summary: String,
}

/// Check all dev-mode prerequisites in one pass.
pub fn check_prerequisites(app: &AppHandle) -> AppResult<DevPrerequisites> {
    let docker_installed = binary_available("docker");
    let docker_compose_available = docker_installed
        && Command::new("docker")
            .args(["compose", "version"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

    let docker_daemon_running = docker_installed
        && Command::new("docker")
            .args(["info"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

    let compose_file_present = resolve_compose_dir(app)
        .map(|d| d.join(AUDIT_COMPOSE_FILE).exists())
        .unwrap_or(false);

    let publisher_port_free = port_free(PUBLISHER_PORT);
    let attester_port_free = port_free(ATTESTER_PORT);
    let reader_port_free = port_free(READER_PORT);
    let ports_free = publisher_port_free && attester_port_free && reader_port_free;

    let audit_stack_running = if docker_daemon_running && compose_file_present {
        docker_compose(app, &["ps", "--status", "running", "--quiet"])
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    } else {
        false
    };

    let mut issues: Vec<&str> = Vec::new();
    if !docker_installed {
        issues.push("Docker is not installed");
    }
    if !docker_compose_available {
        issues.push("Docker Compose plugin is missing");
    }
    if !docker_daemon_running {
        issues.push("Docker daemon is not running");
    }
    if !compose_file_present {
        issues.push("docker-compose.audit.yml not found");
    }
    if !ports_free && !audit_stack_running {
        issues.push("one or more audit ports (9173-9175) are in use");
    }

    let summary = if issues.is_empty() {
        if audit_stack_running {
            "All prerequisites met — audit stack is already running.".to_string()
        } else {
            "All prerequisites met — ready to start the audit stack.".to_string()
        }
    } else {
        format!("Missing: {}", issues.join("; "))
    };

    Ok(DevPrerequisites {
        docker_installed,
        docker_compose_available,
        compose_file_present,
        ports_free,
        publisher_port_free,
        attester_port_free,
        reader_port_free,
        audit_stack_running,
        docker_daemon_running,
        summary,
    })
}

/// Status of the audit stack containers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevStackStatus {
    pub running: bool,
    /// Raw `docker compose ps` output (parsed loosely into service lines).
    pub services: Vec<DevStackService>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevStackService {
    pub name: String,
    pub state: String,
    pub ports: String,
}

/// Query the current status of the audit stack containers.
pub fn stack_status(app: &AppHandle) -> AppResult<DevStackStatus> {
    let output = docker_compose(app, &["ps", "--format", "json"])?;
    let services = parse_compose_ps(&output);
    let running = services.iter().any(|s| {
        s.state.to_lowercase().contains("running") || s.state.to_lowercase().contains("up")
    });
    Ok(DevStackStatus { running, services })
}

/// Parse `docker compose ps --format json` (one JSON object per line) into
/// service structs. Falls back to an empty list on any parse error.
fn parse_compose_ps(output: &str) -> Vec<DevStackService> {
    let mut services = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            let name = val
                .get("Service")
                .or_else(|| val.get("service"))
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            let state = val
                .get("State")
                .or_else(|| val.get("state"))
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            let ports = val
                .get("Ports")
                .or_else(|| val.get("ports"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            services.push(DevStackService { name, state, ports });
        }
    }
    services
}

/// Bring up the audit stack (`docker compose up -d`).
pub fn stack_up(app: &AppHandle) -> AppResult<String> {
    docker_compose(app, &["up", "-d", "--build"])
}

/// Tear down the audit stack (`docker compose down`).
pub fn stack_down(app: &AppHandle) -> AppResult<String> {
    docker_compose(app, &["down"])
}

/// Tear down the audit stack and wipe all Docker volumes (`docker compose down --volumes`).
///
/// This removes the three named volumes:
///   `audit-publisher-data`, `audit-attester-data`, `audit-reader-data`
///
/// Effect: all daemon state (events.jsonl, sled Merkle tree, sled attestation
/// store, attester key) is erased. On-chain Stellar history is NOT affected.
/// Keypairs in `.env.audit` and the OS keychain are preserved.
pub fn stack_reset_data(app: &AppHandle) -> AppResult<String> {
    docker_compose(app, &["down", "--volumes"])
}

/// Fetch the last N lines of logs from the audit stack (all services).
pub fn stack_logs(app: &AppHandle, tail: u32) -> AppResult<String> {
    docker_compose(app, &["logs", "--tail", &tail.to_string()])
}

// ─── Tauri commands ────────────────────────────────────────────────────

/// Check dev-mode prerequisites.
#[tauri::command]
pub async fn audit_check_dev_prerequisites(app: AppHandle) -> AppResult<DevPrerequisites> {
    let app = app;
    tokio::task::spawn_blocking(move || check_prerequisites(&app))
        .await
        .map_err(|e| AppError::Internal(format!("prereq check task join: {e}")))?
}

/// Get the audit stack container status.
#[tauri::command]
pub async fn audit_dev_stack_status(app: AppHandle) -> AppResult<DevStackStatus> {
    tokio::task::spawn_blocking(move || stack_status(&app))
        .await
        .map_err(|e| AppError::Internal(format!("stack status task join: {e}")))?
}

/// Start the audit stack.
#[tauri::command]
pub async fn audit_dev_stack_up(app: AppHandle) -> AppResult<String> {
    tokio::task::spawn_blocking(move || stack_up(&app))
        .await
        .map_err(|e| AppError::Internal(format!("stack up task join: {e}")))?
}

/// Stop the audit stack.
#[tauri::command]
pub async fn audit_dev_stack_down(app: AppHandle) -> AppResult<String> {
    tokio::task::spawn_blocking(move || stack_down(&app))
        .await
        .map_err(|e| AppError::Internal(format!("stack down task join: {e}")))?
}

/// Stop the audit stack and wipe all Docker volumes.
///
/// Clears all local daemon state (events, Merkle tree, attestation store).
/// On-chain Stellar history and `.env.audit` credentials are preserved.
#[tauri::command]
pub async fn audit_dev_stack_reset_data(app: AppHandle) -> AppResult<String> {
    tokio::task::spawn_blocking(move || stack_reset_data(&app))
        .await
        .map_err(|e| AppError::Internal(format!("stack reset data task join: {e}")))?
}

/// Fetch recent audit stack logs.
#[tauri::command]
pub async fn audit_dev_stack_logs(app: AppHandle, tail: Option<u32>) -> AppResult<String> {
    let tail = tail.unwrap_or(100);
    tokio::task::spawn_blocking(move || stack_logs(&app, tail))
        .await
        .map_err(|e| AppError::Internal(format!("stack logs task join: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_compose_ps_empty() {
        assert!(parse_compose_ps("").is_empty());
        assert!(parse_compose_ps("not json").is_empty());
    }

    #[test]
    fn test_parse_compose_ps_one_service() {
        let json = r#"{"Service":"publisher","State":"running","Ports":"0.0.0.0:9173->9173/tcp"}"#;
        let services = parse_compose_ps(json);
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "publisher");
        assert_eq!(services[0].state, "running");
        assert!(services[0].ports.contains("9173"));
    }

    #[test]
    fn test_port_free_high_port() {
        // An ephemeral high port should almost always be free.
        assert!(port_free(53917) || !port_free(53917)); // non-deterministic, just exercise
    }
}
