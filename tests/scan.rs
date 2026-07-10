use iris::errors::IrisError;
use iris::ir::{
    CapturedFrame, IrReceiver, IrSignal, MockReceiver, build_nec_pulses, build_nikai_pulses,
    decode_pulses, group_edge_timestamps,
};
use iris::profiles::Profile;
use iris::scan::{ScanInput, ScanKey, ScanSession, normalize_name, run_interactive_session};
use std::collections::VecDeque;
use std::time::Duration;
use tempfile::tempdir;

#[derive(Default)]
struct ScriptedInput {
    keys: VecDeque<ScanKey>,
}

impl ScriptedInput {
    fn new(keys: impl IntoIterator<Item = ScanKey>) -> Self {
        Self {
            keys: keys.into_iter().collect(),
        }
    }
}

impl ScanInput for ScriptedInput {
    fn poll_key(&mut self, _timeout: Duration) -> Result<Option<ScanKey>, IrisError> {
        Ok(self.keys.pop_front())
    }
}

fn characters(input: &str) -> Vec<ScanKey> {
    input.chars().map(ScanKey::Character).collect()
}

#[test]
fn groups_edges_without_storing_terminating_idle_gap() {
    let timestamps = [
        Duration::from_micros(0),
        Duration::from_micros(9_000),
        Duration::from_micros(13_500),
        Duration::from_micros(14_060),
        Duration::from_micros(14_620),
        Duration::from_micros(40_000),
    ];

    assert_eq!(
        group_edge_timestamps(&timestamps, Duration::from_millis(20)),
        vec![vec![9_000, 4_500, 560, 560]]
    );
}

#[test]
fn decodes_nec_nikai_and_keeps_unknown_as_raw() {
    let mut nec = build_nec_pulses(0x00ff, 0xa25d);
    nec[0] = 9_250;
    assert_eq!(
        decode_pulses(&nec),
        Some(IrSignal::Nec {
            address: 0x00ff,
            command: 0xa25d,
        })
    );
    assert_eq!(
        decode_pulses(&build_nikai_pulses(0x0f7f08, 24)),
        Some(IrSignal::Nikai {
            data: 0x0f7f08,
            bits: 24,
        })
    );
    let raw = CapturedFrame::from_pulses(vec![300, 700, 300], 38_000);
    assert_eq!(
        raw.signal,
        IrSignal::Raw {
            frequency: 38_000,
            pulses: vec![300, 700, 300],
        }
    );
}

#[test]
fn scan_session_creates_directories_logs_immediately_and_generates_profile() {
    let root = tempdir().expect("temp root");
    let output = root.path().join("nested").join("learned");
    let frame = CapturedFrame::from_pulses(build_nec_pulses(0x00ff, 0xa25d), 38_000);
    let mut session = ScanSession::new("Living Room TV", &output, 38_000).expect("session");

    assert_eq!(session.name(), "living_room_tv");
    assert!(session.log_path().exists());
    session.accept("Power Toggle", frame).expect("accept");
    let log = std::fs::read_to_string(session.log_path()).expect("log is appended");
    assert!(log.contains("label = \"Power Toggle\""));
    assert!(log.contains("command = power_toggle"));

    let profile_path = session.finish().expect("profile");
    let profile =
        Profile::from_toml_str(&std::fs::read_to_string(profile_path).expect("profile file"))
            .expect("generated profile parses");
    assert_eq!(profile.brand, "living_room_tv");
    assert_eq!(profile.model, "learned");
    assert_eq!(
        profile.signal_for("power_toggle").expect("command"),
        IrSignal::Nec {
            address: 0x00ff,
            command: 0xa25d,
        }
    );
}

#[test]
fn scan_session_rejects_existing_output_and_normalizes_names() {
    let root = tempdir().expect("temp root");
    std::fs::write(root.path().join("my_tv.toml"), "existing").expect("output");

    assert_eq!(normalize_name(" My TV! "), Some("my_tv".to_string()));
    assert_eq!(normalize_name("---"), None);
    assert!(matches!(
        ScanSession::new("My TV", root.path(), 38_000),
        Err(IrisError::ScanOutputExists { .. })
    ));
}

#[test]
fn interactive_scan_accepts_skips_and_finishes_with_escape() {
    let root = tempdir().expect("temp root");
    let accepted = CapturedFrame::from_pulses(build_nec_pulses(1, 2), 38_000);
    let skipped = CapturedFrame::from_pulses(vec![200, 300, 400], 38_000);
    let mut receiver = MockReceiver::new([Some(accepted), Some(skipped), None]);
    let mut keys = characters("Power Button");
    keys.push(ScanKey::Enter);
    keys.push(ScanKey::Escape);
    keys.push(ScanKey::Escape);
    let mut input = ScriptedInput::new(keys);
    let mut output = Vec::new();
    let mut session = ScanSession::new("Test TV", root.path(), 38_000).expect("session");

    let profile_path =
        run_interactive_session(&mut receiver, &mut input, &mut output, &mut session)
            .expect("interactive scan");

    assert_eq!(session.accepted_count(), 1);
    assert!(profile_path.exists());
    assert!(
        String::from_utf8(output)
            .expect("output")
            .contains("Skipped frame")
    );
}

#[test]
fn ctrl_c_finishes_and_saves_accepted_captures() {
    let root = tempdir().expect("temp root");
    let frame = CapturedFrame::from_pulses(vec![300, 600, 300], 38_000);
    let mut receiver = MockReceiver::new([Some(frame), None]);
    let mut keys = characters("input");
    keys.push(ScanKey::Enter);
    keys.push(ScanKey::CtrlC);
    let mut input = ScriptedInput::new(keys);
    let mut output = Vec::new();
    let mut session = ScanSession::new("Ctrl C", root.path(), 38_000).expect("session");

    let profile_path =
        run_interactive_session(&mut receiver, &mut input, &mut output, &mut session)
            .expect("ctrl-c finish");
    let profile = Profile::from_toml_str(&std::fs::read_to_string(profile_path).expect("profile"))
        .expect("parse");

    assert_eq!(session.accepted_count(), 1);
    assert!(matches!(
        profile.signal_for("input").expect("raw command"),
        IrSignal::Raw { .. }
    ));
}

#[test]
fn mock_receiver_is_injectable_for_scan_workflows() {
    let mut receiver = MockReceiver::new([None]);
    assert!(
        receiver
            .receive_frame(Duration::ZERO)
            .expect("mock receiver")
            .is_none()
    );
}
