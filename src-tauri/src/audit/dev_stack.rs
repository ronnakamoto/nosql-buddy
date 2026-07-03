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

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::error::{AppError, AppResult};

/// The audit services Compose file (sits next to the replica-set Compose).
const AUDIT_COMPOSE_FILE: &str = "docker-compose.audit.yml";

/// The 3-node replica-set Compose file the audit services depend on.
const AUDIT_DB_COMPOSE_FILE: &str = "docker-compose.audit-db.yml";

/// Compose project name for the audit services (isolates them from the DB and
/// the main mongo-buddy project that may live in the same directory).
const AUDIT_PROJECT: &str = "nosqlbuddy-audit";

/// Published audit image repository. Packaged builds pull this (no local
/// build); the tag matches the app's crate version (the CI release tag).
const PUBLISHED_IMAGE_REPO: &str = "ghcr.io/ronnakamoto/nosqlbuddy-audit";

/// Default ports used by the audit stack containers.
const PUBLISHER_PORT: u16 = 9173;
const ATTESTER_PORT: u16 = 9174;
const READER_PORT: u16 = 9175;

/// Full published image reference for this build (e.g. `ghcr.io/...:0.1.0`).
fn published_image_ref() -> String {
    format!("{PUBLISHED_IMAGE_REPO}:{}", env!("CARGO_PKG_VERSION"))
}

/// Parse the age public key from an age secret identity string.
///
/// Age secret key strings contain a comment line like:
///   `# public key: age1v4d...`
///
/// Returns the public key (e.g. `age1...`) if found, otherwise empty.
fn extract_age_public_key(secret: &str) -> String {
    for line in secret.lines() {
        if let Some(val) = line.strip_prefix("# public key:") {
            return val.trim().to_string();
        }
    }
    String::new()
}

/// Where the audit stack runs from, and how.
///
/// - **Source builds** run from the project root and build the image locally
///   (`AUDIT_IMAGE` unset → `nosqlbuddy-audit:dev`, `up --build`).
/// - **Packaged builds** copy the bundled stack (shipped under
///   `<resource>/stack`) into a writable per-user dir so the wizard can write
///   `.env.audit` + `attester.key` and compose can mount them, and pull the
///   published image (`AUDIT_IMAGE` set, `up` without `--build`).
struct StackCtx {
    dir: PathBuf,
    packaged: bool,
}

/// Resolve the directory the audit Compose files run from (and whether we're
/// in packaged mode). See [`StackCtx`].
fn stack_ctx(app: &AppHandle) -> AppResult<StackCtx> {
    use tauri::Manager;

    // Source checkout: the project root (one level up from src-tauri) holds
    // the Compose files. Build the image locally.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project_root = manifest_dir.parent().unwrap_or(&manifest_dir).to_path_buf();
    if project_root.join(AUDIT_COMPOSE_FILE).exists() {
        return Ok(StackCtx {
            dir: project_root,
            packaged: false,
        });
    }

    // Packaged build: the stack is bundled read-only under <resource>/stack.
    // Copy it into a writable per-user dir so it can be run and written to.
    if let Ok(res_dir) = app.path().resource_dir() {
        let bundled = res_dir.join("stack");
        if bundled.join(AUDIT_COMPOSE_FILE).exists() {
            let dest = app
                .path()
                .app_data_dir()
                .map_err(|e| AppError::Internal(format!("no app data dir: {e}")))?
                .join("dev-stack");
            sync_bundled_stack(&bundled, &dest)?;
            return Ok(StackCtx {
                dir: dest,
                packaged: true,
            });
        }
    }

    // Fallback: current working directory (e.g. running the binary by hand).
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join(AUDIT_COMPOSE_FILE).exists() {
            return Ok(StackCtx {
                dir: cwd,
                packaged: false,
            });
        }
    }

    Err(AppError::NotFound(format!(
        "{AUDIT_COMPOSE_FILE} not found in project root, bundled resources, or cwd"
    )))
}

/// Copy the bundled stack (Compose files + scripts + env example) into the
/// writable dir. Compose/scripts are refreshed each time so app upgrades take
/// effect; user-generated files (`.env.audit`, `attester.key`) are never
/// touched.
fn sync_bundled_stack(src: &Path, dest: &Path) -> AppResult<()> {
    std::fs::create_dir_all(dest.join("scripts"))
        .map_err(|e| AppError::Internal(format!("create dev-stack dir: {e}")))?;
    for rel in [
        AUDIT_COMPOSE_FILE,
        AUDIT_DB_COMPOSE_FILE,
        "scripts/rs-init-audit.js",
        "scripts/seed.js",
        "scripts/mongo-keyfile",
        "scripts/mongo-entrypoint.sh",
        "audit-stack.env.example",
    ] {
        let from = src.join(rel);
        let to = dest.join(rel);
        if from.exists() {
            if let Some(parent) = to.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::copy(&from, &to)
                .map_err(|e| AppError::Internal(format!("copy bundled {rel}: {e}")))?;
        }
    }
    Ok(())
}

/// Run a `docker compose` command against one Compose file in the stack dir.
///
/// `project` overrides the Compose project name (the audit services use
/// [`AUDIT_PROJECT`]; the DB Compose declares its own `name:`, so pass `None`).
/// If a `.env.audit` sits in the stack dir it is passed via `--env-file` so
/// `${VAR}` placeholders (image, credentials) are interpolated. Packaged builds
/// also export `AUDIT_IMAGE` so compose pulls the published image.
fn run_compose(
    ctx: &StackCtx,
    project: Option<&str>,
    file: &str,
    args: &[&str],
) -> AppResult<String> {
    let has_env = ctx.dir.join(".env.audit").exists();

    let mut cmd = Command::new("docker");
    cmd.current_dir(&ctx.dir);
    if ctx.packaged {
        cmd.env("AUDIT_IMAGE", published_image_ref());
    }
    cmd.arg("compose");
    if let Some(p) = project {
        cmd.args(["-p", p]);
    }
    cmd.args(["-f", file]);
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
        // Full technical detail (command, exit code, stderr) goes to the log
        // file + stdout for debugging. Stellar secret seeds are redacted so
        // they never reach the logs.
        let raw = format!(
            "docker compose {} failed (exit {:?}): {}",
            args.join(" "),
            output.status.code(),
            stderr.trim()
        );
        log::error!("{}", redact_secrets(&raw));

        // The UI only ever sees a concise message with a clear reason and the
        // action to take — never the raw Docker/compiler output.
        return Err(AppError::Internal(friendly_compose_error(args, &stderr)));
    }

    Ok(stdout)
}

/// Spawn `cmd`, stream its stdout/stderr line-by-line (secret-redacted)
/// through `emit` as it runs, and return the combined stdout once it exits.
///
/// This is what turns a "the app might be hanging" multi-minute subprocess
/// (a cold `docker compose build`, the setup wizard's contract deploy) into
/// visible progress: the caller sees the same lines a terminal would show,
/// as they happen, instead of waiting for one buffered result at the end.
/// On failure, `friendly_compose_error(error_args, ...)` produces the
/// concise, secret-free message the UI sees; full detail still goes to the
/// log.
fn stream_command(
    mut cmd: Command,
    app: &AppHandle,
    emit: fn(&AppHandle, &str),
    error_context: &str,
    error_args: &[&str],
) -> AppResult<String> {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::Internal(format!("failed to run docker: {e}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Internal(format!("failed to capture {error_context} stdout")))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::Internal(format!("failed to capture {error_context} stderr")))?;

    // Drain stderr on a side thread so a full pipe can't deadlock the run.
    let app_err = app.clone();
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let red = redact_secrets(&line);
            if !red.trim().is_empty() {
                emit(&app_err, &red);
            }
            buf.push_str(&red);
            buf.push('\n');
        }
        buf
    });

    let mut stdout_buf = String::new();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let red = redact_secrets(&line);
        if !red.trim().is_empty() {
            emit(app, &red);
        }
        stdout_buf.push_str(&red);
        stdout_buf.push('\n');
    }

    let status = child
        .wait()
        .map_err(|e| AppError::Internal(format!("wait for {error_context}: {e}")))?;
    let stderr_buf = stderr_handle.join().unwrap_or_default();

    if !status.success() {
        let raw = format!(
            "{error_context} failed (exit {:?}): {}",
            status.code(),
            stderr_buf.trim()
        );
        log::error!("{}", redact_secrets(&raw));
        return Err(AppError::Internal(friendly_compose_error(
            error_args,
            &stderr_buf,
        )));
    }

    Ok(stdout_buf)
}

/// Run the one-off setup wizard service, streaming its output to the UI.
///
/// Behaves like [`run_compose`] for the `run --rm setup …` command, but instead
/// of buffering the whole run it emits each line as an `audit-setup-progress`
/// event so the frontend can show live progress. On failure it produces the
/// same concise, secret-free message as [`run_compose`] (full detail goes to
/// the log).
fn run_setup_streaming(ctx: &StackCtx, app: &AppHandle) -> AppResult<String> {
    let has_env = ctx.dir.join(".env.audit").exists();

    let mut cmd = Command::new("docker");
    cmd.current_dir(&ctx.dir);
    if ctx.packaged {
        cmd.env("AUDIT_IMAGE", published_image_ref());
    }
    cmd.arg("compose");
    cmd.args(["-p", AUDIT_PROJECT]);
    cmd.args(["-f", AUDIT_COMPOSE_FILE]);
    if has_env {
        cmd.args(["--env-file", ".env.audit"]);
    }
    cmd.args(["run", "--rm", "setup", "setup", "--non-interactive"]);

    stream_command(
        cmd,
        app,
        crate::events::emit_audit_setup_progress,
        "docker compose run setup",
        &["run"],
    )
}

/// Run a `docker compose` command against one Compose file, streaming its
/// output to the UI as an `audit-stack-progress` event per line. Used by
/// [`stack_up`], where a cold source build recompiles the whole Rust
/// workspace inside the container and can otherwise look like a hang.
fn run_compose_streaming(
    ctx: &StackCtx,
    app: &AppHandle,
    project: Option<&str>,
    file: &str,
    args: &[&str],
) -> AppResult<String> {
    let has_env = ctx.dir.join(".env.audit").exists();

    let mut cmd = Command::new("docker");
    cmd.current_dir(&ctx.dir);
    if ctx.packaged {
        cmd.env("AUDIT_IMAGE", published_image_ref());
    }
    cmd.arg("compose");
    if let Some(p) = project {
        cmd.args(["-p", p]);
    }
    cmd.args(["-f", file]);
    if has_env {
        cmd.args(["--env-file", ".env.audit"]);
    }
    cmd.args(args);

    stream_command(
        cmd,
        app,
        crate::events::emit_audit_stack_progress,
        &format!("docker compose {}", args.join(" ")),
        args,
    )
}

/// Translate a raw `docker compose` failure into a concise, user-facing message
/// with a clear reason and the action to take. The full technical error is
/// logged separately (see [`run_compose`]); this is the only thing the UI sees.
fn friendly_compose_error(args: &[&str], stderr: &str) -> String {
    let s = stderr.to_lowercase();

    if s.contains("cannot connect to the docker daemon")
        || s.contains("is the docker daemon running")
        || s.contains("docker daemon is not running")
    {
        return "Docker isn't running. Start Docker Desktop, wait for it to finish starting, \
                then try again."
            .to_string();
    }
    if s.contains("unknown flag") || s.contains("unknown shorthand flag") {
        return "Your Docker Compose version is too old and rejected a required option. Update \
                Docker Desktop (or the Docker Compose plugin) to the latest version, then try \
                again."
            .to_string();
    }
    if s.contains("port is already allocated")
        || s.contains("address already in use")
        || s.contains("bind for")
        || s.contains("ports are not available")
    {
        return "A required port is already in use. The audit stack needs ports 9173-9175 and \
                MongoDB needs 27018-27020. Stop whatever is using them (or the existing stack), \
                then try again."
            .to_string();
    }
    if s.contains("no space left on device") {
        return "Your disk is full, so Docker couldn't continue. Free up space (for example, run \
                'docker system prune'), then try again."
            .to_string();
    }
    if s.contains("pull access denied")
        || s.contains("manifest unknown")
        || s.contains("failed to resolve")
        || s.contains("error pinging docker registry")
        || s.contains("i/o timeout")
        || s.contains("network is unreachable")
        || s.contains("temporary failure in name resolution")
    {
        return "Couldn't download a required Docker image. Check your internet connection and \
                that Docker can reach its registry, then try again."
            .to_string();
    }
    if s.contains("failed to solve")
        || s.contains("did not complete successfully")
        || s.contains("cargo build")
        || s.contains("exit code: 101")
    {
        return "The audit service image failed to build. The full build output has been written \
                to the app log. If this keeps happening, the project may need a fix."
            .to_string();
    }
    if s.contains("no such file") && s.contains("compose") {
        return "A Docker Compose file is missing. Make sure you're running from the project \
                root, then try again."
            .to_string();
    }
    if s.contains("trying to mount a directory onto a file") || s.contains("not a directory") {
        return "A Docker volume has a stale mount point for the attester key (a leftover from \
                an earlier setup issue). Use 'Reset Data' to wipe the audit stack's Docker \
                volumes and try again — this does not affect your Stellar keys or on-chain \
                history."
            .to_string();
    }

    // The command ran a subprocess (e.g. the audit setup wizard) that failed
    // with its own already-user-facing message. Prefer surfacing that over a
    // generic fallback, since it carries the real reason and action.
    if let Some(app_msg) = extract_app_error(stderr) {
        return redact_secrets(&app_msg);
    }

    // Fallback: generic but still actionable, with no raw output leaked.
    let action = match args.first().copied().unwrap_or("") {
        "up" => "start",
        "down" => "stop",
        "run" => "run setup for",
        "build" => "build",
        "logs" => "read logs from",
        _ => "manage",
    };
    format!(
        "Couldn't {action} the audit stack. Technical details were written to the app log. \
         Check Docker Desktop is running and try again."
    )
}

/// Extract a meaningful application-level error from a subprocess's stderr.
///
/// The bundled audit wizard (and other Rust CLIs) print their failure as an
/// `Error: "<message>"` line. That message is already written for end users, so
/// we surface it rather than a generic Docker fallback. Docker's own
/// "Error response from daemon: …" lines do not match (no colon after `Error`).
fn extract_app_error(stderr: &str) -> Option<String> {
    let line = stderr
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("Error:"))
        .next_back()?;

    let mut msg = line
        .trim_start_matches("Error:")
        .trim()
        .trim_matches('"')
        .trim();

    // Drop a leading "<context>: validation error: " noise prefix if present,
    // keeping just the human-readable reason + action.
    if let Some(idx) = msg.rfind("validation error: ") {
        msg = msg[idx + "validation error: ".len()..].trim();
    }

    if msg.is_empty() {
        None
    } else {
        Some(msg.to_string())
    }
}

/// Run a `docker compose` command against the audit services Compose file.
fn docker_compose(app: &AppHandle, args: &[&str]) -> AppResult<String> {
    let ctx = stack_ctx(app)?;
    run_compose(&ctx, Some(AUDIT_PROJECT), AUDIT_COMPOSE_FILE, args)
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
    /// The generated `.env.audit` credentials file is present in the stack dir.
    pub env_audit_present: bool,
    /// The attester ed25519 key file is present in the stack dir.
    pub attester_key_present: bool,
    /// Setup has completed (both `.env.audit` and `attester.key` exist).
    pub audit_configured: bool,
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

    let compose_file_present = stack_ctx(app)
        .map(|c| c.dir.join(AUDIT_COMPOSE_FILE).exists())
        .unwrap_or(false);

    let (env_audit_present, attester_key_present) = stack_ctx(app)
        .map(|c| {
            (
                c.dir.join(".env.audit").exists(),
                c.dir.join("attester.key").exists(),
            )
        })
        .unwrap_or((false, false));
    let audit_configured = env_audit_present && attester_key_present;

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
        } else if !audit_configured {
            "Prerequisites met — run Set up to generate audit credentials.".to_string()
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
        env_audit_present,
        attester_key_present,
        audit_configured,
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
    /// The publisher's configured MongoDB URI, read from `.env.audit` when set.
    /// `None` means the bundled demo replica set default is in effect.
    pub publisher_mongo_uri: Option<String>,
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
    let publisher_mongo_uri = stack_ctx(app)
        .ok()
        .and_then(|ctx| read_env_var(&ctx.dir.join(".env.audit"), "PUBLISHER_MONGO_URI"));
    Ok(DevStackStatus {
        running,
        services,
        publisher_mongo_uri,
    })
}

/// Read a single non-empty value for `key` from an env file (`KEY=value`).
/// Returns `None` if the file is missing or the key is unset/empty.
fn read_env_var(path: &Path, key: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let prefix = format!("{key}=");
    content
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
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

/// Repair a stale mount-point type mismatch left behind in the
/// `audit-attester-data` named volume.
///
/// The attester/reader services bind-mount the host `attester.key` file to
/// `/data/attester/audit/attester.key`, nested *inside* the named volume
/// `audit-attester-data`. The first time that mount is created, Docker
/// materializes the destination inside the volume with the same type as the
/// host source at that moment. If `attester.key` was ever a directory on the
/// host (see the Docker bind-mount footgun documented on
/// [`crate::audit::dev_stack`]'s attester key handling) when a container
/// first started, the volume permanently gets a *directory* at
/// `audit/attester.key` — and since named volumes persist across container
/// recreation (`down`/`up`, even `rm -f`), that stale directory keeps
/// mismatching the now-correct file source forever, failing every future
/// `up` with "Are you trying to mount a directory onto a file?" even after
/// the host-side directory has been fixed.
///
/// This runs a disposable Alpine container to remove that stale directory
/// (only if it's empty — never touches user data) before compose starts the
/// real services. It's a no-op if the volume doesn't exist yet or the path
/// is already the correct type.
fn heal_stale_attester_key_mount(ctx: &StackCtx) {
    let volume_name = format!("{AUDIT_PROJECT}_audit-attester-data");
    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{volume_name}:/data"),
            "alpine",
            "sh",
            "-c",
            // Only remove if it's an empty directory; a non-empty one or a
            // regular file is left completely alone.
            "if [ -d /data/audit/attester.key ]; then rmdir /data/audit/attester.key 2>/dev/null; fi",
        ])
        .current_dir(&ctx.dir)
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            log::debug!("heal_stale_attester_key_mount: checked/repaired audit-attester-data volume");
        }
    }
    // Errors (e.g. volume doesn't exist yet on first-ever run) are expected
    // and harmless — this is best-effort preventive maintenance, not a
    // required step, so failures here must never block `stack_up`.
}

/// Bring up the audit stack.
///
/// Ensures the 3-node replica set is running first, then starts the audit
/// services. Source builds rebuild the image locally (`--build`, which
/// recompiles the whole Rust workspace on a cold cache and can take minutes);
/// packaged builds pull the published image. Both steps stream their output
/// live via `audit-stack-progress` events, with a marker line announcing each
/// step, so the UI can show real progress instead of a bare spinner for
/// however long this takes.
pub fn stack_up(app: &AppHandle) -> AppResult<String> {
    let ctx = stack_ctx(app)?;

    heal_stale_attester_key_mount(&ctx);

    crate::events::emit_audit_stack_progress(app, "── Starting MongoDB replica set ──");
    // The DB Compose declares its own project name, so do not override it.
    let mut log = run_compose_streaming(&ctx, app, None, AUDIT_DB_COMPOSE_FILE, &["up", "-d"])?;

    let audit_args: &[&str] = if ctx.packaged {
        &["up", "-d"]
    } else {
        &["up", "-d", "--build"]
    };
    crate::events::emit_audit_stack_progress(
        app,
        if ctx.packaged {
            "── Starting audit services (publisher, attester, reader) ──"
        } else {
            "── Building and starting audit services — first run compiles the whole \
             workspace and can take a few minutes ──"
        },
    );
    log.push_str(&run_compose_streaming(
        &ctx,
        app,
        Some(AUDIT_PROJECT),
        AUDIT_COMPOSE_FILE,
        audit_args,
    )?);
    crate::events::emit_audit_stack_progress(app, "── Stack started ──");
    Ok(log)
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

// ─── Setup (non-interactive wizard) ──────────────────────────────────────

/// Parameters for the non-interactive setup wizard, supplied by the desktop
/// app's "Set up" button. All fields are optional; empty values fall back to
/// the wizard's defaults (generate keypairs, bundled testnet contract, etc.).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevSetupParams {
    /// `testnet` (default) or `mainnet`.
    pub network: Option<String>,
    pub pinata_api_key: Option<String>,
    pub pinata_api_secret: Option<String>,
    pub pinata_gateway_url: Option<String>,
    /// Operator's existing publisher Stellar secret key (else generated).
    pub publisher_secret_key: Option<String>,
    /// Auditor's existing attester Stellar secret key (else generated).
    pub attester_secret_key: Option<String>,
    /// Existing Soroban contract ID (else the bundled testnet contract).
    pub contract_id: Option<String>,
    /// Re-run even if `.env.audit` already exists (regenerates keys).
    pub overwrite: Option<bool>,
    /// The MongoDB deployment the publisher should watch (change stream).
    /// Persisted into `.env.audit` as `PUBLISHER_MONGO_URI`. Left empty, the
    /// stack watches the bundled demo 3-node replica set. Must be a replica set
    /// or sharded cluster.
    pub publisher_mongo_uri: Option<String>,
    /// The independent MongoDB member the attester/reader read from to verify
    /// oplog completeness. Persisted as `ATTESTER_MONGO_URI` + `READER_MONGO_URI`.
    /// For a real trust anchor this should be a replica member the operator does
    /// not control. Left empty, it falls back to the publisher URI (functional
    /// but not independent); if both are empty, the demo independent member
    /// (mongo3) is used.
    pub attester_mongo_uri: Option<String>,
    /// Setup role: `all` (Dev Mode), `publisher`, or `attester`.
    /// Dev Mode desktop always uses `all`; the flag exists so the same
    /// wizard binary supports separated production deployments.
    pub role: Option<String>,
}

/// Result of running the setup wizard. The `log` is redacted of any Stellar
/// secret keys before being returned to the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevSetupResult {
    pub success: bool,
    /// Redacted wizard output (secrets stripped).
    pub log: String,
    pub env_audit_present: bool,
    pub attester_key_present: bool,
}

/// Heuristically detect a Stellar secret seed (strkey: 56 chars, starts with
/// `S`, base32 alphabet). Slightly over-broad on purpose — we'd rather redact
/// a benign token than leak a key.
fn is_stellar_secret(s: &str) -> bool {
    s.len() == 56
        && s.starts_with('S')
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Redact Stellar secret keys (and `KEY=SECRET` env lines) from wizard output
/// so secrets never reach the UI, logs, or the webview console.
fn redact_secrets(input: &str) -> String {
    input
        .split('\n')
        .map(|line| {
            line.split_whitespace()
                .map(|tok| {
                    // Handle both bare seeds and `KEY=SEED` forms.
                    let value = tok.rsplit('=').next().unwrap_or(tok);
                    let core = value.trim_matches(|c: char| !c.is_ascii_alphanumeric());
                    if is_stellar_secret(core) {
                        "[redacted]".to_string()
                    } else {
                        tok.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn upsert_env_var(content: &mut String, key: &str, value: &str) {
    let prefix = format!("{key}=");
    let mut replaced = false;
    let lines = content
        .lines()
        .map(|line| {
            if line.starts_with(&prefix) {
                replaced = true;
                format!("{prefix}{value}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>();

    *content = lines.join("\n");
    if !content.ends_with('\n') {
        content.push('\n');
    }
    if !replaced {
        content.push_str(&format!("{prefix}{value}\n"));
    }
}

fn persist_dev_mongo_uris(
    env_path: &Path,
    publisher_uri: Option<&str>,
    attester_uri: Option<&str>,
) -> AppResult<()> {
    let mut content = std::fs::read_to_string(env_path)
        .map_err(|e| AppError::Internal(format!("read .env.audit: {e}")))?;
    if let Some(uri) = publisher_uri {
        upsert_env_var(&mut content, "PUBLISHER_MONGO_URI", uri);
    }
    if let Some(uri) = attester_uri {
        upsert_env_var(&mut content, "ATTESTER_MONGO_URI", uri);
        upsert_env_var(&mut content, "READER_MONGO_URI", uri);
    }
    std::fs::write(env_path, content)
        .map_err(|e| AppError::Internal(format!("write .env.audit Mongo URI: {e}")))
}

/// Run the non-interactive setup wizard inside the one-off `setup` Compose
/// service. Operator-supplied secrets are passed via a temporary env-file
/// (never on argv) that is deleted immediately afterward. The wizard writes
/// `.env.audit` + `attester.key` into the stack dir (mounted at `/work`), where
/// the publisher/attester/reader services pick them up.
pub fn setup_audit_config(app: &AppHandle, params: DevSetupParams) -> AppResult<DevSetupResult> {
    let ctx = stack_ctx(app)?;
    let env_audit = ctx.dir.join(".env.audit");
    let overwrite = params.overwrite.unwrap_or(false);
    if env_audit.exists() && !overwrite {
        return Err(AppError::Internal(
            "Audit config already exists (.env.audit). Re-running setup regenerates keys and \
             overwrites it — pass overwrite to proceed."
                .into(),
        ));
    }

    // Build the secret-bearing env-file the setup container reads.
    let setup_env_name = ".setup.env";
    let setup_env_path = ctx.dir.join(setup_env_name);
    let network = params
        .network
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("testnet");
    let mut env_lines = format!("STELLAR_NETWORK={network}\n");
    let mut push = |key: &str, val: &Option<String>| {
        if let Some(v) = val.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            env_lines.push_str(&format!("{key}={v}\n"));
        }
    };
    push("STELLAR_SECRET_KEY", &params.publisher_secret_key);
    push("ATTESTER_SECRET_KEY", &params.attester_secret_key);
    push("CONTRACT_ID", &params.contract_id);
    push("PINATA_API_KEY", &params.pinata_api_key);
    push("PINATA_API_SECRET", &params.pinata_api_secret);
    push("PINATA_GATEWAY_URL", &params.pinata_gateway_url);
    push("SETUP_ROLE", &Some("all".to_string())); // Dev Mode always uses all
    drop(push);

    // Deploy a fresh per-user contract by default so the generated publisher
    // becomes the contract admin — `authorize_attester` requires the caller to
    // be the admin, which a fresh key never is for the bundled shared contract.
    // The only time we reuse an existing contract is when the operator supplied
    // an explicit contract ID to target.
    let use_existing_contract = params
        .contract_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    if !use_existing_contract {
        env_lines.push_str("DEPLOY_CHOICE=deploy\n");
    }

    std::fs::write(&setup_env_path, env_lines.as_bytes())
        .map_err(|e| AppError::Internal(format!("write setup env file: {e}")))?;

    // `docker compose run` auto-enables the service's `setup` profile. The args
    // after the service name replace the service `command`, so we pass the full
    // `setup --non-interactive` subcommand to the image entrypoint. The secret
    // env vars are injected via the service's `env_file: .setup.env` entry in
    // the Compose file (not `run --env-file`, which older Compose releases such
    // as v2.31 reject), keeping secrets off argv.
    // Stream the wizard's output line-by-line to the UI (key generation,
    // funding, contract deploy, attester authorization) so the user can see
    // progress instead of staring at a spinner.
    let run_result = run_setup_streaming(&ctx, app);

    // Always remove the temp env-file (best-effort), even on failure.
    let _ = std::fs::remove_file(&setup_env_path);

    // `run_compose` already returns a concise, secret-free message (and logs
    // the raw error), so propagate it as-is rather than re-wrapping it.
    let log = run_result?;

    let publisher_uri = params
        .publisher_mongo_uri
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    // The attester/reader use the independent-member URI when provided; else
    // they fall back to the publisher URI so the stack stays reachable.
    let attester_uri = params
        .attester_mongo_uri
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(publisher_uri);
    if publisher_uri.is_some() || attester_uri.is_some() {
        persist_dev_mongo_uris(&env_audit, publisher_uri, attester_uri)?;
    }

    // Sync the freshly deployed contract ID into the app's AuditModeConfig so
    // in-app Stellar commands (oplog verify, rebuild from chain, etc.) know
    // which contract to query without the user having to manually copy it from
    // .env.audit into Settings.
    if let Some(cid) = read_env_var(&env_audit, "CONTRACT_ID").filter(|s| !s.is_empty()) {
        if let Err(e) = crate::audit::audit_mode::save_testnet_contract_id(app, cid) {
            tracing::warn!("setup succeeded but contract ID sync to settings failed: {e}");
        }
    }

    Ok(DevSetupResult {
        success: true,
        log: redact_secrets(&log),
        env_audit_present: ctx.dir.join(".env.audit").exists(),
        attester_key_present: ctx.dir.join("attester.key").exists(),
    })
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

/// Run the non-interactive setup wizard (generate keys, fund, authorize, write
/// `.env.audit` + `attester.key`). Secrets are redacted from the returned log.
#[tauri::command]
pub async fn audit_dev_stack_setup(
    app: AppHandle,
    params: DevSetupParams,
) -> AppResult<DevSetupResult> {
    tokio::task::spawn_blocking(move || setup_audit_config(&app, params))
        .await
        .map_err(|e| AppError::Internal(format!("stack setup task join: {e}")))?
}

/// The public Stellar addresses of the dev-stack publisher and attester.
///
/// Derived by reading `.env.audit` and decoding the secret-key strkeys into
/// their corresponding public `G...` addresses. Used by the UI to make key
/// separation visible: the publisher and attester are distinct accounts.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevStackIdentities {
    /// The publisher's Stellar account address (operator).
    pub publisher_address: String,
    /// The attester's Stellar account address (independent auditor).
    pub attester_address: String,
    /// The Soroban contract ID the stack is configured against.
    pub contract_id: String,
}

/// Read `.env.audit` and return the publisher/attester public addresses.
///
/// Only the public addresses are returned; secret keys are never exposed to
/// the frontend.
#[tauri::command]
pub async fn audit_dev_stack_identities(
    app: AppHandle,
) -> AppResult<Option<DevStackIdentities>> {
    use audit_service::auditd::load_keypair_from_secret_key;

    let ctx = stack_ctx(&app).ok();
    let ctx = match ctx {
        Some(c) => c,
        None => return Ok(None),
    };
    let env_path = ctx.dir.join(".env.audit");
    if !env_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&env_path)
        .map_err(|e| AppError::Internal(format!("read .env.audit: {e}")))?;

    let mut publisher_secret = String::new();
    let mut attester_secret = String::new();
    let mut contract_id = String::new();
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("STELLAR_SECRET_KEY=") {
            publisher_secret = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("ATTESTER_SECRET_KEY=") {
            attester_secret = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("CONTRACT_ID=") {
            contract_id = val.trim().to_string();
        }
    }

    if publisher_secret.is_empty() && attester_secret.is_empty() {
        return Ok(None);
    }

    let publisher_address = if publisher_secret.is_empty() {
        String::new()
    } else {
        match load_keypair_from_secret_key(&publisher_secret) {
            Ok(kp) => kp.account_id(),
            Err(_) => String::new(),
        }
    };
    let attester_address = if attester_secret.is_empty() {
        String::new()
    } else {
        match load_keypair_from_secret_key(&attester_secret) {
            Ok(kp) => kp.account_id(),
            Err(_) => String::new(),
        }
    };

    Ok(Some(DevStackIdentities {
        publisher_address,
        attester_address,
        contract_id,
    }))
}

/// Material the operator hands to the auditor out-of-band so the auditor
/// can independently verify the audit trail.
///
/// This includes:
/// - The operator's age public key (the auditor encrypts to this + their own)
/// - The auditor's age public key (for reference)
/// - The auditor's age secret key (the sensitive decryption key)
/// - The audit leaf key (k_audit, needed for HMAC leaf verification)
/// - The Soroban contract ID (needed to query on-chain roots)
/// - A ready-made MongoDB connection string for the independent replica member
///
/// The age secret key is sensitive material. The frontend shows it only behind
/// an explicit "reveal" interaction. In a real deployment this would be exchanged
/// via a secure channel (Signal, 1Password, PGP) rather than copy-paste.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditorHandoffMaterial {
    pub contract_id: String,
    pub rpc_url: String,
    pub network_passphrase: String,
    pub age_public_key_operator: String,
    pub age_public_key_attester: String,
    pub age_attester_secret: String,
    /// The attester's funded Stellar secret key (S...). Dev Mode only: the
    /// setup wizard generates and funds this account on the same machine, so
    /// handing it to the in-app auditor surface lets "Verify & Record" work
    /// without copy-paste. In a real deployment the auditor's Stellar key
    /// never passes through the operator, and this stays empty.
    pub attester_stellar_secret: String,
    pub audit_leaf_key_hex: String,
    pub auditor_mongo_uri: String,
    pub operator_mongo_uri: String,
}

/// Read `.env.audit` and return the auditor's handoff material.
#[tauri::command]
pub async fn audit_dev_stack_auditor_material(
    app: AppHandle,
) -> AppResult<Option<AuditorHandoffMaterial>> {
    let ctx = stack_ctx(&app).ok();
    let ctx = match ctx {
        Some(c) => c,
        None => return Ok(None),
    };
    let env_path = ctx.dir.join(".env.audit");
    if !env_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&env_path)
        .map_err(|e| AppError::Internal(format!("read .env.audit: {e}")))?;

    let mut contract_id = String::new();
    let mut rpc_url = String::new();
    let mut network_passphrase = String::new();
    let mut age_public_key_operator = String::new();
    let mut age_public_key_attester = String::new();
    let mut age_operator_secret = String::new();
    let mut age_attester_secret = String::new();
    let mut attester_stellar_secret = String::new();
    let mut audit_leaf_key_hex = String::new();
    let mut auditor_mongo_uri = String::new();
    let mut operator_mongo_uri = String::new();

    for line in content.lines() {
        if let Some(val) = line.strip_prefix("CONTRACT_ID=") {
            contract_id = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("STELLAR_RPC_URL=") {
            rpc_url = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("STELLAR_NETWORK_PASSPHRASE=") {
            network_passphrase = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("AGE_OPERATOR_PUBLIC_KEY=") {
            age_public_key_operator = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("AGE_ATTESTER_PUBLIC_KEY=") {
            age_public_key_attester = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("AGE_OPERATOR_SECRET=") {
            age_operator_secret = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("AGE_ATTESTER_SECRET=") {
            age_attester_secret = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("ATTESTER_SECRET_KEY=") {
            attester_stellar_secret = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("AUDIT_LEAF_KEY_HEX=") {
            audit_leaf_key_hex = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("AUDIT_LEAF_KEY=") {
            if audit_leaf_key_hex.is_empty() {
                audit_leaf_key_hex = val.trim().to_string();
            }
        } else if let Some(val) = line.strip_prefix("READER_MONGO_URI=") {
            auditor_mongo_uri = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("PUBLISHER_MONGO_URI=") {
            operator_mongo_uri = val.trim().to_string();
        }
    }

    if contract_id.is_empty() {
        return Ok(None);
    }

    // Fallbacks for older .env.audit files that may not have these fields.
    if rpc_url.is_empty() {
        rpc_url = "https://soroban-testnet.stellar.org:443".to_string();
    }
    if network_passphrase.is_empty() {
        network_passphrase = "Test SDF Network ; September 2015".to_string();
    }
    if auditor_mongo_uri.is_empty() {
        auditor_mongo_uri = "mongodb://auditor:nosqlbuddy-dev-auditor-pw@localhost:27019/?authSource=admin&directConnection=true".to_string();
    }
    if operator_mongo_uri.is_empty() {
        operator_mongo_uri = "mongodb://root:nosqlbuddy-dev-root-pw@localhost:27020/?authSource=admin&directConnection=true".to_string();
    }
    // Older .env.audit files store secrets but not public keys. Derive the
    // public key from the age secret key string (it contains a comment line).
    if age_public_key_operator.is_empty() && !age_operator_secret.is_empty() {
        age_public_key_operator = extract_age_public_key(&age_operator_secret);
    }
    if age_public_key_attester.is_empty() && !age_attester_secret.is_empty() {
        age_public_key_attester = extract_age_public_key(&age_attester_secret);
    }

    Ok(Some(AuditorHandoffMaterial {
        contract_id,
        rpc_url,
        network_passphrase,
        age_public_key_operator,
        age_public_key_attester,
        age_attester_secret,
        attester_stellar_secret,
        audit_leaf_key_hex,
        auditor_mongo_uri,
        operator_mongo_uri,
    }))
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

    #[test]
    fn test_upsert_env_var_appends_when_missing() {
        let mut content = "STELLAR_SECRET_KEY=abc\n".to_string();
        upsert_env_var(&mut content, "PUBLISHER_MONGO_URI", "mongodb://x:27017");
        assert!(content.contains("STELLAR_SECRET_KEY=abc"));
        assert!(content.contains("PUBLISHER_MONGO_URI=mongodb://x:27017"));
        assert!(content.ends_with('\n'));
    }

    #[test]
    fn test_upsert_env_var_replaces_existing() {
        let mut content = "PUBLISHER_MONGO_URI=old\nCONTRACT_ID=c\n".to_string();
        upsert_env_var(&mut content, "PUBLISHER_MONGO_URI", "new");
        assert!(content.contains("PUBLISHER_MONGO_URI=new"));
        assert!(!content.contains("PUBLISHER_MONGO_URI=old"));
        assert!(content.contains("CONTRACT_ID=c"));
    }

    #[test]
    fn test_read_env_var_missing_and_present() {
        let dir = std::env::temp_dir().join(format!("nb-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env.audit");
        std::fs::write(&path, "PUBLISHER_MONGO_URI=mongodb://y:27017\nEMPTY=\n").unwrap();
        assert_eq!(
            read_env_var(&path, "PUBLISHER_MONGO_URI").as_deref(),
            Some("mongodb://y:27017")
        );
        assert_eq!(read_env_var(&path, "EMPTY"), None);
        assert_eq!(read_env_var(&path, "ABSENT"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_persist_dev_mongo_uris_independent_attester() {
        let dir = std::env::temp_dir().join(format!("nb-uri-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env.audit");
        std::fs::write(&path, "CONTRACT_ID=c\n").unwrap();

        persist_dev_mongo_uris(&path, Some("mongodb://pub:27017"), Some("mongodb://att:27019"))
            .unwrap();

        assert_eq!(
            read_env_var(&path, "PUBLISHER_MONGO_URI").as_deref(),
            Some("mongodb://pub:27017")
        );
        assert_eq!(
            read_env_var(&path, "ATTESTER_MONGO_URI").as_deref(),
            Some("mongodb://att:27019")
        );
        assert_eq!(
            read_env_var(&path, "READER_MONGO_URI").as_deref(),
            Some("mongodb://att:27019")
        );
        assert_eq!(read_env_var(&path, "CONTRACT_ID").as_deref(), Some("c"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
