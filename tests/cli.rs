use assert_cmd::Command;
use predicates::str::contains;
use tempfile::tempdir;

const TELSTAR_PROFILE: &str = r#"
brand = "telstar"
model = "xxx"
device_type = "tv"
carrier_frequency = 38000
protocol = "nec"

[commands]
power = { type = "nec", address = "0x00FF", command = "0xA25D" }
"#;

#[test]
fn start_saves_active_profile_and_send_uses_it() {
    let config_dir = tempdir().expect("config dir");
    let profile_dir = tempdir().expect("profile dir");
    let telstar_dir = profile_dir.path().join("tv").join("telstar");
    std::fs::create_dir_all(&telstar_dir).expect("profile dirs");
    std::fs::write(telstar_dir.join("xxx.toml"), TELSTAR_PROFILE).expect("profile");

    Command::cargo_bin("iris")
        .expect("binary")
        .env("IRIS_CONFIG_DIR", config_dir.path())
        .env("IRIS_PROFILE_DIR", profile_dir.path())
        .args(["start", "telstar xxx"])
        .assert()
        .success()
        .stdout(contains("Loaded active profile telstar/xxx"));

    Command::cargo_bin("iris")
        .expect("binary")
        .env("IRIS_CONFIG_DIR", config_dir.path())
        .env("IRIS_PROFILE_DIR", profile_dir.path())
        .args(["send", "power", "--dry-run"])
        .assert()
        .success()
        .stdout(contains("NEC"))
        .stdout(contains("0x00FF"))
        .stdout(contains("0xA25D"));
}
