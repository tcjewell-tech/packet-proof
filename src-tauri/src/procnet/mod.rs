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
