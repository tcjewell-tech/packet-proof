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
        let _ = GetExtendedTcpTable(None, &mut size, false, AF_INET.0 as u32, TCP_TABLE_OWNER_PID_ALL, 0);
        let mut buf = vec![0u8; size as usize];
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
