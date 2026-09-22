use serde::Serialize;

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "macos")]
pub mod macos;

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

pub fn connections_for_pid(pid: u32) -> ConnectionQuery {
    #[cfg(target_os = "windows")]
    return windows::connections_for_pid(pid);

    #[cfg(target_os = "macos")]
    return macos::connections_for_pid(pid);
}

fn pids_with_remote_endpoints() -> std::collections::HashSet<u32> {
    #[cfg(target_os = "windows")]
    return windows::pids_with_remote_endpoints();

    #[cfg(target_os = "macos")]
    return macos::pids_with_remote_endpoints();
}

/// Filter `list_processes()` down to the processes that actually have a
/// remote connection.
///
/// This enumerates the OS connection table(s) **once** and filters against
/// the resulting PID set. The previous shape called `connections_for_pid`
/// per process, and each of those calls is a full system-wide snapshot
/// (`GetExtendedTcpTable` + `GetExtendedUdpTable` on Windows, a fresh `lsof`
/// fork on macOS) — ~600 kernel table enumerations, or ~300 forks, to answer
/// one question, taking tens of seconds to minutes on a busy machine.
pub fn list_candidate_processes() -> Vec<ProcessInfo> {
    filter_to_pids(list_processes(), &pids_with_remote_endpoints())
}

/// The pure half of `list_candidate_processes`, split out so the filtering
/// rule is testable without touching the OS.
fn filter_to_pids(processes: Vec<ProcessInfo>, pids: &std::collections::HashSet<u32>) -> Vec<ProcessInfo> {
    processes.into_iter().filter(|p| pids.contains(&p.pid)).collect()
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

    #[test]
    fn keeps_only_processes_present_in_the_connection_table() {
        let processes = vec![
            ProcessInfo { pid: 1, name: "idle.exe".into() },
            ProcessInfo { pid: 2, name: "Discord.exe".into() },
            ProcessInfo { pid: 3, name: "zoom.exe".into() },
        ];
        let pids = std::collections::HashSet::from([2, 3, 99]);
        let kept = filter_to_pids(processes, &pids);
        assert_eq!(kept.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![2, 3]);
    }

    #[test]
    fn an_empty_connection_table_yields_no_candidates() {
        let processes = vec![ProcessInfo { pid: 1, name: "idle.exe".into() }];
        assert_eq!(filter_to_pids(processes, &std::collections::HashSet::new()), vec![]);
    }
}
