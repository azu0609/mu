// Pairing devices with the shared host on this machine, and unpairing them.
use crate::{Result, remote::Remote};
use mu_client::{Endpoint, Invite};
use qrcode::{QrCode, render::unicode::Dense1x2};
use std::{
    net::{IpAddr, SocketAddr},
    time::{SystemTime, UNIX_EPOCH},
};

// Shows an invite for one device, as a QR code and a link. `address` is where
// devices reach this host, for when the host's own guess will not do: a
// Tailscale name, say.
pub fn pair(endpoint: Endpoint, address: Option<&str>) -> Result<()> {
    let invitation = Remote::connect(endpoint)?.invite()?;
    let chosen = match address {
        Some(address) => with_port(address, invitation.port),
        None => match invitation.addresses.first() {
            Some(&ip) => SocketAddr::new(ip, invitation.port).to_string(),
            None => return Err("Cannot tell where devices reach this host; name it: mu pair <address>".into()),
        },
    };
    let link = Invite { address: chosen, cert: invitation.cert, code: invitation.code }.to_string();
    let image = QrCode::new(&link)?.render::<Dense1x2>().quiet_zone(true).build();
    // Dark on light whatever the terminal's colors, so cameras can read it.
    for line in image.lines() {
        println!("\x1b[30;107m{line}\x1b[0m");
    }
    println!("\n{link}\n\nScan or paste it in the mu app within 10 minutes. It pairs one device.");
    let others: Vec<_> = invitation.addresses.iter().skip(1).map(ToString::to_string).collect();
    if address.is_none() && !others.is_empty() {
        println!("Not reachable there? `mu pair <address>` with one of {}.", others.join(", "));
    }
    Ok(())
}

fn with_port(address: &str, port: u16) -> String {
    if address.parse::<SocketAddr>().is_ok() {
        return address.into();
    }
    if let Ok(ip) = address.parse::<IpAddr>() {
        return SocketAddr::new(ip, port).to_string();
    }
    match address.rsplit_once(':') {
        Some((_, port)) if port.parse::<u16>().is_ok() => address.into(),
        _ => format!("{address}:{port}"),
    }
}

pub fn devices(endpoint: Endpoint) -> Result<()> {
    let devices = Remote::connect(endpoint)?.devices()?;
    if devices.is_empty() {
        println!("No devices are paired. `mu pair` pairs one.");
        return Ok(());
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let width = devices.iter().map(|device| device.name.chars().count()).max().unwrap_or(0);
    for device in devices {
        let paired = match now.saturating_sub(device.paired) / 86_400 {
            0 => "today".into(),
            1 => "yesterday".into(),
            days => format!("{days} days ago"),
        };
        println!("{}  {:<width$}  paired {paired}", device.id, device.name);
    }
    Ok(())
}

pub fn unpair(endpoint: Endpoint, id: &str) -> Result<()> {
    Remote::connect(endpoint)?.unpair(id)?;
    println!("Unpaired {id}. It can no longer reach this host.");
    Ok(())
}
