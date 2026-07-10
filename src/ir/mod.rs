use crate::errors::IrisError;
use std::collections::VecDeque;
use std::fmt::Write as _;
#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
use std::fs::File;
#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
use std::io::Write as IoWrite;
#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
use std::os::fd::AsRawFd;
#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
use std::thread;
use std::time::Duration;
#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
use std::time::Instant;

/// A captured receiver frame. `pulses` always preserves the original mark/space
/// timings even when a known protocol can be recognized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFrame {
    pub pulses: Vec<u32>,
    pub signal: IrSignal,
}

impl CapturedFrame {
    pub fn from_pulses(pulses: Vec<u32>, carrier_frequency: u32) -> Self {
        let signal = decode_pulses(&pulses).unwrap_or(IrSignal::Raw {
            frequency: carrier_frequency,
            pulses: pulses.clone(),
        });
        Self { pulses, signal }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrSignal {
    Nec { address: u16, command: u16 },
    Nikai { data: u32, bits: u8 },
    Raw { frequency: u32, pulses: Vec<u32> },
}

pub trait IrTransmitter: Send + Sync {
    fn send(&self, signal: IrSignal, repeat: u32) -> Result<(), IrisError>;

    fn send_with_frequency(
        &self,
        signal: IrSignal,
        repeat: u32,
        carrier_frequency: u32,
    ) -> Result<(), IrisError> {
        let _ = carrier_frequency;
        self.send(signal, repeat)
    }
}

/// An interrupt-driven source of demodulated IR frames.
pub trait IrReceiver {
    /// Waits up to `timeout` for a complete frame. A complete frame is separated
    /// from the following activity by at least 20 ms of idle time.
    fn receive_frame(&mut self, timeout: Duration) -> Result<Option<CapturedFrame>, IrisError>;
}

#[derive(Debug, Default, Clone)]
pub struct MockReceiver {
    frames: Arc<Mutex<VecDeque<Option<CapturedFrame>>>>,
}

impl MockReceiver {
    pub fn new(frames: impl IntoIterator<Item = Option<CapturedFrame>>) -> Self {
        Self {
            frames: Arc::new(Mutex::new(frames.into_iter().collect())),
        }
    }

    pub fn push_frame(&self, frame: CapturedFrame) -> Result<(), IrisError> {
        let mut frames = self.frames.lock().map_err(|_| {
            IrisError::IoPlain(std::io::Error::other("mock receiver lock poisoned"))
        })?;
        frames.push_back(Some(frame));
        Ok(())
    }
}

impl IrReceiver for MockReceiver {
    fn receive_frame(&mut self, _timeout: Duration) -> Result<Option<CapturedFrame>, IrisError> {
        let mut frames = self.frames.lock().map_err(|_| {
            IrisError::IoPlain(std::io::Error::other("mock receiver lock poisoned"))
        })?;
        Ok(frames.pop_front().flatten())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentSignal {
    pub signal: IrSignal,
    pub repeat: u32,
}

#[derive(Debug, Default, Clone)]
pub struct MockTransmitter {
    sent: Arc<Mutex<Vec<SentSignal>>>,
}

impl MockTransmitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sent(&self) -> Vec<SentSignal> {
        match self.sent.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => Vec::new(),
        }
    }
}

impl IrTransmitter for MockTransmitter {
    fn send(&self, signal: IrSignal, repeat: u32) -> Result<(), IrisError> {
        let mut guard = self.sent.lock().map_err(|_| {
            IrisError::IoPlain(std::io::Error::other("mock transmitter lock poisoned"))
        })?;
        guard.push(SentSignal { signal, repeat });
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct DryRunTransmitter {
    messages: Arc<Mutex<Vec<String>>>,
}

impl DryRunTransmitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn messages(&self) -> Vec<String> {
        match self.messages.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => Vec::new(),
        }
    }
}

impl IrTransmitter for DryRunTransmitter {
    fn send(&self, signal: IrSignal, repeat: u32) -> Result<(), IrisError> {
        let message = describe_signal(&signal, repeat);
        println!("{message}");
        let mut guard = self.messages.lock().map_err(|_| {
            IrisError::IoPlain(std::io::Error::other("dry-run transmitter lock poisoned"))
        })?;
        guard.push(message);
        Ok(())
    }
}

pub fn describe_signal(signal: &IrSignal, repeat: u32) -> String {
    match signal {
        IrSignal::Nec { address, command } => {
            format!("Dry run: NEC address=0x{address:04X} command=0x{command:04X} repeat={repeat}")
        }
        IrSignal::Nikai { data, bits } => {
            format!("Dry run: NIKAI data=0x{data:06X} bits={bits} repeat={repeat}")
        }
        IrSignal::Raw { frequency, pulses } => {
            let mut preview = String::new();
            for (idx, pulse) in pulses.iter().take(8).enumerate() {
                if idx > 0 {
                    let _ = write!(preview, ",");
                }
                let _ = write!(preview, "{pulse}");
            }
            format!(
                "Dry run: RAW frequency={frequency} pulses=[{preview}] count={} repeat={repeat}",
                pulses.len()
            )
        }
    }
}

pub fn build_nikai_pulses(data: u32, bits: u8) -> Vec<u32> {
    let bit_count = bits.clamp(1, 32);
    let mut pulses = Vec::with_capacity(usize::from(bit_count) * 2 + 4);
    pulses.push(4000);
    pulses.push(4000);
    for bit in (0..bit_count).rev() {
        pulses.push(500);
        if (data >> bit) & 1 == 1 {
            pulses.push(1000);
        } else {
            pulses.push(2000);
        }
    }
    pulses.push(500);
    pulses.push(8500);
    pulses
}

pub fn build_nec_pulses(address: u16, command: u16) -> Vec<u32> {
    build_nec_raw32_pulses(u32::from(address) | (u32::from(command) << 16))
}

/// Builds a NEC frame from its 32 on-air bits. Bits are emitted least-significant
/// bit first, with alternating MARK and SPACE durations in microseconds.
pub fn build_nec_raw32_pulses(data: u32) -> Vec<u32> {
    let mut pulses = Vec::with_capacity(67);
    pulses.extend([9000, 4500]);
    for bit in 0..32 {
        pulses.push(562);
        pulses.push(if data & (1 << bit) == 0 { 562 } else { 1687 });
    }
    pulses.push(562);
    pulses
}

/// Groups monotonically increasing edge timestamps into mark/space pulse lists.
/// An idle gap starts a new frame and is intentionally not included in either
/// frame's stored timings.
pub fn group_edge_timestamps(timestamps: &[Duration], idle_gap: Duration) -> Vec<Vec<u32>> {
    let Some((&first, rest)) = timestamps.split_first() else {
        return Vec::new();
    };

    let mut frames = Vec::new();
    let mut previous = first;
    let mut pulses = Vec::new();
    for &timestamp in rest {
        let Some(delta) = timestamp.checked_sub(previous) else {
            previous = timestamp;
            continue;
        };
        previous = timestamp;
        if delta >= idle_gap {
            if !pulses.is_empty() {
                frames.push(std::mem::take(&mut pulses));
            }
            continue;
        }
        let micros = u32::try_from(delta.as_micros()).unwrap_or(u32::MAX);
        if micros > 0 {
            pulses.push(micros);
        }
    }
    if !pulses.is_empty() {
        frames.push(pulses);
    }
    frames
}

pub fn decode_pulses(pulses: &[u32]) -> Option<IrSignal> {
    decode_nec(pulses).or_else(|| decode_nikai(pulses))
}

fn decode_nec(pulses: &[u32]) -> Option<IrSignal> {
    if pulses.len() != 67 || !timing_matches(pulses[0], 9_000) || !timing_matches(pulses[1], 4_500)
    {
        return None;
    }

    let mut values = [0_u16; 2];
    for bit in 0..32 {
        let mark = pulses[2 + bit * 2];
        let space = pulses[3 + bit * 2];
        if !timing_matches(mark, 560) {
            return None;
        }
        let one = timing_matches(space, 1_690);
        let zero = timing_matches(space, 560);
        if !one && !zero {
            return None;
        }
        if one {
            values[bit / 16] |= 1 << (bit % 16);
        }
    }
    timing_matches(pulses[66], 560).then_some(IrSignal::Nec {
        address: values[0],
        command: values[1],
    })
}

fn decode_nikai(pulses: &[u32]) -> Option<IrSignal> {
    if pulses.len() < 6
        || !timing_matches(pulses[0], 4_000)
        || !timing_matches(pulses[1], 4_000)
        || !timing_matches(*pulses.last()?, 8_500)
        || !timing_matches(pulses[pulses.len() - 2], 500)
    {
        return None;
    }
    let bit_count = (pulses.len() - 4) / 2;
    if bit_count == 0 || bit_count > 32 || pulses.len() != bit_count * 2 + 4 {
        return None;
    }

    let mut data = 0_u32;
    for bit in 0..bit_count {
        let mark = pulses[2 + bit * 2];
        let space = pulses[3 + bit * 2];
        if !timing_matches(mark, 500) {
            return None;
        }
        let one = timing_matches(space, 1_000);
        let zero = timing_matches(space, 2_000);
        if !one && !zero {
            return None;
        }
        data = (data << 1) | u32::from(one);
    }
    Some(IrSignal::Nikai {
        data,
        bits: bit_count as u8,
    })
}

fn timing_matches(actual: u32, expected: u32) -> bool {
    let tolerance = (expected * 35 / 100).max(150);
    actual.abs_diff(expected) <= tolerance
}

#[derive(Debug, Clone)]
pub struct RppalTransmitter {
    #[allow(dead_code)]
    pin: u8,
    #[allow(dead_code)]
    carrier_frequency: u32,
}

impl RppalTransmitter {
    pub fn new(pin: u8, carrier_frequency: u32) -> Result<Self, IrisError> {
        #[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
        {
            let path = lirc_device_path();
            File::options()
                .write(true)
                .open(&path)
                .map_err(|_| IrisError::IrTransmitterUnavailable { path })?;
            Ok(Self {
                pin,
                carrier_frequency,
            })
        }
        #[cfg(not(all(feature = "rpi-gpio", target_os = "linux")))]
        {
            let _ = (pin, carrier_frequency);
            Err(IrisError::GpioUnavailable)
        }
    }

    /// Sends an unmodulated-duration MARK with the configured hardware carrier.
    /// This is intended for checking the carrier with a receiver or oscilloscope.
    pub fn send_carrier(&self, duration: Duration) -> Result<(), IrisError> {
        #[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
        {
            for chunk in lirc_duration_chunks(duration) {
                self.send_lirc_pulses(&[chunk], self.carrier_frequency)?;
            }
            Ok(())
        }
        #[cfg(not(all(feature = "rpi-gpio", target_os = "linux")))]
        {
            let _ = duration;
            Err(IrisError::GpioUnavailable)
        }
    }

    #[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
    fn send_lirc_pulses(&self, pulses: &[u32], carrier_frequency: u32) -> Result<(), IrisError> {
        let transmitted = lirc_pulses_to_write(pulses);
        validate_lirc_pulses(transmitted)?;
        let path = lirc_device_path();
        let file = File::options()
            .write(true)
            .open(&path)
            .map_err(|_| IrisError::IrTransmitterUnavailable { path: path.clone() })?;
        configure_lirc(&file, &path, carrier_frequency)?;
        write_lirc_pulses(&file, transmitted).map_err(IrisError::IoPlain)
    }
}

impl IrTransmitter for RppalTransmitter {
    fn send(&self, signal: IrSignal, repeat: u32) -> Result<(), IrisError> {
        self.send_with_frequency(signal, repeat, self.carrier_frequency)
    }

    fn send_with_frequency(
        &self,
        signal: IrSignal,
        repeat: u32,
        carrier_frequency: u32,
    ) -> Result<(), IrisError> {
        #[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
        {
            use fs2::FileExt;
            use std::fs::OpenOptions;

            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open("/tmp/iris-transmitter.lock")
                .map_err(IrisError::IoPlain)?;
            lock.lock_exclusive().map_err(IrisError::IoPlain)?;
            let (pulses, frequency) = match signal {
                IrSignal::Nec { address, command } => {
                    (build_nec_pulses(address, command), carrier_frequency)
                }
                IrSignal::Nikai { data, bits } => {
                    (build_nikai_pulses(data, bits), carrier_frequency)
                }
                IrSignal::Raw { frequency, pulses } => (pulses, frequency),
            };
            for idx in 0..repeat.max(1) {
                self.send_lirc_pulses(&pulses, frequency)?;
                if idx + 1 < repeat.max(1) {
                    thread::sleep(Duration::from_millis(40));
                }
            }
            let _ = lock.unlock();
            Ok(())
        }
        #[cfg(not(all(feature = "rpi-gpio", target_os = "linux")))]
        {
            let _ = (signal, repeat, carrier_frequency);
            Err(IrisError::GpioUnavailable)
        }
    }
}

#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
fn lirc_device_path() -> PathBuf {
    std::env::var_os("IRIS_LIRC_DEVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/lirc0"))
}

#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
fn configure_lirc(file: &File, path: &PathBuf, carrier_frequency: u32) -> Result<(), IrisError> {
    let mut features = 0_u32;
    ioctl_u32(file, LIRC_GET_FEATURES, &mut features).map_err(IrisError::IoPlain)?;
    if features & LIRC_CAN_SEND_PULSE == 0 {
        return Err(IrisError::IrTransmitterUnsupported { path: path.clone() });
    }

    let mut mode = LIRC_MODE_PULSE;
    ioctl_u32(file, LIRC_SET_SEND_MODE, &mut mode).map_err(IrisError::IoPlain)?;
    let mut frequency = carrier_frequency;
    ioctl_u32(file, LIRC_SET_SEND_CARRIER, &mut frequency).map_err(IrisError::IoPlain)?;
    let mut duty_cycle = 50_u32;
    ioctl_u32(file, LIRC_SET_SEND_DUTY_CYCLE, &mut duty_cycle).map_err(IrisError::IoPlain)
}

#[cfg(any(test, all(feature = "rpi-gpio", target_os = "linux")))]
fn lirc_pulses_to_write(pulses: &[u32]) -> &[u32] {
    let end = if pulses.len().is_multiple_of(2) {
        pulses.len().saturating_sub(1)
    } else {
        pulses.len()
    };
    &pulses[..end]
}

#[cfg(any(test, all(feature = "rpi-gpio", target_os = "linux")))]
fn validate_lirc_pulses(pulses: &[u32]) -> Result<(), IrisError> {
    if let Some(&duration_us) = pulses
        .iter()
        .find(|&&duration_us| duration_us > LIRC_MAX_PULSE_DURATION_US)
    {
        return Err(IrisError::IrPulseDurationTooLong { duration_us });
    }
    Ok(())
}

#[cfg(any(test, all(feature = "rpi-gpio", target_os = "linux")))]
fn lirc_duration_chunks(duration: Duration) -> Vec<u32> {
    let mut remaining = duration.as_micros();
    let mut chunks = Vec::new();
    while remaining > 0 {
        let chunk = remaining.min(u128::from(LIRC_MAX_PULSE_DURATION_US)) as u32;
        chunks.push(chunk);
        remaining -= u128::from(chunk);
    }
    chunks
}

#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
fn write_lirc_pulses(file: &File, pulses: &[u32]) -> std::io::Result<()> {
    let mut bytes = Vec::with_capacity(pulses.len() * std::mem::size_of::<u32>());
    for pulse in pulses {
        bytes.extend_from_slice(&pulse.to_ne_bytes());
    }
    let mut file = file;
    file.write_all(&bytes)
}

#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
fn ioctl_u32(file: &File, request: libc::c_ulong, value: &mut u32) -> std::io::Result<()> {
    // SAFETY: `file` owns a valid descriptor, the request expects a pointer to a
    // writable u32, and `value` remains alive for the duration of the syscall.
    let result = unsafe { libc::ioctl(file.as_raw_fd(), request, value) };
    if result == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
const LIRC_CAN_SEND_PULSE: u32 = 0x0000_0002;
#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
const LIRC_MODE_PULSE: u32 = 0x0000_0002;
#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
const LIRC_GET_FEATURES: libc::c_ulong = 0x8004_6900;
#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
const LIRC_SET_SEND_MODE: libc::c_ulong = 0x4004_6911;
#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
const LIRC_SET_SEND_CARRIER: libc::c_ulong = 0x4004_6913;
#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
const LIRC_SET_SEND_DUTY_CYCLE: libc::c_ulong = 0x4004_6915;

#[cfg(any(test, all(feature = "rpi-gpio", target_os = "linux")))]
const LIRC_MAX_PULSE_DURATION_US: u32 = 500_000;

#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
const FRAME_IDLE_GAP: Duration = Duration::from_millis(20);

pub struct RppalReceiver {
    #[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
    pin: u8,
    #[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
    carrier_frequency: u32,
    #[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
    input: rppal::gpio::InputPin,
    #[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
    last_edge: Option<Duration>,
    #[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
    pulses: Vec<u32>,
}

impl RppalReceiver {
    pub fn new(pin: u8, carrier_frequency: u32) -> Result<Self, IrisError> {
        #[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
        {
            use rppal::gpio::Trigger;

            let gpio =
                rppal::gpio::Gpio::new().map_err(|_| IrisError::GpioPermissionDenied { pin })?;
            let mut input = gpio
                .get(pin)
                .map_err(|_| IrisError::GpioUnavailable)?
                .into_input();
            input
                .set_interrupt(Trigger::Both, None)
                .map_err(|_| IrisError::GpioPermissionDenied { pin })?;
            Ok(Self {
                pin,
                carrier_frequency,
                input,
                last_edge: None,
                pulses: Vec::new(),
            })
        }
        #[cfg(not(all(feature = "rpi-gpio", target_os = "linux")))]
        {
            let _ = (pin, carrier_frequency);
            Err(IrisError::GpioUnavailable)
        }
    }

    #[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
    fn finish_pending(&mut self) -> Option<CapturedFrame> {
        self.last_edge = None;
        let pulses = std::mem::take(&mut self.pulses);
        (pulses.len() >= 2).then(|| CapturedFrame::from_pulses(pulses, self.carrier_frequency))
    }
}

impl IrReceiver for RppalReceiver {
    fn receive_frame(&mut self, timeout: Duration) -> Result<Option<CapturedFrame>, IrisError> {
        #[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
        {
            use rppal::gpio::Trigger;

            let deadline = Instant::now() + timeout;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Ok(None);
                }
                let wait = remaining.min(FRAME_IDLE_GAP);
                let event = self
                    .input
                    .poll_interrupt(false, Some(wait))
                    .map_err(|_| IrisError::GpioPermissionDenied { pin: self.pin })?;
                let Some(event) = event else {
                    if self.last_edge.is_some() && wait == FRAME_IDLE_GAP {
                        if let Some(frame) = self.finish_pending() {
                            return Ok(Some(frame));
                        }
                    }
                    continue;
                };

                let Some(last_edge) = self.last_edge else {
                    if event.trigger == Trigger::FallingEdge {
                        self.last_edge = Some(event.timestamp);
                    }
                    continue;
                };
                let Some(delta) = event.timestamp.checked_sub(last_edge) else {
                    self.last_edge = Some(event.timestamp);
                    continue;
                };
                if delta >= FRAME_IDLE_GAP {
                    let completed = self.finish_pending();
                    if event.trigger == Trigger::FallingEdge {
                        self.last_edge = Some(event.timestamp);
                    }
                    if completed.is_some() {
                        return Ok(completed);
                    }
                    continue;
                }

                let micros = u32::try_from(delta.as_micros()).unwrap_or(u32::MAX);
                if micros > 0 {
                    self.pulses.push(micros);
                }
                self.last_edge = Some(event.timestamp);
            }
        }
        #[cfg(not(all(feature = "rpi-gpio", target_os = "linux")))]
        {
            let _ = timeout;
            Err(IrisError::GpioUnavailable)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carrier_duration_is_split_into_driver_safe_chunks() {
        assert_eq!(
            lirc_duration_chunks(Duration::from_secs(2)),
            vec![500_000, 500_000, 500_000, 500_000]
        );
    }

    #[test]
    fn validates_only_pulses_written_to_lirc() {
        let pulses = [560, 850_000];
        assert_eq!(lirc_pulses_to_write(&pulses), &[560]);
        assert!(validate_lirc_pulses(lirc_pulses_to_write(&pulses)).is_ok());

        let error = validate_lirc_pulses(&[500_001]).expect_err("long pulse is rejected");
        assert!(matches!(
            error,
            IrisError::IrPulseDurationTooLong {
                duration_us: 500_001
            }
        ));
    }
}
