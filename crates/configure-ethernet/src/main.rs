//! configure-ethernet — bring up the first Ethernet interface.

use std::fs;
use std::io::Write;
use std::net::Ipv4Addr;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

use clap::{Parser, Subcommand};
use thiserror::Error;

const DEFAULT_CONFIG_PATH: &str = "/data/etc/configure-ethernet.conf";
const DEFAULT_PRIMARY: &str = "192.168.0.120";
const DEFAULT_SECONDARY: &str = "192.168.1.120";
const DEFAULT_PREFIX_LEN: u8 = 24;
const PING_COUNT: &str = "1";
const PING_TIMEOUT_SEC: &str = "2";
const DHCP_WAIT: Duration = Duration::from_secs(5);

const IP_BIN: &str = "/sbin/ip";
const DHCLIENT_BIN: &str = "/sbin/dhclient";
const PING_BIN: &str = "/bin/ping";

#[derive(Debug, Error)]
enum Error {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Settings {
    primary_addr: String,
    secondary_addr: String,
}

#[derive(Parser, Debug)]
#[command(
    name = "configure-ethernet",
    about = "Bring up Ethernet (static / DHCP) for BigFred OS",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Configure once and exit (default)
    Up,
    Configure,
    Start,
    /// Exit 0 if link+IPv4 look OK
    Check,
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    match cli.command.unwrap_or(Commands::Up) {
        Commands::Up | Commands::Configure | Commands::Start => {
            if let Err(e) = run_configure(Path::new(DEFAULT_CONFIG_PATH)) {
                eprintln!("configure-ethernet: {e}");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Commands::Check => {
            if check_connected() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

fn run_configure(config_path: &Path) -> Result<()> {
    let cfg = load_or_create_config(config_path, DEFAULT_PRIMARY, DEFAULT_SECONDARY)?;
    let _ = run_cmd(IP_BIN, &["link", "set", "lo", "up"]);
    if connect(&cfg) {
        return Ok(());
    }
    Err(Error::Other(
        "failed to configure ethernet (static and DHCP)".into(),
    ))
}

fn check_connected() -> bool {
    let Ok(iface) = first_ethernet_interface() else {
        return false;
    };
    iface_link_up(&iface) && iface_has_ipv4(&iface)
}

fn connect(cfg: &Settings) -> bool {
    let Ok(iface) = first_ethernet_interface() else {
        eprintln!("configure-ethernet: no Ethernet interface found");
        return false;
    };
    println!("configure-ethernet: using interface {iface}");
    let _ = run_cmd("/bin/killall", &["dhclient"]);

    if try_static(&iface, &cfg.primary_addr) {
        println!(
            "configure-ethernet: static {} OK (gateway {})",
            cfg.primary_addr,
            gateway_for(&cfg.primary_addr)
        );
        return true;
    }
    if try_static(&iface, &cfg.secondary_addr) {
        println!(
            "configure-ethernet: static {} OK (gateway {})",
            cfg.secondary_addr,
            gateway_for(&cfg.secondary_addr)
        );
        return true;
    }
    if try_dhcp(&iface) {
        println!("configure-ethernet: DHCP OK");
        return true;
    }
    eprintln!("configure-ethernet: failed to configure {iface} (static and DHCP)");
    false
}

fn iface_link_up(iface: &str) -> bool {
    let Ok(out) = Command::new(IP_BIN)
        .args(["link", "show", "dev", iface])
        .output()
    else {
        return false;
    };
    let s = String::from_utf8_lossy(&out.stdout);
    s.contains("state UP") || s.contains(",UP")
}

fn load_or_create_config(
    path: &Path,
    default_primary: &str,
    default_secondary: &str,
) -> Result<Settings> {
    let defaults = Settings {
        primary_addr: default_primary.to_string(),
        secondary_addr: default_secondary.to_string(),
    };
    match fs::read_to_string(path) {
        Ok(text) => Ok(parse_config(&text, defaults)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Err(w) = write_config(path, &defaults) {
                eprintln!("warning: cannot write {}: {w}", path.display());
            }
            Ok(defaults)
        }
        Err(e) => Err(e.into()),
    }
}

fn parse_config(text: &str, defaults: Settings) -> Settings {
    let mut cfg = defaults;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if value.parse::<Ipv4Addr>().is_err() {
            continue;
        }
        match key.to_ascii_uppercase().as_str() {
            "PRIMARY" | "PRIMARY_ADDRESS" | "ADDRESS" => cfg.primary_addr = value.to_string(),
            "SECONDARY" | "SECONDARY_ADDRESS" | "FALLBACK" | "FALLBACK_ADDRESS" => {
                cfg.secondary_addr = value.to_string();
            }
            _ => {}
        }
    }
    cfg
}

fn write_config(path: &Path, cfg: &Settings) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = format!(
        "# configure-ethernet static addresses (edit to match club subnet)\n\
PRIMARY={}\n\
SECONDARY={}\n",
        cfg.primary_addr, cfg.secondary_addr
    );
    let mut f = fs::File::create(path)?;
    f.write_all(content.as_bytes())?;
    Ok(())
}

fn first_ethernet_interface() -> Result<String> {
    let dir = Path::new("/sys/class/net");
    let mut names = Vec::new();
    for ent in fs::read_dir(dir)? {
        let ent = ent?;
        let name = ent.file_name().to_string_lossy().into_owned();
        if name == "lo" {
            continue;
        }
        if is_wireless(&name) {
            continue;
        }
        names.push(name);
    }
    names.sort();
    names
        .into_iter()
        .next()
        .ok_or_else(|| Error::Other("no Ethernet interface found".into()))
}

fn is_wireless(iface: &str) -> bool {
    Path::new("/sys/class/net")
        .join(iface)
        .join("wireless")
        .exists()
}

fn try_static(iface: &str, addr: &str) -> bool {
    let gw = gateway_for(addr);
    if gw.is_empty() {
        return false;
    }
    if let Err(e) = configure_static(iface, addr) {
        eprintln!("configure-ethernet: static {addr} on {iface}: {e}");
        return false;
    }
    if ping_host(&gw) {
        return true;
    }
    eprintln!("configure-ethernet: no reply from gateway {gw}");
    false
}

fn configure_static(iface: &str, addr: &str) -> Result<()> {
    run_cmd(IP_BIN, &["link", "set", "dev", iface, "up"])?;
    run_cmd(IP_BIN, &["addr", "flush", "dev", iface])?;
    let cidr = format!("{addr}/{DEFAULT_PREFIX_LEN}");
    run_cmd(IP_BIN, &["addr", "add", &cidr, "dev", iface])
}

fn ping_host(host: &str) -> bool {
    run_cmd(PING_BIN, &["-c", PING_COUNT, "-W", PING_TIMEOUT_SEC, host]).is_ok()
}

fn try_dhcp(iface: &str) -> bool {
    let _ = run_cmd(IP_BIN, &["addr", "flush", "dev", iface]);
    let _ = run_cmd(IP_BIN, &["link", "set", "dev", iface, "up"]);
    if let Err(e) = run_cmd(DHCLIENT_BIN, &[iface]) {
        eprintln!("configure-ethernet: dhclient on {iface}: {e}");
        return false;
    }
    thread::sleep(DHCP_WAIT);
    if !iface_has_ipv4(iface) {
        eprintln!("configure-ethernet: no IPv4 address on {iface} after DHCP");
        return false;
    }
    if let Some(gw) = default_gateway() {
        if ping_host(&gw) {
            return true;
        }
    }
    iface_has_ipv4(iface)
}

fn iface_has_ipv4(iface: &str) -> bool {
    let Ok(out) = Command::new(IP_BIN)
        .args(["-4", "addr", "show", "dev", iface])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).contains("inet ")
}

fn default_gateway() -> Option<String> {
    let out = Command::new(IP_BIN)
        .args(["route", "show", "default"])
        .output()
        .ok()?;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        for i in 0..fields.len() {
            if fields[i] == "via" {
                if let Some(gw) = fields.get(i + 1) {
                    return Some((*gw).to_string());
                }
            }
        }
    }
    None
}

fn gateway_for(addr: &str) -> String {
    let Ok(ip) = addr.parse::<Ipv4Addr>() else {
        return String::new();
    };
    let o = ip.octets();
    Ipv4Addr::new(o[0], o[1], o[2], 1).to_string()
}

fn run_cmd(bin: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(bin)
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "{bin} {:?} exited {}",
            args,
            status.code().unwrap_or(-1)
        )))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_config_defaults() {
        let defaults = Settings {
            primary_addr: "192.168.0.120".into(),
            secondary_addr: "192.168.1.120".into(),
        };
        let cfg = parse_config("", defaults.clone());
        assert_eq!(cfg, defaults);
    }

    #[test]
    fn parse_config_overrides() {
        let text = "# club\nPRIMARY=10.0.0.50\nSECONDARY=10.0.1.50\n";
        let defaults = Settings {
            primary_addr: "192.168.0.120".into(),
            secondary_addr: "192.168.1.120".into(),
        };
        let cfg = parse_config(text, defaults);
        assert_eq!(cfg.primary_addr, "10.0.0.50");
        assert_eq!(cfg.secondary_addr, "10.0.1.50");
    }

    #[test]
    fn parse_config_ignores_invalid_ip() {
        let text = "PRIMARY=not-an-ip\nSECONDARY=192.168.1.99\n";
        let defaults = Settings {
            primary_addr: "192.168.0.120".into(),
            secondary_addr: "192.168.1.120".into(),
        };
        let cfg = parse_config(text, defaults);
        assert_eq!(cfg.primary_addr, "192.168.0.120");
        assert_eq!(cfg.secondary_addr, "192.168.1.99");
    }

    #[test]
    fn gateway_for_works() {
        assert_eq!(gateway_for("192.168.0.120"), "192.168.0.1");
        assert_eq!(gateway_for("10.20.30.40"), "10.20.30.1");
        assert_eq!(gateway_for("bad"), "");
    }

    #[test]
    fn load_or_create_writes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("configure-ethernet.conf");
        let cfg = load_or_create_config(&path, "192.168.0.120", "192.168.1.120").unwrap();
        assert_eq!(cfg.primary_addr, "192.168.0.120");
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("PRIMARY=192.168.0.120"));
    }

    #[test]
    fn load_reads_existing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.conf");
        fs::write(&path, "PRIMARY=172.16.0.8\nSECONDARY=172.16.1.8\n").unwrap();
        let cfg = load_or_create_config(&path, "192.168.0.120", "192.168.1.120").unwrap();
        assert_eq!(cfg.primary_addr, "172.16.0.8");
        assert_eq!(cfg.secondary_addr, "172.16.1.8");
    }
}
