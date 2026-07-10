use assert_cmd::Command;
use predicates::str::contains;
use tempfile::tempdir;

const TELSTAR_PROFILE: &str = r#"
brand = "telstar"
model = "generic"
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
    std::fs::write(telstar_dir.join("generic.toml"), TELSTAR_PROFILE).expect("profile");

    Command::cargo_bin("iris")
        .expect("binary")
        .env("IRIS_CONFIG_DIR", config_dir.path())
        .env("IRIS_PROFILE_DIR", profile_dir.path())
        .args(["start", "telstar"])
        .assert()
        .success()
        .stdout(contains("Loaded active profile telstar/generic"));

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

#[test]
fn start_accepts_optional_model() {
    let config_dir = tempdir().expect("config dir");
    let profile_dir = tempdir().expect("profile dir");
    let telstar_dir = profile_dir.path().join("tv").join("telstar");
    std::fs::create_dir_all(&telstar_dir).expect("profile dirs");
    std::fs::write(
        telstar_dir.join("ttc04.toml"),
        TELSTAR_PROFILE.replace("generic", "ttc04"),
    )
    .expect("profile");

    Command::cargo_bin("iris")
        .expect("binary")
        .env("IRIS_CONFIG_DIR", config_dir.path())
        .env("IRIS_PROFILE_DIR", profile_dir.path())
        .args(["start", "telstar", "--model", "TTC04"])
        .assert()
        .success()
        .stdout(contains("Loaded active profile telstar/ttc04"));
}

#[test]
fn home_assistant_setup_command_persists_network_discovery_config() {
    let config_dir = tempdir().expect("config dir");

    Command::cargo_bin("iris")
        .expect("binary")
        .env("IRIS_CONFIG_DIR", config_dir.path())
        .args(["home-assistant", "setup"])
        .assert()
        .success()
        .stdout(contains("Home Assistant discovery is ready"))
        .stdout(contains("server = 0.0.0.0:8787"))
        .stdout(contains("api_token ="));

    let config = std::fs::read_to_string(config_dir.path().join("config.toml"))
        .expect("config file written");
    assert!(config.contains("server_host = \"0.0.0.0\""));
    assert!(config.contains("server_port = 8787"));
    assert!(config.contains("discovery_enabled = true"));
    assert!(config.contains("device_id = "));
    assert!(config.contains("api_token = "));
}

#[test]
fn config_sets_receiver_pin_and_scan_help_lists_output_options() {
    let config_dir = tempdir().expect("config dir");

    Command::cargo_bin("iris")
        .expect("binary")
        .env("IRIS_CONFIG_DIR", config_dir.path())
        .args(["config", "set", "receiver_gpio_pin", "24"])
        .assert()
        .success()
        .stdout(contains("Set receiver_gpio_pin = 24"));

    let config = std::fs::read_to_string(config_dir.path().join("config.toml"))
        .expect("config file written");
    assert!(config.contains("receiver_gpio_pin = 24"));

    Command::cargo_bin("iris")
        .expect("binary")
        .args(["scan", "--help"])
        .assert()
        .success()
        .stdout(contains("--name <NAME>"))
        .stdout(contains("--path <PATH>"));
}
