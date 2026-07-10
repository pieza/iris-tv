# IRIS

[![Release](https://img.shields.io/github/v/release/pieza/iris-tv?display_name=tag&sort=semver)](https://github.com/pieza/iris-tv/releases)
[![Build](https://img.shields.io/github/actions/workflow/status/pieza/iris-tv/build.yml?branch=main&label=build)](https://github.com/pieza/iris-tv/actions/workflows/build.yml)
[![Test](https://img.shields.io/github/actions/workflow/status/pieza/iris-tv/test.yml?branch=main&label=test)](https://github.com/pieza/iris-tv/actions/workflows/test.yml)

IRIS is a Rust CLI for turning an infrared-controlled TV into a command-driven TV through a Raspberry Pi 3. It loads editable TOML remote profiles and sends IR commands through a GPIO-connected infrared LED.

The first supported profile is Telstar, but profiles are data files under `profiles/<device_type>/<brand>/<model>.toml`; the Rust source does not hardcode remote codes.

## Hardware

Recommended parts:

- Raspberry Pi 3 with Raspberry Pi OS.
- 940 nm infrared emitter LED.
- Breadboard and jumper wires.
- NPN transistor such as 2N2222 or PN2222.
- Base resistor around 1 kOhm.
- IR LED current-limiting resistor sized for your LED and supply voltage.
- A 38 kHz demodulated IR receiver module for learning mode.

Basic transistor wiring:

```text
Raspberry Pi GPIO 18 --- 1 kOhm --- NPN base
Raspberry Pi GND ------------------- NPN emitter
5V or 3.3V --- resistor --- IR LED anode
IR LED cathode --------------------- NPN collector
```

Check your LED current rating and resistor values before powering the circuit.

### Reliable IR transmission

IRIS sends through Linux's kernel IR transmitter interface instead of trying to
create a 38 kHz carrier with userspace sleeps. On a Raspberry Pi 3 using BCM
GPIO 18, enable the `gpio-ir-tx` overlay and reboot before using `iris send`:

```bash
echo 'dtoverlay=gpio-ir-tx,gpio_pin=18' | sudo tee -a /boot/firmware/config.txt
sudo reboot
```

The transmitter is exposed as `/dev/lirc0` by default. Set `IRIS_LIRC_DEVICE`
when your system assigned a different LIRC device. The kernel generates a 38
kHz, 50% duty-cycle carrier for MARK periods and keeps the LED off for SPACE
periods.

For `iris scan`, wire a standard active-low demodulated receiver as follows:

```text
Receiver OUT --- Raspberry Pi BCM GPIO 23 (or receiver_gpio_pin)
Receiver GND --- Raspberry Pi GND
Receiver VCC --- a supply appropriate for the module
```

The receiver output must be 3.3 V-compatible; do not connect a 5 V logic output directly to a Raspberry Pi GPIO. The emitter and receiver must share ground.

## Install On Raspberry Pi

Install the latest Raspberry Pi ARM64 release package directly from GitHub:

```bash
curl -fsSL https://raw.githubusercontent.com/pieza/iris-tv/main/scripts/install.sh | bash
```

Install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/pieza/iris-tv/main/scripts/install.sh | bash -s -- V1.6.4
```

The installer downloads the release asset, installs `iris` to `/usr/local/bin/iris`, and installs editable profiles to `/usr/local/share/iris/profiles`.

## Local Build

Install Rust on the Pi, clone or copy this project, then build with the GPIO backend:

```bash
cargo build --release --features rpi-gpio
sudo install -m 0755 target/release/iris /usr/local/bin/iris
```

For development on a non-Pi machine, build without GPIO and use dry-run:

```bash
cargo build
iris send power --dry-run
```

## Configuration

IRIS reads user config from:

```text
~/.config/iris/config.toml
```

Default values:

```toml
gpio_pin = 18
receiver_gpio_pin = 23
carrier_frequency = 38000
default_repeat = 1
log_level = "info"
server_host = "127.0.0.1"
server_port = 8787
api_token = ""
device_id = ""
device_name = "IRIS TV"
discovery_enabled = true
```

Set values from the CLI:

```bash
iris config set gpio_pin 18
iris config set receiver_gpio_pin 23
iris config set carrier_frequency 38000
iris config set default_repeat 1
iris config set device_name "Living Room IRIS"
```

Enable debug logs with:

```bash
RUST_LOG=debug iris send power
```

## Transmitter Diagnostics

On the Raspberry Pi, these commands help compare IRIS with the original remote:

```bash
# Send the 32 NEC on-air bits LSB-first with the required NEC timings.
iris debug send-nec-raw32 0x1AE5807F

# Emit only a 38 kHz carrier for two seconds.
iris debug carrier --duration 2

# Send a profile command and print what receiver_gpio_pin captures.
iris debug send-and-capture power
```

## Basic Use

Load a profile:

```bash
iris start telstar
```

Load a model-specific profile when you have captured codes for that model:

```bash
iris start telstar --model TTC04
```

For the Telstar TTS040490KK / TCL L40S4900I Linux Smart TV, use:

```bash
iris start telstar --model TTS040490KK
```

Send commands using the active profile:

```bash
iris send power
iris send volume_up
iris send volume_down
iris send mute
```

Repeat a command:

```bash
iris send volume_up --repeat 3
```

Preview without touching GPIO:

```bash
iris send power --dry-run
```

List and inspect profiles:

```bash
iris list brands
iris list models telstar
iris profile show telstar/generic
iris status
```

## Multiple Devices

IRIS can control multiple infrared devices through the same LED. Register each physical device with a stable name and profile, then run one server for all of them:

```bash
iris device add living-room-tv telstar/generic --name "Living Room TV"
iris device add bedroom-fan <fan-brand>/<fan-model> --name "Bedroom Fan"
iris device list
iris device use living-room-tv

iris send power
iris send power --device bedroom_fan
iris daemon start
```

This creates config entries like:

```toml
default_device = "living_room_tv"

[[devices]]
id = "living_room_tv"
name = "Living Room TV"
profile = "telstar/generic"

[[devices]]
id = "bedroom_fan"
name = "Bedroom Fan"
profile = "fan_brand/fan_model"
```

IRIS sends through one bounded FIFO hardware queue, so waveforms never overlap. If the queue is full, the API returns `429 Too Many Requests` rather than sending colliding pulses. Direct CLI sends also use the same operating-system lock as the daemon.

`iris serve` and `iris daemon start` now serve all registered devices. The older `iris serve telstar` and `iris daemon start telstar` forms remain available and update the compatible `default` device.

Existing single-profile configurations are migrated automatically to a `default` device. `iris start telstar` continues to select that device for existing scripts.

## Profiles

Profiles live under:

```text
profiles/<device_type>/<brand>/<model>.toml
```

Example:

```toml
brand = "telstar"
model = "generic"
device_type = "tv"
carrier_frequency = 38000
protocol = "nec"

[commands]
power = { type = "nec", address = "0x00FF", command = "0xA25D" }
volume_up = { type = "nec", address = "0x00FF", command = "0x629D" }
raw_demo = { type = "raw", frequency = 38000, pulses = [9000, 4500, 560, 560] }
```

To add a TV or fan, create its TOML file in the matching device-type directory:

```text
profiles/tv/telstar/ttc04.toml
profiles/fan/fan/generic.toml
```

Then load it:

```bash
iris start telstar --model TTC04
```

The included Telstar codes are templates. Telstar remotes may vary by model, so replace the values after capturing the real remote codes.

## Learn A Remote

With a 38 kHz active-low receiver connected, start a learning session on the Raspberry Pi:

```bash
iris scan --name "Living Room TV" --path ./learned-remotes
```

If `--name` is omitted, IRIS prompts before opening the receiver. If `--path` is omitted, it writes to the current directory and creates missing directories. Names are normalized to snake case, so this example creates `living_room_tv.txt` and `living_room_tv.toml`.

The session shows `press Esc to finish`. Aim the remote at the receiver and press one button at a time. For every capture IRIS shows a recognized NEC/Nikai result when possible, along with the raw mark/space durations. Enter a command name to accept it, or press Esc at the command prompt to skip that frame. IRIS discards repeat frames queued by that same button press before listening again. Press Esc while waiting for the next button (or Ctrl+C) to finish and write the TOML profile.

Accepted captures are appended immediately to the readable `.txt` session log using the entered label. On finish, the `.toml` file is generated as a usable `brand = "living_room_tv"`, `model = "learned"` profile; recognized codes use NEC or Nikai commands and all other captures use raw timings. IRIS refuses to start if either output file already exists, so it never overwrites a prior learning session.

Use `--device-type fan` when learning a fan profile. Fan profiles can declare Home Assistant controls without hardcoding them in IRIS:

```toml
device_type = "fan"

[home_assistant.fan]
power_on = "power_on"
power_off = "power_off"
oscillate = "oscillate"

[home_assistant.fan.presets]
low = "speed_low"
medium = "speed_medium"
high = "speed_high"
```

## Local Server

Run a foreground local API server:

```bash
iris serve
```

Run it in the background:

```bash
iris daemon start
iris daemon stop
```

Default bind address:

```text
127.0.0.1:8787
```

Endpoints:

```text
GET  /health
GET  /devices
GET  /devices/{device_id}
POST /devices/{device_id}/send/{command}

# compatibility aliases for the default device
GET  /profile
POST /send/power
POST /send/volume_up
POST /send/volume_down
POST /send/mute
POST /send/input
POST /send/{command}
```

The server listens only on localhost by default. If you change `server_host` to a LAN address such as `0.0.0.0`, configure an API token first:

```bash
iris config set api_token "replace-with-a-long-random-value"
```

Then call protected endpoints with:

```bash
curl -X POST \
  -H "Authorization: Bearer replace-with-a-long-random-value" \
  http://127.0.0.1:8787/send/power
```

## Home Assistant

IRIS can be discovered by Home Assistant over Zeroconf/mDNS as a local IR hub. Home Assistant can only show IRIS as a discovered device after the IRIS custom integration has been installed once.

The Home Assistant integration lives in its own repository:

```text
https://github.com/pieza/iris-home-assistant
```

Recommended IRIS setup on the Raspberry Pi:

```bash
iris start telstar
iris home-assistant setup
iris daemon start telstar
```

`iris home-assistant setup` generates and persists a bridge ID, generates an `api_token` if one is missing, enables discovery, and configures the server to listen on `0.0.0.0:8787`. The Home Assistant integration configures that bridge once and creates a separate Home Assistant device for every registered IRIS TV or fan. Protected endpoints still require the token.

Install `iris-home-assistant` through HACS as a custom repository of type `Integration`. After Home Assistant restarts, accept the discovered IRIS device and enter the API token printed by `iris home-assistant setup`.

## Development

Run checks:

```bash
cargo fmt --check
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test --no-default-features
cargo check --features rpi-gpio
```
