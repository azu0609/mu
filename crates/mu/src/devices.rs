use crate::connect;
use mu_client::{CreateInvite, Invite, ListDevices, Unpair};
use mu_host::Result;
use qrcode::{QrCode, render::unicode::Dense1x2};
use std::{
    net::{IpAddr, SocketAddr},
    time::{SystemTime, UNIX_EPOCH},
};

pub async fn pair(address: Option<&str>) -> Result<()> {
    let invitation = connect().await?.call(CreateInvite {}).await?;
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

pub async fn devices() -> Result<()> {
    let devices = connect().await?.call(ListDevices {}).await?;
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

pub async fn unpair(id: &str) -> Result<()> {
    connect().await?.call(Unpair { device_id: id.into() }).await?;
    println!("Unpaired {id}. It can no longer reach this host.");
    Ok(())
}
