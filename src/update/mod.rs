//! Safe self-update support for the published Raspberry Pi ARM64 package.
//!
//! The updater intentionally delegates HTTPS and archive handling to the same
//! ubiquitous tools used by `scripts/install.sh`, but validates the unpacked
//! layout before it asks for elevated privileges.

use crate::daemon;
use crate::errors::IrisError;
use serde::Deserialize;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

const RELEASE_URL: &str = "https://api.github.com/repos/pieza/iris-tv/releases/latest";
const ASSET_NAME: &str = "iris-aarch64-unknown-linux-gnu.tar.gz";
const BINARY_PATH: &str = "/usr/local/bin/iris";
const PROFILES_PATH: &str = "/usr/local/share/iris/profiles";

#[derive(Debug)]
pub struct UpdateOptions {
    pub check_only: bool,
    pub replace_profiles: bool,
    pub state_dir: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateResult {
    UpToDate {
        installed: SemanticVersion,
        latest: SemanticVersion,
    },
    Available {
        installed: SemanticVersion,
        latest: SemanticVersion,
    },
    Installed {
        installed: SemanticVersion,
        profiles: &'static str,
        daemon_restarted: bool,
    },
}

/// The stable release version format used by IRIS tags (`Vmajor.minor.patch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemanticVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl std::fmt::Display for SemanticVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

trait ProcessRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<Vec<u8>, IrisError>;
}

struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<Vec<u8>, IrisError> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|source| IrisError::UpdateCommandFailed {
                command: program.to_string(),
                reason: source.to_string(),
            })?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(IrisError::UpdateCommandFailed {
                command: format_command(program, args),
                reason: if detail.is_empty() {
                    format!("exited with {}", output.status)
                } else {
                    detail
                },
            })
        }
    }
}

pub fn run(options: UpdateOptions) -> Result<UpdateResult, IrisError> {
    let runner = SystemProcessRunner;
    let release = fetch_release(&runner)?;
    let installed = package_version(env!("CARGO_PKG_VERSION"))?;
    let latest = package_version(&release.tag_name)?;

    if latest <= installed {
        return Ok(UpdateResult::UpToDate { installed, latest });
    }
    let asset = release_asset(&release)?;
    if options.check_only {
        return Ok(UpdateResult::Available { installed, latest });
    }

    validate_installation()?;
    install_release(&runner, asset, latest, &options)
}

fn release_asset(release: &Release) -> Result<&ReleaseAsset, IrisError> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == ASSET_NAME)
        .ok_or(IrisError::UpdateAssetMissing)
}

fn fetch_release(runner: &dyn ProcessRunner) -> Result<Release, IrisError> {
    let args = vec!["-fsSL".to_string(), RELEASE_URL.to_string()];
    let body = runner.run("curl", &args)?;
    let release: Release =
        serde_json::from_slice(&body).map_err(|source| IrisError::InvalidRelease {
            reason: source.to_string(),
        })?;
    if release.draft || release.prerelease {
        return Err(IrisError::InvalidRelease {
            reason: "latest endpoint returned a draft or prerelease".to_string(),
        });
    }
    Ok(release)
}

fn install_release(
    runner: &dyn ProcessRunner,
    asset: &ReleaseAsset,
    latest: SemanticVersion,
    options: &UpdateOptions,
) -> Result<UpdateResult, IrisError> {
    let temp = create_temp_dir()?;
    let result = install_release_in(runner, asset, latest, options, &temp);
    let _ = std::fs::remove_dir_all(&temp);
    result
}

fn install_release_in(
    runner: &dyn ProcessRunner,
    asset: &ReleaseAsset,
    latest: SemanticVersion,
    options: &UpdateOptions,
    temp: &Path,
) -> Result<UpdateResult, IrisError> {
    let archive = temp.join(ASSET_NAME);
    run_command(
        runner,
        "curl",
        vec![
            "-fsSL".into(),
            asset.browser_download_url.clone(),
            "-o".into(),
            path(&archive),
        ],
    )?;
    run_command(
        runner,
        "tar",
        vec!["-xzf".into(), path(&archive), "-C".into(), path(temp)],
    )?;

    let package = temp.join("iris");
    let binary = package.join("iris");
    let profiles = package.join("profiles");
    validate_package(&binary, &profiles)?;

    let profiles_installed = install_profiles(runner, &profiles, options.replace_profiles)?;

    // Staging the executable is privileged but does not affect a running
    // daemon. Only after it succeeds do we stop the daemon and atomically move
    // the staged file into place.
    let staged_binary =
        Path::new("/usr/local/bin").join(format!(".iris-update-{}", uuid::Uuid::new_v4()));
    privileged(
        runner,
        vec![
            "install".into(),
            "-m".into(),
            "0755".into(),
            path(&binary),
            path(&staged_binary),
        ],
    )?;

    let daemon_was_running = daemon::is_running(&options.state_dir)?;
    if daemon_was_running {
        daemon::stop(&options.state_dir)?;
    }

    if let Err(error) = privileged(
        runner,
        vec![
            "mv".into(),
            "-f".into(),
            path(&staged_binary),
            BINARY_PATH.into(),
        ],
    ) {
        if daemon_was_running {
            let _ = daemon::start(&options.state_dir);
        }
        return Err(error);
    }

    if daemon_was_running {
        daemon::start(&options.state_dir)?;
    }
    Ok(UpdateResult::Installed {
        installed: latest,
        profiles: profiles_installed,
        daemon_restarted: daemon_was_running,
    })
}

fn install_profiles(
    runner: &dyn ProcessRunner,
    source: &Path,
    replace: bool,
) -> Result<&'static str, IrisError> {
    let destination = Path::new(PROFILES_PATH);
    if destination.exists() && !replace {
        return Ok("preserved");
    }
    if replace && destination.exists() {
        privileged(runner, vec!["rm".into(), "-rf".into(), path(destination)])?;
    }
    privileged(
        runner,
        vec!["mkdir".into(), "-p".into(), "/usr/local/share/iris".into()],
    )?;
    privileged(
        runner,
        vec!["cp".into(), "-R".into(), path(source), path(destination)],
    )?;
    Ok(if replace { "replaced" } else { "installed" })
}

fn privileged(runner: &dyn ProcessRunner, command: Vec<String>) -> Result<(), IrisError> {
    if is_root() {
        let (program, args) =
            command
                .split_first()
                .ok_or_else(|| IrisError::InvalidUpdatePackage {
                    reason: "empty install command".into(),
                })?;
        run_command(runner, program, args.to_vec())
    } else {
        let mut args = Vec::with_capacity(command.len());
        args.extend(command);
        run_command(runner, "sudo", args)
    }
}

fn run_command(
    runner: &dyn ProcessRunner,
    program: &str,
    args: Vec<String>,
) -> Result<(), IrisError> {
    runner.run(program, &args).map(|_| ())
}

fn validate_installation() -> Result<(), IrisError> {
    let architecture = std::env::consts::ARCH;
    if architecture != "aarch64" && architecture != "arm64" {
        return Err(IrisError::UpdateUnsupportedArchitecture {
            architecture: architecture.to_string(),
        });
    }
    let executable = std::env::current_exe().map_err(IrisError::IoPlain)?;
    if executable != Path::new(BINARY_PATH) {
        return Err(IrisError::UpdateUnsupportedInstallation { path: executable });
    }
    Ok(())
}

fn validate_package(binary: &Path, profiles: &Path) -> Result<(), IrisError> {
    let metadata = std::fs::metadata(binary).map_err(|_| IrisError::InvalidUpdatePackage {
        reason: "missing iris/iris executable".to_string(),
    })?;
    if !metadata.is_file() || !is_executable(&metadata) {
        return Err(IrisError::InvalidUpdatePackage {
            reason: "iris/iris is not executable".to_string(),
        });
    }
    if !profiles.is_dir() {
        return Err(IrisError::InvalidUpdatePackage {
            reason: "missing iris/profiles directory".to_string(),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &std::fs::Metadata) -> bool {
    true
}

fn package_version(value: &str) -> Result<SemanticVersion, IrisError> {
    let normalized = value.trim().trim_start_matches(['v', 'V']);
    if normalized.contains(['-', '+']) {
        return Err(IrisError::InvalidRelease {
            reason: format!("release tag `{value}` is not stable"),
        });
    }
    let parts = normalized.split('.').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err(IrisError::InvalidRelease {
            reason: format!("invalid version tag `{value}`; expected major.minor.patch"),
        });
    }
    let parse = |part: &str| {
        part.parse::<u64>().map_err(|_| IrisError::InvalidRelease {
            reason: format!("invalid version tag `{value}`; expected major.minor.patch"),
        })
    };
    Ok(SemanticVersion {
        major: parse(parts[0])?,
        minor: parse(parts[1])?,
        patch: parse(parts[2])?,
    })
}

fn create_temp_dir() -> Result<PathBuf, IrisError> {
    let path = std::env::temp_dir().join(format!("iris-update-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&path).map_err(|source| IrisError::io(&path, source))?;
    Ok(path)
}

fn path(path: &Path) -> String {
    path.as_os_str().to_string_lossy().into_owned()
}

fn format_command(program: &str, args: &[String]) -> String {
    std::iter::once(OsStr::new(program).to_string_lossy().into_owned())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(target_os = "linux")]
fn is_root() -> bool {
    // libc is already a Linux dependency for the GPIO implementation.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(target_os = "linux"))]
fn is_root() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticRunner(Vec<u8>);

    impl ProcessRunner for StaticRunner {
        fn run(&self, program: &str, _: &[String]) -> Result<Vec<u8>, IrisError> {
            assert_eq!(program, "curl");
            Ok(self.0.clone())
        }
    }

    #[test]
    fn accepts_upper_and_lowercase_tag_prefixes() {
        assert_eq!(package_version("V1.6.5").unwrap().to_string(), "1.6.5");
        assert_eq!(package_version("v1.6.5").unwrap().to_string(), "1.6.5");
    }

    #[test]
    fn compares_remote_versions_semantically() {
        let current = package_version("1.6.5").unwrap();
        assert!(package_version("V1.6.6").unwrap() > current);
        assert!(package_version("v1.6.4").unwrap() < current);
        assert_eq!(package_version("1.6.5").unwrap(), current);
    }

    #[test]
    fn rejects_invalid_and_prerelease_tags() {
        assert!(package_version("latest").is_err());
        assert!(package_version("v1.7.0-rc.1").is_err());
    }

    #[test]
    fn parses_stable_release_and_selects_arm64_asset() {
        let runner = StaticRunner(
            br#"{"tag_name":"V1.6.6","draft":false,"prerelease":false,"assets":[{"name":"iris-aarch64-unknown-linux-gnu.tar.gz","browser_download_url":"https://example.test/iris.tar.gz"}]}"#.to_vec(),
        );
        let release = fetch_release(&runner).unwrap();
        assert_eq!(
            package_version(&release.tag_name).unwrap().to_string(),
            "1.6.6"
        );
        assert_eq!(
            release_asset(&release).unwrap().browser_download_url,
            "https://example.test/iris.tar.gz"
        );
    }

    #[test]
    fn reports_missing_arm64_asset() {
        let release = Release {
            tag_name: "V1.6.6".into(),
            draft: false,
            prerelease: false,
            assets: vec![],
        };
        assert!(matches!(
            release_asset(&release),
            Err(IrisError::UpdateAssetMissing)
        ));
    }

    #[test]
    fn validates_package_layout() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("iris");
        std::fs::create_dir_all(package.join("profiles")).unwrap();
        let binary = package.join("iris");
        std::fs::write(&binary, "binary").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&binary).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&binary, permissions).unwrap();
        }
        validate_package(&binary, &package.join("profiles")).unwrap();
    }
}
