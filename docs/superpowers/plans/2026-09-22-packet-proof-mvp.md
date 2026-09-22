# Packet Proof MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Packet Proof v2.0 MVP — a Tauri desktop app with a native continuous MTR engine, hop zone tagging, a running-process/target picker, SQLite-backed baseline + timed-session history, dashboard charts, PDF export, and a system tray.

**Architecture:** Rust backend (`src-tauri/`) owns all probing, storage, OS integration, and report generation; a plain HTML/CSS/JS frontend (no framework, no bundler — `withGlobalTauri`) renders the dashboard and talks to the backend via Tauri commands and events. The probing engine is isolated behind a `ProbeEngine` trait so the one genuinely risky external dependency (`trippy-core`, an explicitly unstable crate) touches exactly one file; everything else — aggregation, zone classification, adaptive rate, storage, OS process/connection lookup — is pure or thin-OS-wrapper Rust that's unit-tested without needing raw sockets or elevation.

**Tech Stack:** Tauri 2, trippy-core, rusqlite (bundled sqlite), sysinfo, `windows` crate (IP Helper API), genpdf + plotters, plain JS frontend.

**Spec:** [docs/superpowers/specs/2026-09-22-packet-proof-design.md](../specs/2026-09-22-packet-proof-design.md)

## Global Constraints

- Windows and macOS only (per spec §3).
- No account system, no cloud sync, no telemetry — all data stays local (spec §3).
- No throughput/speed testing in this pass (spec §3).
- No Npcap/packet-capture UDP sniffing — OS connection-table APIs only; the
  UDP-blind-spot limitation must be surfaced in the UI, never silently
  dropped (spec §2 item 6, §3).
- No bundled ASN/IP-ownership database — zone classification is the
  RFC1918 private-IP heuristic only (spec §3, §4.5).
- The app always runs elevated (relaunch-at-launch model) — there is no
  privileged-helper/IPC split in this pass (spec §4.1).
- Baseline logger targets are fixed for this pass: LAN gateway + 1.1.1.1 +
  8.8.8.8 (spec §6.3) — not user-configurable.
- Session probe rate is adaptive starting at 1/sec (spec §4.3), not a
  flat fixed rate.

---

## Task 1: Scaffold the Tauri project

**Files:**
- Create: `src-tauri/Cargo.toml`
- Create: `src-tauri/tauri.conf.json`
- Create: `src-tauri/src/main.rs`
- Create: `src/index.html`
- Create: `src/dashboard.js`
- Create: `.gitignore`

**Interfaces:**
- Produces: a running Tauri window loading `src/index.html`, with
  `window.__TAURI__` available globally to later frontend tasks (no npm
  bundler in this project — plain `<script>` tags throughout).

- [ ] **Step 1: Install the Tauri CLI and scaffold**

```bash
cargo install tauri-cli --version "^2"
cargo tauri init --ci \
  --app-name "packet-proof" \
  --window-title "Packet Proof" \
  --dev-path "../src" \
  --dist-dir "../src" \
  --before-dev-command "" \
  --before-build-command ""
```

This creates `src-tauri/` with a default `Cargo.toml`, `tauri.conf.json`,
and `src/main.rs`.

- [ ] **Step 2: Enable the global Tauri API for plain-script frontend use**

Edit `src-tauri/tauri.conf.json`, add under the top-level `"app"` key:

```json
{
  "app": {
    "withGlobalTauri": true,
    "windows": [
      {
        "title": "Packet Proof",
        "width": 1100,
        "height": 750
      }
    ]
  }
}
```

- [ ] **Step 3: Add core dependencies**

```bash
cd src-tauri
cargo add serde --features derive
cargo add serde_json
cargo add uuid --features v4
cargo add rusqlite --features bundled
cargo add sysinfo
cargo add genpdf
cargo add plotters --no-default-features --features bitmap_backend
```

Windows-only and macOS-only deps are added per-target so `cargo build`
stays clean on the other platform:

```bash
cargo add windows --target "cfg(target_os = \"windows\")" --features \
  Win32_NetworkManagement_IpHelper,Win32_NetworkManagement_Ndis,Win32_Networking_WinSock,Win32_Foundation,Win32_Security,Win32_System_Threading,Win32_UI_Shell,Win32_UI_WindowsAndMessaging
cargo add libc --target "cfg(target_os = \"macos\")"
```

`trippy-core` is added in Task 4, next to the code that uses it, since its
exact current version needs to be resolved at that point.

- [ ] **Step 4: Write a minimal `main.rs` that just opens the window**

```rust
// src-tauri/src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running Packet Proof");
}
```

- [ ] **Step 5: Write a placeholder frontend**

```html
<!-- src/index.html -->
<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <title>Packet Proof</title>
  </head>
  <body>
    <h1>Packet Proof</h1>
    <div id="app">Loading…</div>
    <script src="dashboard.js"></script>
  </body>
</html>
```

```js
// src/dashboard.js
document.getElementById("app").textContent = "Packet Proof scaffold OK";
```

- [ ] **Step 6: Verify it builds and runs**

Run: `cd src-tauri && cargo tauri dev`
Expected: a window opens titled "Packet Proof" showing "Packet Proof
scaffold OK". Close it, then `Ctrl+C` the dev process.

- [ ] **Step 7: `.gitignore` and commit**

```
# .gitignore
/src-tauri/target/
/node_modules/
*.sqlite
*.sqlite-wal
*.sqlite-shm
```

```bash
git add .gitignore src src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tauri.conf.json src-tauri/src/main.rs
git commit -m "Scaffold Tauri project with plain-JS frontend"
```

---

## Task 2: Elevation check and relaunch

**Files:**
- Create: `src-tauri/src/elevation.rs`
- Modify: `src-tauri/src/main.rs`
- Test: `src-tauri/src/elevation.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `elevation::ElevationState` (enum: `Elevated`, `NotElevated`),
  `elevation::decide_relaunch(state: &ElevationState) -> bool`,
  `elevation::check_elevation() -> ElevationState`,
  `elevation::relaunch_elevated() -> !`. `main.rs` calls `check_elevation`
  before building the Tauri app; if `decide_relaunch` is true, it calls
  `relaunch_elevated()` (which never returns — it exits the process) instead
  of starting the UI.

- [ ] **Step 1: Write the failing test for the pure decision logic**

```rust
// src-tauri/src/elevation.rs
#[derive(Debug, PartialEq, Eq)]
pub enum ElevationState {
    Elevated,
    NotElevated,
}

pub fn decide_relaunch(state: &ElevationState) -> bool {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relaunches_when_not_elevated() {
        assert!(decide_relaunch(&ElevationState::NotElevated));
    }

    #[test]
    fn does_not_relaunch_when_already_elevated() {
        assert!(!decide_relaunch(&ElevationState::Elevated));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test elevation::tests -- --nocapture`
Expected: FAIL (`not implemented`) on both tests.

- [ ] **Step 3: Implement the pure logic and the platform-specific checks**

```rust
// src-tauri/src/elevation.rs (replace unimplemented! body, append below)
pub fn decide_relaunch(state: &ElevationState) -> bool {
    matches!(state, ElevationState::NotElevated)
}

#[cfg(target_os = "windows")]
pub fn check_elevation() -> ElevationState {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return ElevationState::NotElevated;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        if ok && elevation.TokenIsElevated != 0 {
            ElevationState::Elevated
        } else {
            ElevationState::NotElevated
        }
    }
}

#[cfg(target_os = "macos")]
pub fn check_elevation() -> ElevationState {
    if unsafe { libc::geteuid() } == 0 {
        ElevationState::Elevated
    } else {
        ElevationState::NotElevated
    }
}

#[cfg(target_os = "windows")]
pub fn relaunch_elevated() -> ! {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let exe = std::env::current_exe().expect("current_exe");
    let exe_wide: Vec<u16> = OsStr::new(&exe).encode_wide().chain(std::iter::once(0)).collect();
    let verb: Vec<u16> = OsStr::new("runas").encode_wide().chain(std::iter::once(0)).collect();

    unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(exe_wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
    std::process::exit(0)
}

#[cfg(target_os = "macos")]
pub fn relaunch_elevated() -> ! {
    let exe = std::env::current_exe().expect("current_exe");
    // `do shell script` is TWO parsers deep: AppleScript parses the `-e`
    // script text (so the path must be escaped as an AppleScript string
    // literal), and then hands the resulting string to /bin/sh (so the
    // path also needs shell quoting, or a space/`;`/`$` etc. either breaks
    // a perfectly normal install path like "/Applications/Packet
    // Proof.app/..." or — worse, since this runs `with administrator
    // privileges` — lets a crafted path run arbitrary shell commands as
    // root). `quoted form of` is AppleScript's built-in for producing a
    // shell-safe single-quoted token at runtime, which handles the shell
    // layer; the string literal escaping below still handles the
    // AppleScript layer.
    let escaped_path = exe
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let script = format!(
        "do shell script quoted form of \"{escaped_path}\" with administrator privileges"
    );
    let _ = std::process::Command::new("osascript").arg("-e").arg(script).status();
    std::process::exit(0)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test elevation::tests -- --nocapture`
Expected: PASS (2/2).

Note: `check_elevation`/`relaunch_elevated` are OS syscalls and are not
unit-tested — they're covered by the manual-QA elevation-flow check on
both OSes (spec §6), not by this test suite.

- [ ] **Step 5: Wire into `main.rs`**

```rust
// src-tauri/src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod elevation;

fn main() {
    if elevation::decide_relaunch(&elevation::check_elevation()) {
        elevation::relaunch_elevated();
    }

    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running Packet Proof");
}
```

- [ ] **Step 6: Manual verification**

Run: `cd src-tauri && cargo tauri dev`
Expected (Windows): a UAC prompt appears before the window shows; accepting
it opens the window normally. Expected (macOS): an administrator
password/Touch ID prompt appears first, then the window opens. Record which
OS you tested — the other OS's path must be verified separately before this
task is considered done end-to-end.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/elevation.rs src-tauri/src/main.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "Add elevation check and relaunch-elevated flow"
```

---

## Task 3: Core engine types, HopAggregator, and ProbeEngine trait

**Files:**
- Create: `src-tauri/src/mtr/mod.rs`
- Create: `src-tauri/src/mtr/engine.rs`
- Modify: `src-tauri/src/main.rs`
- Test: `src-tauri/src/mtr/engine.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `mtr::engine::RawHop { hop_index: u8, addr: Option<std::net::IpAddr>, rtt_ms: Option<f64> }`
  - `mtr::engine::RoundResult { hops: Vec<RawHop> }`
  - `mtr::engine::EngineError` (enum, `Other(String)` variant is enough for
    now)
  - `mtr::engine::EngineHandle { pub stop: Box<dyn FnOnce() + Send> }`
  - `mtr::engine::ProbeEngine` trait:
    `fn spawn(&self, target: std::net::IpAddr, round_interval: std::time::Duration, on_round: Box<dyn Fn(RoundResult) + Send + Sync>) -> Result<EngineHandle, EngineError>`
  - `mtr::engine::HopAggregator` with `new() -> Self`,
    `record_round(&mut self, hops: &[RawHop])`,
    `snapshot(&self) -> Vec<HopSnapshot>` where
    `HopSnapshot { hop_index: u8, addr: Option<IpAddr>, rtt_best_ms: Option<f64>, rtt_avg_ms: Option<f64>, rtt_worst_ms: Option<f64>, loss_pct: f64 }`
    (zone is added on top of this in Task 7 — `HopSnapshot` intentionally
    has no zone field yet).
- Consumes: nothing (this is the foundation layer).

- [ ] **Step 1: Write the failing tests for `HopAggregator`**

```rust
// src-tauri/src/mtr/engine.rs
use std::net::IpAddr;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RawHop {
    pub hop_index: u8,
    pub addr: Option<IpAddr>,
    pub rtt_ms: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct RoundResult {
    pub hops: Vec<RawHop>,
}

#[derive(Debug)]
pub enum EngineError {
    Other(String),
}

pub struct EngineHandle {
    pub stop: Box<dyn FnOnce() + Send>,
}

pub trait ProbeEngine: Send + Sync {
    fn spawn(
        &self,
        target: IpAddr,
        round_interval: Duration,
        on_round: Box<dyn Fn(RoundResult) + Send + Sync>,
    ) -> Result<EngineHandle, EngineError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct HopSnapshot {
    pub hop_index: u8,
    pub addr: Option<IpAddr>,
    pub rtt_best_ms: Option<f64>,
    pub rtt_avg_ms: Option<f64>,
    pub rtt_worst_ms: Option<f64>,
    pub loss_pct: f64,
}

pub struct HopAggregator;

impl HopAggregator {
    pub fn new() -> Self {
        unimplemented!()
    }

    pub fn record_round(&mut self, hops: &[RawHop]) {
        unimplemented!()
    }

    pub fn snapshot(&self) -> Vec<HopSnapshot> {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hop(idx: u8, addr: Option<&str>, rtt: Option<f64>) -> RawHop {
        RawHop { hop_index: idx, addr: addr.map(|a| a.parse().unwrap()), rtt_ms: rtt }
    }

    #[test]
    fn empty_aggregator_has_no_hops() {
        let agg = HopAggregator::new();
        assert_eq!(agg.snapshot(), vec![]);
    }

    #[test]
    fn single_round_single_hop() {
        let mut agg = HopAggregator::new();
        agg.record_round(&[hop(1, Some("192.168.1.1"), Some(5.0))]);
        let snap = agg.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].hop_index, 1);
        assert_eq!(snap[0].addr, Some("192.168.1.1".parse().unwrap()));
        assert_eq!(snap[0].rtt_best_ms, Some(5.0));
        assert_eq!(snap[0].rtt_avg_ms, Some(5.0));
        assert_eq!(snap[0].rtt_worst_ms, Some(5.0));
        assert_eq!(snap[0].loss_pct, 0.0);
    }

    #[test]
    fn best_avg_worst_across_rounds() {
        let mut agg = HopAggregator::new();
        agg.record_round(&[hop(1, Some("1.1.1.1"), Some(10.0))]);
        agg.record_round(&[hop(1, Some("1.1.1.1"), Some(30.0))]);
        agg.record_round(&[hop(1, Some("1.1.1.1"), Some(20.0))]);
        let snap = agg.snapshot();
        assert_eq!(snap[0].rtt_best_ms, Some(10.0));
        assert_eq!(snap[0].rtt_worst_ms, Some(30.0));
        assert_eq!(snap[0].rtt_avg_ms, Some(20.0));
    }

    #[test]
    fn loss_pct_counts_non_responding_rounds() {
        let mut agg = HopAggregator::new();
        agg.record_round(&[hop(1, Some("1.1.1.1"), Some(10.0))]);
        agg.record_round(&[hop(1, None, None)]);
        agg.record_round(&[hop(1, Some("1.1.1.1"), Some(10.0))]);
        agg.record_round(&[hop(1, None, None)]);
        let snap = agg.snapshot();
        assert_eq!(snap[0].loss_pct, 50.0);
    }

    #[test]
    fn last_known_addr_is_kept_after_a_loss() {
        let mut agg = HopAggregator::new();
        agg.record_round(&[hop(1, Some("1.1.1.1"), Some(10.0))]);
        agg.record_round(&[hop(1, None, None)]);
        let snap = agg.snapshot();
        assert_eq!(snap[0].addr, Some("1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn snapshot_is_sorted_by_hop_index() {
        let mut agg = HopAggregator::new();
        agg.record_round(&[hop(2, Some("2.2.2.2"), Some(2.0)), hop(1, Some("1.1.1.1"), Some(1.0))]);
        let snap = agg.snapshot();
        assert_eq!(snap.iter().map(|h| h.hop_index).collect::<Vec<_>>(), vec![1, 2]);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test mtr::engine::tests -- --nocapture`
Expected: FAIL (`not implemented`) on every test.

- [ ] **Step 3: Implement `HopAggregator`**

```rust
// src-tauri/src/mtr/engine.rs (replace the HopAggregator impl block)
use std::collections::BTreeMap;

struct HopAccumulator {
    sent: u32,
    received: u32,
    rtt_sum_ms: f64,
    rtt_best_ms: Option<f64>,
    rtt_worst_ms: Option<f64>,
    last_addr: Option<IpAddr>,
}

impl HopAccumulator {
    fn new() -> Self {
        Self { sent: 0, received: 0, rtt_sum_ms: 0.0, rtt_best_ms: None, rtt_worst_ms: None, last_addr: None }
    }

    fn record(&mut self, hop: &RawHop) {
        self.sent += 1;
        if let Some(addr) = hop.addr {
            self.last_addr = Some(addr);
        }
        if let Some(rtt) = hop.rtt_ms {
            self.received += 1;
            self.rtt_sum_ms += rtt;
            self.rtt_best_ms = Some(self.rtt_best_ms.map_or(rtt, |b: f64| b.min(rtt)));
            self.rtt_worst_ms = Some(self.rtt_worst_ms.map_or(rtt, |w: f64| w.max(rtt)));
        }
    }

    fn snapshot(&self, hop_index: u8) -> HopSnapshot {
        let loss_pct = if self.sent == 0 {
            0.0
        } else {
            100.0 * (1.0 - self.received as f64 / self.sent as f64)
        };
        HopSnapshot {
            hop_index,
            addr: self.last_addr,
            rtt_best_ms: self.rtt_best_ms,
            rtt_avg_ms: if self.received == 0 { None } else { Some(self.rtt_sum_ms / self.received as f64) },
            rtt_worst_ms: self.rtt_worst_ms,
            loss_pct,
        }
    }
}

pub struct HopAggregator {
    hops: BTreeMap<u8, HopAccumulator>,
}

impl HopAggregator {
    pub fn new() -> Self {
        Self { hops: BTreeMap::new() }
    }

    pub fn record_round(&mut self, hops: &[RawHop]) {
        for hop in hops {
            self.hops.entry(hop.hop_index).or_insert_with(HopAccumulator::new).record(hop);
        }
    }

    pub fn snapshot(&self) -> Vec<HopSnapshot> {
        self.hops.iter().map(|(idx, acc)| acc.snapshot(*idx)).collect()
    }
}
```

Delete the old `unimplemented!()` stubs for `new`/`record_round`/`snapshot`
that were left above the test module (the replacement above supersedes
them) — the file should end up with exactly one `impl HopAggregator` block.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test mtr::engine::tests -- --nocapture`
Expected: PASS (6/6).

- [ ] **Step 5: Create the `mtr` module and register it**

```rust
// src-tauri/src/mtr/mod.rs
pub mod engine;
```

```rust
// src-tauri/src/main.rs — add near the top, after `mod elevation;`
mod mtr;
```

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/mtr src-tauri/src/main.rs
git commit -m "Add ProbeEngine trait and HopAggregator with unit tests"
```

---

## Task 4: TrippyEngine — real ICMP probing via trippy-core

**Files:**
- Create: `src-tauri/src/mtr/trippy_engine.rs`
- Modify: `src-tauri/src/mtr/mod.rs`

**Interfaces:**
- Consumes: `mtr::engine::{RawHop, RoundResult, EngineError, EngineHandle, ProbeEngine}` (Task 3).
- Produces: `mtr::trippy_engine::TrippyEngine` (a zero-field struct implementing `ProbeEngine`).

This is the one task in the plan that touches `trippy-core` directly — an
explicitly unstable crate (its own docs say the public API is "highly
likely to change"). Everything else in the app depends only on the
`ProbeEngine` trait from Task 3, so a breaking change in `trippy-core` is
contained to this one file. The field names and method signatures below
were confirmed against the current `trippy-core` docs (`Builder`, `Tracer`,
`Round`, `ProbeStatus::Complete(ProbeComplete)`, `TimeToLive`) at spec-writing
time; if `cargo build` reports a mismatch, re-check
`https://docs.rs/trippy-core/latest/trippy_core/` for the current shape of
whichever type the compiler flags — the crate's own instability warning
means this is expected maintenance, not a sign the design is wrong.

- [ ] **Step 1: Add the dependency**

```bash
cd src-tauri
cargo add trippy-core
```

- [ ] **Step 2: Implement the adapter**

```rust
// src-tauri/src/mtr/trippy_engine.rs
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use trippy_core::{Builder, PrivilegeMode, Protocol, ProbeStatus, Round};

use crate::mtr::engine::{EngineError, EngineHandle, ProbeEngine, RawHop, RoundResult};

pub struct TrippyEngine;

impl ProbeEngine for TrippyEngine {
    fn spawn(
        &self,
        target: IpAddr,
        round_interval: Duration,
        on_round: Box<dyn Fn(RoundResult) + Send + Sync>,
    ) -> Result<EngineHandle, EngineError> {
        let tracer = Builder::new(target)
            .protocol(Protocol::Icmp)
            .privilege_mode(PrivilegeMode::Privileged)
            .max_rounds(None)
            .min_round_duration(round_interval)
            .max_round_duration(round_interval)
            .build()
            .map_err(|e| EngineError::Other(format!("trippy build failed: {e}")))?;

        let (_tracer, join_handle) = tracer
            .spawn_with(move |round: &Round| {
                on_round(RoundResult { hops: round_to_raw_hops(round) });
            })
            .map_err(|e| EngineError::Other(format!("trippy spawn failed: {e}")))?;

        Ok(EngineHandle {
            stop: Box::new(move || {
                // Dropping the join handle does not stop the background thread;
                // trippy's Tracer has no public stop() at time of writing, so we
                // detach it. The thread exits when the process does (this app
                // never needs to stop a single tracer without stopping the app,
                // except when SessionRunner (Task 9) restarts a session at a new
                // rate — see that task for how it works around this).
                drop(join_handle);
            }),
        })
    }
}

fn round_to_raw_hops(round: &Round) -> Vec<RawHop> {
    let max_ttl = round.largest_ttl.0;
    let mut completed: HashMap<u8, RawHop> = HashMap::new();

    for probe in round.probes {
        if let ProbeStatus::Complete(c) = probe {
            let ttl = c.ttl.0;
            let rtt_ms = c
                .received
                .duration_since(c.sent)
                .unwrap_or_default()
                .as_secs_f64()
                * 1000.0;
            completed.insert(ttl, RawHop { hop_index: ttl, addr: Some(c.host), rtt_ms: Some(rtt_ms) });
        }
    }

    (1..=max_ttl)
        .map(|ttl| completed.remove(&ttl).unwrap_or(RawHop { hop_index: ttl, addr: None, rtt_ms: None }))
        .collect()
}
```

No unit tests here — this file only maps external types to our own, and
doing so correctly requires a real raw socket and a real network path,
which is exactly the manual-QA territory the spec calls out (§6). The
`round_to_raw_hops` mapping logic is deliberately kept simple enough to
review by inspection.

- [ ] **Step 3: Register the module**

```rust
// src-tauri/src/mtr/mod.rs
pub mod engine;
pub mod trippy_engine;
```

- [ ] **Step 4: Manual verification against a real target**

Add a temporary block to `main()` (removed again in Task 5, which replaces
it with the real command-based wiring):

```rust
// src-tauri/src/main.rs — temporary, for this step only
{
    use crate::mtr::engine::ProbeEngine;
    let engine = crate::mtr::trippy_engine::TrippyEngine;
    let target: std::net::IpAddr = "1.1.1.1".parse().unwrap();
    let _handle = engine
        .spawn(target, std::time::Duration::from_secs(1), Box::new(|round| {
            println!("{round:?}");
        }))
        .expect("spawn failed");
    std::thread::sleep(std::time::Duration::from_secs(10));
}
```

Run: `cd src-tauri && cargo run` (elevated, per Task 2's flow)
Expected: hop data prints to the console for a real trace to 1.1.1.1 within
the 10-second window — at minimum the final hop shows `addr: Some(1.1.1.1)`
with a plausible RTT. Remove this temporary block before committing; Task
5 replaces it with the real wiring.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/mtr/trippy_engine.rs src-tauri/src/mtr/mod.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "Add TrippyEngine: real ICMP probing via trippy-core"
```

---

## Task 5: End-to-end milestone — live hop table for a hard-coded target

**Files:**
- Create: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src/index.html`
- Create: `src/hop-table.js`

**Interfaces:**
- Consumes: `mtr::engine::{ProbeEngine, HopAggregator, HopSnapshot}` (Task
  3), `mtr::trippy_engine::TrippyEngine` (Task 4).
- Produces: Tauri command `start_demo_session() -> Result<(), String>`
  (temporary — replaced by the real `start_session`/`stop_session` in Task
  9) and a `hop-update` event carrying `Vec<HopSnapshot>` as JSON. This
  task exists purely to prove the whole pipe — raw sockets through to
  pixels on screen — works before building aggregation polish, zones, or
  the UI around it.

This is the milestone the source spec called out explicitly: get the
engine working end-to-end against one hard-coded target before adding
anything else.

- [ ] **Step 1: Make `HopSnapshot` serializable**

```rust
// src-tauri/src/mtr/engine.rs — add derive to the existing struct
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HopSnapshot {
```

- [ ] **Step 2: Write the command**

```rust
// src-tauri/src/commands.rs
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter, State};

use crate::mtr::engine::{HopAggregator, ProbeEngine};
use crate::mtr::trippy_engine::TrippyEngine;

pub struct DemoState(pub Mutex<Option<HopAggregator>>);

#[tauri::command]
pub fn start_demo_session(app: AppHandle, state: State<DemoState>) -> Result<(), String> {
    let target = "1.1.1.1".parse().unwrap();
    let aggregator = std::sync::Arc::new(Mutex::new(HopAggregator::new()));
    let agg_for_callback = aggregator.clone();

    let engine = TrippyEngine;
    let app_for_callback = app.clone();
    let _handle = engine
        .spawn(
            target,
            Duration::from_secs(1),
            Box::new(move |round| {
                let mut agg = agg_for_callback.lock().unwrap();
                agg.record_round(&round.hops);
                let snapshot = agg.snapshot();
                let _ = app_for_callback.emit("hop-update", snapshot);
            }),
        )
        .map_err(|e| format!("{e:?}"))?;

    *state.0.lock().unwrap() = Some(HopAggregator::new());
    // The handle is intentionally leaked for this temporary demo command;
    // Task 9's SessionRunner owns proper lifecycle management (start/stop).
    std::mem::forget(_handle);
    Ok(())
}
```

- [ ] **Step 3: Register the command and state in `main.rs`**

```rust
// src-tauri/src/main.rs
mod commands;

fn main() {
    if elevation::decide_relaunch(&elevation::check_elevation()) {
        elevation::relaunch_elevated();
    }

    tauri::Builder::default()
        .manage(commands::DemoState(std::sync::Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![commands::start_demo_session])
        .run(tauri::generate_context!())
        .expect("error while running Packet Proof");
}
```

(Remove the temporary manual-verification block from Task 4 Step 4 if it's
still present.)

- [ ] **Step 4: Write the frontend hop table**

```js
// src/hop-table.js
function renderHopTable(hops) {
  const rows = hops
    .map(
      (h) => `
    <tr>
      <td>${h.hop_index}</td>
      <td>${h.addr ?? "—"}</td>
      <td>${h.rtt_best_ms?.toFixed(1) ?? "—"}</td>
      <td>${h.rtt_avg_ms?.toFixed(1) ?? "—"}</td>
      <td>${h.rtt_worst_ms?.toFixed(1) ?? "—"}</td>
      <td>${h.loss_pct.toFixed(0)}%</td>
    </tr>`
    )
    .join("");

  document.getElementById("hop-table-body").innerHTML = rows;
}

window.__TAURI__.event.listen("hop-update", (event) => {
  renderHopTable(event.payload);
});

window.__TAURI__.core.invoke("start_demo_session").catch((err) => {
  console.error("start_demo_session failed:", err);
});
```

- [ ] **Step 5: Wire it into `index.html`**

```html
<!-- src/index.html -->
<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <title>Packet Proof</title>
  </head>
  <body>
    <h1>Packet Proof</h1>
    <table>
      <thead>
        <tr><th>Hop</th><th>IP</th><th>Best</th><th>Avg</th><th>Worst</th><th>Loss</th></tr>
      </thead>
      <tbody id="hop-table-body"></tbody>
    </table>
    <script src="hop-table.js"></script>
  </body>
</html>
```

- [ ] **Step 6: Manual verification**

Run: `cd src-tauri && cargo tauri dev`
Expected: after elevating, the window shows a live-updating table tracing
the path to 1.1.1.1, with best/avg/worst RTT and loss % per hop refreshing
roughly once a second.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs src-tauri/src/mtr/engine.rs src/index.html src/hop-table.js
git commit -m "End-to-end milestone: live hop table against a hard-coded target"
```

---

## Task 6: Zone classification

**Files:**
- Create: `src-tauri/src/mtr/zone.rs`
- Modify: `src-tauri/src/mtr/mod.rs`
- Test: `src-tauri/src/mtr/zone.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `mtr::zone::Zone` (enum: `Home`, `IspGateway`, `BeyondIsp`,
  derives `Debug, Clone, Copy, PartialEq, serde::Serialize`),
  `mtr::zone::is_private(ip: std::net::IpAddr) -> bool`,
  `mtr::zone::classify_hops(addrs: &[Option<std::net::IpAddr>]) -> Vec<Zone>`
  (same length as input; index-for-index correspondence),
  `Zone::as_db_str(&self) -> &'static str` (returns `"home"` /
  `"isp_gateway"` / `"beyond_isp"` — the exact strings the SQLite `zone`
  column stores; any later code that needs a `Zone` as a DB string MUST use
  this rather than `format!("{:?}", zone)`, since `Debug`'s output has no
  underscore between words).

- [ ] **Step 1: Write the failing tests**

```rust
// src-tauri/src/mtr/zone.rs
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    Home,
    IspGateway,
    BeyondIsp,
}

pub fn is_private(ip: IpAddr) -> bool {
    unimplemented!()
}

pub fn classify_hops(addrs: &[Option<IpAddr>]) -> Vec<Zone> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn rfc1918_ranges_are_private() {
        assert!(is_private(ip("10.0.0.1")));
        assert!(is_private(ip("172.16.5.1")));
        assert!(is_private(ip("172.31.255.254")));
        assert!(is_private(ip("192.168.1.1")));
    }

    #[test]
    fn public_ips_are_not_private() {
        assert!(!is_private(ip("1.1.1.1")));
        assert!(!is_private(ip("172.32.0.1"))); // just outside 172.16.0.0/12
        assert!(!is_private(ip("8.8.8.8")));
    }

    #[test]
    fn classic_home_then_gateway_then_beyond() {
        let hops = vec![Some(ip("192.168.1.1")), Some(ip("10.0.0.1")), Some(ip("203.0.113.1")), Some(ip("198.51.100.1"))];
        let zones = classify_hops(&hops);
        assert_eq!(zones, vec![Zone::Home, Zone::Home, Zone::IspGateway, Zone::BeyondIsp]);
    }

    #[test]
    fn no_home_hop_first_public_hop_is_gateway() {
        let hops = vec![Some(ip("203.0.113.1")), Some(ip("198.51.100.1"))];
        let zones = classify_hops(&hops);
        assert_eq!(zones, vec![Zone::IspGateway, Zone::BeyondIsp]);
    }

    #[test]
    fn non_responding_hop_inherits_current_phase_without_advancing() {
        // hop 2 doesn't respond; hop 3 is the true first public hop and
        // must still be classified as the gateway, not "beyond".
        let hops = vec![Some(ip("192.168.1.1")), None, Some(ip("203.0.113.1"))];
        let zones = classify_hops(&hops);
        assert_eq!(zones, vec![Zone::Home, Zone::Home, Zone::IspGateway]);
    }

    #[test]
    fn non_responding_hop_before_any_response_defaults_to_home() {
        let hops = vec![None, Some(ip("192.168.1.1"))];
        let zones = classify_hops(&hops);
        assert_eq!(zones, vec![Zone::Home, Zone::Home]);
    }

    #[test]
    fn as_db_str_uses_snake_case_not_debug_format() {
        assert_eq!(Zone::Home.as_db_str(), "home");
        assert_eq!(Zone::IspGateway.as_db_str(), "isp_gateway");
        assert_eq!(Zone::BeyondIsp.as_db_str(), "beyond_isp");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test mtr::zone::tests -- --nocapture`
Expected: FAIL (`not implemented`) on every test that calls `is_private` or
`classify_hops`; the `as_db_str` test fails to compile until Step 3 adds
the method — that's expected too.

- [ ] **Step 3: Implement**

```rust
// src-tauri/src/mtr/zone.rs (replace the two unimplemented! functions)
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private(),
        IpAddr::V6(_) => false, // RFC1918 is IPv4-only; the spec defines no IPv6 heuristic.
    }
}

#[derive(PartialEq)]
enum Phase {
    BeforeOrInHome,
    Gateway,
    Beyond,
}

pub fn classify_hops(addrs: &[Option<IpAddr>]) -> Vec<Zone> {
    let mut phase = Phase::BeforeOrInHome;
    let mut zones = Vec::with_capacity(addrs.len());

    for addr in addrs {
        let zone = match (&phase, addr) {
            (Phase::BeforeOrInHome, Some(a)) if is_private(*a) => Zone::Home,
            (Phase::BeforeOrInHome, Some(a)) if !is_private(*a) => {
                phase = Phase::Gateway;
                Zone::IspGateway
            }
            (Phase::BeforeOrInHome, None) => Zone::Home,
            (Phase::Gateway, _) => {
                phase = Phase::Beyond;
                Zone::BeyondIsp
            }
            (Phase::Beyond, _) => Zone::BeyondIsp,
            _ => unreachable!(),
        };
        zones.push(zone);
    }

    zones
}

impl Zone {
    /// The exact string the SQLite `zone` column stores (spec §4.4's
    /// schema). Deliberately not `Debug`-derived — `Debug` on `IspGateway`
    /// prints "IspGateway", not "isp_gateway", and every consumer that
    /// reads the column back (report.rs, any future query) expects the
    /// snake_case form.
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Zone::Home => "home",
            Zone::IspGateway => "isp_gateway",
            Zone::BeyondIsp => "beyond_isp",
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test mtr::zone::tests -- --nocapture`
Expected: PASS (7/7).

- [ ] **Step 5: Register the module**

```rust
// src-tauri/src/mtr/mod.rs
pub mod engine;
pub mod trippy_engine;
pub mod zone;
```

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/mtr/zone.rs src-tauri/src/mtr/mod.rs
git commit -m "Add RFC1918-based hop zone classification with unit tests"
```

---

## Task 7: Wire zone classification into the live hop table

**Files:**
- Modify: `src-tauri/src/mtr/engine.rs`
- Modify: `src/hop-table.js`

**Interfaces:**
- Consumes: `mtr::zone::{Zone, classify_hops}` (Task 6).
- Produces: `HopSnapshot` gains a `pub zone: Zone` field; `HopAggregator::snapshot`
  now zone-tags every hop before returning.

- [ ] **Step 1: Add the field and classify in `snapshot()`**

```rust
// src-tauri/src/mtr/engine.rs
use crate::mtr::zone::{classify_hops, Zone};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HopSnapshot {
    pub hop_index: u8,
    pub addr: Option<IpAddr>,
    pub rtt_best_ms: Option<f64>,
    pub rtt_avg_ms: Option<f64>,
    pub rtt_worst_ms: Option<f64>,
    pub loss_pct: f64,
    pub zone: Zone,
}
```

```rust
// src-tauri/src/mtr/engine.rs — replace HopAggregator::snapshot
pub fn snapshot(&self) -> Vec<HopSnapshot> {
    let addrs: Vec<Option<IpAddr>> = self.hops.values().map(|acc| acc.last_addr).collect();
    let zones = classify_hops(&addrs);

    self.hops
        .iter()
        .zip(zones)
        .map(|((idx, acc), zone)| {
            let mut snap = acc.snapshot(*idx);
            snap.zone = zone;
            snap
        })
        .collect()
}
```

`HopAccumulator::snapshot` (Task 3) currently constructs `HopSnapshot`
directly without a `zone` field — update it to take the zone as a
parameter instead:

```rust
// src-tauri/src/mtr/engine.rs
fn snapshot(&self, hop_index: u8, zone: Zone) -> HopSnapshot {
    let loss_pct = if self.sent == 0 {
        0.0
    } else {
        100.0 * (1.0 - self.received as f64 / self.sent as f64)
    };
    HopSnapshot {
        hop_index,
        addr: self.last_addr,
        rtt_best_ms: self.rtt_best_ms,
        rtt_avg_ms: if self.received == 0 { None } else { Some(self.rtt_sum_ms / self.received as f64) },
        rtt_worst_ms: self.rtt_worst_ms,
        loss_pct,
        zone,
    }
}
```

and update the call site above to `acc.snapshot(*idx, zone)` directly
instead of mutating `snap.zone` afterward (drop the `let mut snap = ...`
two-step — just call `acc.snapshot(*idx, zone)` in the `.map`).

- [ ] **Step 2: Update the existing tests to pass a zone**

Every `HopAggregator` test in Task 3 that inspects `HopSnapshot` fields
still compiles unchanged (they don't reference `.zone`), but add one new
test confirming zone tagging is live:

```rust
// src-tauri/src/mtr/engine.rs, inside mod tests
#[test]
fn snapshot_includes_zone_classification() {
    let mut agg = HopAggregator::new();
    agg.record_round(&[hop(1, Some("192.168.1.1"), Some(1.0)), hop(2, Some("203.0.113.1"), Some(5.0))]);
    let snap = agg.snapshot();
    assert_eq!(snap[0].zone, crate::mtr::zone::Zone::Home);
    assert_eq!(snap[1].zone, crate::mtr::zone::Zone::IspGateway);
}
```

- [ ] **Step 3: Run all `mtr` tests**

Run: `cd src-tauri && cargo test mtr:: -- --nocapture`
Expected: PASS, including the new zone test.

- [ ] **Step 4: Color-code the frontend table by zone**

```js
// src/hop-table.js — replace renderHopTable
const ZONE_LABELS = { home: "Home", isp_gateway: "ISP Gateway", beyond_isp: "Beyond ISP" };
const ZONE_CLASSES = { home: "zone-home", isp_gateway: "zone-gateway", beyond_isp: "zone-beyond" };

function renderHopTable(hops) {
  const rows = hops
    .map(
      (h) => `
    <tr class="${ZONE_CLASSES[h.zone]}">
      <td>${h.hop_index}</td>
      <td>${h.addr ?? "—"}</td>
      <td>${h.rtt_best_ms?.toFixed(1) ?? "—"}</td>
      <td>${h.rtt_avg_ms?.toFixed(1) ?? "—"}</td>
      <td>${h.rtt_worst_ms?.toFixed(1) ?? "—"}</td>
      <td>${h.loss_pct.toFixed(0)}%</td>
      <td>${ZONE_LABELS[h.zone]}</td>
    </tr>`
    )
    .join("");

  document.getElementById("hop-table-body").innerHTML = rows;
}

window.__TAURI__.event.listen("hop-update", (event) => {
  renderHopTable(event.payload);
});

window.__TAURI__.core.invoke("start_demo_session").catch((err) => {
  console.error("start_demo_session failed:", err);
});
```

Add a "Zone" header cell to `src/index.html`'s `<thead>` row, and basic
CSS so the zones are visually distinct:

```html
<!-- src/index.html — add to <thead><tr>, after the Loss column -->
<th>Zone</th>
```

```html
<!-- src/index.html — add inside <head> -->
<style>
  .zone-home { background: #e8f5e9; }
  .zone-gateway { background: #fff8e1; }
  .zone-beyond { background: #e3f2fd; }
</style>
```

- [ ] **Step 5: Manual verification**

Run: `cd src-tauri && cargo tauri dev`
Expected: the live table now shows a Zone column and each row is tinted by
zone (green for home, amber for ISP gateway, blue for beyond).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/mtr/engine.rs src/hop-table.js src/index.html
git commit -m "Zone-tag hop snapshots and color-code the live hop table"
```

---

## Task 8: Adaptive session rate controller

**Files:**
- Create: `src-tauri/src/mtr/rate.rs`
- Modify: `src-tauri/src/mtr/mod.rs`
- Test: `src-tauri/src/mtr/rate.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `mtr::rate::AdaptiveRate` with `new() -> Self`,
  `current_interval(&self) -> std::time::Duration`,
  `record_result(&mut self, lost: bool)`.

The window is a trailing 10-sample buffer. Adjustment only happens once
the window is full (10 samples), to avoid reacting to noise on the first
few probes. Above 20% loss in the window, the interval backs off ×1.5 (cap
3000ms). Once loss is under 5% and the window is entirely full of fresh
low-loss data, the interval steps back down ×(1/1.5) (floor 1000ms). Per
spec §4.3.

- [ ] **Step 1: Write the failing tests**

```rust
// src-tauri/src/mtr/rate.rs
use std::collections::VecDeque;
use std::time::Duration;

const WINDOW: usize = 10;
const FLOOR_MS: u64 = 1000;
const CEILING_MS: u64 = 3000;
const BACKOFF_LOSS_THRESHOLD: f64 = 0.20;
const RECOVER_LOSS_THRESHOLD: f64 = 0.05;
const FACTOR: f64 = 1.5;

pub struct AdaptiveRate {
    interval_ms: u64,
    window: VecDeque<bool>,
}

impl AdaptiveRate {
    pub fn new() -> Self {
        unimplemented!()
    }

    pub fn current_interval(&self) -> Duration {
        unimplemented!()
    }

    pub fn record_result(&mut self, lost: bool) {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_one_second() {
        let r = AdaptiveRate::new();
        assert_eq!(r.current_interval(), Duration::from_millis(1000));
    }

    #[test]
    fn does_not_adjust_before_window_is_full() {
        let mut r = AdaptiveRate::new();
        for _ in 0..9 {
            r.record_result(true); // 100% loss, but window isn't full yet
        }
        assert_eq!(r.current_interval(), Duration::from_millis(1000));
    }

    #[test]
    fn backs_off_once_window_full_of_high_loss() {
        let mut r = AdaptiveRate::new();
        for _ in 0..10 {
            r.record_result(true); // 100% loss > 20% threshold
        }
        assert_eq!(r.current_interval(), Duration::from_millis(1500));
    }

    #[test]
    fn backoff_is_capped_at_three_seconds() {
        let mut r = AdaptiveRate::new();
        for _ in 0..100 {
            r.record_result(true);
        }
        assert_eq!(r.current_interval(), Duration::from_millis(3000));
    }

    #[test]
    fn recovers_once_window_full_of_low_loss() {
        let mut r = AdaptiveRate::new();
        for _ in 0..10 {
            r.record_result(true); // push it up to 1500ms first
        }
        assert_eq!(r.current_interval(), Duration::from_millis(1500));
        for _ in 0..10 {
            r.record_result(false);
        }
        // Not 1000ms: the window is a sliding 10-sample buffer that
        // re-evaluates on every call once full, not just once every 10
        // calls. As `false`s displace the original `true`s one at a time,
        // loss_pct stays above the 20% backoff threshold for the first 7
        // of these 10 calls (driving the interval back up to the 3000ms
        // ceiling), sits between the two thresholds (no change) for the
        // next 2, and only drops below the 5% recovery threshold on the
        // 10th call — at which point a single proportional recovery step
        // takes 3000ms to 2000ms, not to the floor. (A single-step
        // recovery reaching 1000ms here would only happen if the loop
        // that reaches this point were implemented as an un-specified
        // multi-step convergence-to-floor rather than the one proportional
        // step above — a bug caught during this plan's own task review.)
        assert_eq!(r.current_interval(), Duration::from_millis(2000));
    }

    #[test]
    fn floor_is_one_second() {
        let mut r = AdaptiveRate::new();
        for _ in 0..30 {
            r.record_result(false);
        }
        assert_eq!(r.current_interval(), Duration::from_millis(1000));
    }

    #[test]
    fn moderate_loss_between_thresholds_does_not_adjust() {
        let mut r = AdaptiveRate::new();
        // 10% loss: 1 lost in every 10 — above the 5% recovery bar, below
        // the 20% backoff bar, so the interval should stay put.
        for i in 0..20 {
            r.record_result(i % 10 == 0);
        }
        assert_eq!(r.current_interval(), Duration::from_millis(1000));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test mtr::rate::tests -- --nocapture`
Expected: FAIL (`not implemented`) on every test.

- [ ] **Step 3: Implement**

```rust
// src-tauri/src/mtr/rate.rs (replace the three unimplemented! bodies)
impl AdaptiveRate {
    pub fn new() -> Self {
        Self { interval_ms: FLOOR_MS, window: VecDeque::with_capacity(WINDOW) }
    }

    pub fn current_interval(&self) -> Duration {
        Duration::from_millis(self.interval_ms)
    }

    pub fn record_result(&mut self, lost: bool) {
        self.window.push_back(lost);
        if self.window.len() > WINDOW {
            self.window.pop_front();
        }
        if self.window.len() < WINDOW {
            return;
        }

        let loss_pct = self.window.iter().filter(|&&l| l).count() as f64 / WINDOW as f64;
        if loss_pct > BACKOFF_LOSS_THRESHOLD {
            self.interval_ms = ((self.interval_ms as f64 * FACTOR) as u64).min(CEILING_MS);
        } else if loss_pct < RECOVER_LOSS_THRESHOLD {
            self.interval_ms = ((self.interval_ms as f64 / FACTOR) as u64).max(FLOOR_MS);
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test mtr::rate::tests -- --nocapture`
Expected: PASS (7/7).

- [ ] **Step 5: Register the module**

```rust
// src-tauri/src/mtr/mod.rs
pub mod engine;
pub mod rate;
pub mod trippy_engine;
pub mod zone;
```

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/mtr/rate.rs src-tauri/src/mtr/mod.rs
git commit -m "Add adaptive session probe rate controller with unit tests"
```

---

## Task 9: SessionRunner — orchestrate engine + aggregator + adaptive rate

**Files:**
- Create: `src-tauri/src/mtr/session.rs`
- Modify: `src-tauri/src/mtr/mod.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs`
- Test: `src-tauri/src/mtr/session.rs` (inline `#[cfg(test)]`, using a fake `ProbeEngine`)

**Interfaces:**
- Consumes: `mtr::engine::{ProbeEngine, HopAggregator, HopSnapshot, RawHop, RoundResult, EngineHandle, EngineError}` (Task 3), `mtr::rate::AdaptiveRate` (Task 8).
- Produces: `mtr::session::SessionRunner::start(engine: Arc<dyn ProbeEngine>, target: IpAddr, on_snapshot: Box<dyn Fn(Vec<HopSnapshot>) + Send + Sync>) -> Self`,
  `SessionRunner::stop(self)`. Replaces the demo wiring from Task 5.

`trippy-core`'s `Tracer` has no public "change round duration on the fly"
API (confirmed against current docs — see Task 4's note on `trippy-core`'s
instability). `SessionRunner` works around this by restarting the engine
(stop + respawn) whenever `AdaptiveRate`'s interval changes, using the new
interval. This is the one thing this task must get right — it's exercised
via a fake `ProbeEngine` so it's fully unit-tested without real sockets.

- [ ] **Step 1: Write the failing tests using a fake engine**

```rust
// src-tauri/src/mtr/session.rs
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::mtr::engine::{EngineError, EngineHandle, HopAggregator, HopSnapshot, ProbeEngine, RawHop, RoundResult};
use crate::mtr::rate::AdaptiveRate;

pub struct SessionRunner {
    stop: Option<Box<dyn FnOnce() + Send>>,
}

impl SessionRunner {
    pub fn start(
        engine: Arc<dyn ProbeEngine>,
        target: IpAddr,
        on_snapshot: Box<dyn Fn(Vec<HopSnapshot>) + Send + Sync>,
    ) -> Self {
        unimplemented!()
    }

    pub fn stop(self) {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records every (target, interval) it was asked to spawn with, and lets
    /// the test drive rounds manually via the stored callback.
    ///
    /// The callback is stored as `Arc`, not `Box`, and `drive_round` clones
    /// it out of the lock before invoking it. This matters because invoking
    /// the callback can itself call back into `spawn` (SessionRunner's
    /// restart-on-interval-change path does exactly this) — if `drive_round`
    /// held the `last_callback` `MutexGuard` across the call (as it would
    /// with `Box` + `.as_ref()`), that reentrant `spawn` call's own
    /// `last_callback.lock()` would deadlock against itself on the same
    /// thread, since `std::sync::Mutex` isn't reentrant. Cloning the `Arc`
    /// out first drops the guard before the callback ever runs.
    struct FakeEngine {
        spawns: Mutex<Vec<Duration>>,
        last_callback: Mutex<Option<Arc<dyn Fn(RoundResult) + Send + Sync>>>,
    }

    impl FakeEngine {
        fn new() -> Arc<Self> {
            Arc::new(Self { spawns: Mutex::new(vec![]), last_callback: Mutex::new(None) })
        }

        fn drive_round(&self, hops: Vec<RawHop>) {
            let cb = self.last_callback.lock().unwrap().clone();
            (cb.unwrap())(RoundResult { hops });
        }
    }

    impl ProbeEngine for FakeEngine {
        fn spawn(
            &self,
            _target: IpAddr,
            interval: Duration,
            on_round: Box<dyn Fn(RoundResult) + Send + Sync>,
        ) -> Result<EngineHandle, EngineError> {
            self.spawns.lock().unwrap().push(interval);
            *self.last_callback.lock().unwrap() = Some(Arc::from(on_round));
            Ok(EngineHandle { stop: Box::new(|| {}) })
        }
    }

    fn hop(idx: u8, addr: Option<&str>, rtt: Option<f64>) -> RawHop {
        RawHop { hop_index: idx, addr: addr.map(|a| a.parse().unwrap()), rtt_ms: rtt }
    }

    #[test]
    fn starts_with_a_one_second_interval() {
        let engine = FakeEngine::new();
        let _runner = SessionRunner::start(engine.clone(), "1.1.1.1".parse().unwrap(), Box::new(|_| {}));
        assert_eq!(*engine.spawns.lock().unwrap(), vec![Duration::from_millis(1000)]);
    }

    #[test]
    fn forwards_hop_snapshots_to_the_callback() {
        let engine = FakeEngine::new();
        let received = Arc::new(Mutex::new(None));
        let received_for_cb = received.clone();
        let _runner = SessionRunner::start(
            engine.clone(),
            "1.1.1.1".parse().unwrap(),
            Box::new(move |snap| *received_for_cb.lock().unwrap() = Some(snap)),
        );

        engine.drive_round(vec![hop(1, Some("1.1.1.1"), Some(5.0))]);

        let snap = received.lock().unwrap().clone().unwrap();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].hop_index, 1);
    }

    #[test]
    fn restarts_engine_with_new_interval_once_rate_backs_off() {
        let engine = FakeEngine::new();
        let _runner = SessionRunner::start(engine.clone(), "1.1.1.1".parse().unwrap(), Box::new(|_| {}));

        // Last hop lost 10 times in a row -> AdaptiveRate backs off past its
        // window-full threshold, which must trigger a respawn.
        for _ in 0..10 {
            engine.drive_round(vec![hop(1, None, None)]);
        }

        let spawns = engine.spawns.lock().unwrap();
        assert_eq!(spawns.len(), 2);
        assert_eq!(spawns[1], Duration::from_millis(1500));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test mtr::session::tests -- --nocapture`
Expected: FAIL (`not implemented`).

- [ ] **Step 3: Implement `SessionRunner`**

```rust
// src-tauri/src/mtr/session.rs (replace the impl block)
struct Shared {
    engine: Arc<dyn ProbeEngine>,
    target: IpAddr,
    aggregator: Mutex<HopAggregator>,
    rate: Mutex<AdaptiveRate>,
    on_snapshot: Box<dyn Fn(Vec<HopSnapshot>) + Send + Sync>,
    current_handle: Mutex<Option<EngineHandle>>,
}

impl SessionRunner {
    pub fn start(
        engine: Arc<dyn ProbeEngine>,
        target: IpAddr,
        on_snapshot: Box<dyn Fn(Vec<HopSnapshot>) + Send + Sync>,
    ) -> Self {
        let shared = Arc::new(Shared {
            engine,
            target,
            aggregator: Mutex::new(HopAggregator::new()),
            rate: Mutex::new(AdaptiveRate::new()),
            on_snapshot,
            current_handle: Mutex::new(None),
        });

        spawn_with_current_interval(&shared);

        SessionRunner {
            stop: Some(Box::new(move || {
                if let Some(handle) = shared.current_handle.lock().unwrap().take() {
                    (handle.stop)();
                }
            })),
        }
    }

    pub fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

fn spawn_with_current_interval(shared: &Arc<Shared>) {
    let interval = shared.rate.lock().unwrap().current_interval();
    let shared_for_callback = shared.clone();

    let handle = shared
        .engine
        .spawn(
            shared.target,
            interval,
            Box::new(move |round| on_round(&shared_for_callback, round)),
        )
        .expect("engine spawn failed");

    *shared.current_handle.lock().unwrap() = Some(handle);
}

fn on_round(shared: &Arc<Shared>, round: RoundResult) {
    shared.aggregator.lock().unwrap().record_round(&round.hops);

    let last_hop_lost = round.hops.last().map(|h| h.addr.is_none()).unwrap_or(false);
    let interval_before = shared.rate.lock().unwrap().current_interval();
    shared.rate.lock().unwrap().record_result(last_hop_lost);
    let interval_after = shared.rate.lock().unwrap().current_interval();

    let snapshot = shared.aggregator.lock().unwrap().snapshot();
    (shared.on_snapshot)(snapshot);

    if interval_after != interval_before {
        if let Some(old_handle) = shared.current_handle.lock().unwrap().take() {
            (old_handle.stop)();
        }
        spawn_with_current_interval(shared);
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test mtr::session::tests -- --nocapture`
Expected: PASS (3/3).

- [ ] **Step 5: Register the module**

```rust
// src-tauri/src/mtr/mod.rs
pub mod engine;
pub mod rate;
pub mod session;
pub mod trippy_engine;
pub mod zone;
```

- [ ] **Step 6: Replace the Task 5 demo command with real `start_session`/`stop_session`**

```rust
// src-tauri/src/commands.rs — replace the whole file's session-related content
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, State};

use crate::mtr::session::SessionRunner;
use crate::mtr::trippy_engine::TrippyEngine;

pub struct SessionState(pub Mutex<Option<SessionRunner>>);

#[tauri::command]
pub fn start_session(app: AppHandle, state: State<SessionState>, target: String) -> Result<(), String> {
    let target: IpAddr = target.parse().map_err(|_| "invalid target IP".to_string())?;
    let mut guard = state.0.lock().unwrap();
    if guard.is_some() {
        return Err("a session is already running".to_string());
    }

    let app_for_callback = app.clone();
    let runner = SessionRunner::start(
        Arc::new(TrippyEngine),
        target,
        Box::new(move |snapshot| {
            let _ = app_for_callback.emit("hop-update", snapshot);
        }),
    );
    *guard = Some(runner);
    Ok(())
}

#[tauri::command]
pub fn stop_session(state: State<SessionState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    if let Some(runner) = guard.take() {
        runner.stop();
        Ok(())
    } else {
        Err("no session is running".to_string())
    }
}
```

- [ ] **Step 7: Update `main.rs`**

```rust
// src-tauri/src/main.rs
mod commands;
mod elevation;
mod mtr;

fn main() {
    if elevation::decide_relaunch(&elevation::check_elevation()) {
        elevation::relaunch_elevated();
    }

    tauri::Builder::default()
        .manage(commands::SessionState(std::sync::Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![commands::start_session, commands::stop_session])
        .run(tauri::generate_context!())
        .expect("error while running Packet Proof");
}
```

- [ ] **Step 8: Update the frontend to start/stop a real session**

```js
// src/hop-table.js — replace the bottom two lines
window.__TAURI__.event.listen("hop-update", (event) => {
  renderHopTable(event.payload);
});
```

```js
// src/dashboard.js
async function startSession(target) {
  await window.__TAURI__.core.invoke("start_session", { target });
}

async function stopSession() {
  await window.__TAURI__.core.invoke("stop_session");
}

window.startSession = startSession;
window.stopSession = stopSession;
```

```html
<!-- src/index.html — add above the table -->
<div>
  <input id="target-input" value="1.1.1.1" />
  <button onclick="startSession(document.getElementById('target-input').value)">Start</button>
  <button onclick="stopSession()">Stop</button>
</div>
<script src="dashboard.js"></script>
```

- [ ] **Step 9: Manual verification**

Run: `cd src-tauri && cargo tauri dev`
Expected: entering an IP and clicking Start begins live tracing to that
target; Stop halts updates; Start again works for a new target.

- [ ] **Step 10: Commit**

```bash
git add src-tauri/src/mtr/session.rs src-tauri/src/mtr/mod.rs src-tauri/src/commands.rs src-tauri/src/main.rs src/dashboard.js src/hop-table.js src/index.html
git commit -m "Add SessionRunner tying engine, aggregator, and adaptive rate together"
```

---

## Task 10: SQLite schema and Db module

**Files:**
- Create: `src-tauri/src/db.rs`
- Modify: `src-tauri/src/main.rs`
- Test: `src-tauri/src/db.rs` (inline `#[cfg(test)]`, in-memory SQLite)

**Interfaces:**
- Produces:
  - `db::ProbeRecord { session_type: String, target: String, session_id: Option<String>, timestamp: i64, hop_index: u8, hop_ip: Option<String>, rtt_best_ms: Option<f64>, rtt_avg_ms: Option<f64>, rtt_worst_ms: Option<f64>, loss_pct: f64, zone: String }`
  - `db::Db::open(path: &std::path::Path) -> rusqlite::Result<Self>`
  - `db::Db::insert_record(&self, record: &ProbeRecord) -> rusqlite::Result<()>`
  - `db::Db::query_history(&self, target: &str, session_type_filter: Option<&str>, since: i64, until: i64) -> rusqlite::Result<Vec<ProbeRecord>>`

- [ ] **Step 1: Write the failing tests against an in-memory database**

```rust
// src-tauri/src/db.rs
use rusqlite::Connection;

#[derive(Debug, Clone, PartialEq)]
pub struct ProbeRecord {
    pub session_type: String,
    pub target: String,
    pub session_id: Option<String>,
    pub timestamp: i64,
    pub hop_index: u8,
    pub hop_ip: Option<String>,
    pub rtt_best_ms: Option<f64>,
    pub rtt_avg_ms: Option<f64>,
    pub rtt_worst_ms: Option<f64>,
    pub loss_pct: f64,
    pub zone: String,
}

pub struct Db {
    conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS probe_records (
  id            INTEGER PRIMARY KEY,
  session_type  TEXT NOT NULL,
  target        TEXT NOT NULL,
  session_id    TEXT,
  timestamp     INTEGER NOT NULL,
  hop_index     INTEGER NOT NULL,
  hop_ip        TEXT,
  rtt_best_ms   REAL,
  rtt_avg_ms    REAL,
  rtt_worst_ms  REAL,
  loss_pct      REAL NOT NULL,
  zone          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_probe_records_target_time ON probe_records(target, timestamp);
CREATE INDEX IF NOT EXISTS idx_probe_records_session ON probe_records(session_id);
";

impl Db {
    pub fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        unimplemented!()
    }

    fn from_connection(conn: Connection) -> rusqlite::Result<Self> {
        unimplemented!()
    }

    pub fn insert_record(&self, record: &ProbeRecord) -> rusqlite::Result<()> {
        unimplemented!()
    }

    pub fn query_history(
        &self,
        target: &str,
        session_type_filter: Option<&str>,
        since: i64,
        until: i64,
    ) -> rusqlite::Result<Vec<ProbeRecord>> {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Db {
        Db::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn sample(session_type: &str, target: &str, ts: i64, hop: u8) -> ProbeRecord {
        ProbeRecord {
            session_type: session_type.to_string(),
            target: target.to_string(),
            session_id: None,
            timestamp: ts,
            hop_index: hop,
            hop_ip: Some("1.1.1.1".to_string()),
            rtt_best_ms: Some(5.0),
            rtt_avg_ms: Some(6.0),
            rtt_worst_ms: Some(8.0),
            loss_pct: 0.0,
            zone: "beyond_isp".to_string(),
        }
    }

    #[test]
    fn insert_and_query_round_trip() {
        let db = test_db();
        let rec = sample("baseline", "1.1.1.1", 1000, 1);
        db.insert_record(&rec).unwrap();

        let results = db.query_history("1.1.1.1", None, 0, 2000).unwrap();
        assert_eq!(results, vec![rec]);
    }

    #[test]
    fn query_filters_by_target() {
        let db = test_db();
        db.insert_record(&sample("baseline", "1.1.1.1", 1000, 1)).unwrap();
        db.insert_record(&sample("baseline", "8.8.8.8", 1000, 1)).unwrap();

        let results = db.query_history("1.1.1.1", None, 0, 2000).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target, "1.1.1.1");
    }

    #[test]
    fn query_filters_by_time_range() {
        let db = test_db();
        db.insert_record(&sample("baseline", "1.1.1.1", 500, 1)).unwrap();
        db.insert_record(&sample("baseline", "1.1.1.1", 1500, 1)).unwrap();
        db.insert_record(&sample("baseline", "1.1.1.1", 2500, 1)).unwrap();

        let results = db.query_history("1.1.1.1", None, 1000, 2000).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].timestamp, 1500);
    }

    #[test]
    fn query_filters_by_session_type() {
        let db = test_db();
        db.insert_record(&sample("baseline", "1.1.1.1", 1000, 1)).unwrap();
        db.insert_record(&sample("timed", "1.1.1.1", 1000, 1)).unwrap();

        let results = db.query_history("1.1.1.1", Some("timed"), 0, 2000).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_type, "timed");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test db::tests -- --nocapture`
Expected: FAIL (`not implemented`) on every test.

- [ ] **Step 3: Implement**

```rust
// src-tauri/src/db.rs (replace the four unimplemented! bodies)
impl Db {
    pub fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn insert_record(&self, record: &ProbeRecord) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO probe_records
             (session_type, target, session_id, timestamp, hop_index, hop_ip,
              rtt_best_ms, rtt_avg_ms, rtt_worst_ms, loss_pct, zone)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                record.session_type,
                record.target,
                record.session_id,
                record.timestamp,
                record.hop_index,
                record.hop_ip,
                record.rtt_best_ms,
                record.rtt_avg_ms,
                record.rtt_worst_ms,
                record.loss_pct,
                record.zone,
            ],
        )?;
        Ok(())
    }

    pub fn query_history(
        &self,
        target: &str,
        session_type_filter: Option<&str>,
        since: i64,
        until: i64,
    ) -> rusqlite::Result<Vec<ProbeRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_type, target, session_id, timestamp, hop_index, hop_ip,
                    rtt_best_ms, rtt_avg_ms, rtt_worst_ms, loss_pct, zone
             FROM probe_records
             WHERE target = ?1
               AND timestamp BETWEEN ?2 AND ?3
               AND (?4 IS NULL OR session_type = ?4)
             ORDER BY timestamp ASC",
        )?;

        let rows = stmt.query_map(
            rusqlite::params![target, since, until, session_type_filter],
            |row| {
                Ok(ProbeRecord {
                    session_type: row.get(0)?,
                    target: row.get(1)?,
                    session_id: row.get(2)?,
                    timestamp: row.get(3)?,
                    hop_index: row.get(4)?,
                    hop_ip: row.get(5)?,
                    rtt_best_ms: row.get(6)?,
                    rtt_avg_ms: row.get(7)?,
                    rtt_worst_ms: row.get(8)?,
                    loss_pct: row.get(9)?,
                    zone: row.get(10)?,
                })
            },
        )?;

        rows.collect()
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test db::tests -- --nocapture`
Expected: PASS (4/4).

- [ ] **Step 5: Register the module**

```rust
// src-tauri/src/main.rs — add near the other mod declarations
mod db;
```

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db.rs src-tauri/src/main.rs
git commit -m "Add SQLite schema and Db module with unit tests"
```

---

## Task 11: DB writer actor and wiring session writes through it

**Files:**
- Create: `src-tauri/src/db_writer.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/mtr/session.rs`

**Interfaces:**
- Consumes: `db::{Db, ProbeRecord}` (Task 10).
- Produces: `db_writer::spawn_db_writer(db_path: std::path::PathBuf) -> std::sync::mpsc::Sender<db::ProbeRecord>`
  (spawns a dedicated OS thread owning the one write connection; returns a
  cloneable sender any part of the app can push records through).
  `SessionRunner::start` gains a `record_sink: std::sync::mpsc::Sender<db::ProbeRecord>`
  parameter so every aggregated round also gets persisted, tagged with a
  fresh `session_id` (via the `uuid` crate) for the lifetime of that
  `SessionRunner`.

- [ ] **Step 1: Implement the writer actor**

No unit test here — it's a thin thread-plus-channel wrapper around the
already-tested `Db`; the meaningful logic (schema, insert, query) is
already covered in Task 10. This task is verified by the manual check in
Step 4.

```rust
// src-tauri/src/db_writer.rs
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};

use crate::db::{Db, ProbeRecord};

pub fn spawn_db_writer(db_path: PathBuf) -> Sender<ProbeRecord> {
    let (tx, rx) = mpsc::channel::<ProbeRecord>();

    std::thread::spawn(move || {
        let db = Db::open(&db_path).expect("failed to open sqlite db");
        for record in rx {
            if let Err(e) = db.insert_record(&record) {
                eprintln!("db write failed: {e}");
            }
        }
    });

    tx
}
```

- [ ] **Step 2: Thread a record sink through `SessionRunner`**

```rust
// src-tauri/src/mtr/session.rs — add imports
use std::sync::mpsc::Sender;
use crate::db::ProbeRecord;
```

```rust
// src-tauri/src/mtr/session.rs — extend Shared and start()
struct Shared {
    engine: Arc<dyn ProbeEngine>,
    target: IpAddr,
    target_label: String,
    session_id: String,
    aggregator: Mutex<HopAggregator>,
    rate: Mutex<AdaptiveRate>,
    on_snapshot: Box<dyn Fn(Vec<HopSnapshot>) + Send + Sync>,
    current_handle: Mutex<Option<EngineHandle>>,
    record_sink: Sender<ProbeRecord>,
}

impl SessionRunner {
    pub fn start(
        engine: Arc<dyn ProbeEngine>,
        target: IpAddr,
        on_snapshot: Box<dyn Fn(Vec<HopSnapshot>) + Send + Sync>,
        record_sink: Sender<ProbeRecord>,
    ) -> Self {
        let shared = Arc::new(Shared {
            engine,
            target,
            target_label: target.to_string(),
            session_id: uuid::Uuid::new_v4().to_string(),
            aggregator: Mutex::new(HopAggregator::new()),
            rate: Mutex::new(AdaptiveRate::new()),
            on_snapshot,
            current_handle: Mutex::new(None),
            record_sink,
        });

        spawn_with_current_interval(&shared);

        SessionRunner {
            stop: Some(Box::new(move || {
                if let Some(handle) = shared.current_handle.lock().unwrap().take() {
                    (handle.stop)();
                }
            })),
        }
    }
    // stop() is unchanged
}
```

```rust
// src-tauri/src/mtr/session.rs — extend on_round to persist each snapshot
fn on_round(shared: &Arc<Shared>, round: RoundResult) {
    shared.aggregator.lock().unwrap().record_round(&round.hops);

    let last_hop_lost = round.hops.last().map(|h| h.addr.is_none()).unwrap_or(false);
    let interval_before = shared.rate.lock().unwrap().current_interval();
    shared.rate.lock().unwrap().record_result(last_hop_lost);
    let interval_after = shared.rate.lock().unwrap().current_interval();

    let snapshot = shared.aggregator.lock().unwrap().snapshot();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    for hop in &snapshot {
        let _ = shared.record_sink.send(ProbeRecord {
            session_type: "timed".to_string(),
            target: shared.target_label.clone(),
            session_id: Some(shared.session_id.clone()),
            timestamp: now,
            hop_index: hop.hop_index,
            hop_ip: hop.addr.map(|a| a.to_string()),
            rtt_best_ms: hop.rtt_best_ms,
            rtt_avg_ms: hop.rtt_avg_ms,
            rtt_worst_ms: hop.rtt_worst_ms,
            loss_pct: hop.loss_pct,
            zone: hop.zone.as_db_str().to_string(),
        });
    }

    (shared.on_snapshot)(snapshot);

    if interval_after != interval_before {
        if let Some(old_handle) = shared.current_handle.lock().unwrap().take() {
            (old_handle.stop)();
        }
        spawn_with_current_interval(shared);
    }
}
```

Update Task 9's three `SessionRunner::start` calls in the `mod tests`
block to pass a throwaway sink:
`let (tx, _rx) = std::sync::mpsc::channel(); SessionRunner::start(engine.clone(), "1.1.1.1".parse().unwrap(), Box::new(|_| {}), tx)`.

- [ ] **Step 3: Wire the writer into `main.rs` and `start_session`**

```rust
// src-tauri/src/main.rs
mod db_writer;

fn main() {
    if elevation::decide_relaunch(&elevation::check_elevation()) {
        elevation::relaunch_elevated();
    }

    tauri::Builder::default()
        .setup(|app| {
            let db_path = app.path().app_data_dir()?.join("packet_proof.sqlite");
            std::fs::create_dir_all(db_path.parent().unwrap())?;
            let sink = db_writer::spawn_db_writer(db_path);
            app.manage(commands::DbSink(sink));
            Ok(())
        })
        .manage(commands::SessionState(std::sync::Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![commands::start_session, commands::stop_session])
        .run(tauri::generate_context!())
        .expect("error while running Packet Proof");
}
```

```rust
// src-tauri/src/commands.rs — add the DbSink wrapper and thread it through start_session
pub struct DbSink(pub std::sync::mpsc::Sender<crate::db::ProbeRecord>);

#[tauri::command]
pub fn start_session(
    app: AppHandle,
    state: State<SessionState>,
    db_sink: State<DbSink>,
    target: String,
) -> Result<(), String> {
    let target: IpAddr = target.parse().map_err(|_| "invalid target IP".to_string())?;
    let mut guard = state.0.lock().unwrap();
    if guard.is_some() {
        return Err("a session is already running".to_string());
    }

    let app_for_callback = app.clone();
    let runner = SessionRunner::start(
        Arc::new(TrippyEngine),
        target,
        Box::new(move |snapshot| {
            let _ = app_for_callback.emit("hop-update", snapshot);
        }),
        db_sink.0.clone(),
    );
    *guard = Some(runner);
    Ok(())
}
```

`Sender<T>` is `Clone`, so every command handler invocation can clone it
out of Tauri's managed state cheaply.

- [ ] **Step 4: Manual verification**

Run: `cd src-tauri && cargo tauri dev`, start a session, let it run for
~10 seconds, stop it. Then inspect the DB directly:

```bash
sqlite3 "$(cargo tauri info 2>/dev/null | grep -o '/.*app_data.*' || echo '<app data dir>/packet_proof.sqlite')" \
  "SELECT COUNT(*) FROM probe_records WHERE session_type = 'timed';"
```

(If `cargo tauri info` doesn't resolve the path directly, find the app
data directory Tauri used for your OS — Windows:
`%APPDATA%/com.packet-proof.app`, macOS: `~/Library/Application
Support/com.packet-proof.app` — and point `sqlite3` at the `.sqlite` file
there instead.)

Expected: a non-zero row count matching roughly the number of seconds the
session ran times the number of hops.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db_writer.rs src-tauri/src/main.rs src-tauri/src/commands.rs src-tauri/src/mtr/session.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "Add DB writer actor and persist timed-session hop records"
```

---

## Task 12: Baseline logger

**Files:**
- Create: `src-tauri/src/mtr/baseline.rs`
- Modify: `src-tauri/src/mtr/mod.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs`
- Test: `src-tauri/src/mtr/baseline.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `mtr::engine::{ProbeEngine, HopAggregator}` (Task 3), `db::ProbeRecord` (Task 10).
- Produces: `mtr::baseline::default_gateway() -> Option<std::net::IpAddr>`,
  `mtr::baseline::fixed_targets(gateway: Option<std::net::IpAddr>) -> Vec<std::net::IpAddr>`
  (pure — testable without a real network),
  `mtr::baseline::BaselineLogger::start(engine: Arc<dyn ProbeEngine>, targets: Vec<IpAddr>, record_sink: Sender<ProbeRecord>) -> Self`,
  `BaselineLogger::stop(self)`. Commands `start_baseline`/`stop_baseline`.

- [ ] **Step 1: Write the failing test for the pure target-list logic**

```rust
// src-tauri/src/mtr/baseline.rs
use std::net::IpAddr;

pub const CLOUDFLARE_DNS: &str = "1.1.1.1";
pub const GOOGLE_DNS: &str = "8.8.8.8";

pub fn fixed_targets(gateway: Option<IpAddr>) -> Vec<IpAddr> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_gateway_and_both_public_dns_targets_when_gateway_known() {
        let gw: IpAddr = "192.168.1.1".parse().unwrap();
        let targets = fixed_targets(Some(gw));
        assert_eq!(
            targets,
            vec![gw, CLOUDFLARE_DNS.parse().unwrap(), GOOGLE_DNS.parse().unwrap()]
        );
    }

    #[test]
    fn omits_gateway_when_unknown() {
        let targets = fixed_targets(None);
        assert_eq!(targets, vec![CLOUDFLARE_DNS.parse().unwrap(), GOOGLE_DNS.parse().unwrap()]);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test mtr::baseline::tests -- --nocapture`
Expected: FAIL (`not implemented`).

- [ ] **Step 3: Implement the pure logic, gateway lookup, and `BaselineLogger`**

```rust
// src-tauri/src/mtr/baseline.rs (replace the unimplemented! body, append below)
pub fn fixed_targets(gateway: Option<IpAddr>) -> Vec<IpAddr> {
    let mut targets = vec![];
    if let Some(gw) = gateway {
        targets.push(gw);
    }
    targets.push(CLOUDFLARE_DNS.parse().unwrap());
    targets.push(GOOGLE_DNS.parse().unwrap());
    targets
}

#[cfg(target_os = "windows")]
pub fn default_gateway() -> Option<IpAddr> {
    // GetBestRoute2/GetIpForwardTable2 is the correct low-level API, but for
    // the MVP the simplest reliable source is `route print` output, which
    // Windows always has available without extra dependencies.
    let output = std::process::Command::new("route").arg("print").arg("0.0.0.0").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find(|l| l.trim_start().starts_with("0.0.0.0"))
        .and_then(|l| l.split_whitespace().nth(2))
        .and_then(|s| s.parse().ok())
}

#[cfg(target_os = "macos")]
pub fn default_gateway() -> Option<IpAddr> {
    let output = std::process::Command::new("route").arg("-n").arg("get").arg("default").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find(|l| l.trim_start().starts_with("gateway:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
}

pub struct BaselineLogger {
    stop_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl BaselineLogger {
    pub fn start(
        engine: std::sync::Arc<dyn crate::mtr::engine::ProbeEngine>,
        targets: Vec<IpAddr>,
        record_sink: std::sync::mpsc::Sender<crate::db::ProbeRecord>,
    ) -> Self {
        let stop_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_flag_for_thread = stop_flag.clone();

        std::thread::spawn(move || {
            use std::sync::atomic::Ordering;
            while !stop_flag_for_thread.load(Ordering::Relaxed) {
                for target in &targets {
                    if stop_flag_for_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    run_single_probe_round(&engine, *target, &record_sink);
                }
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        });

        Self { stop_flag }
    }

    pub fn stop(self) {
        self.stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn run_single_probe_round(
    engine: &std::sync::Arc<dyn crate::mtr::engine::ProbeEngine>,
    target: IpAddr,
    record_sink: &std::sync::mpsc::Sender<crate::db::ProbeRecord>,
) {
    use crate::mtr::engine::HopAggregator;
    use std::sync::{Arc, Mutex};

    let aggregator = Arc::new(Mutex::new(HopAggregator::new()));
    let agg_for_callback = aggregator.clone();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done_for_callback = done.clone();

    let handle = engine
        .spawn(
            target,
            std::time::Duration::from_millis(200),
            Box::new(move |round| {
                agg_for_callback.lock().unwrap().record_round(&round.hops);
                done_for_callback.store(true, std::sync::atomic::Ordering::Relaxed);
            }),
        )
        .expect("baseline probe spawn failed");

    // One round is enough for a baseline sample; poll briefly for it, then
    // tear the tracer down rather than leaving 3 tracers running between
    // every 60s tick.
    for _ in 0..25 {
        if done.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    (handle.stop)();

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    for hop in aggregator.lock().unwrap().snapshot() {
        let _ = record_sink.send(crate::db::ProbeRecord {
            session_type: "baseline".to_string(),
            target: target.to_string(),
            session_id: None,
            timestamp: now,
            hop_index: hop.hop_index,
            hop_ip: hop.addr.map(|a| a.to_string()),
            rtt_best_ms: hop.rtt_best_ms,
            rtt_avg_ms: hop.rtt_avg_ms,
            rtt_worst_ms: hop.rtt_worst_ms,
            loss_pct: hop.loss_pct,
            zone: hop.zone.as_db_str().to_string(),
        });
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test mtr::baseline::tests -- --nocapture`
Expected: PASS (2/2). (`default_gateway` and `BaselineLogger` are
OS/thread-driven and covered by the manual check in Step 6, same rationale
as Task 4 and Task 11.)

- [ ] **Step 5: Register the module and add commands**

```rust
// src-tauri/src/mtr/mod.rs
pub mod baseline;
pub mod engine;
pub mod rate;
pub mod session;
pub mod trippy_engine;
pub mod zone;
```

```rust
// src-tauri/src/commands.rs — add
use crate::mtr::baseline::{default_gateway, fixed_targets, BaselineLogger};

pub struct BaselineState(pub Mutex<Option<BaselineLogger>>);

#[tauri::command]
pub fn start_baseline(state: State<BaselineState>, db_sink: State<DbSink>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    if guard.is_some() {
        return Err("baseline is already running".to_string());
    }
    let targets = fixed_targets(default_gateway());
    let logger = BaselineLogger::start(Arc::new(TrippyEngine), targets, db_sink.0.clone());
    *guard = Some(logger);
    Ok(())
}

#[tauri::command]
pub fn stop_baseline(state: State<BaselineState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    if let Some(logger) = guard.take() {
        logger.stop();
        Ok(())
    } else {
        Err("baseline is not running".to_string())
    }
}
```

```rust
// src-tauri/src/main.rs — register state and commands, start baseline automatically after setup
.setup(|app| {
    let db_path = app.path().app_data_dir()?.join("packet_proof.sqlite");
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    // .clone() here, not a plain move: Task 18 reuses `db_path` later in
    // this same closure to open a second, read-only connection.
    let sink = db_writer::spawn_db_writer(db_path.clone());
    app.manage(commands::DbSink(sink.clone()));

    let targets = mtr::baseline::fixed_targets(mtr::baseline::default_gateway());
    let logger = mtr::baseline::BaselineLogger::start(std::sync::Arc::new(mtr::trippy_engine::TrippyEngine), targets, sink);
    app.manage(commands::BaselineState(std::sync::Mutex::new(Some(logger))));

    Ok(())
})
.manage(commands::SessionState(std::sync::Mutex::new(None)))
.invoke_handler(tauri::generate_handler![
    commands::start_session,
    commands::stop_session,
    commands::start_baseline,
    commands::stop_baseline
])
```

- [ ] **Step 6: Manual verification**

Run: `cd src-tauri && cargo tauri dev`, leave it running for at least 2
minutes, then check the DB as in Task 11 Step 4 but for
`session_type = 'baseline'` — expect roughly 2 rows per target (one per
60s tick) with `target` values for the gateway, `1.1.1.1`, and `8.8.8.8`.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/mtr/baseline.rs src-tauri/src/mtr/mod.rs src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "Add always-on baseline logger against gateway + fixed public targets"
```

---

## Task 13: Process listing

**Files:**
- Create: `src-tauri/src/procnet/mod.rs`
- Modify: `src-tauri/src/main.rs`
- Test: `src-tauri/src/procnet/mod.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `procnet::ProcessInfo { pid: u32, name: String }`,
  `procnet::Protocol` (enum: `Tcp`, `Udp`), `procnet::Endpoint { remote_ip: String, remote_port: u16, protocol: Protocol }`,
  `procnet::ConnectionQuery` (enum: `Found(Vec<Endpoint>)`, `NoneDetected`)
  — the shared shape both platform backends (Tasks 14, 15) return,
  `procnet::list_processes() -> Vec<ProcessInfo>` (all processes, via
  `sysinfo`; filtering to only those with connections happens in Task 16
  once a platform backend exists to check against).

- [ ] **Step 1: Write the failing test for a small pure helper**

Most of this module is a direct `sysinfo` wrapper (not meaningfully unit
testable — it reflects real OS process state). The one pure piece worth
testing is de-duplicating/sorting process names for display:

```rust
// src-tauri/src/procnet/mod.rs
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Endpoint {
    pub remote_ip: String,
    pub remote_port: u16,
    pub protocol: Protocol,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", content = "endpoints")]
#[serde(rename_all = "snake_case")]
pub enum ConnectionQuery {
    Found(Vec<Endpoint>),
    NoneDetected,
}

fn sort_by_name(mut processes: Vec<ProcessInfo>) -> Vec<ProcessInfo> {
    unimplemented!()
}

pub fn list_processes() -> Vec<ProcessInfo> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_case_insensitively_by_name() {
        let input = vec![
            ProcessInfo { pid: 1, name: "zoom.exe".into() },
            ProcessInfo { pid: 2, name: "Discord.exe".into() },
            ProcessInfo { pid: 3, name: "apex_legends.exe".into() },
        ];
        let sorted = sort_by_name(input);
        assert_eq!(
            sorted.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["apex_legends.exe", "Discord.exe", "zoom.exe"]
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test procnet::tests -- --nocapture`
Expected: FAIL (`not implemented`).

- [ ] **Step 3: Implement**

```rust
// src-tauri/src/procnet/mod.rs (replace the two unimplemented! bodies)
fn sort_by_name(mut processes: Vec<ProcessInfo>) -> Vec<ProcessInfo> {
    processes.sort_by_key(|p| p.name.to_lowercase());
    processes
}

pub fn list_processes() -> Vec<ProcessInfo> {
    use sysinfo::System;

    let mut sys = System::new_all();
    sys.refresh_all();

    let processes = sys
        .processes()
        .values()
        .map(|p| ProcessInfo { pid: p.pid().as_u32(), name: p.name().to_string_lossy().to_string() })
        .collect();

    sort_by_name(processes)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test procnet::tests -- --nocapture`
Expected: PASS (1/1).

- [ ] **Step 5: Register the module**

```rust
// src-tauri/src/main.rs
mod procnet;
```

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/procnet/mod.rs src-tauri/src/main.rs
git commit -m "Add process listing and shared procnet types"
```

---

## Task 14: Windows connection lookup

**Files:**
- Create: `src-tauri/src/procnet/windows.rs`
- Modify: `src-tauri/src/procnet/mod.rs`
- Test: `src-tauri/src/procnet/windows.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `procnet::{Endpoint, Protocol, ConnectionQuery}` (Task 13).
- Produces: `procnet::windows::connections_for_pid(pid: u32) -> ConnectionQuery`,
  and the pure row-parsing helper `procnet::windows::rows_to_endpoints(rows: &[(u32, std::net::IpAddr, u16, Protocol)], pid: u32) -> Vec<Endpoint>`
  that the real `GetExtendedTcpTable`/`GetExtendedUdpTable` calls feed
  through, kept separate specifically so it's testable without a real
  Windows connection table.

- [ ] **Step 1: Write the failing test for the pure filter/map logic**

```rust
// src-tauri/src/procnet/windows.rs
#![cfg(target_os = "windows")]

use std::net::IpAddr;

use crate::procnet::{ConnectionQuery, Endpoint, Protocol};

/// (owning_pid, remote_ip, remote_port, protocol) — the shape both the TCP
/// and UDP table rows reduce to once we've pulled the fields we need out of
/// the raw `MIB_TCPROW_OWNER_PID` / `MIB_UDPROW_OWNER_PID` structs.
type Row = (u32, IpAddr, u16, Protocol);

pub fn rows_to_endpoints(rows: &[Row], pid: u32) -> Vec<Endpoint> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_to_matching_pid_only() {
        let rows = vec![
            (100, "1.1.1.1".parse().unwrap(), 443, Protocol::Tcp),
            (200, "8.8.8.8".parse().unwrap(), 53, Protocol::Udp),
        ];
        let endpoints = rows_to_endpoints(&rows, 100);
        assert_eq!(endpoints, vec![Endpoint { remote_ip: "1.1.1.1".to_string(), remote_port: 443, protocol: Protocol::Tcp }]);
    }

    #[test]
    fn returns_empty_when_pid_has_no_rows() {
        let rows = vec![(200, "8.8.8.8".parse().unwrap(), 53, Protocol::Udp)];
        assert_eq!(rows_to_endpoints(&rows, 999), vec![]);
    }

    #[test]
    fn returns_multiple_endpoints_for_same_pid() {
        let rows = vec![
            (100, "1.1.1.1".parse().unwrap(), 443, Protocol::Tcp),
            (100, "9.9.9.9".parse().unwrap(), 27015, Protocol::Udp),
        ];
        let endpoints = rows_to_endpoints(&rows, 100);
        assert_eq!(endpoints.len(), 2);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run (on Windows): `cd src-tauri && cargo test procnet::windows::tests -- --nocapture`
Expected: FAIL (`not implemented`).

- [ ] **Step 3: Implement the pure logic**

```rust
// src-tauri/src/procnet/windows.rs (replace the unimplemented! body)
pub fn rows_to_endpoints(rows: &[Row], pid: u32) -> Vec<Endpoint> {
    rows.iter()
        .filter(|(owner, ..)| *owner == pid)
        .map(|(_, ip, port, proto)| Endpoint { remote_ip: ip.to_string(), remote_port: *port, protocol: *proto })
        .collect()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run (on Windows): `cd src-tauri && cargo test procnet::windows::tests -- --nocapture`
Expected: PASS (3/3).

- [ ] **Step 5: Implement the real table calls**

```rust
// src-tauri/src/procnet/windows.rs — append below rows_to_endpoints
use windows::Win32::Foundation::NO_ERROR;
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCPTABLE_OWNER_PID, MIB_TCP_TABLE_OWNER_PID_ALL,
    MIB_UDPTABLE_OWNER_PID, UDP_TABLE_OWNER_PID,
};
use windows::Win32::Networking::WinSock::AF_INET;

pub fn connections_for_pid(pid: u32) -> ConnectionQuery {
    let mut rows: Vec<Row> = vec![];
    rows.extend(tcp_rows());
    rows.extend(udp_rows());

    let endpoints = rows_to_endpoints(&rows, pid);
    if endpoints.is_empty() {
        ConnectionQuery::NoneDetected
    } else {
        ConnectionQuery::Found(endpoints)
    }
}

fn tcp_rows() -> Vec<Row> {
    unsafe {
        let mut size: u32 = 0;
        let _ = GetExtendedTcpTable(None, &mut size, false, AF_INET.0 as u32, MIB_TCP_TABLE_OWNER_PID_ALL, 0);
        let mut buf = vec![0u8; size as usize];
        if GetExtendedTcpTable(
            Some(buf.as_mut_ptr() as *mut _),
            &mut size,
            false,
            AF_INET.0 as u32,
            MIB_TCP_TABLE_OWNER_PID_ALL,
            0,
        ) != NO_ERROR.0
        {
            return vec![];
        }

        let table = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
        let entries = std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);
        entries
            .iter()
            .filter(|e| e.dwRemoteAddr != 0) // 0.0.0.0 means "not yet connected"
            .map(|e| {
                let ip = std::net::Ipv4Addr::from(e.dwRemoteAddr.to_be());
                let port = (e.dwRemotePort as u16).to_be();
                (e.dwOwningPid, IpAddr::V4(ip), port, Protocol::Tcp)
            })
            .collect()
    }
}

fn udp_rows() -> Vec<Row> {
    unsafe {
        let mut size: u32 = 0;
        let _ = GetExtendedUdpTable(None, &mut size, false, AF_INET.0 as u32, UDP_TABLE_OWNER_PID, 0);
        let mut buf = vec![0u8; size as usize];
        if GetExtendedUdpTable(
            Some(buf.as_mut_ptr() as *mut _),
            &mut size,
            false,
            AF_INET.0 as u32,
            UDP_TABLE_OWNER_PID,
            0,
        ) != NO_ERROR.0
        {
            return vec![];
        }

        // UDP sockets that never called connect() have no remote endpoint —
        // MIB_UDPROW_OWNER_PID carries no remote address field at all, which
        // is exactly the spec's item 6 limitation. We can enumerate which
        // PIDs have *a* UDP socket open, but not who it's talking to.
        let table = &*(buf.as_ptr() as *const MIB_UDPTABLE_OWNER_PID);
        let _entries = std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);
        vec![] // see Task 16 for how "no remote UDP endpoint" surfaces in the UI
    }
}
```

Note: `udp_rows` deliberately returns no endpoints — `MIB_UDPROW_OWNER_PID`
has no remote-address field, confirming the limitation the spec calls out
in item 6. It's kept as a real (if empty-producing) function rather than
deleted so `connections_for_pid`'s shape doesn't need to change if a future
pass adds a real UDP data source; for now, TCP is the only real source of
`Found(...)` results on Windows.

- [ ] **Step 6: Register the module**

```rust
// src-tauri/src/procnet/mod.rs
#[cfg(target_os = "windows")]
pub mod windows;
```

- [ ] **Step 7: Manual verification (Windows only)**

Run: `cd src-tauri && cargo tauri dev` (elevated). With a browser or game
open, find its PID (Task Manager) and temporarily call
`procnet::windows::connections_for_pid(<pid>)` from a scratch `#[test]` or
a temporary `println!` in `main()`. Expected: at least one TCP endpoint
with a plausible remote IP:port for a process that has an open connection.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/procnet/windows.rs src-tauri/src/procnet/mod.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "Add Windows TCP/UDP connection-table lookup via IP Helper API"
```

---

## Task 15: macOS connection lookup

**Files:**
- Create: `src-tauri/src/procnet/macos.rs`
- Modify: `src-tauri/src/procnet/mod.rs`
- Test: `src-tauri/src/procnet/macos.rs` (inline `#[cfg(test)]`, against captured sample output)

**Interfaces:**
- Consumes: `procnet::{Endpoint, Protocol, ConnectionQuery}` (Task 13).
- Produces: `procnet::macos::connections_for_pid(pid: u32) -> ConnectionQuery`,
  and the pure parser `procnet::macos::parse_lsof_output(raw: &str) -> Vec<Endpoint>`
  that `connections_for_pid` feeds `lsof`'s captured stdout through.

`lsof -i -a -p <pid>` output looks like:

```
COMMAND   PID   USER   FD   TYPE   DEVICE SIZE/OFF NODE NAME
Discord   501   tom    23u  IPv4  0x1234      0t0  TCP  192.168.1.5:54321->104.16.1.1:443 (ESTABLISHED)
Discord   501   tom    25u  IPv4  0x5678      0t0  UDP  192.168.1.5:60000->35.186.224.25:19305
```

- [ ] **Step 1: Write the failing test against captured sample output**

```rust
// src-tauri/src/procnet/macos.rs
#![cfg(target_os = "macos")]

use crate::procnet::{ConnectionQuery, Endpoint, Protocol};

pub fn parse_lsof_output(raw: &str) -> Vec<Endpoint> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
COMMAND   PID   USER   FD   TYPE   DEVICE SIZE/OFF NODE NAME
Discord   501   tom    23u  IPv4  0x1234      0t0  TCP  192.168.1.5:54321->104.16.1.1:443 (ESTABLISHED)
Discord   501   tom    25u  IPv4  0x5678      0t0  UDP  192.168.1.5:60000->35.186.224.25:19305
";

    #[test]
    fn parses_tcp_and_udp_rows() {
        let endpoints = parse_lsof_output(SAMPLE);
        assert_eq!(
            endpoints,
            vec![
                Endpoint { remote_ip: "104.16.1.1".to_string(), remote_port: 443, protocol: Protocol::Tcp },
                Endpoint { remote_ip: "35.186.224.25".to_string(), remote_port: 19305, protocol: Protocol::Udp },
            ]
        );
    }

    #[test]
    fn ignores_the_header_row() {
        let endpoints = parse_lsof_output("COMMAND   PID   USER   FD   TYPE   DEVICE SIZE/OFF NODE NAME\n");
        assert_eq!(endpoints, vec![]);
    }

    #[test]
    fn ignores_rows_without_a_remote_endpoint() {
        // A listening socket, not a connection to a remote host.
        let listening = "Discord   501   tom    10u  IPv4  0x9999      0t0  TCP  *:12345 (LISTEN)\n";
        assert_eq!(parse_lsof_output(listening), vec![]);
    }

    #[test]
    fn handles_empty_output() {
        assert_eq!(parse_lsof_output(""), vec![]);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run (on macOS): `cd src-tauri && cargo test procnet::macos::tests -- --nocapture`
Expected: FAIL (`not implemented`).

- [ ] **Step 3: Implement the parser**

```rust
// src-tauri/src/procnet/macos.rs (replace the unimplemented! body)
pub fn parse_lsof_output(raw: &str) -> Vec<Endpoint> {
    raw.lines()
        .skip(1) // header row
        .filter_map(parse_line)
        .collect()
}

fn parse_line(line: &str) -> Option<Endpoint> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let protocol = match fields.get(7)? {
        &"TCP" => Protocol::Tcp,
        &"UDP" => Protocol::Udp,
        _ => return None,
    };

    let name_field = fields.get(8)?;
    let (_, remote) = name_field.split_once("->")?;
    let remote = remote.split(' ').next()?; // strip a trailing "(ESTABLISHED)" if present
    let (ip, port) = remote.rsplit_once(':')?;

    Some(Endpoint { remote_ip: ip.to_string(), remote_port: port.parse().ok()?, protocol })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run (on macOS): `cd src-tauri && cargo test procnet::macos::tests -- --nocapture`
Expected: PASS (4/4).

- [ ] **Step 5: Implement `connections_for_pid`**

```rust
// src-tauri/src/procnet/macos.rs — append
pub fn connections_for_pid(pid: u32) -> ConnectionQuery {
    let output = std::process::Command::new("lsof")
        .args(["-i", "-a", "-p", &pid.to_string()])
        .output();

    let Ok(output) = output else {
        return ConnectionQuery::NoneDetected;
    };

    let endpoints = parse_lsof_output(&String::from_utf8_lossy(&output.stdout));
    if endpoints.is_empty() {
        ConnectionQuery::NoneDetected
    } else {
        ConnectionQuery::Found(endpoints)
    }
}
```

- [ ] **Step 6: Register the module**

```rust
// src-tauri/src/procnet/mod.rs
#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "macos")]
pub mod macos;
```

- [ ] **Step 7: Manual verification (macOS only)**

Run: `cd src-tauri && cargo tauri dev` (elevated). Pick a PID of a process
with an open connection (`lsof -i -a -p <pid>` in a terminal to confirm),
call `procnet::macos::connections_for_pid(<pid>)` the same way as Task 14
Step 7. Expected: matches what the terminal `lsof` command showed.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/procnet/macos.rs src-tauri/src/procnet/mod.rs
git commit -m "Add macOS connection lookup via lsof parsing"
```

---

## Task 16: Unified connection commands and process-picker UI

**Files:**
- Modify: `src-tauri/src/procnet/mod.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs`
- Create: `src/process-picker.js`
- Modify: `src/index.html`

**Interfaces:**
- Consumes: `procnet::{list_processes, ProcessInfo, ConnectionQuery}` (Task
  13), `procnet::windows::connections_for_pid` / `procnet::macos::connections_for_pid`
  (Tasks 14/15).
- Produces: `procnet::connections_for_pid(pid: u32) -> ConnectionQuery`
  (the one cross-platform entry point — dispatches to whichever OS module
  is compiled in), Tauri commands
  `list_candidate_processes() -> Vec<ProcessInfo>` (processes filtered to
  those with at least one connection) and
  `list_connections_for_pid(pid: u32) -> ConnectionQuery`.

- [ ] **Step 1: Add the cross-platform dispatch function**

```rust
// src-tauri/src/procnet/mod.rs — append
pub fn connections_for_pid(pid: u32) -> ConnectionQuery {
    #[cfg(target_os = "windows")]
    return windows::connections_for_pid(pid);

    #[cfg(target_os = "macos")]
    return macos::connections_for_pid(pid);
}

pub fn list_candidate_processes() -> Vec<ProcessInfo> {
    list_processes()
        .into_iter()
        .filter(|p| matches!(connections_for_pid(p.pid), ConnectionQuery::Found(_)))
        .collect()
}
```

No new unit test: this composes two already-tested pieces (Task 13's
`list_processes`, Tasks 14/15's platform `connections_for_pid`) and its
correctness is really "does it reflect real OS state," which is the
manual-QA territory those tasks already covered.

- [ ] **Step 2: Add the commands**

```rust
// src-tauri/src/commands.rs — add
use crate::procnet::{ConnectionQuery, ProcessInfo};

#[tauri::command]
pub fn list_candidate_processes() -> Vec<ProcessInfo> {
    crate::procnet::list_candidate_processes()
}

#[tauri::command]
pub fn list_connections_for_pid(pid: u32) -> ConnectionQuery {
    crate::procnet::connections_for_pid(pid)
}
```

```rust
// src-tauri/src/main.rs — add to invoke_handler!
commands::list_candidate_processes,
commands::list_connections_for_pid
```

- [ ] **Step 3: Write the process-picker UI**

```js
// src/process-picker.js
async function loadProcessList() {
  const processes = await window.__TAURI__.core.invoke("list_candidate_processes");
  const select = document.getElementById("process-select");
  select.innerHTML = processes
    .map((p) => `<option value="${p.pid}">${p.name} (pid ${p.pid})</option>`)
    .join("");
}

async function loadConnectionsForSelectedProcess() {
  const pid = Number(document.getElementById("process-select").value);
  const result = await window.__TAURI__.core.invoke("list_connections_for_pid", { pid });
  const container = document.getElementById("connection-list");

  if (result.status === "none_detected") {
    container.innerHTML =
      "<p>No active connections detected. If this is a game using unconnected UDP sockets, " +
      "Packet Proof can't see its remote server through OS connection tables in this version " +
      "— enter the target IP manually instead.</p>";
    return;
  }

  container.innerHTML = result.endpoints
    .map(
      (e) => `
      <label>
        <input type="radio" name="candidate-target" value="${e.remote_ip}" />
        ${e.remote_ip}:${e.remote_port} (${e.protocol.toUpperCase()})
      </label>`
    )
    .join("<br/>");
}

window.loadProcessList = loadProcessList;
window.loadConnectionsForSelectedProcess = loadConnectionsForSelectedProcess;
```

- [ ] **Step 4: Wire it into `index.html`**

```html
<!-- src/index.html — add above the existing target-input row -->
<div>
  <button onclick="loadProcessList()">Refresh process list</button>
  <select id="process-select" onchange="loadConnectionsForSelectedProcess()"></select>
  <div id="connection-list"></div>
</div>
<script src="process-picker.js"></script>
```

Update the Start button (from Task 9) to prefer a selected radio target
when one exists:

```js
// src/dashboard.js — replace startSession's caller in index.html's onclick, or add a helper
function currentTarget() {
  const picked = document.querySelector('input[name="candidate-target"]:checked');
  return picked ? picked.value : document.getElementById("target-input").value;
}
window.currentTarget = currentTarget;
```

```html
<!-- src/index.html — update the Start button -->
<button onclick="startSession(currentTarget())">Start</button>
```

- [ ] **Step 5: Manual verification**

Run: `cd src-tauri && cargo tauri dev`. Click "Refresh process list",
confirm it populates with real running processes; pick one with a known
open connection (e.g. a browser), confirm its remote endpoints list;
select one and click Start, confirm the session traces to that IP. Pick a
process you know has no active connections and confirm the "no active
connections detected" message appears instead of an empty list.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/procnet/mod.rs src-tauri/src/commands.rs src-tauri/src/main.rs src/process-picker.js src/index.html src/dashboard.js
git commit -m "Add unified connection commands and process-picker UI"
```

---

## Task 17: Session controls — duration presets and custom duration

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/mtr/session.rs`
- Modify: `src/dashboard.js`
- Modify: `src/index.html`

**Interfaces:**
- Consumes: `SessionRunner::start`/`stop` (Task 9/11).
- Produces: `start_session` command gains a `duration_secs: Option<u64>`
  parameter; when present, the session auto-stops after that many seconds.
  `SessionRunner` gains an optional auto-stop timer.

- [ ] **Step 1: Add auto-stop to `SessionRunner`**

```rust
// src-tauri/src/mtr/session.rs — extend start()'s signature and body
impl SessionRunner {
    pub fn start(
        engine: Arc<dyn ProbeEngine>,
        target: IpAddr,
        on_snapshot: Box<dyn Fn(Vec<HopSnapshot>) + Send + Sync>,
        record_sink: Sender<ProbeRecord>,
        duration: Option<Duration>,
    ) -> Self {
        let shared = Arc::new(Shared {
            engine,
            target,
            target_label: target.to_string(),
            session_id: uuid::Uuid::new_v4().to_string(),
            aggregator: Mutex::new(HopAggregator::new()),
            rate: Mutex::new(AdaptiveRate::new()),
            on_snapshot,
            current_handle: Mutex::new(None),
            record_sink,
        });

        spawn_with_current_interval(&shared);

        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        if let Some(duration) = duration {
            let shared_for_timer = shared.clone();
            let stop_flag_for_timer = stop_flag.clone();
            std::thread::spawn(move || {
                std::thread::sleep(duration);
                if !stop_flag_for_timer.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Some(handle) = shared_for_timer.current_handle.lock().unwrap().take() {
                        (handle.stop)();
                    }
                }
            });
        }

        SessionRunner {
            stop: Some(Box::new(move || {
                stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                if let Some(handle) = shared.current_handle.lock().unwrap().take() {
                    (handle.stop)();
                }
            })),
        }
    }
}
```

Update every existing `SessionRunner::start` call site to add the new
final argument — that's four in total: the three tests in `mtr::session`'s
`mod tests` (Task 9) plus `commands::start_session` (Task 11). All three
tests pass `None`; `commands::start_session` is handled in Step 4 below,
which passes the real duration through instead.

- [ ] **Step 2: Add a test for the new parameter's plumbing**

```rust
// src-tauri/src/mtr/session.rs, inside mod tests
#[test]
fn accepts_a_duration_without_panicking() {
    let engine = FakeEngine::new();
    let (tx, _rx) = std::sync::mpsc::channel();
    let runner = SessionRunner::start(
        engine,
        "1.1.1.1".parse().unwrap(),
        Box::new(|_| {}),
        tx,
        Some(Duration::from_millis(50)),
    );
    std::thread::sleep(Duration::from_millis(100));
    runner.stop(); // must not panic even though the auto-stop timer already fired
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cd src-tauri && cargo test mtr::session::tests -- --nocapture`
Expected: PASS (5/5).

- [ ] **Step 4: Update `start_session`**

```rust
// src-tauri/src/commands.rs
#[tauri::command]
pub fn start_session(
    app: AppHandle,
    state: State<SessionState>,
    db_sink: State<DbSink>,
    target: String,
    duration_secs: Option<u64>,
) -> Result<(), String> {
    let target: IpAddr = target.parse().map_err(|_| "invalid target IP".to_string())?;
    let mut guard = state.0.lock().unwrap();
    if guard.is_some() {
        return Err("a session is already running".to_string());
    }

    let app_for_callback = app.clone();
    let runner = SessionRunner::start(
        Arc::new(TrippyEngine),
        target,
        Box::new(move |snapshot| {
            let _ = app_for_callback.emit("hop-update", snapshot);
        }),
        db_sink.0.clone(),
        duration_secs.map(std::time::Duration::from_secs),
    );
    *guard = Some(runner);
    Ok(())
}
```

- [ ] **Step 5: Add duration presets to the UI**

```html
<!-- src/index.html — replace the Start/Stop button row -->
<div>
  <select id="duration-select">
    <option value="900">15 min</option>
    <option value="1800">30 min</option>
    <option value="3600">60 min</option>
    <option value="custom">Custom…</option>
  </select>
  <input id="custom-duration-input" type="number" min="1" placeholder="minutes" style="display:none" />
  <button onclick="startSession(currentTarget(), currentDurationSecs())">Start</button>
  <button onclick="stopSession()">Stop</button>
</div>
```

```js
// src/dashboard.js
document.addEventListener("DOMContentLoaded", () => {
  const durationSelect = document.getElementById("duration-select");
  const customInput = document.getElementById("custom-duration-input");
  durationSelect?.addEventListener("change", () => {
    customInput.style.display = durationSelect.value === "custom" ? "inline" : "none";
  });
});

function currentDurationSecs() {
  const select = document.getElementById("duration-select");
  if (select.value === "custom") {
    const minutes = Number(document.getElementById("custom-duration-input").value || 0);
    return minutes > 0 ? minutes * 60 : null;
  }
  return Number(select.value);
}
window.currentDurationSecs = currentDurationSecs;

async function startSession(target, durationSecs) {
  await window.__TAURI__.core.invoke("start_session", { target, durationSecs });
}
```

- [ ] **Step 6: Manual verification**

Run: `cd src-tauri && cargo tauri dev`. Start a session with the 15-min
preset — confirm it's running; stop it manually to avoid waiting 15
minutes. Then start one with a short custom duration (e.g. type `1` for 1
minute, or temporarily test with a code-level short duration) and confirm
it auto-stops (the hop table stops updating) without you clicking Stop.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/mtr/session.rs src/dashboard.js src/index.html
git commit -m "Add timed-session duration presets and custom duration with auto-stop"
```

---

## Task 18: History query commands

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/db.rs`
- Test: `src-tauri/src/db.rs` (extend existing `#[cfg(test)]`)

**Interfaces:**
- Consumes: `db::{Db, ProbeRecord}` (Task 10).
- Produces: `db::ProbeRecord` gains `serde::Serialize`; commands
  `get_baseline_history(target: String, since: i64, until: i64) -> Vec<ProbeRecord>`
  and `get_session_history(session_id: String) -> Vec<ProbeRecord>` (the
  latter needs a new `Db` query since `query_history`, Task 10, filters by
  target+time range, not by session id).

- [ ] **Step 1: Write the failing test for the new session-id query**

```rust
// src-tauri/src/db.rs — add to impl Db
pub fn query_session(&self, session_id: &str) -> rusqlite::Result<Vec<ProbeRecord>> {
    unimplemented!()
}
```

```rust
// src-tauri/src/db.rs, inside mod tests
#[test]
fn query_session_returns_only_that_sessions_records() {
    let db = test_db();
    let mut a = sample("timed", "1.1.1.1", 1000, 1);
    a.session_id = Some("session-a".to_string());
    let mut b = sample("timed", "1.1.1.1", 1000, 1);
    b.session_id = Some("session-b".to_string());
    db.insert_record(&a).unwrap();
    db.insert_record(&b).unwrap();

    let results = db.query_session("session-a").unwrap();
    assert_eq!(results, vec![a]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test db::tests -- --nocapture`
Expected: FAIL (`not implemented`) on the new test; the other 4 still pass.

- [ ] **Step 3: Implement**

```rust
// src-tauri/src/db.rs (replace the unimplemented! body)
pub fn query_session(&self, session_id: &str) -> rusqlite::Result<Vec<ProbeRecord>> {
    let mut stmt = self.conn.prepare(
        "SELECT session_type, target, session_id, timestamp, hop_index, hop_ip,
                rtt_best_ms, rtt_avg_ms, rtt_worst_ms, loss_pct, zone
         FROM probe_records
         WHERE session_id = ?1
         ORDER BY timestamp ASC, hop_index ASC",
    )?;

    let rows = stmt.query_map(rusqlite::params![session_id], |row| {
        Ok(ProbeRecord {
            session_type: row.get(0)?,
            target: row.get(1)?,
            session_id: row.get(2)?,
            timestamp: row.get(3)?,
            hop_index: row.get(4)?,
            hop_ip: row.get(5)?,
            rtt_best_ms: row.get(6)?,
            rtt_avg_ms: row.get(7)?,
            rtt_worst_ms: row.get(8)?,
            loss_pct: row.get(9)?,
            zone: row.get(10)?,
        })
    })?;

    rows.collect()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test db::tests -- --nocapture`
Expected: PASS (5/5).

- [ ] **Step 5: Make `ProbeRecord` serializable and add commands**

```rust
// src-tauri/src/db.rs
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProbeRecord {
```

```rust
// src-tauri/src/commands.rs — add
pub struct DbHandle(pub Arc<crate::db::Db>);

#[tauri::command]
pub fn get_baseline_history(db: State<DbHandle>, target: String, since: i64, until: i64) -> Result<Vec<crate::db::ProbeRecord>, String> {
    db.0.query_history(&target, Some("baseline"), since, until).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_session_history(db: State<DbHandle>, session_id: String) -> Result<Vec<crate::db::ProbeRecord>, String> {
    db.0.query_session(&session_id).map_err(|e| e.to_string())
}
```

This introduces a second, read-only `Db` handle alongside the write-only
`db_writer` actor's connection — SQLite's WAL mode (Task 10) allows a
concurrent reader and writer safely. Wire it into `main.rs`'s `setup`:

```rust
// src-tauri/src/main.rs — inside .setup(|app| { ... })
let read_db = std::sync::Arc::new(db::Db::open(&db_path)?);
app.manage(commands::DbHandle(read_db));
```

```rust
// src-tauri/src/main.rs — add to invoke_handler!
commands::get_baseline_history,
commands::get_session_history
```

- [ ] **Step 6: Manual verification**

Run: `cd src-tauri && cargo tauri dev`, let the baseline run for a couple
of minutes, then from the browser dev tools console (Tauri windows support
right-click → Inspect in dev builds) run:
`await window.__TAURI__.core.invoke("get_baseline_history", { target: "1.1.1.1", since: 0, until: Math.floor(Date.now()/1000) })`
Expected: an array of the baseline records logged so far for `1.1.1.1`.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/db.rs src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "Add history query commands backing the dashboard charts"
```

---

## Task 19: Historical charts UI

**Files:**
- Create: `src/charts.js`
- Modify: `src/index.html`

**Interfaces:**
- Consumes: `get_baseline_history` command (Task 18).
- Produces: a dependency-free canvas line chart rendering RTT-avg and
  loss % over a selectable time range, for the baseline data.

A hand-rolled canvas renderer is used instead of pulling in a charting
library — the frontend has no bundler (Task 1's `withGlobalTauri`
plain-script setup) and the chart need here (two line series over time) is
simple enough not to need one.

- [ ] **Step 1: Write the chart renderer**

```js
// src/charts.js
function drawLineChart(canvas, series, { color, yMax, yMin = 0 }) {
  const ctx = canvas.getContext("2d");
  const { width, height } = canvas;
  ctx.clearRect(0, 0, width, height);

  if (series.length < 2) return;

  const xs = series.map((p) => p.x);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const spanX = maxX - minX || 1;
  const spanY = yMax - yMin || 1;

  ctx.strokeStyle = color;
  ctx.lineWidth = 2;
  ctx.beginPath();
  series.forEach((point, i) => {
    const px = ((point.x - minX) / spanX) * width;
    const py = height - ((point.y - yMin) / spanY) * height;
    if (i === 0) ctx.moveTo(px, py);
    else ctx.lineTo(px, py);
  });
  ctx.stroke();
}

async function loadBaselineCharts() {
  const target = document.getElementById("chart-target-select").value;
  const rangeHours = Number(document.getElementById("chart-range-select").value);
  const until = Math.floor(Date.now() / 1000);
  const since = until - rangeHours * 3600;

  const records = await window.__TAURI__.core.invoke("get_baseline_history", { target, since, until });

  // One record per hop per tick; the chart cares about the final
  // (highest-index) hop per timestamp, i.e. reachability of the target itself.
  const byTimestamp = new Map();
  for (const r of records) {
    const existing = byTimestamp.get(r.timestamp);
    if (!existing || r.hop_index > existing.hop_index) {
      byTimestamp.set(r.timestamp, r);
    }
  }
  const finalHopRecords = [...byTimestamp.values()].sort((a, b) => a.timestamp - b.timestamp);

  const rttSeries = finalHopRecords
    .filter((r) => r.rtt_avg_ms != null)
    .map((r) => ({ x: r.timestamp, y: r.rtt_avg_ms }));
  const lossSeries = finalHopRecords.map((r) => ({ x: r.timestamp, y: r.loss_pct }));

  const maxRtt = Math.max(10, ...rttSeries.map((p) => p.y));
  drawLineChart(document.getElementById("rtt-chart"), rttSeries, { color: "#1976d2", yMax: maxRtt });
  drawLineChart(document.getElementById("loss-chart"), lossSeries, { color: "#d32f2f", yMax: 100 });
}

window.loadBaselineCharts = loadBaselineCharts;
```

No unit test: this is DOM/canvas rendering with no meaningful logic beyond
straightforward point-to-pixel mapping, verified visually in Step 3.

- [ ] **Step 2: Wire it into `index.html`**

```html
<!-- src/index.html — add a new section below the hop table -->
<h2>Baseline history</h2>
<div>
  <select id="chart-target-select">
    <option value="1.1.1.1">1.1.1.1</option>
    <option value="8.8.8.8">8.8.8.8</option>
  </select>
  <select id="chart-range-select">
    <option value="6">6 hours</option>
    <option value="24" selected>24 hours</option>
    <option value="168">7 days</option>
  </select>
  <button onclick="loadBaselineCharts()">Refresh</button>
</div>
<p>Avg RTT (ms)</p>
<canvas id="rtt-chart" width="800" height="150"></canvas>
<p>Loss %</p>
<canvas id="loss-chart" width="800" height="150"></canvas>
<script src="charts.js"></script>
```

The gateway target isn't in the dropdown because its IP varies per
network — a future pass could populate it dynamically via a command that
exposes `mtr::baseline::default_gateway()`; out of scope for the MVP chart
view.

- [ ] **Step 3: Manual verification**

Run: `cd src-tauri && cargo tauri dev`, let the baseline log a few data
points (wait a few minutes, or temporarily shorten the 60s interval in
Task 12 for faster local testing, then revert), click Refresh. Expected:
two line charts render showing RTT and loss over the selected range, and
switching the target/range dropdowns and clicking Refresh updates them.

- [ ] **Step 4: Commit**

```bash
git add src/charts.js src/index.html
git commit -m "Add historical baseline RTT/loss charts to the dashboard"
```

---

## Task 20: PDF session report

**Files:**
- Create: `src-tauri/src/report.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src/dashboard.js`
- Modify: `src/index.html`
- Test: `src-tauri/src/report.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `db::ProbeRecord` (Task 10).
- Produces: `report::SessionSummary { target: String, session_id: String, hop_count: usize, avg_loss_pct: f64, worst_hop_rtt_ms: Option<f64> }`,
  `report::summarize_session(records: &[ProbeRecord]) -> SessionSummary`
  (pure — the testable part), `report::generate_session_report(records: &[ProbeRecord], out_path: &std::path::Path) -> Result<(), String>`
  (genpdf rendering — not unit tested, verified manually). Command
  `export_session_report(session_id: String) -> Result<String, String>`
  (returns the saved file path).

- [ ] **Step 1: Write the failing test for the pure summary logic**

```rust
// src-tauri/src/report.rs
use crate::db::ProbeRecord;

#[derive(Debug, PartialEq)]
pub struct SessionSummary {
    pub target: String,
    pub session_id: String,
    pub hop_count: usize,
    pub avg_loss_pct: f64,
    pub worst_hop_rtt_ms: Option<f64>,
}

pub fn summarize_session(records: &[ProbeRecord]) -> SessionSummary {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(hop: u8, loss: f64, worst: Option<f64>) -> ProbeRecord {
        ProbeRecord {
            session_type: "timed".to_string(),
            target: "1.1.1.1".to_string(),
            session_id: Some("s1".to_string()),
            timestamp: 1000,
            hop_index: hop,
            hop_ip: Some("1.1.1.1".to_string()),
            rtt_best_ms: Some(1.0),
            rtt_avg_ms: Some(2.0),
            rtt_worst_ms: worst,
            loss_pct: loss,
            zone: "beyond_isp".to_string(),
        }
    }

    #[test]
    fn summarizes_hop_count_avg_loss_and_worst_rtt() {
        let records = vec![record(1, 0.0, Some(5.0)), record(2, 10.0, Some(40.0)), record(3, 20.0, Some(15.0))];
        let summary = summarize_session(&records);
        assert_eq!(summary.target, "1.1.1.1");
        assert_eq!(summary.session_id, "s1");
        assert_eq!(summary.hop_count, 3);
        assert!((summary.avg_loss_pct - 10.0).abs() < 0.001);
        assert_eq!(summary.worst_hop_rtt_ms, Some(40.0));
    }

    #[test]
    fn handles_no_records() {
        let summary = summarize_session(&[]);
        assert_eq!(summary.hop_count, 0);
        assert_eq!(summary.avg_loss_pct, 0.0);
        assert_eq!(summary.worst_hop_rtt_ms, None);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test report::tests -- --nocapture`
Expected: FAIL (`not implemented`).

- [ ] **Step 3: Implement the summary logic**

```rust
// src-tauri/src/report.rs (replace the unimplemented! body)
pub fn summarize_session(records: &[ProbeRecord]) -> SessionSummary {
    if records.is_empty() {
        return SessionSummary {
            target: String::new(),
            session_id: String::new(),
            hop_count: 0,
            avg_loss_pct: 0.0,
            worst_hop_rtt_ms: None,
        };
    }

    let target = records[0].target.clone();
    let session_id = records[0].session_id.clone().unwrap_or_default();
    let hop_count = records.iter().map(|r| r.hop_index).collect::<std::collections::HashSet<_>>().len();
    let avg_loss_pct = records.iter().map(|r| r.loss_pct).sum::<f64>() / records.len() as f64;
    let worst_hop_rtt_ms = records.iter().filter_map(|r| r.rtt_worst_ms).fold(None, |acc: Option<f64>, v| {
        Some(acc.map_or(v, |a| a.max(v)))
    });

    SessionSummary { target, session_id, hop_count, avg_loss_pct, worst_hop_rtt_ms }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test report::tests -- --nocapture`
Expected: PASS (2/2).

- [ ] **Step 5: Implement PDF generation**

```rust
// src-tauri/src/report.rs — append
use genpdf::elements::{Break, Paragraph, TableLayout};
use genpdf::{Document, SimplePageDecorator};

pub fn generate_session_report(records: &[ProbeRecord], out_path: &std::path::Path) -> Result<(), String> {
    let summary = summarize_session(records);

    let font_family = genpdf::fonts::from_files("./fonts", "LiberationSans", None)
        .map_err(|e| format!("failed to load fonts: {e}"))?;
    let mut doc = Document::new(font_family);
    doc.set_title("Packet Proof — Session Report");
    let mut decorator = SimplePageDecorator::new();
    decorator.set_margins(10);
    doc.set_page_decorator(decorator);

    doc.push(Paragraph::new(format!("Session Report — {}", summary.target)));
    doc.push(Paragraph::new(format!("Session ID: {}", summary.session_id)));
    doc.push(Paragraph::new(format!("Hops: {}", summary.hop_count)));
    doc.push(Paragraph::new(format!("Average loss: {:.1}%", summary.avg_loss_pct)));
    doc.push(Paragraph::new(format!(
        "Worst hop RTT: {}",
        summary.worst_hop_rtt_ms.map(|v| format!("{v:.1}ms")).unwrap_or_else(|| "—".to_string())
    )));
    doc.push(Break::new(1));

    let mut table = TableLayout::new(vec![1, 2, 1, 1, 1, 1, 1]);
    table.set_cell_decorator(genpdf::elements::FrameCellDecorator::new(true, true, false));
    let header = ["Hop", "IP", "Best", "Avg", "Worst", "Loss %", "Zone"];
    let mut row = table.row();
    for h in header {
        row.push_element(Paragraph::new(h));
    }
    row.push().map_err(|e| format!("{e}"))?;

    let mut by_hop: Vec<&ProbeRecord> = records.iter().collect();
    by_hop.sort_by_key(|r| r.hop_index);
    by_hop.dedup_by_key(|r| r.hop_index);

    for r in by_hop {
        let mut row = table.row();
        row.push_element(Paragraph::new(r.hop_index.to_string()));
        row.push_element(Paragraph::new(r.hop_ip.clone().unwrap_or_else(|| "—".to_string())));
        row.push_element(Paragraph::new(r.rtt_best_ms.map(|v| format!("{v:.1}")).unwrap_or_else(|| "—".to_string())));
        row.push_element(Paragraph::new(r.rtt_avg_ms.map(|v| format!("{v:.1}")).unwrap_or_else(|| "—".to_string())));
        row.push_element(Paragraph::new(r.rtt_worst_ms.map(|v| format!("{v:.1}")).unwrap_or_else(|| "—".to_string())));
        row.push_element(Paragraph::new(format!("{:.0}%", r.loss_pct)));
        row.push_element(Paragraph::new(r.zone.clone()));
        row.push().map_err(|e| format!("{e}"))?;
    }
    doc.push(table);

    doc.render_to_file(out_path).map_err(|e| format!("failed to write pdf: {e}"))
}
```

`genpdf` needs a font file at build/run time. Add a small bundled font:

```bash
mkdir -p src-tauri/fonts
```

Download the four Liberation Sans TTF files (`LiberationSans-Regular.ttf`,
`-Bold.ttf`, `-Italic.ttf`, `-BoldItalic.ttf` — SIL Open Font License, safe
to bundle) into `src-tauri/fonts/`, from
`https://github.com/liberationfonts/liberation-fonts/releases` (the
"Liberation Fonts" TTF release archive). `genpdf::fonts::from_files`
expects exactly those four filenames alongside the given family name.

- [ ] **Step 6: Register the module and add the command**

```rust
// src-tauri/src/main.rs
mod report;
```

```rust
// src-tauri/src/commands.rs — add
#[tauri::command]
pub fn export_session_report(app: AppHandle, db: State<DbHandle>, session_id: String) -> Result<String, String> {
    let records = db.0.query_session(&session_id).map_err(|e| e.to_string())?;
    if records.is_empty() {
        return Err("no records found for that session".to_string());
    }

    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("reports");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out_path = dir.join(format!("session-{session_id}.pdf"));

    crate::report::generate_session_report(&records, &out_path)?;
    Ok(out_path.to_string_lossy().to_string())
}
```

```rust
// src-tauri/src/main.rs — add to invoke_handler!
commands::export_session_report
```

- [ ] **Step 7: Track the current session ID in the frontend so export has something to call with**

```js
// src/dashboard.js
let currentSessionId = null;

window.__TAURI__.event.listen("session-started", (event) => {
  currentSessionId = event.payload.session_id;
});

async function exportSessionReport() {
  if (!currentSessionId) {
    alert("Start a session first.");
    return;
  }
  const path = await window.__TAURI__.core.invoke("export_session_report", { sessionId: currentSessionId });
  alert(`Report saved to ${path}`);
}
window.exportSessionReport = exportSessionReport;
```

`SessionRunner` (Task 9/11) doesn't currently expose its generated
`session_id` back to the caller — add that:

```rust
// src-tauri/src/mtr/session.rs — SessionRunner gains a field and getter
pub struct SessionRunner {
    stop: Option<Box<dyn FnOnce() + Send>>,
    pub session_id: String,
}
```

Set it in `start()` (`session_id: shared.session_id.clone()` alongside the
existing `stop: Some(...)` in the returned struct), and emit it from
`commands::start_session`:

```rust
// src-tauri/src/commands.rs — in start_session, after building `runner`
let _ = app.emit("session-started", serde_json::json!({ "session_id": runner.session_id }));
*guard = Some(runner);
```

- [ ] **Step 8: Add the export button**

```html
<!-- src/index.html — next to the Stop button -->
<button onclick="exportSessionReport()">Export session PDF</button>
```

- [ ] **Step 9: Manual verification**

Run: `cd src-tauri && cargo tauri dev`, start a short session, let it
gather a few rounds, stop it, click "Export session PDF". Expected: an
alert with a file path; open that PDF and confirm it shows the target,
session ID, summary stats, and a hop table with zone tags matching what
the dashboard showed.

- [ ] **Step 10: Commit**

```bash
git add src-tauri/src/report.rs src-tauri/src/commands.rs src-tauri/src/main.rs src-tauri/src/mtr/session.rs src-tauri/fonts src/dashboard.js src/index.html
git commit -m "Add PDF session report generation and export command"
```

---

## Task 21: PDF trend report

**Files:**
- Modify: `src-tauri/src/report.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src/dashboard.js`
- Modify: `src/index.html`
- Test: `src-tauri/src/report.rs` (extend existing `#[cfg(test)]`)

**Interfaces:**
- Consumes: `db::ProbeRecord` (Task 10), `report::SessionSummary`-style
  pattern from Task 20.
- Produces: `report::TrendPoint { timestamp: i64, rtt_avg_ms: Option<f64>, loss_pct: f64 }`,
  `report::records_to_trend_points(records: &[ProbeRecord]) -> Vec<TrendPoint>`
  (pure — the testable part; collapses per-hop records down to one point
  per timestamp using the final hop, same rule as Task 19's chart),
  `report::generate_trend_report(records: &[ProbeRecord], target: &str, range_label: &str, out_path: &Path) -> Result<(), String>`
  (plotters PNG + genpdf — not unit tested). Command
  `export_trend_report(target: String, since: i64, until: i64, range_label: String) -> Result<String, String>`.

- [ ] **Step 1: Write the failing test for the pure point-collapsing logic**

```rust
// src-tauri/src/report.rs — add near the top
#[derive(Debug, PartialEq)]
pub struct TrendPoint {
    pub timestamp: i64,
    pub rtt_avg_ms: Option<f64>,
    pub loss_pct: f64,
}

pub fn records_to_trend_points(records: &[ProbeRecord]) -> Vec<TrendPoint> {
    unimplemented!()
}
```

```rust
// src-tauri/src/report.rs, inside mod tests
fn hop_record(ts: i64, hop: u8, loss: f64) -> ProbeRecord {
    ProbeRecord {
        session_type: "baseline".to_string(),
        target: "1.1.1.1".to_string(),
        session_id: None,
        timestamp: ts,
        hop_index: hop,
        hop_ip: Some("1.1.1.1".to_string()),
        rtt_best_ms: Some(1.0),
        rtt_avg_ms: Some(10.0 + hop as f64),
        rtt_worst_ms: Some(20.0),
        loss_pct: loss,
        zone: "beyond_isp".to_string(),
    }
}

#[test]
fn collapses_to_one_point_per_timestamp_using_the_final_hop() {
    let records = vec![hop_record(1000, 1, 0.0), hop_record(1000, 3, 5.0), hop_record(1000, 2, 0.0)];
    let points = records_to_trend_points(&records);
    assert_eq!(points, vec![TrendPoint { timestamp: 1000, rtt_avg_ms: Some(13.0), loss_pct: 5.0 }]);
}

#[test]
fn sorts_points_by_timestamp() {
    let records = vec![hop_record(2000, 1, 0.0), hop_record(1000, 1, 0.0)];
    let points = records_to_trend_points(&records);
    assert_eq!(points.iter().map(|p| p.timestamp).collect::<Vec<_>>(), vec![1000, 2000]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test report::tests -- --nocapture`
Expected: FAIL (`not implemented`) on the two new tests; the earlier two
still pass.

- [ ] **Step 3: Implement**

```rust
// src-tauri/src/report.rs (replace the unimplemented! body)
pub fn records_to_trend_points(records: &[ProbeRecord]) -> Vec<TrendPoint> {
    use std::collections::BTreeMap;

    let mut by_timestamp: BTreeMap<i64, &ProbeRecord> = BTreeMap::new();
    for r in records {
        by_timestamp
            .entry(r.timestamp)
            .and_modify(|existing| {
                if r.hop_index > existing.hop_index {
                    *existing = r;
                }
            })
            .or_insert(r);
    }

    by_timestamp
        .into_iter()
        .map(|(timestamp, r)| TrendPoint { timestamp, rtt_avg_ms: r.rtt_avg_ms, loss_pct: r.loss_pct })
        .collect()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test report::tests -- --nocapture`
Expected: PASS (4/4).

- [ ] **Step 5: Implement chart rasterization and PDF assembly**

```rust
// src-tauri/src/report.rs — append
use plotters::prelude::*;

fn render_trend_chart_png(points: &[TrendPoint], out_path: &std::path::Path) -> Result<(), String> {
    let root = BitMapBackend::new(out_path, (800, 300)).into_drawing_area();
    root.fill(&WHITE).map_err(|e| e.to_string())?;

    let min_x = points.first().map(|p| p.timestamp).unwrap_or(0);
    let max_x = points.last().map(|p| p.timestamp).unwrap_or(1);
    let max_y = points.iter().filter_map(|p| p.rtt_avg_ms).fold(10.0, f64::max);

    let mut chart = ChartBuilder::on(&root)
        .margin(10)
        .x_label_area_size(30)
        .y_label_area_size(40)
        .build_cartesian_2d(min_x..max_x.max(min_x + 1), 0.0..max_y)
        .map_err(|e| e.to_string())?;

    chart.configure_mesh().draw().map_err(|e| e.to_string())?;
    chart
        .draw_series(LineSeries::new(
            points.iter().filter_map(|p| p.rtt_avg_ms.map(|v| (p.timestamp, v))),
            &BLUE,
        ))
        .map_err(|e| e.to_string())?;

    root.present().map_err(|e| e.to_string())
}

pub fn generate_trend_report(
    records: &[ProbeRecord],
    target: &str,
    range_label: &str,
    out_path: &std::path::Path,
) -> Result<(), String> {
    let points = records_to_trend_points(records);

    let chart_path = out_path.with_extension("png");
    render_trend_chart_png(&points, &chart_path)?;

    let font_family = genpdf::fonts::from_files("./fonts", "LiberationSans", None)
        .map_err(|e| format!("failed to load fonts: {e}"))?;
    let mut doc = Document::new(font_family);
    doc.set_title("Packet Proof — Trend Report");
    let mut decorator = SimplePageDecorator::new();
    decorator.set_margins(10);
    doc.set_page_decorator(decorator);

    doc.push(Paragraph::new(format!("Trend Report — {target}")));
    doc.push(Paragraph::new(format!("Range: {range_label}")));
    doc.push(Paragraph::new(format!("Data points: {}", points.len())));
    doc.push(Break::new(1));

    let image = genpdf::elements::Image::from_path(&chart_path).map_err(|e| format!("failed to embed chart: {e}"))?;
    doc.push(image);

    doc.render_to_file(out_path).map_err(|e| format!("failed to write pdf: {e}"))?;
    let _ = std::fs::remove_file(&chart_path);
    Ok(())
}
```

- [ ] **Step 6: Add the command**

```rust
// src-tauri/src/commands.rs — add
#[tauri::command]
pub fn export_trend_report(
    app: AppHandle,
    db: State<DbHandle>,
    target: String,
    since: i64,
    until: i64,
    range_label: String,
) -> Result<String, String> {
    let records = db.0.query_history(&target, Some("baseline"), since, until).map_err(|e| e.to_string())?;
    if records.is_empty() {
        return Err("no baseline records found for that range".to_string());
    }

    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("reports");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let safe_target = target.replace(':', "_").replace('.', "-");
    let out_path = dir.join(format!("trend-{safe_target}.pdf"));

    crate::report::generate_trend_report(&records, &target, &range_label, &out_path)?;
    Ok(out_path.to_string_lossy().to_string())
}
```

```rust
// src-tauri/src/main.rs — add to invoke_handler!
commands::export_trend_report
```

- [ ] **Step 7: Add the export button next to the charts**

```html
<!-- src/index.html — next to the chart Refresh button -->
<button onclick="exportTrendReport()">Export trend PDF</button>
```

```js
// src/charts.js — append
async function exportTrendReport() {
  const target = document.getElementById("chart-target-select").value;
  const rangeSelect = document.getElementById("chart-range-select");
  const rangeHours = Number(rangeSelect.value);
  const rangeLabel = rangeSelect.selectedOptions[0].textContent;
  const until = Math.floor(Date.now() / 1000);
  const since = until - rangeHours * 3600;

  const path = await window.__TAURI__.core.invoke("export_trend_report", { target, since, until, rangeLabel });
  alert(`Report saved to ${path}`);
}
window.exportTrendReport = exportTrendReport;
```

- [ ] **Step 8: Manual verification**

Run: `cd src-tauri && cargo tauri dev`, let baseline data accumulate for a
few minutes, click "Export trend PDF". Expected: an alert with a file
path; open the PDF and confirm it shows the target, range label, point
count, and an embedded line chart of RTT over time.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/report.rs src-tauri/src/commands.rs src-tauri/src/main.rs src/charts.js src/index.html src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "Add PDF trend report generation and export command"
```

---

## Task 22: System tray

**Files:**
- Create: `src-tauri/src/tray.rs`
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Consumes: `commands::{start_baseline, stop_baseline, start_session, BaselineState}` and app-managed state (Tasks 9-12).
- Produces: `tray::build_tray(app: &tauri::App) -> tauri::Result<()>`, called
  once from `main.rs`'s `setup`.

No unit test — this is pure Tauri tray/menu wiring, calling the same
already-tested command functions directly (not through IPC, since it runs
in-process). Verified manually in Step 3.

- [ ] **Step 1: Implement the tray**

```rust
// src-tauri/src/tray.rs
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

pub fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let start_baseline_item = MenuItem::with_id(app, "start_baseline", "Start baseline logger", true, None::<&str>)?;
    let stop_baseline_item = MenuItem::with_id(app, "stop_baseline", "Stop baseline logger", true, None::<&str>)?;
    let new_session_item = MenuItem::with_id(app, "new_session", "New timed session…", true, None::<&str>)?;
    let show_item = MenuItem::with_id(app, "show", "Show dashboard", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[&start_baseline_item, &stop_baseline_item, &new_session_item, &show_item, &quit_item],
    )?;

    TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Packet Proof — elevated, baseline running")
        .on_menu_event(|app: &AppHandle<Wry>, event| match event.id().as_ref() {
            "start_baseline" => {
                let state = app.state::<crate::commands::BaselineState>();
                let db_sink = app.state::<crate::commands::DbSink>();
                let _ = crate::commands::start_baseline(state, db_sink);
            }
            "stop_baseline" => {
                let state = app.state::<crate::commands::BaselineState>();
                let _ = crate::commands::stop_baseline(state);
            }
            "new_session" | "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}
```

- [ ] **Step 2: Wire it into `main.rs`**

```rust
// src-tauri/src/main.rs
mod tray;

fn main() {
    if elevation::decide_relaunch(&elevation::check_elevation()) {
        elevation::relaunch_elevated();
    }

    tauri::Builder::default()
        .setup(|app| {
            let db_path = app.path().app_data_dir()?.join("packet_proof.sqlite");
            std::fs::create_dir_all(db_path.parent().unwrap())?;
            let sink = db_writer::spawn_db_writer(db_path.clone());
            app.manage(commands::DbSink(sink.clone()));

            let read_db = std::sync::Arc::new(db::Db::open(&db_path)?);
            app.manage(commands::DbHandle(read_db));

            let targets = mtr::baseline::fixed_targets(mtr::baseline::default_gateway());
            let logger = mtr::baseline::BaselineLogger::start(std::sync::Arc::new(mtr::trippy_engine::TrippyEngine), targets, sink);
            app.manage(commands::BaselineState(std::sync::Mutex::new(Some(logger))));

            tray::build_tray(app)?;

            Ok(())
        })
        .manage(commands::SessionState(std::sync::Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            commands::start_session,
            commands::stop_session,
            commands::start_baseline,
            commands::stop_baseline,
            commands::list_candidate_processes,
            commands::list_connections_for_pid,
            commands::get_baseline_history,
            commands::get_session_history,
            commands::export_session_report,
            commands::export_trend_report
        ])
        .run(tauri::generate_context!())
        .expect("error while running Packet Proof");
}
```

- [ ] **Step 3: Manual verification**

Run: `cd src-tauri && cargo tauri dev`. Expected: a tray icon appears;
right-clicking (Windows) or clicking (macOS) it shows the five menu items;
"Stop baseline logger" then "Start baseline logger" toggles baseline
activity (confirm via the DB check from Task 11 Step 4 before/after);
"Show dashboard" focuses the main window; "Quit" closes the app fully.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/tray.rs src-tauri/src/main.rs
git commit -m "Add system tray with baseline/session controls and status"
```

---

## Self-Review Notes

**Spec coverage** — every P0 item from the spec maps to a task:
item 1 (engine) → Tasks 3-4, 9; item 2 (zones) → Task 6-7; item 3
(baseline) → Task 12; item 4 (timed sessions) → Task 17; item 5 (process
picker) → Task 13, 16; item 6 (target picker + UDP limitation) → Tasks 14,
15, 16; item 7 (SQLite) → Task 10-11; item 8 (dashboard) → Tasks 5, 7, 16,
19; item 9 (PDF export) → Tasks 20-21; item 10 (tray) → Task 22. The three
resolved open decisions (elevation, adaptive rate, fixed baseline targets)
are implemented in Tasks 2, 8-9, and 12 respectively.

**Placeholder scan** — no TBD/TODO markers; every step shows real code or
a concrete verification command. The exceptions are explicitly-labeled
manual-QA steps (elevation flow, live trippy-core output, Windows/macOS
connection lookups, tray behavior, PDF visual inspection) — these are
genuinely not unit-testable without a real OS/network/raw-socket
environment, and the spec's own testing section (§6) calls this out as the
expected split rather than a gap.

**Type consistency** — `RawHop`/`RoundResult` (Task 3) are used unchanged
through `TrippyEngine` (Task 4), `SessionRunner` (Task 9), and
`BaselineLogger` (Task 12). `HopSnapshot` gains its `zone` field in Task 7
and that shape is what Tasks 9, 11, and 16's frontend consumers all rely
on. `ProbeRecord` (Task 10) is constructed identically by `SessionRunner`
(Task 11) and `BaselineLogger` (Task 12) and consumed identically by
`report.rs` (Tasks 20-21) and the history commands (Task 18).
`ConnectionQuery`/`Endpoint`/`Protocol` (Task 13) are the exact types both
platform backends (Tasks 14-15) return and Task 16's commands forward to
the frontend unchanged.
