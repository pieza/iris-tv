# IRIS Architecture

IRIS is split into small modules so GPIO access, profile parsing, command handling, and server mode can evolve independently.

## Modules

- `cli`: Clap command definitions and command orchestration.
- `config`: User config loading and persistence for `~/.config/iris/config.toml`, including named device registrations.
- `profiles`: TOML profile parsing, brand/model resolution, and command lookup.
- `ir`: Protocol-neutral IR signal types plus dry-run, mock, and Raspberry Pi GPIO transmitters and receivers.
- `scan`: Interactive receiver-learning session, terminal input, session logging, and generated profile serialization.
- `server`: Multi-device local HTTP API and a bounded FIFO transmitter dispatcher for automation systems.
- `daemon`: PID-file based background server management.
- `errors`: Typed user-facing error messages.

## Data Flow

`iris start telstar` resolves the generic Telstar profile, validates its TOML, and saves `telstar/generic` as the active profile in the user config. `iris start telstar --model TTC04` resolves a model-specific `telstar/ttc04` profile when one exists.

`iris send power --device living_room_tv` loads the named device profile, resolves `power` to an `IrSignal`, and sends it through the shared transmitter. The server places all API sends on one FIFO dispatcher so waveforms never overlap.

## Extension Points

Add a new TV by adding a TOML profile under `profiles/tv/<brand>/<model>.toml`.

Add another IR protocol by extending `CommandDefinition`, converting it into a new `IrSignal` variant, and adding the encoder/transmitter support in `ir`.

Add another control surface by reusing `ProfileStore`, `ConfigStore`, and the `IrTransmitter` trait rather than talking to GPIO directly.
