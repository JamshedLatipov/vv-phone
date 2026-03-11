# Commercial Softphone in Rust

A high-performance, cross-platform SIP softphone implemented in Rust.

## Features (Implemented in Skeleton)

- SIP Stack (Request/Response parsing and building)
- UDP Transport
- Core Data Structures (Accounts, Call States)
- Configuration management (TOML)
- RTP Packetization foundation
- CLI Interface
- UI Skeleton (ready for egui)

## Architecture

- `src/sip/` — SIP stack.
- `src/media/` — RTP and media logic.
- `src/ui/` — GUI components.
- `src/config/` — Configuration management.
- `src/core/` — Common structures.
- `src/cli/` — CLI tool.

## Build and Run

1. Install Rust 1.90+
2. Build: `cargo build --release`
3. Run: `softphone --ui --account default`

Note: GUI and Audio features require system libraries (ALSA, Fontconfig).
