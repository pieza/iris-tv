use crate::errors::IrisError;
use std::collections::VecDeque;
use std::fmt::Write as _;
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
    let mut pulses = Vec::with_capacity(67);
    pulses.push(9000);
    pulses.push(4500);
    append_lsb_bits(&mut pulses, address);
    append_lsb_bits(&mut pulses, command);
    pulses.push(560);
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

fn append_lsb_bits(pulses: &mut Vec<u32>, value: u16) {
    for bit in 0..16 {
        pulses.push(560);
        if (value >> bit) & 1 == 1 {
            pulses.push(1690);
        } else {
            pulses.push(560);
        }
    }
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
            let _ = rppal::gpio::Gpio::new()
                .map_err(|_| IrisError::GpioPermissionDenied { pin })?
                .get(pin)
                .map_err(|_| IrisError::GpioUnavailable)?;
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
}

impl IrTransmitter for RppalTransmitter {
    fn send(&self, signal: IrSignal, repeat: u32) -> Result<(), IrisError> {
        #[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
        {
            let gpio = rppal::gpio::Gpio::new()
                .map_err(|_| IrisError::GpioPermissionDenied { pin: self.pin })?;
            let mut pin = gpio
                .get(self.pin)
                .map_err(|_| IrisError::GpioUnavailable)?
                .into_output_low();
            let pulses = match signal {
                IrSignal::Nec { address, command } => build_nec_pulses(address, command),
                IrSignal::Nikai { data, bits } => build_nikai_pulses(data, bits),
                IrSignal::Raw { pulses, .. } => pulses,
            };
            for idx in 0..repeat.max(1) {
                send_pulses(&mut pin, self.carrier_frequency, &pulses);
                if idx + 1 < repeat.max(1) {
                    thread::sleep(Duration::from_millis(40));
                }
            }
            Ok(())
        }
        #[cfg(not(all(feature = "rpi-gpio", target_os = "linux")))]
        {
            let _ = (signal, repeat);
            Err(IrisError::GpioUnavailable)
        }
    }
}

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

#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
fn send_pulses(pin: &mut rppal::gpio::OutputPin, carrier_frequency: u32, pulses: &[u32]) {
    let period = Duration::from_micros(1_000_000 / carrier_frequency.max(1) as u64);
    let half_period = period / 2;
    for (idx, pulse) in pulses.iter().enumerate() {
        let duration = Duration::from_micros(u64::from(*pulse));
        if idx % 2 == 0 {
            let start = std::time::Instant::now();
            while start.elapsed() < duration {
                pin.set_high();
                thread::sleep(half_period);
                pin.set_low();
                thread::sleep(half_period);
            }
        } else {
            pin.set_low();
            thread::sleep(duration);
        }
    }
    pin.set_low();
}
