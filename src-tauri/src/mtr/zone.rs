use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    Home,
    IspGateway,
    BeyondIsp,
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
