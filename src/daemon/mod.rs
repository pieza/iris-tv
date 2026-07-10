use crate::errors::IrisError;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn start(state_dir: &Path) -> Result<u32, IrisError> {
    std::fs::create_dir_all(state_dir).map_err(|source| IrisError::io(state_dir, source))?;
    let pid_path = pid_path(state_dir);
    if let Some(pid) = read_pid(&pid_path)? {
        if process_alive(pid) {
            return Err(IrisError::DaemonAlreadyRunning { pid });
        }
        remove_pid(&pid_path)?;
    }

    let exe = std::env::current_exe().map_err(IrisError::IoPlain)?;
    let child = Command::new(exe)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(IrisError::IoPlain)?;
    let pid = child.id();
    std::fs::write(&pid_path, pid.to_string())
        .map_err(|source| IrisError::io(&pid_path, source))?;
    Ok(pid)
}

pub fn stop(state_dir: &Path) -> Result<(), IrisError> {
    let pid_path = pid_path(state_dir);
    let Some(pid) = read_pid(&pid_path)? else {
        return Err(IrisError::DaemonNotRunning);
    };

    if process_alive(pid) {
        terminate_process(pid)?;
    }
    remove_pid(&pid_path)
}

/// Returns whether the daemon PID file refers to a currently running process.
/// A stale or malformed PID file is treated as not running.
pub fn is_running(state_dir: &Path) -> Result<bool, IrisError> {
    Ok(read_pid(&pid_path(state_dir))?.is_some_and(process_alive))
}

fn pid_path(state_dir: &Path) -> PathBuf {
    state_dir.join("iris.pid")
}

fn read_pid(pid_path: &Path) -> Result<Option<u32>, IrisError> {
    if !pid_path.exists() {
        return Ok(None);
    }
    let raw =
        std::fs::read_to_string(pid_path).map_err(|source| IrisError::io(pid_path, source))?;
    match raw.trim().parse::<u32>() {
        Ok(pid) => Ok(Some(pid)),
        Err(_) => Ok(None),
    }
}

fn remove_pid(pid_path: &Path) -> Result<(), IrisError> {
    if pid_path.exists() {
        std::fs::remove_file(pid_path).map_err(|source| IrisError::io(pid_path, source))?;
    }
    Ok(())
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}")])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn terminate_process(pid: u32) -> Result<(), IrisError> {
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .map_err(IrisError::IoPlain)?;
    if status.success() {
        Ok(())
    } else {
        Err(IrisError::DaemonNotRunning)
    }
}

#[cfg(not(windows))]
fn terminate_process(pid: u32) -> Result<(), IrisError> {
    let status = Command::new("kill")
        .arg(pid.to_string())
        .status()
        .map_err(IrisError::IoPlain)?;
    if status.success() {
        Ok(())
    } else {
        Err(IrisError::DaemonNotRunning)
    }
}
