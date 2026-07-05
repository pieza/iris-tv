use iris::config::{AppConfig, ConfigStore};
use iris::errors::IrisError;
use iris::ir::{DryRunTransmitter, IrSignal, IrTransmitter, MockTransmitter, build_nec_pulses};
use iris::profiles::{Profile, ProfileId, ProfileStore};
use tempfile::tempdir;

const TELSTAR_PROFILE: &str = r#"
brand = "telstar"
model = "xxx"
device_type = "tv"
carrier_frequency = 38000
protocol = "nec"

[commands]
power = { type = "nec", address = "0x00FF", command = "0xA25D" }
raw_demo = { type = "raw", frequency = 38000, pulses = [9000, 4500, 560] }
"#;

#[test]
fn parses_nec_and_raw_profile_commands() {
    let profile = Profile::from_toml_str(TELSTAR_PROFILE).expect("profile parses");

    assert_eq!(profile.brand, "telstar");
    assert_eq!(profile.model, "xxx");
    assert_eq!(profile.commands.len(), 2);
    assert_eq!(
        profile.signal_for("power").expect("power signal"),
        IrSignal::Nec {
            address: 0x00FF,
            command: 0xA25D
        }
    );
    assert_eq!(
        profile.signal_for("raw_demo").expect("raw signal"),
        IrSignal::Raw {
            frequency: 38_000,
            pulses: vec![9000, 4500, 560]
        }
    );
}

#[test]
fn resolves_brand_model_to_profile_id_and_file_path() {
    let id = ProfileId::parse("Telstar XXX").expect("profile id");

    assert_eq!(id.brand, "telstar");
    assert_eq!(id.model, "xxx");
    assert_eq!(id.key(), "telstar/xxx");
    assert_eq!(
        id.relative_path().to_string_lossy().replace('\\', "/"),
        "tv/telstar/xxx.toml"
    );
}

#[test]
fn profile_store_lists_brands_and_models() {
    let root = tempdir().expect("temp root");
    let profile_path = root.path().join("tv").join("telstar");
    std::fs::create_dir_all(&profile_path).expect("profile dir");
    std::fs::write(profile_path.join("xxx.toml"), TELSTAR_PROFILE).expect("profile file");

    let store = ProfileStore::new(root.path());

    assert_eq!(store.list_brands().expect("brands"), vec!["telstar"]);
    assert_eq!(
        store.list_models("telstar").expect("models"),
        vec!["xxx".to_string()]
    );
    assert_eq!(store.load("telstar xxx").expect("load").model, "xxx");
}

#[test]
fn config_store_persists_active_profile() {
    let root = tempdir().expect("temp root");
    let store = ConfigStore::new(root.path());
    let config = AppConfig {
        active_profile: Some("telstar/xxx".to_string()),
        gpio_pin: 23,
        ..AppConfig::default()
    };

    store.save(&config).expect("save config");
    let loaded = store.load().expect("load config");

    assert_eq!(loaded.active_profile.as_deref(), Some("telstar/xxx"));
    assert_eq!(loaded.gpio_pin, 23);
}

#[test]
fn nec_signal_builder_uses_expected_header_and_bit_timings() {
    let pulses = build_nec_pulses(0x0001, 0x0000);

    assert_eq!(&pulses[0..2], &[9000, 4500]);
    assert_eq!(&pulses[2..4], &[560, 1690]);
    assert_eq!(&pulses[4..6], &[560, 560]);
    assert_eq!(pulses.last(), Some(&560));
    assert_eq!(pulses.len(), 67);
}

#[test]
fn missing_command_reports_clear_error() {
    let profile = Profile::from_toml_str(TELSTAR_PROFILE).expect("profile parses");
    let err = profile
        .signal_for("volume_up")
        .expect_err("missing command");

    assert!(matches!(err, IrisError::CommandNotFound { .. }));
    assert!(err.to_string().contains("volume_up"));
}

#[test]
fn dry_run_transmitter_records_description_without_gpio() {
    let tx = DryRunTransmitter::new();

    tx.send(
        IrSignal::Nec {
            address: 0x00FF,
            command: 0xA25D,
        },
        3,
    )
    .expect("dry run send");

    let messages = tx.messages();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("NEC"));
    assert!(messages[0].contains("repeat=3"));
}

#[test]
fn mock_transmitter_records_signals() {
    let tx = MockTransmitter::new();

    tx.send(
        IrSignal::Raw {
            frequency: 38_000,
            pulses: vec![1, 2, 3],
        },
        2,
    )
    .expect("mock send");

    let sent = tx.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].repeat, 2);
    assert_eq!(
        sent[0].signal,
        IrSignal::Raw {
            frequency: 38_000,
            pulses: vec![1, 2, 3]
        }
    );
}
