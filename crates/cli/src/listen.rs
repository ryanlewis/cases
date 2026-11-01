//! The address serve listens on (SV-02, IN-11).

use std::net::IpAddr;

use crate::error::Error;

/// Checks that `addr`, as `--listen` takes it, is `HOST:PORT` with a
/// loopback host: `localhost`, or an address in 127.0.0.0/8 or `::1`,
/// which is written in brackets as `[::1]:7337`. Port 0 is allowed.
///
/// # Errors
///
/// When `addr` is not `HOST:PORT`, has no port, or its host is not
/// loopback. The error names the address.
pub(crate) fn check_loopback(addr: &str) -> Result<(), Error> {
    let (host, port) =
        split(addr).ok_or_else(|| Error::new(format!("--listen {addr:?}: not HOST:PORT")))?;
    if port.is_empty() {
        return Err(Error::new(format!("--listen {addr:?}: no port")));
    }
    if host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()) {
        return Ok(());
    }
    Err(Error::new(format!(
        "--listen {addr:?}: only a loopback address (127.0.0.1, ::1 or localhost) is allowed"
    )))
}

/// `addr` as its host and port. An IPv6 host is in brackets, which are taken
/// off; any other host has no `:`.
pub(crate) fn split(addr: &str) -> Option<(&str, &str)> {
    if let Some(rest) = addr.strip_prefix('[') {
        let (host, port) = rest.split_once(']')?;
        return Some((host, port.strip_prefix(':')?));
    }
    let (host, port) = addr.rsplit_once(':')?;
    (!host.contains(':')).then_some((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loopback_address_is_allowed() {
        for addr in [
            "127.0.0.1:7337",
            "127.0.0.1:0",
            "127.1.2.3:7337",
            "[::1]:7337",
            "localhost:7337",
        ] {
            assert!(check_loopback(addr).is_ok(), "{addr}");
        }
    }

    #[test]
    fn any_other_address_is_refused_by_name() {
        for (addr, want) in [
            ("0.0.0.0:7337", "only a loopback address"),
            ("192.168.1.2:7337", "only a loopback address"),
            ("[::]:7337", "only a loopback address"),
            (":7337", "only a loopback address"),
            ("example.com:7337", "only a loopback address"),
            ("127.0.0.1:", "no port"),
            ("127.0.0.1", "not HOST:PORT"),
            ("::1:7337", "not HOST:PORT"),
            ("[::1]7337", "not HOST:PORT"),
        ] {
            let error = check_loopback(addr).unwrap_err().to_string();
            assert!(error.contains(want), "{addr}: {error}");
            assert!(error.contains(&format!("{addr:?}")), "{addr}: {error}");
        }
    }
}
