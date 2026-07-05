use crate::errors::IrisError;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
use std::thread;
#[cfg(all(feature = "rpi-gpio", target_os = "linux"))]
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrSignal {
    Nec { address: u16, command: u16 },
    Raw { frequency: u32, pulses: Vec<u32> },
}

pub trait IrTransmitter: Send + Sync {
    fn send(&self, signal: IrSignal, repeat: u32) -> Result<(), IrisError>;
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

pub fn build_nec_pulses(address: u16, command: u16) -> Vec<u32> {
    let mut pulses = Vec::with_capacity(67);
    pulses.push(9000);
    pulses.push(4500);
    append_lsb_bits(&mut pulses, address);
    append_lsb_bits(&mut pulses, command);
    pulses.push(560);
    pulses
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
            use rppal::gpio::OutputPin;
            let gpio = rppal::gpio::Gpio::new()
                .map_err(|_| IrisError::GpioPermissionDenied { pin: self.pin })?;
            let mut pin = gpio
                .get(self.pin)
                .map_err(|_| IrisError::GpioUnavailable)?
                .into_output_low();
            let pulses = match signal {
                IrSignal::Nec { address, command } => build_nec_pulses(address, command),
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
