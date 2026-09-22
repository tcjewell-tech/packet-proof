#![cfg(target_os = "windows")]

use std::net::IpAddr;

use crate::procnet::{ConnectionQuery, Endpoint, Protocol};

/// (owning_pid, remote_ip, remote_port, protocol) — the shape both the TCP
/// and UDP table rows reduce to once we've pulled the fields we need out of
/// the raw `MIB_TCPROW_OWNER_PID` / `MIB_UDPROW_OWNER_PID` structs.
type Row = (u32, IpAddr, u16, Protocol);

pub fn rows_to_endpoints(rows: &[Row], pid: u32) -> Vec<Endpoint> {
    rows.iter()
        .filter(|(owner, ..)| *owner == pid)
        .map(|(_, ip, port, proto)| Endpoint { remote_ip: ip.to_string(), remote_port: *port, protocol: *proto })
        .collect()
}

use windows::Win32::Foundation::NO_ERROR;
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCPTABLE_OWNER_PID, MIB_UDPTABLE_OWNER_PID,
    TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
};
use windows::Win32::Networking::WinSock::AF_INET;

/// Enumerate the system-wide TCP and UDP connection tables **once**.
///
/// Each call is a full kernel snapshot of both tables, so callers that need
/// to answer a question about many processes must call this once and filter
/// the result per PID — not call `connections_for_pid` in a loop, which is
/// what made the process picker take tens of seconds.
pub fn all_rows() -> Vec<Row> {
    let mut rows: Vec<Row> = vec![];
    rows.extend(tcp_rows());
    rows.extend(udp_rows());
    rows
}

/// The PIDs that own at least one connection with a remote endpoint, from a
/// single pass over the tables.
pub fn pids_with_remote_endpoints() -> std::collections::HashSet<u32> {
    all_rows().into_iter().map(|(owner, ..)| owner).collect()
}

pub fn connections_for_pid(pid: u32) -> ConnectionQuery {
    let endpoints = rows_to_endpoints(&all_rows(), pid);
    if endpoints.is_empty() {
        ConnectionQuery::NoneDetected
    } else {
        ConnectionQuery::Found(endpoints)
    }
}

fn tcp_rows() -> Vec<Row> {
    unsafe {
        let mut size: u32 = 0;
        let _ = GetExtendedTcpTable(None, &mut size, false, AF_INET.0 as u32, TCP_TABLE_OWNER_PID_ALL, 0);
        // A `Vec<u8>` has alignment 1, but MIB_TCPTABLE_OWNER_PID needs 4
        // (its fields are u32). Allocating u32s gives the buffer the right
        // alignment for the cast below, which is otherwise UB even though a
        // general-purpose allocator happens to over-align in practice.
        let mut buf: Vec<u32> = vec![0u32; (size as usize).div_ceil(4)];
        if GetExtendedTcpTable(
            Some(buf.as_mut_ptr() as *mut _),
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
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
        // Same alignment rationale as `tcp_rows` above.
        let mut buf: Vec<u32> = vec![0u32; (size as usize).div_ceil(4)];
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

    /// Live check against the real kernel tables — not run by default because
    /// it depends on the machine actually having an open TCP connection. Run
    /// with `cargo test procnet::windows -- --ignored --nocapture`.
    ///
    /// This is the check that a change to the raw buffer allocation (see the
    /// alignment comment in `tcp_rows`) has not broken the struct cast.
    #[test]
    #[ignore]
    fn reads_the_live_connection_table() {
        let rows = all_rows();
        println!("{} live rows", rows.len());
        assert!(!rows.is_empty(), "expected at least one open TCP connection on this machine");
        for (pid, ip, port, proto) in rows.iter().take(10) {
            println!("pid {pid} -> {ip}:{port} ({proto:?})");
            assert_ne!(*pid, 0, "owning pid should be populated");
            assert_ne!(*port, 0, "remote port should be populated");
            assert!(!ip.is_unspecified(), "rows with no remote address are filtered out");
        }

        let pids = pids_with_remote_endpoints();
        assert!(!pids.is_empty());
        let one = rows[0].0;
        assert!(pids.contains(&one));
        assert!(matches!(connections_for_pid(one), ConnectionQuery::Found(e) if !e.is_empty()));
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
