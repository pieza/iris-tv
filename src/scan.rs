use crate::errors::IrisError;
use crate::ir::{CapturedFrame, IrReceiver, IrSignal};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanKey {
    Escape,
    CtrlC,
    Enter,
    Backspace,
    Character(char),
}

pub trait ScanInput {
    fn poll_key(&mut self, timeout: Duration) -> Result<Option<ScanKey>, IrisError>;
}

/// Raw-terminal keyboard input used by the real scan command. Its `Drop`
/// implementation restores the terminal on success, errors, and Ctrl+C.
pub struct TerminalInput {
    raw_mode_enabled: bool,
}

impl TerminalInput {
    pub fn new() -> Result<Self, IrisError> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self {
            raw_mode_enabled: true,
        })
    }
}

impl Drop for TerminalInput {
    fn drop(&mut self) {
        if self.raw_mode_enabled {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
}

impl ScanInput for TerminalInput {
    fn poll_key(&mut self, timeout: Duration) -> Result<Option<ScanKey>, IrisError> {
        if !event::poll(timeout)? {
            return Ok(None);
        }
        let Event::Key(key) = event::read()? else {
            return Ok(None);
        };
        if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
            return Ok(None);
        }
        let scan_key = match key.code {
            KeyCode::Esc => ScanKey::Escape,
            KeyCode::Enter => ScanKey::Enter,
            KeyCode::Backspace => ScanKey::Backspace,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => ScanKey::CtrlC,
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                ScanKey::Character(ch)
            }
            _ => return Ok(None),
        };
        Ok(Some(scan_key))
    }
}

#[derive(Debug)]
pub struct ScanSession {
    name: String,
    device_type: String,
    carrier_frequency: u32,
    log_path: PathBuf,
    profile_path: PathBuf,
    log: File,
    entries: BTreeMap<String, AcceptedCapture>,
}

#[derive(Debug, Clone)]
struct AcceptedCapture {
    frame: CapturedFrame,
}

impl ScanSession {
    pub fn new(
        requested_name: &str,
        output_directory: impl AsRef<Path>,
        carrier_frequency: u32,
    ) -> Result<Self, IrisError> {
        let name = normalize_name(requested_name).ok_or(IrisError::InvalidScanName)?;
        let output_directory = output_directory.as_ref();
        std::fs::create_dir_all(output_directory)
            .map_err(|source| IrisError::io(output_directory, source))?;
        let log_path = output_directory.join(format!("{name}.txt"));
        let profile_path = output_directory.join(format!("{name}.toml"));
        if log_path.exists() {
            return Err(IrisError::ScanOutputExists { path: log_path });
        }
        if profile_path.exists() {
            return Err(IrisError::ScanOutputExists { path: profile_path });
        }
        let log = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&log_path)
            .map_err(|source| IrisError::io(&log_path, source))?;
        Ok(Self {
            name,
            device_type: "tv".to_string(),
            carrier_frequency,
            log_path,
            profile_path,
            log,
            entries: BTreeMap::new(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_device_type(&mut self, device_type: &str) -> Result<(), IrisError> {
        match device_type {
            "tv" | "fan" => {
                self.device_type = device_type.to_string();
                Ok(())
            }
            _ => Err(IrisError::InvalidConfigKey {
                key: "device_type".to_string(),
            }),
        }
    }

    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    pub fn profile_path(&self) -> &Path {
        &self.profile_path
    }

    pub fn accept(
        &mut self,
        entered_label: &str,
        frame: CapturedFrame,
    ) -> Result<String, IrisError> {
        let command = normalize_name(entered_label).ok_or(IrisError::InvalidScanName)?;
        let entered_label = entered_label.trim().to_string();
        writeln!(self.log, "label = {entered_label:?}")?;
        writeln!(self.log, "command = {command}")?;
        writeln!(self.log, "decoded = {}", capture_description(&frame.signal))?;
        writeln!(self.log, "pulses = {:?}", frame.pulses)?;
        writeln!(self.log)?;
        self.log.flush()?;
        self.entries
            .insert(command.clone(), AcceptedCapture { frame });
        Ok(command)
    }

    pub fn finish(&mut self) -> Result<PathBuf, IrisError> {
        self.log.flush()?;
        if self.profile_path.exists() {
            return Err(IrisError::ScanOutputExists {
                path: self.profile_path.clone(),
            });
        }
        let profile = LearnedProfile {
            brand: self.name.clone(),
            model: "learned".to_string(),
            device_type: self.device_type.clone(),
            carrier_frequency: self.carrier_frequency,
            commands: self
                .entries
                .iter()
                .map(|(command, entry)| {
                    (command.clone(), LearnedCommand::from(&entry.frame.signal))
                })
                .collect(),
        };
        let serialized = toml::to_string_pretty(&profile)?;
        let mut profile_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.profile_path)
            .map_err(|source| IrisError::io(&self.profile_path, source))?;
        profile_file.write_all(serialized.as_bytes())?;
        profile_file.flush()?;
        Ok(self.profile_path.clone())
    }

    pub fn accepted_count(&self) -> usize {
        self.entries.len()
    }
}

pub fn normalize_name(input: &str) -> Option<String> {
    let normalized = input
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    (!normalized.is_empty()).then_some(normalized)
}

pub fn prompt_session_name() -> Result<String, IrisError> {
    let mut stdout = std::io::stdout().lock();
    write!(stdout, "Profile name: ")?;
    stdout.flush()?;
    let mut input = String::new();
    std::io::stdin().lock().read_line(&mut input)?;
    normalize_name(&input)
        .ok_or(IrisError::InvalidScanName)
        .map(|_| input.trim().to_string())
}

pub fn run_interactive_session<R: IrReceiver, I: ScanInput, W: Write>(
    receiver: &mut R,
    input: &mut I,
    output: &mut W,
    session: &mut ScanSession,
) -> Result<PathBuf, IrisError> {
    writeln!(output, "Listening for IR frames — press Esc to finish.")?;
    output.flush()?;
    loop {
        if let Some(frame) = receiver.receive_frame(Duration::from_millis(50))? {
            writeln!(output, "\nCaptured: {}", capture_description(&frame.signal))?;
            writeln!(output, "Raw pulses: {:?}", frame.pulses)?;
            match prompt_command_name(input, output)? {
                PromptResult::Accept(label) => {
                    let command = session.accept(&label, frame)?;
                    writeln!(
                        output,
                        "Saved as `{command}`. Listening for the next frame."
                    )?;
                    output.flush()?;
                }
                PromptResult::Skip => {
                    writeln!(output, "Skipped frame. Listening for the next frame.")?;
                    output.flush()?;
                }
                PromptResult::Finish => return session.finish(),
            }
            continue;
        }

        if let Some(ScanKey::Escape | ScanKey::CtrlC) = input.poll_key(Duration::from_millis(1))? {
            return session.finish();
        }
    }
}

enum PromptResult {
    Accept(String),
    Skip,
    Finish,
}

fn prompt_command_name<I: ScanInput, W: Write>(
    input: &mut I,
    output: &mut W,
) -> Result<PromptResult, IrisError> {
    write!(output, "Command name (Enter to save, Esc to skip): ")?;
    output.flush()?;
    let mut label = String::new();
    loop {
        let Some(key) = input.poll_key(Duration::from_millis(50))? else {
            continue;
        };
        match key {
            ScanKey::Escape => {
                writeln!(output)?;
                return Ok(PromptResult::Skip);
            }
            ScanKey::CtrlC => {
                writeln!(output)?;
                return Ok(PromptResult::Finish);
            }
            ScanKey::Enter if normalize_name(&label).is_some() => {
                writeln!(output)?;
                return Ok(PromptResult::Accept(label));
            }
            ScanKey::Backspace if label.pop().is_some() => {
                write!(output, "\u{8} \u{8}")?;
                output.flush()?;
            }
            ScanKey::Backspace => {}
            ScanKey::Character(ch) => {
                label.push(ch);
                write!(output, "{ch}")?;
                output.flush()?;
            }
            _ => {}
        }
    }
}

fn capture_description(signal: &IrSignal) -> String {
    match signal {
        IrSignal::Nec { address, command } => {
            format!("NEC address=0x{address:04X} command=0x{command:04X}")
        }
        IrSignal::Nikai { data, bits } => format!("NIKAI data=0x{data:06X} bits={bits}"),
        IrSignal::Raw { frequency, .. } => format!("RAW frequency={frequency}"),
    }
}

#[derive(Serialize)]
struct LearnedProfile {
    brand: String,
    model: String,
    device_type: String,
    carrier_frequency: u32,
    commands: BTreeMap<String, LearnedCommand>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum LearnedCommand {
    Nec { address: String, command: String },
    Nikai { data: String, bits: u8 },
    Raw { frequency: u32, pulses: Vec<u32> },
}

impl From<&IrSignal> for LearnedCommand {
    fn from(signal: &IrSignal) -> Self {
        match signal {
            IrSignal::Nec { address, command } => Self::Nec {
                address: format!("0x{address:04X}"),
                command: format!("0x{command:04X}"),
            },
            IrSignal::Nikai { data, bits } => Self::Nikai {
                data: format!("0x{data:06X}"),
                bits: *bits,
            },
            IrSignal::Raw { frequency, pulses } => Self::Raw {
                frequency: *frequency,
                pulses: pulses.clone(),
            },
        }
    }
}
