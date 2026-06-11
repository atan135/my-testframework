use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{logging::LogEvent, state::SharedState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccessScope {
    Private,
    Unrestricted,
}

impl AccessScope {
    pub(crate) fn from_env_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "private" | "lan" | "intranet" | "internal" => Self::Private,
            "unrestricted" | "public" | "any" | "all" | "open" => Self::Unrestricted,
            other => {
                eprintln!("Invalid QA_ACCESS_SCOPE value '{other}', falling back to 'private'.");
                Self::Private
            }
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Unrestricted => "unrestricted",
        }
    }

    fn allows(self, ip: IpAddr) -> bool {
        match self {
            Self::Private => is_private_network_ip(ip),
            Self::Unrestricted => true,
        }
    }
}

pub(crate) async fn enforce_network_access(
    State(state): State<SharedState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let access_scope = state.config.access_scope;
    if access_scope.allows(remote_addr.ip()) {
        return next.run(request).await;
    }

    let path = request.uri().path().to_string();
    LogEvent::warn("network_access_rejected")
        .field("remoteAddr", remote_addr.to_string())
        .field("path", path)
        .field("accessScope", access_scope.as_str())
        .emit();

    (
        StatusCode::FORBIDDEN,
        "QA register server is restricted to private network clients.",
    )
        .into_response()
}

fn is_private_network_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => {
            ip.is_loopback() || is_ipv6_unique_local(ip) || is_ipv6_unicast_link_local(ip)
        }
    }
}

fn is_ipv6_unique_local(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

fn is_ipv6_unicast_link_local(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_scope_allows_loopback_and_private_networks() {
        let allowed = [
            IpAddr::from([127, 0, 0, 1]),
            IpAddr::from([10, 1, 2, 3]),
            IpAddr::from([172, 16, 1, 1]),
            IpAddr::from([192, 168, 1, 20]),
            IpAddr::from([169, 254, 10, 10]),
            "::1".parse().unwrap(),
            "fc00::1".parse().unwrap(),
            "fd12:3456::1".parse().unwrap(),
            "fe80::1".parse().unwrap(),
        ];

        for ip in allowed {
            assert!(
                AccessScope::Private.allows(ip),
                "expected {ip} to be allowed"
            );
        }
    }

    #[test]
    fn private_scope_rejects_public_networks() {
        let rejected = [
            IpAddr::from([8, 8, 8, 8]),
            IpAddr::from([1, 1, 1, 1]),
            "2001:4860:4860::8888".parse().unwrap(),
        ];

        for ip in rejected {
            assert!(
                !AccessScope::Private.allows(ip),
                "expected {ip} to be rejected"
            );
        }
    }

    #[test]
    fn unrestricted_scope_allows_public_networks() {
        assert!(AccessScope::Unrestricted.allows(IpAddr::from([8, 8, 8, 8])));
        assert!(AccessScope::Unrestricted.allows("2001:4860:4860::8888".parse().unwrap()));
    }

    #[test]
    fn access_scope_accepts_aliases_and_falls_back_to_private() {
        assert_eq!(AccessScope::from_env_value("lan"), AccessScope::Private);
        assert_eq!(
            AccessScope::from_env_value("unrestricted"),
            AccessScope::Unrestricted
        );
        assert_eq!(
            AccessScope::from_env_value("open"),
            AccessScope::Unrestricted
        );
        assert_eq!(AccessScope::from_env_value("invalid"), AccessScope::Private);
    }
}
