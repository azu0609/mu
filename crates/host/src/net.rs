use crate::{Result, devices::sha256, store};
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use std::{
    fs,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::Path,
    sync::Arc,
};

pub struct Identity {
    pub tls: Arc<ServerConfig>,
    pub cert: String,
}

pub fn identity(dir: &Path) -> Result<Identity> {
    let (cert_path, key_path) = (dir.join("cert.der"), dir.join("key.der"));
    let (cert, key) = match (fs::read(&cert_path), fs::read(&key_path)) {
        (Ok(cert), Ok(key)) => (cert, key),
        _ => {
            let made = rcgen::generate_simple_self_signed(vec!["mu".to_owned()])?;
            let (cert, key) = (made.cert.der().to_vec(), made.signing_key.serialize_der());
            store::write(&key_path, &key)?;
            store::write(&cert_path, &cert)?;
            (cert, key)
        }
    };
    let fingerprint = sha256(&cert);
    let mut tls = ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(cert)], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    tls.alpn_protocols = vec![b"h2".to_vec()];
    Ok(Identity { tls: Arc::new(tls), cert: fingerprint })
}

pub fn hostname() -> String {
    let mut name = [0u8; 256];
    // SAFETY: the buffer is valid for its whole length.
    if unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len()) } != 0 {
        return String::new();
    }
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    String::from_utf8_lossy(&name[..end]).into_owned()
}

// Likeliest first: Tailscale, then private networks, then the rest.
pub fn addresses(bound: IpAddr) -> Vec<IpAddr> {
    if !bound.is_unspecified() {
        return vec![bound];
    }
    let mut addresses: Vec<_> = interfaces()
        .into_iter()
        .filter(|ip| !ip.is_loopback() && !link_local(ip) && (bound.is_ipv6() || ip.is_ipv4()))
        .collect();
    addresses.sort_by_key(|ip| (rank(ip), *ip));
    addresses.dedup();
    addresses
}

fn rank(ip: &IpAddr) -> u8 {
    match ip {
        IpAddr::V4(ip) => match ip.octets() {
            [100, b, ..] if (64..128).contains(&b) => 0,
            _ if ip.is_private() => 2,
            _ => 4,
        },
        IpAddr::V6(ip) => match ip.segments() {
            [0xfd7a, 0x115c, 0xa1e0, ..] => 1,
            [first, ..] if first & 0xfe00 == 0xfc00 => 3,
            _ => 5,
        },
    }
}

fn link_local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_link_local(),
        IpAddr::V6(ip) => ip.segments()[0] & 0xffc0 == 0xfe80,
    }
}

fn interfaces() -> Vec<IpAddr> {
    let mut addresses = vec![];
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a linked list that stays valid until freeifaddrs,
    // and each address points to the sockaddr variant its family names.
    unsafe {
        if libc::getifaddrs(&mut list) != 0 {
            return addresses;
        }
        let mut entry = list;
        while let Some(interface) = entry.as_ref() {
            if let Some(address) = interface.ifa_addr.as_ref() {
                match address.sa_family as i32 {
                    libc::AF_INET => {
                        let address = &*interface.ifa_addr.cast::<libc::sockaddr_in>();
                        addresses.push(IpAddr::V4(Ipv4Addr::from(u32::from_be(address.sin_addr.s_addr))));
                    }
                    libc::AF_INET6 => {
                        let address = &*interface.ifa_addr.cast::<libc::sockaddr_in6>();
                        addresses.push(IpAddr::V6(Ipv6Addr::from(address.sin6_addr.s6_addr)));
                    }
                    _ => (),
                }
            }
            entry = interface.ifa_next;
        }
        libc::freeifaddrs(list);
    }
    addresses
}
