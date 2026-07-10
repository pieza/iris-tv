use iris::config::{AppConfig, ConfigStore, resolve_profile_root};
use iris::discovery;
use iris::errors::IrisError;
use iris::ir::{
    DryRunTransmitter, IrSignal, IrTransmitter, MockTransmitter, build_nec_pulses,
    build_nec_raw32_pulses, build_nikai_pulses,
};
use iris::profiles::{Profile, ProfileId, ProfileStore};
use tempfile::tempdir;

const TELSTAR_PROFILE: &str = r#"
brand = "telstar"
model = "generic"
device_type = "tv"
carrier_frequency = 38000
protocol = "nec"

[commands]
power = { type = "nec", address = "0x00FF", command = "0xA25D" }
raw_demo = { type = "raw", frequency = 38000, pulses = [9000, 4500, 560] }
home = { type = "nikai", data = "0x0F7F08" }
"#;

#[test]
fn parses_nec_and_raw_profile_commands() {
    let profile = Profile::from_toml_str(TELSTAR_PROFILE).expect("profile parses");

    assert_eq!(profile.brand, "telstar");
    assert_eq!(profile.model, "generic");
    assert_eq!(profile.commands.len(), 3);
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
    assert_eq!(
        profile.signal_for("home").expect("home signal"),
        IrSignal::Nikai {
            data: 0x0F7F08,
            bits: 24
        }
    );
}

#[test]
fn resolves_brand_model_to_profile_id_and_file_path() {
    let id = ProfileId::from_brand_model("Telstar", None);

    assert_eq!(id.brand, "telstar");
    assert_eq!(id.model, "generic");
    assert_eq!(id.key(), "telstar/generic");

    let model_id = ProfileId::from_brand_model("Telstar", Some("TTC04"));
    assert_eq!(model_id.brand, "telstar");
    assert_eq!(model_id.model, "ttc04");
    assert_eq!(model_id.key(), "telstar/ttc04");
}

#[test]
fn profile_store_lists_brands_and_models() {
    let root = tempdir().expect("temp root");
    let profile_path = root.path().join("tv").join("telstar");
    std::fs::create_dir_all(&profile_path).expect("profile dir");
    std::fs::write(profile_path.join("generic.toml"), TELSTAR_PROFILE).expect("profile file");
    let fan_path = root.path().join("fan").join("fan");
    std::fs::create_dir_all(&fan_path).expect("fan profile dir");
    std::fs::write(
        fan_path.join("generic.toml"),
        r#"
brand = "fan"
model = "generic"
device_type = "fan"

[commands]
power = { type = "nec", address = "0xBF40", command = "0xED12" }
"#,
    )
    .expect("fan profile file");

    let store = ProfileStore::new(root.path());

    assert_eq!(store.list_brands().expect("brands"), vec!["fan", "telstar"]);
    assert_eq!(
        store.list_models("telstar").expect("models"),
        vec!["generic".to_string()]
    );
    assert_eq!(
        store.load_brand_model("telstar", None).expect("load").model,
        "generic"
    );
    assert_eq!(
        store.list_models("fan").expect("fan models"),
        vec!["generic".to_string()]
    );

    std::fs::create_dir_all(root.path().join("tv").join("empty")).expect("empty profile dir");
    assert_eq!(
        store.list_models("empty").expect("empty models"),
        Vec::<String>::new()
    );
}

#[test]
fn profile_store_loads_fan_profiles_from_fan_directory() {
    let root = tempdir().expect("temp root");
    let profile_path = root.path().join("fan").join("generic_fan");
    std::fs::create_dir_all(&profile_path).expect("profile dir");
    std::fs::write(
        profile_path.join("generic.toml"),
        r#"
brand = "generic_fan"
model = "generic"
device_type = "fan"

[commands]
power = { type = "nec", address = "0xBF40", command = "0xED12" }
"#,
    )
    .expect("profile file");

    let store = ProfileStore::new(root.path());
    let profile = store
        .load_brand_model("generic_fan", None)
        .expect("fan profile");

    assert_eq!(profile.device_type, "fan");
}

#[test]
fn bundled_generic_fan_profile_uses_captured_nec_commands() {
    let store =
        ProfileStore::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles"));
    let profile = store
        .load_brand_model("fan", None)
        .expect("bundled fan profile");

    assert_eq!(profile.device_type, "fan");
    assert_eq!(
        profile.signal_for("power").expect("power signal"),
        IrSignal::Nec {
            address: 0x7F80,
            command: 0xE51A,
        }
    );
    assert_eq!(
        profile.signal_for("rotate").expect("rotate signal"),
        IrSignal::Nec {
            address: 0x7F80,
            command: 0xFC03,
        }
    );
    assert_eq!(
        profile.signal_for("speed").expect("speed signal"),
        IrSignal::Nec {
            address: 0x7F80,
            command: 0xFE01,
        }
    );
}

#[test]
fn bundled_telstar_tts040490kk_profile_uses_captured_nec_power() {
    let store =
        ProfileStore::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles"));
    let profile = store
        .load_brand_model("telstar", Some("tts040490kk"))
        .expect("bundled Telstar profile");

    assert_eq!(profile.commands.len(), 3);
    assert_eq!(
        profile.signal_for("power").expect("power signal"),
        IrSignal::Nec {
            address: 0xBF40,
            command: 0xED12,
        }
    );
    assert_eq!(
        profile
            .signal_for("volume_down")
            .expect("volume down signal"),
        IrSignal::Nec {
            address: 0xBF40,
            command: 0xE11E,
        }
    );
    assert_eq!(
        profile.signal_for("volume_up").expect("volume up signal"),
        IrSignal::Nec {
            address: 0xBF40,
            command: 0xE51A,
        }
    );
}

#[test]
fn config_store_persists_active_profile() {
    let root = tempdir().expect("temp root");
    let store = ConfigStore::new(root.path());
    let config = AppConfig {
        active_profile: Some("telstar/generic".to_string()),
        gpio_pin: 23,
        ..AppConfig::default()
    };

    store.save(&config).expect("save config");
    let loaded = store.load().expect("load config");

    assert_eq!(loaded.active_profile.as_deref(), Some("telstar/generic"));
    assert_eq!(loaded.gpio_pin, 23);
    assert_eq!(loaded.receiver_gpio_pin, 23);
}

#[test]
fn legacy_active_profile_migrates_to_default_registered_device() {
    let root = tempdir().expect("temp root");
    let store = ConfigStore::new(root.path());
    std::fs::create_dir_all(root.path()).expect("config dir");
    std::fs::write(
        root.path().join("config.toml"),
        "active_profile = \"telstar/generic\"\n",
    )
    .expect("legacy config");

    let config = store.load().expect("load migrated config");
    assert_eq!(config.default_device.as_deref(), Some("default"));
    assert_eq!(
        config.device(None).expect("default device").profile,
        "telstar/generic"
    );
}

#[test]
fn configured_devices_can_be_added_selected_and_removed() {
    let mut config = AppConfig::default();
    config
        .add_device(
            "Living Room TV",
            "Living Room TV".to_string(),
            "telstar/generic".to_string(),
        )
        .expect("add tv");
    config
        .add_device("fan", "Bedroom Fan".to_string(), "generic/fan".to_string())
        .expect("add fan");
    config.use_device("fan").expect("use fan");

    assert_eq!(config.default_device.as_deref(), Some("fan"));
    assert_eq!(config.devices.len(), 2);
    config.remove_device("fan").expect("remove fan");
    assert_eq!(config.default_device.as_deref(), Some("living_room_tv"));
}

#[test]
fn config_store_generates_and_persists_device_id_once() {
    let root = tempdir().expect("temp root");
    let store = ConfigStore::new(root.path());

    let first = store.ensure_device_id().expect("first id");
    let second = store.ensure_device_id().expect("second id");
    let loaded = store.load().expect("load config");

    assert!(!first.is_empty());
    assert_eq!(first, second);
    assert_eq!(loaded.device_id.as_deref(), Some(first.as_str()));
}

#[test]
fn home_assistant_setup_prepares_network_config_and_token() {
    let root = tempdir().expect("temp root");
    let store = ConfigStore::new(root.path());

    let prepared = store.prepare_home_assistant().expect("setup config");
    let loaded = store.load().expect("load config");

    assert_eq!(prepared.server_host, "0.0.0.0");
    assert_eq!(prepared.server_port, 8787);
    assert!(prepared.discovery_enabled);
    assert!(
        prepared
            .device_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
    );
    assert!(
        prepared
            .api_token
            .as_deref()
            .is_some_and(|token| token.len() >= 32)
    );
    assert_eq!(loaded, prepared);
}

#[test]
fn mdns_service_info_contains_home_assistant_discovery_metadata() {
    let config = AppConfig {
        device_id: Some("iris-test-device".to_string()),
        device_name: "Living Room IRIS".to_string(),
        api_token: Some("secret".to_string()),
        ..AppConfig::default()
    };

    let service = discovery::build_service_info(&config).expect("service info");

    assert_eq!(service.get_type(), "_iris-tv._tcp.local.");
    assert_eq!(service.get_port(), config.server_port);
    assert_eq!(service.get_property_val_str("id"), Some("iris-test-device"));
    assert_eq!(
        service.get_property_val_str("name"),
        Some("Living Room IRIS")
    );
    assert_eq!(
        service.get_property_val_str("bridge_id"),
        Some("iris-test-device")
    );
    assert_eq!(service.get_property_val_str("api_version"), Some("2"));
    assert_eq!(service.get_property_val_str("auth_required"), Some("true"));
}

#[test]
fn profile_root_prefers_local_profiles_then_installed_profiles() {
    let root = tempdir().expect("temp root");
    let installed = root.path().join("installed").join("profiles");

    assert_eq!(resolve_profile_root(root.path(), &installed), installed);

    std::fs::create_dir_all(root.path().join("profiles")).expect("local profiles dir");

    assert_eq!(
        resolve_profile_root(root.path(), &installed),
        root.path().join("profiles")
    );
}

#[test]
fn nec_signal_builder_uses_expected_header_and_bit_timings() {
    let pulses = build_nec_pulses(0x0001, 0x0000);

    assert_eq!(&pulses[0..2], &[9000, 4500]);
    assert_eq!(&pulses[2..4], &[562, 1687]);
    assert_eq!(&pulses[4..6], &[562, 562]);
    assert_eq!(pulses.last(), Some(&562));
    assert_eq!(pulses.len(), 67);
}

#[test]
fn raw32_nec_builder_emits_all_bits_lsb_first() {
    let pulses = build_nec_raw32_pulses(0x1AE5_807F);

    assert_eq!(&pulses[0..2], &[9000, 4500]);
    // 0x7F is sent first and its least-significant bit is one.
    assert_eq!(&pulses[2..4], &[562, 1687]);
    // The next bit is also one.
    assert_eq!(&pulses[4..6], &[562, 1687]);
    // The eighth bit of 0x7F is zero.
    assert_eq!(&pulses[16..18], &[562, 562]);
    assert_eq!(pulses.last(), Some(&562));
    assert_eq!(pulses.len(), 67);
}

#[test]
fn nikai_signal_builder_uses_expected_header_and_msb_bit_timings() {
    let pulses = build_nikai_pulses(0b101, 3);

    assert_eq!(&pulses[0..2], &[4000, 4000]);
    assert_eq!(&pulses[2..4], &[500, 1000]);
    assert_eq!(&pulses[4..6], &[500, 2000]);
    assert_eq!(&pulses[6..8], &[500, 1000]);
    assert_eq!(&pulses[8..10], &[500, 8500]);
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
