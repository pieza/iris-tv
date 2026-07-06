# IRIS

[![Release](https://img.shields.io/github/v/release/pieza/iris-tv?display_name=tag&sort=semver)](https://github.com/pieza/iris-tv/releases)
[![Build](https://img.shields.io/github/actions/workflow/status/pieza/iris-tv/build.yml?branch=main&label=build)](https://github.com/pieza/iris-tv/actions/workflows/build.yml)
[![Test](https://img.shields.io/github/actions/workflow/status/pieza/iris-tv/test.yml?branch=main&label=test)](https://github.com/pieza/iris-tv/actions/workflows/test.yml)

IRIS is a Rust CLI for turning an infrared-controlled TV into a command-driven TV through a Raspberry Pi 3. It loads editable TOML remote profiles and sends IR commands through a GPIO-connected infrared LED.

The first supported profile is Telstar, but profiles are data files under `profiles/tv/<brand>/<model>.toml`; the Rust source does not hardcode remote codes.

## Hardware

Recommended parts:

- Raspberry Pi 3 with Raspberry Pi OS.
- 940 nm infrared emitter LED.
- Breadboard and jumper wires.
- NPN transistor such as 2N2222 or PN2222.
- Base resistor around 1 kOhm.
- IR LED current-limiting resistor sized for your LED and supply voltage.
- Optional future receiver module, usually 38 kHz, for learning mode.

Basic transistor wiring:

```text
Raspberry Pi GPIO 18 --- 1 kOhm --- NPN base
Raspberry Pi GND ------------------- NPN emitter
5V or 3.3V --- resistor --- IR LED anode
IR LED cathode --------------------- NPN collector
```

Check your LED current rating and resistor values before powering the circuit.

## Install On Raspberry Pi

Install the latest Raspberry Pi ARM64 release package directly from GitHub:

```bash
curl -fsSL https://raw.githubusercontent.com/pieza/iris-tv/main/scripts/install.sh | bash
```

Install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/pieza/iris-tv/main/scripts/install.sh | bash -s -- V1.5.0
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
carrier_frequency = 38000
active_profile = "telstar/generic"
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
iris config set carrier_frequency 38000
iris config set default_repeat 1
iris config set device_name "Living Room IRIS"
```

Enable debug logs with:

```bash
RUST_LOG=debug iris send power
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

Try candidate power codes interactively:

```bash
iris scan power
```

IRIS sends one candidate at a time and waits for Enter before the next one. If a code works, note the candidate name printed in the terminal. Use `--repeat` to send each candidate multiple times:

```bash
iris scan power --repeat 5
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

## Profiles

Profiles live under:

```text
profiles/tv/<brand>/<model>.toml
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

To add a new TV, create a new TOML file such as:

```text
profiles/tv/telstar/ttc04.toml
```

Then load it:

```bash
iris start telstar --model TTC04
```

The included Telstar codes are templates. Telstar remotes may vary by model, so replace the values after capturing the real remote codes.

## Local Server

Run a foreground local API server:

```bash
iris serve telstar
```

Run it in the background:

```bash
iris daemon start telstar
iris daemon stop
```

Default bind address:

```text
127.0.0.1:8787
```

Endpoints:

```text
GET  /health
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

IRIS can be discovered by Home Assistant over Zeroconf/mDNS as a local TV bridge. Home Assistant can only show IRIS as a discovered device after the IRIS custom integration has been installed once.

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

`iris home-assistant setup` generates and persists a `device_id`, generates an `api_token` if one is missing, enables discovery, and configures the server to listen on `0.0.0.0:8787`. Protected endpoints still require the token.

Install `iris-home-assistant` through HACS as a custom repository of type `Integration`. After Home Assistant restarts, accept the discovered IRIS device and enter the API token printed by `iris home-assistant setup`.

## Development

Run checks:

```bash
cargo fmt --check
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test --no-default-features
cargo check --features rpi-gpio
```
