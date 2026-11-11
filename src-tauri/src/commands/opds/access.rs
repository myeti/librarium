//! Who may use the server: the Host check and, in LAN mode, HTTP Basic auth.
//! [`guard`] runs before every route, including the fallback (see the
//! security model in the parent module's docs).

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::connect_info::Connected;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum::serve::IncomingStream;
use base64::prelude::*;

use super::limits::LimitedListener;

/// User name for HTTP Basic auth in LAN mode.
pub const OPDS_USER: &str = "librarium";

/// Who may use the server.
pub(super) struct Access {
    pub share_on_lan: bool,
    /// Required (and non-empty) when `share_on_lan`.
    pub password: Option<String>,
    /// This machine's LAN addresses, accepted as Host in LAN mode.
    pub lan_ips: Vec<IpAddr>,
}

/// Per-connection info: the local address the client connected to, which is
/// the only IP it can legitimately put in its Host header.
#[derive(Clone, Copy, Debug)]
pub(super) struct ConnInfo {
    pub local: SocketAddr,
}

impl Connected<IncomingStream<'_, LimitedListener>> for ConnInfo {
    fn connect_info(target: IncomingStream<'_, LimitedListener>) -> Self {
        let local = target
            .io()
            .get_ref()
            .local_addr()
            .unwrap_or_else(|_| SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)));
        ConnInfo { local }
    }
}

/// The axum middleware: Host check, then auth in LAN mode. Also marks every
/// response `nosniff`.
pub(super) async fn guard(
    State(access): State<Arc<Access>>,
    request: Request,
    next: Next,
) -> Response {
    let local_ip = request
        .extensions()
        .get::<ConnectInfo<ConnInfo>>()
        .map(|c| c.0.local.ip());
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| request.uri().host().map(str::to_string));
    if !host_allowed(host.as_deref(), &access, local_ip) {
        return plain(StatusCode::FORBIDDEN, "Forbidden: unexpected Host header");
    }
    if access.share_on_lan {
        let password = access.password.as_deref().unwrap_or("");
        if !basic_auth_ok(request.headers(), password) {
            let mut response = plain(StatusCode::UNAUTHORIZED, "Authentication required");
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"Librarium\", charset=\"UTF-8\""),
            );
            return response;
        }
    }
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn plain(status: StatusCode, text: &'static str) -> Response {
    let mut response = Response::new(Body::from(text));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

/// The host name of a Host header value, lowercase and without a trailing
/// dot (`[::1]:8042` → `::1`, `Localhost.:8042` → `localhost`). `None` for
/// anything malformed: a port that isn't a number, text after `]`.
pub(super) fn host_name(host: &str) -> Option<String> {
    let host = host.trim();
    let name = if let Some(rest) = host.strip_prefix('[') {
        let (inside, after) = rest.split_once(']')?;
        if !(after.is_empty()
            || after
                .strip_prefix(':')
                .is_some_and(|p| p.parse::<u16>().is_ok()))
        {
            return None;
        }
        inside
    } else {
        match host.rsplit_once(':') {
            Some((name, port)) => {
                port.parse::<u16>().ok()?;
                name
            }
            None => host,
        }
    };
    Some(name.trim_end_matches('.').to_ascii_lowercase()).filter(|n| !n.is_empty())
}

/// Whether a request with this Host header may be served: `localhost` and
/// loopback IPs always; in LAN mode, also this machine's LAN IPs and the IP
/// the client connected to (`local_ip`, when known). Host *names* other than
/// `localhost` never are: that defeats DNS rebinding.
pub(super) fn host_allowed(host: Option<&str>, access: &Access, local_ip: Option<IpAddr>) -> bool {
    let Some(name) = host.and_then(host_name) else {
        return false;
    };
    if name == "localhost" {
        return true;
    }
    let Ok(ip) = name.parse::<IpAddr>() else {
        return false;
    };
    if ip.is_loopback() {
        return true;
    }
    access.share_on_lan && (Some(ip) == local_ip || access.lan_ips.contains(&ip))
}

/// Check `Authorization: Basic …` against [`OPDS_USER`] and `password`.
/// An empty expected password never matches.
pub(super) fn basic_auth_ok(headers: &HeaderMap, password: &str) -> bool {
    if password.is_empty() {
        return false;
    }
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some((scheme, encoded)) = value.trim().split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("basic") {
        return false;
    }
    let Ok(decoded) = BASE64_STANDARD.decode(encoded.trim()) else {
        return false;
    };
    let Some(colon) = decoded.iter().position(|&b| b == b':') else {
        return false;
    };
    let (user, pass) = (&decoded[..colon], &decoded[colon + 1..]);
    // Evaluate both so the timing doesn't reveal which part was wrong.
    let user_ok = constant_time_eq(user, OPDS_USER.as_bytes());
    let pass_ok = constant_time_eq(pass, password.as_bytes());
    user_ok & pass_ok
}

/// Compare two byte strings in time that depends only on their lengths, not
/// on the position of the first difference (no early exit).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() ^ b.len()) as u64;
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= u64::from(x ^ y);
    }
    std::hint::black_box(diff) == 0
}

/// This machine's address on the LAN, from the routing table: "connecting" a
/// UDP socket only selects the route and source address, no packet is sent.
/// `None` when offline.
pub(super) fn primary_lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic(user: &str, pass: &str) -> String {
        format!("Basic {}", BASE64_STANDARD.encode(format!("{user}:{pass}")))
    }

    #[test]
    fn empty_password_never_authenticates() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&basic(OPDS_USER, "")).unwrap(),
        );
        assert!(!basic_auth_ok(&headers, ""));
        assert!(!basic_auth_ok(&headers, "x"));
    }

    #[test]
    fn constant_time_comparison() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn host_names_are_parsed_strictly() {
        assert_eq!(host_name("[::1]:8042").as_deref(), Some("::1"));
        assert_eq!(host_name("[::1]").as_deref(), Some("::1"));
        assert_eq!(host_name("LocalHost.:80").as_deref(), Some("localhost"));
        assert_eq!(host_name("192.168.1.20").as_deref(), Some("192.168.1.20"));
        for malformed in [
            " example.com:x ",
            "[::1]\"><x",
            "[::1]:",
            "[::1]:99999",
            "localhost:",
            ":8042",
            "",
            "[::1",
        ] {
            assert_eq!(host_name(malformed), None, "{malformed:?}");
        }
    }

    #[test]
    fn only_loopback_and_own_addresses_are_allowed() {
        let local = Access {
            share_on_lan: false,
            password: None,
            lan_ips: vec!["192.168.1.20".parse().unwrap()],
        };
        let lan = Access {
            share_on_lan: true,
            ..local
        };
        let own: IpAddr = "10.0.0.9".parse().unwrap();
        for host in ["localhost", "127.0.0.1:8042", "[::1]:8042", "127.8.9.10"] {
            assert!(host_allowed(Some(host), &lan, None), "{host}");
        }
        assert!(host_allowed(Some("192.168.1.20:8042"), &lan, None));
        assert!(host_allowed(Some("10.0.0.9"), &lan, Some(own)));
        assert!(!host_allowed(Some("10.0.0.8"), &lan, Some(own)));
        assert!(
            !host_allowed(Some("mymac.local"), &lan, Some(own)),
            "names other than localhost: DNS rebinding"
        );
        assert!(!host_allowed(None, &lan, Some(own)));
        let local = Access {
            share_on_lan: false,
            ..lan
        };
        assert!(
            !host_allowed(Some("192.168.1.20"), &local, None),
            "LAN addresses only in LAN mode"
        );
    }
}
