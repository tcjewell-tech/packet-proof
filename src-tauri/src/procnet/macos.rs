#![cfg(target_os = "macos")]

use crate::procnet::{ConnectionQuery, Endpoint, Protocol};

pub fn parse_lsof_output(raw: &str) -> Vec<Endpoint> {
    parse_lsof_output_with_pids(raw).into_iter().map(|(_, endpoint)| endpoint).collect()
}

/// Same parse, but keeping each row's owning PID — needed when one
/// system-wide `lsof -i` replaces N per-PID invocations.
pub fn parse_lsof_output_with_pids(raw: &str) -> Vec<(u32, Endpoint)> {
    raw.lines()
        .skip(1) // header row
        .filter_map(parse_line)
        .collect()
}

fn parse_line(line: &str) -> Option<(u32, Endpoint)> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let pid: u32 = fields.get(1)?.parse().ok()?;
    let protocol = match fields.get(7)? {
        &"TCP" => Protocol::Tcp,
        &"UDP" => Protocol::Udp,
        _ => return None,
    };

    let name_field = fields.get(8)?;
    let (_, remote) = name_field.split_once("->")?;
    let remote = remote.split(' ').next()?; // strip a trailing "(ESTABLISHED)" if present
    let (ip, port) = remote.rsplit_once(':')?;

    Some((pid, Endpoint { remote_ip: ip.to_string(), remote_port: port.parse().ok()?, protocol }))
}

pub fn connections_for_pid(pid: u32) -> ConnectionQuery {
    let output = std::process::Command::new("lsof")
        .args(["-i", "-n", "-P", "-a", "-p", &pid.to_string()])
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

/// The PIDs that own at least one connection with a remote endpoint, from a
/// **single** `lsof` invocation.
///
/// The process picker used to fork one `lsof -p <pid>` per process, which on
/// a machine with a few hundred processes meant a few hundred forks to answer
/// one question.
pub fn pids_with_remote_endpoints() -> std::collections::HashSet<u32> {
    // -n / -P skip DNS and service-name lookups, which are the slow part of
    // an unfiltered `lsof -i`.
    let Ok(output) = std::process::Command::new("lsof").args(["-i", "-n", "-P"]).output() else {
        return std::collections::HashSet::new();
    };

    parse_lsof_output_with_pids(&String::from_utf8_lossy(&output.stdout))
        .into_iter()
        .map(|(pid, _)| pid)
        .collect()
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

    const MULTI_PROCESS_SAMPLE: &str = "\
COMMAND   PID   USER   FD   TYPE   DEVICE SIZE/OFF NODE NAME
Discord   501   tom    23u  IPv4  0x1234      0t0  TCP  192.168.1.5:54321->104.16.1.1:443 (ESTABLISHED)
Spotify   933   tom    11u  IPv4  0x2222      0t0  TCP  192.168.1.5:54999->35.186.224.25:443 (ESTABLISHED)
Spotify   933   tom    12u  IPv4  0x3333      0t0  UDP  192.168.1.5:60000->35.186.224.25:19305
sshd      777   root   3u   IPv4  0x4444      0t0  TCP  *:22 (LISTEN)
";

    #[test]
    fn keeps_the_owning_pid_for_each_row() {
        let rows = parse_lsof_output_with_pids(MULTI_PROCESS_SAMPLE);
        assert_eq!(rows.iter().map(|(pid, _)| *pid).collect::<Vec<_>>(), vec![501, 933, 933]);
    }

    #[test]
    fn a_listening_only_process_is_not_a_candidate() {
        let pids: std::collections::HashSet<u32> =
            parse_lsof_output_with_pids(MULTI_PROCESS_SAMPLE).into_iter().map(|(pid, _)| pid).collect();
        assert!(pids.contains(&501));
        assert!(pids.contains(&933));
        assert!(!pids.contains(&777), "a LISTEN row has no remote endpoint");
    }
}
