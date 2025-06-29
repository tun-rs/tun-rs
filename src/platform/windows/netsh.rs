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
pub fn set_interface_metric(index: u32, metric: u16) -> io::Result<()> {
    let cmd = format!("netsh interface ip set interface {index} metric={metric}");
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
pub fn exe_command(cmd: &mut Command) -> io::Result<()> {
    let out = cmd.creation_flags(CREATE_NO_WINDOW).output()?;
    let command = cmd
        .get_args()
        .map(|s| s.to_string_lossy().to_string())
        .collect::<Vec<String>>();
    output(&command.join(" ").to_string(), out)
}
pub fn delete_interface_ip(index: u32, address: IpAddr) -> io::Result<()> {
    let cmd = format!(
        "netsh interface {} delete address {index} {address}",
        if address.is_ipv4() { "ip" } else { "ipv6" }
    );
    exe_cmd(&cmd)
}

/// 设置网卡ip
pub fn set_interface_ip(
    index: u32,
    address: IpAddr,
    netmask: IpAddr,
    gateway: Option<IpAddr>,
) -> io::Result<()> {
    let mut binding = Command::new("netsh");

    let cmd = if address.is_ipv4() {
        binding
            .arg("interface")
            .arg("ipv4")
            .arg("set")
            .arg("address")
            .arg(index.to_string().as_str())
            .arg("source=static")
            .arg(format!("address={address}",).as_str())
            .arg(format!("mask={netmask}",).as_str())
    } else {
        let prefix_len = ipnet::ip_mask_to_prefix(netmask)
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        binding
            .arg("interface")
            .arg("ipv6")
            .arg("set")
            .arg("address")
            .arg(index.to_string().as_str())
            .arg(format!("address={address}/{prefix_len}").as_str())
    };

    if let Some(gateway) = gateway {
        _ = cmd.arg(format!("gateway={gateway}",).as_str());
    }
    exe_command(cmd)
}

pub fn set_interface_mtu(index: u32, mtu: u32) -> io::Result<()> {
    let cmd = format!("netsh interface ipv4 set subinterface {index}  mtu={mtu} store=persistent",);
    exe_cmd(&cmd)
}
pub fn set_interface_mtu_v6(index: u32, mtu: u32) -> io::Result<()> {
    let cmd = format!("netsh interface ipv6 set subinterface {index}  mtu={mtu} store=persistent",);
    exe_cmd(&cmd)
}
