# Packet Proof

A desktop app for Windows and macOS that shows where your connection goes bad: your own network, your ISP, or further out. It works like WinMTR with extras, and it can trace the exact server a running game or app is talking to.

Everything stays on your machine. There are no accounts, no cloud sync and no telemetry.

## What it does

- **Live hop table.** Continuous ICMP probing with best, average and worst latency and loss per hop.
- **Zone tags.** Each hop is marked as home network, ISP gateway or beyond ISP, so you can see whose problem it is.
- **Game and app targets.** Pick a running process and Packet Proof lists the servers it is connected to.
- **Timed sessions.** Run for 15, 30 or 60 minutes, or a custom length. The probe rate backs off when the connection is already struggling.
- **Background baseline.** Probes your gateway, 1.1.1.1 and 8.8.8.8 every 60 seconds and keeps the history in a local SQLite database.
- **Charts and PDF reports.** Trend charts in the app, plus session and trend reports you can hand to your ISP.
- **System tray.** Start and stop the baseline, start a session, and check status.

## Limits

- It needs administrator rights, because raw ICMP probing does. The app asks for them at launch and exits if you decline.
- UDP sockets that never call `connect()` have no remote address in the OS connection tables, so some games show "no active connections detected".
- Zone tags come from private-IP ranges only. There is no IP-ownership database.
- There is no speed test and no packet capture.

## Build and run

You need [Rust](https://rustup.rs) and the Tauri CLI:

```bash
cargo install tauri-cli --version "^2"
```

Run in development:

```bash
cargo tauri dev
```

Build an installer:

```bash
cargo tauri build
```

Run the tests (no administrator rights needed):

```bash
cd src-tauri && cargo test
```

## Layout

- `src-tauri/` holds the Rust backend: probing (`mtr/`), process and connection lookup (`procnet/`), storage (`db.rs`), PDF reports (`report.rs`), tray and elevation.
- `src/` holds the frontend in plain HTML, CSS and JavaScript, with no bundler.
- `docs/superpowers/` holds the design spec and the build plan.

Built with Tauri 2, [trippy-core](https://crates.io/crates/trippy-core), rusqlite, genpdf and plotters.
