# Packet Proof — Design Spec (v2.0 MVP / P0)

Status: approved for planning
Supersedes: v1.0 prompt (shell-out traceroute approach)
Source: Packet-Proof-Spec-v2.0.docx §3–§6 (rationale, full scope, open decisions)

## 1. Summary

Packet Proof is a cross-platform (Windows + macOS) desktop app built with Tauri
(Rust backend, HTML/CSS/JS frontend). It is a bundled, extended WinMTR: a
native continuous MTR-style engine with live per-hop latency/loss, hop zone
tagging (home network / ISP gateway / beyond ISP), and the ability to target
the specific server a running game or app is connected to. All data stays
local — no accounts, no cloud sync, no telemetry.

## 2. Scope (P0)

1. Native MTR probing engine — continuous ICMP probing with incrementing TTL,
   aggregating per-hop best/avg/worst RTT and loss %, refreshed live. Built on
   the `trippy-core` crate rather than shelling out to `tracert`/`traceroute`
   or hand-rolling raw-socket handling.
2. Hop zone classification — pure heuristic, no external data: RFC1918
   private ranges (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16) → "Home network
   / router"; first public-IP hop after that → "ISP gateway"; everything
   after → "Beyond ISP".
3. Continuous background baseline — probes every 60s to the LAN gateway plus
   a fixed public target set (1.1.1.1, 8.8.8.8), always running, logged to
   SQLite for long-term trend charts. Fixed target list for this pass (see
   §6.3).
4. Timed sessions — user picks a duration (15/30/60 min presets, plus custom)
   and a target; engine runs at an adaptive rate (see §6.2) for that window,
   independent of the baseline logger.
5. Running-process picker — list active processes filtered to those with
   open network connections, so the user can select "the game currently
   running."
6. Auto-detected target picker — for the selected process, query live
   network connections via OS APIs and list remote endpoints as candidates:
   - Windows: `GetExtendedTcpTable` / `GetExtendedUdpTable` via the `windows`
     crate, filtered by PID.
   - macOS: shell out to `lsof -i -a -p <pid>` and parse.
   - UDP sockets that never called `connect()` won't show a remote endpoint
     through these APIs. This is surfaced explicitly in the UI (a "no active
     connections detected" state), not silently dropped.
7. SQLite local storage — schema supports both baseline and session
   probe/hop records: target, timestamp, session type, hop index, hop IP,
   RTT best/avg/worst, loss %, zone tag.
8. Dashboard UI — live auto-refreshing hop table during an active session
   (IP, RTT best/avg/worst, loss %, zone, color-coded by zone); historical
   RTT/loss charts for baseline data over a selectable time range.
9. PDF report export — session report (target, duration, hop table with
   zone tags, summary stats) and baseline/trend report (multi-day pattern,
   charts).
10. System tray — start/stop the baseline logger, launch a new timed
    session, quick current-status glance.

## 3. Explicit constraints

- No account system, no cloud sync, no telemetry — all data stays local.
- No throughput/speed testing in this pass.
- No packet-capture/Npcap-based UDP sniffing in this pass — OS
  connection-table APIs only, with the limitation surfaced in the UI.
- No bundled ASN/IP-ownership database — zone classification is the
  private-IP heuristic only.
- Windows and macOS only.

## 4. Architecture

### 4.1 Process & privilege model

Raw-socket ICMP tracing (via `trippy-core`) requires elevated privileges on
both Windows and Linux; macOS has a partial unprivileged path
(`IPPROTO_ICMP` + `IP_HDRINCL`) but it's inconsistent enough not to rely on
for an MVP that must behave predictably on both target platforms.

**Decision: always elevate at launch.** Before any UI paints, the app checks
whether it is already running elevated:
- Windows: token-elevation check (`GetTokenInformation` /
  `TokenElevationType`).
- macOS: `geteuid() == 0`.

If not elevated, the app relaunches itself elevated and exits the original
process:
- Windows: `ShellExecuteW` with the `"runas"` verb (triggers the standard
  UAC prompt).
- macOS: `osascript -e 'do shell script "<path> <args>" with administrator
  privileges'` (triggers a password/Touch ID prompt).

The whole app runs elevated — there is no split privileged-helper process
with IPC for the MVP. This matches WinMTR's own model (it also requires
admin on Windows) and avoids building a second process boundary and IPC
layer for this pass. Elevation state is shown persistently in the dashboard
header and the tray tooltip, since every probing feature depends on it.

### 4.2 Probing engine (`mtr/engine.rs`)

`trippy-core`'s `Tracer` (via its `Builder` API, run in background mode via
`spawn_with(callback)`) is wrapped behind a small `ProbeEngine` trait. This
lets the aggregation and rate-control logic be unit-tested against a fake
engine implementation without requiring raw sockets (and thus elevation) in
CI or local test runs.

Two independent `Tracer` instances run concurrently, never sharing state:
- The baseline logger: fixed 60s cadence, 3 fixed targets (gateway,
  1.1.1.1, 8.8.8.8), always running once the app is elevated and started.
- The active timed session (when one is running): adaptive rate (§4.3)
  against the user-selected target.

Each `Round` from a tracer is converted into per-hop aggregate updates (best
/avg/worst RTT, loss %) and pushed to both the UI (via a Tauri event) and
the SQLite writer (via the `mpsc` channel described in §4.4).

### 4.3 Adaptive session probe rate

Timed sessions start at 1 probe/sec (matching WinMTR/classic MTR defaults).
The engine tracks loss % over a trailing 10-probe window:
- If loss exceeds 20% in that window, the interval backs off by ×1.5, up to
  a 3s cap.
- If loss stays under 5% for 10 consecutive probes, the interval steps back
  down toward the 1s floor (same ×1.5 factor, inverted).

This is a bounded, simple backoff — not a PID controller — intended only to
avoid the probing itself adding noticeable load to an already-struggling
connection during active gameplay. The baseline logger's 60s cadence is
fixed and does not use this mechanism (it's already sparse).

### 4.4 Storage (`db.rs`)

`rusqlite`, WAL mode. Single writer: both tracers push hop records through
one `mpsc` channel to a dedicated DB-writer task, so baseline and session
writes never contend for SQLite's single-writer lock. The dashboard UI reads
through a separate read connection (safe to do concurrently with a WAL-mode
writer).

Schema (both baseline and session records share one table, distinguished by
`session_type`):

```
CREATE TABLE probe_records (
  id            INTEGER PRIMARY KEY,
  session_type  TEXT NOT NULL,      -- 'baseline' | 'timed'
  target        TEXT NOT NULL,      -- IP or hostname the session/baseline targets
  session_id    TEXT,               -- NULL for baseline; UUID per timed session
  timestamp     INTEGER NOT NULL,   -- unix epoch seconds
  hop_index     INTEGER NOT NULL,
  hop_ip        TEXT,               -- NULL if hop didn't respond
  rtt_best_ms   REAL,
  rtt_avg_ms    REAL,
  rtt_worst_ms  REAL,
  loss_pct      REAL NOT NULL,
  zone          TEXT NOT NULL       -- 'home' | 'isp_gateway' | 'beyond_isp'
);
CREATE INDEX idx_probe_records_target_time ON probe_records(target, timestamp);
CREATE INDEX idx_probe_records_session ON probe_records(session_id);
```

### 4.5 Zone classification (`mtr/zone.rs`)

Pure function, no I/O, no external data — ideal for unit testing:
- RFC1918 (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16) → `home`.
- First public-IP hop after the last private-IP hop → `isp_gateway`.
- Every subsequent hop → `beyond_isp`.

### 4.6 Process & connection picker (`procnet/`)

- Process listing: `sysinfo` crate (cross-platform), filtered post-hoc to
  processes that have at least one open connection.
- Windows (`procnet/windows.rs`): `GetExtendedTcpTable` /
  `GetExtendedUdpTable` via the `windows` crate, filtered by PID.
- macOS (`procnet/macos.rs`): shell out to `lsof -i -a -p <pid>`, parse
  stdout.
- Both return a common `Vec<Endpoint>` shape carrying enough info to render
  the "no active connections detected" state explicitly when a process has
  only unconnected UDP sockets (which these APIs cannot see a remote
  endpoint for).

### 4.7 Tauri command/event boundary

- Events (backend → frontend): `hop-update`, `baseline-tick`,
  `elevation-state`.
- Commands (frontend → backend): `start_session`, `stop_session`,
  `list_candidate_processes`, `list_connections_for_pid`,
  `get_baseline_history`, `get_session_history`, `export_session_report`,
  `export_trend_report`, `start_baseline`, `stop_baseline`.
  `get_baseline_history`/`get_session_history` back the dashboard's live and
  historical charts (§2 item 8); the `export_*` commands are PDF-only and
  independently re-query the data they need rather than reusing chart state.
- The system tray (`tray.rs`) drives the same commands as the dashboard UI —
  one code path regardless of entry point.

### 4.8 PDF export (`report.rs`)

- `genpdf` (built on `printpdf`) for report layout: text, headings, tables.
- Trend charts for the baseline/trend report are rasterized server-side with
  the `plotters` crate to PNG and embedded as images — not a re-render of
  the frontend's live JS charts. Keeps report generation deterministic and
  decoupled from UI state.
- Two report types, per §2 item 9: session report, baseline/trend report.

## 5. Suggested structure

```
src-tauri/
  src/
    main.rs               # elevation check/relaunch, app bootstrap
    mtr/
      engine.rs            # ProbeEngine trait, trippy-core wrapper, adaptive rate
      zone.rs               # RFC1918-based zone classification (pure fn)
    procnet/
      windows.rs            # GetExtendedTcpTable / GetExtendedUdpTable
      macos.rs               # lsof parsing
    db.rs                    # rusqlite schema + queries (baseline + session)
    report.rs                 # PDF generation (genpdf + plotters)
    tray.rs
  Cargo.toml
src/                          # frontend
  dashboard.html/js
  hop-table.js                 # live WinMTR-style table, zone color-coding
  process-picker.js
  charts.js
docs/superpowers/specs/         # this file
```

## 6. Testing approach

- Unit tests: `zone.rs` (pure function, full branch coverage), `db.rs`
  (in-memory SQLite round-trips for schema/queries), engine rate-adaptation
  logic (via a fake `ProbeEngine` — no real sockets needed).
- `procnet` parsers: unit tests against captured sample output (raw `lsof`
  lines, TCP table row shapes) rather than requiring live network state.
- Manual QA (not unit-testable): elevation flow on both OSes, live probing
  against a real target, PDF export opening correctly, tray start/stop.

## 7. Build order

1. Scaffold Tauri project.
2. Engine against a single hard-coded target: probe → aggregate → display
   live in a bare hop table. (End-to-end skeleton before anything else.)
3. Zone tagging.
4. SQLite storage.
5. Baseline logger.
6. Process/connection picker.
7. Timed sessions with adaptive rate.
8. Dashboard charts.
9. PDF export.
10. System tray.

## 8. Resolved decisions (previously open in §6 of the source spec)

- **Elevated privileges**: always elevate at launch (relaunch elevated,
  exit original process). See §4.1.
- **Session probe rate ceiling**: adaptive, starting at 1/sec, backing off
  under sustained loss. See §4.3.
- **Baseline target list**: fixed (gateway + 1.1.1.1 + 8.8.8.8) for this
  pass, not user-configurable.
