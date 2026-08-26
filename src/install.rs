//! Installs the daemon so it survives logout/reboot: copies the running
//! binary to a stable location and writes a systemd unit that starts
//! `skillastic monitor resume` at login (user scope) or boot (system scope).

use crate::error::{Result, SkillasticError};
use crate::monitor::Scope;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Set to skip the actual `systemctl` calls in [`install_unit`] — the unit
/// file is still written. Used by tests so they don't register a real
/// systemd unit on the machine running them.
pub const SKIP_SYSTEMCTL_ENV: &str = "SKILLASTIC_SKIP_SYSTEMCTL";

pub fn bin_dir(scope: Scope) -> Result<PathBuf> {
    Ok(match scope {
        Scope::User => user_home()?.join(".local/bin"),
        Scope::System => PathBuf::from("/usr/local/bin"),
    })
}

fn user_home() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| SkillasticError::Other("$HOME is not set".into()))
}

/// Copies the currently running binary into the scope's bin directory,
/// unless it's already running from there. Returns the installed path.
pub fn ensure_binary_installed(scope: Scope) -> Result<PathBuf> {
    let current = std::env::current_exe()?;
    install_binary_at(&current, &bin_dir(scope)?)
}

fn install_binary_at(current: &Path, target_dir: &Path) -> Result<PathBuf> {
    let target = target_dir.join("skillastic");
    let current_canon = current.canonicalize().unwrap_or_else(|_| current.to_path_buf());
    let target_canon = target.canonicalize().unwrap_or_else(|_| target.clone());
    if current_canon == target_canon {
        return Ok(target);
    }
    fs::create_dir_all(target_dir)?;
    fs::copy(current, &target)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&target)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&target, perms)?;
    }
    Ok(target)
}

fn systemd_unit_dir(scope: Scope) -> Result<PathBuf> {
    Ok(match scope {
        Scope::User => user_home()?.join(".config/systemd/user"),
        Scope::System => PathBuf::from("/etc/systemd/system"),
    })
}

fn unit_contents(scope: Scope, bin_path: &Path) -> String {
    let (target, extra_arg) = match scope {
        Scope::User => ("default.target", ""),
        Scope::System => ("multi-user.target", " --system"),
    };
    format!(
        "[Unit]\n\
Description=Skillastic daemon supervisor\n\
Documentation=https://github.com/elci-group/skillastic\n\
After=network.target\n\
\n\
[Service]\n\
Type=oneshot\n\
RemainAfterExit=yes\n\
ExecStart={} monitor resume{extra_arg}\n\
StandardOutput=journal\n\
StandardError=journal\n\
SyslogIdentifier=skillastic\n\
\n\
[Install]\n\
WantedBy={target}\n",
        bin_path.display(),
    )
}

/// Writes the systemd unit for `scope` and enables + starts it (unless
/// [`SKIP_SYSTEMCTL_ENV`] is set). Returns the unit file path.
pub fn install_unit(scope: Scope, bin_path: &Path) -> Result<PathBuf> {
    let dir = systemd_unit_dir(scope)?;
    fs::create_dir_all(&dir)?;
    let unit_path = dir.join("skillastic.service");
    fs::write(&unit_path, unit_contents(scope, bin_path))?;

    if std::env::var_os(SKIP_SYSTEMCTL_ENV).is_some() {
        return Ok(unit_path);
    }

    run_systemctl(scope, &["daemon-reload"])?;
    run_systemctl(scope, &["enable", "--now", "skillastic.service"])?;
    Ok(unit_path)
}

fn run_systemctl(scope: Scope, args: &[&str]) -> Result<()> {
    let mut cmd = Command::new("systemctl");
    if scope == Scope::User {
        cmd.arg("--user");
    }
    cmd.args(args);
    let output = cmd
        .output()
        .map_err(|e| SkillasticError::Other(format!("failed to run systemctl: {e}")))?;
    if !output.status.success() {
        return Err(SkillasticError::Other(format!(
            "systemctl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn user_unit_targets_login_and_omits_system_flag() {
        let contents = unit_contents(Scope::User, Path::new("/home/x/.local/bin/skillastic"));
        assert!(contents.contains("ExecStart=/home/x/.local/bin/skillastic monitor resume\n"));
        assert!(contents.contains("WantedBy=default.target"));
        assert!(!contents.contains("--system"));
    }

    #[test]
    fn system_unit_targets_boot_and_passes_system_flag() {
        let contents = unit_contents(Scope::System, Path::new("/usr/local/bin/skillastic"));
        assert!(contents.contains("ExecStart=/usr/local/bin/skillastic monitor resume --system\n"));
        assert!(contents.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn installs_binary_into_target_dir_with_exec_bit() {
        let dir = TempDir::new().unwrap();
        let fake_current = dir.path().join("source-binary");
        fs::write(&fake_current, b"#!/bin/sh\necho hi\n").unwrap();
        let target_dir = dir.path().join("bin");

        let installed = install_binary_at(&fake_current, &target_dir).unwrap();
        assert_eq!(installed, target_dir.join("skillastic"));
        assert!(installed.is_file());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&installed).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[test]
    fn skips_copy_when_already_installed() {
        let dir = TempDir::new().unwrap();
        let target_dir = dir.path().join("bin");
        fs::create_dir_all(&target_dir).unwrap();
        let target = target_dir.join("skillastic");
        fs::write(&target, b"already here").unwrap();

        let installed = install_binary_at(&target, &target_dir).unwrap();
        assert_eq!(installed, target);
        assert_eq!(fs::read(&target).unwrap(), b"already here");
    }
}
