use std::io;
use std::net::IpAddr;
use std::os::windows::process::CommandExt;
use std::process::{Command, Output};

use encoding_rs::GBK;
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

pub fn set_interface_name(old_name: &str, new_name: &str) -> io::Result<()> {
    let cmd = format!(" netsh interface set interface name={old_name:?} newname={new_name:?}");
    exe_cmd(&cmd)
}
pub fn exe_cmd(cmd: &str) -> io::Result<()> {
    let out = Command::new("cmd")
        .creation_flags(CREATE_NO_WINDOW)
        .arg("/C")
        .arg(cmd)
        .output()?;
    output(cmd, out)
}
fn gbk_to_utf8(bytes: &[u8]) -> String {
    let (msg, _, _) = GBK.decode(bytes);
    msg.to_string()
}
fn output(cmd: &str, out: Output) -> io::Result<()> {
    if !out.status.success() {
        let msg = if !out.stderr.is_empty() {
            match std::str::from_utf8(&out.stderr) {
                Ok(msg) => msg.to_string(),
                Err(_) => gbk_to_utf8(&out.stderr),
            }
        } else if !out.stdout.is_empty() {
            match std::str::from_utf8(&out.stdout) {
                Ok(msg) => msg.to_string(),
                Err(_) => gbk_to_utf8(&out.stdout),
            }
        } else {
            String::new()
        };
        return Err(io::Error::other(format!(
            "cmd=\"{cmd}\",out=\"{}\"",
            msg.trim()
        )));
    }
    Ok(())
}
pub fn set_primary_dns(index: u32, address: IpAddr) -> io::Result<()> {
    let (family, addr_str) = match address {
        IpAddr::V4(v4) => ("ipv4", v4.to_string()),
        IpAddr::V6(v6) => ("ipv6", v6.to_string()),
    };
    let cmd = format!("netsh interface {family} set dnsservers {index} static {addr_str} primary");
    exe_cmd(&cmd)
}
pub fn add_secondary_dns(index: u32, address: IpAddr, index_pos: u32) -> io::Result<()> {
    let (family, addr_str) = match address {
        IpAddr::V4(v4) => ("ipv4", v4.to_string()),
        IpAddr::V6(v6) => ("ipv6", v6.to_string()),
    };
    let cmd =
        format!("netsh interface {family} add dnsservers {index} {addr_str} index={index_pos}");
    exe_cmd(&cmd)
}
pub fn clear_dns_servers(index: u32, is_ipv4: bool) -> io::Result<()> {
    let family = if is_ipv4 { "ipv4" } else { "ipv6" };
    let cmd = format!("netsh interface {family} set dnsservers {index} source=dhcp");
    exe_cmd(&cmd)
}
pub fn set_dns_servers(index: u32, dns_servers: &[IpAddr]) -> io::Result<()> {
    if dns_servers.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "DNS servers list cannot be empty",
        ));
    }

    let is_ipv4 = dns_servers[0].is_ipv4();
    if !dns_servers.iter().all(|addr| addr.is_ipv4() == is_ipv4) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "All DNS servers must be either IPv4 or IPv6",
        ));
    }

    clear_dns_servers(index, is_ipv4)?;

    set_primary_dns(index, dns_servers[0])?;

    for (i, &addr) in dns_servers.iter().skip(1).enumerate() {
        add_secondary_dns(index, addr, (i + 2) as u32)?;
    }

    Ok(())
}
