#![cfg(target_os = "macos")]

use crate::procnet::{ConnectionQuery, Endpoint, Protocol};

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
