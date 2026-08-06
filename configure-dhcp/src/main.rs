//! configure-dhcp — event WiFi DHCP for BigFred OS.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use configure_dhcp::dhcp::DhcpDefaults;
use configure_dhcp::run::{run_check, run_up, Paths};
use configure_dhcp::stack::omada::OmadaStack;
use configure_dhcp::stack::Registry;

#[derive(Parser, Debug)]
#[command(
    name = "configure-dhcp",
    about = "Start dnsmasq when an event WiFi stack (Omada) is detected on the LAN",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Detect stacks, enable DHCP if needed, promote reservations (default)
    Up,
    /// Alias for up
    Configure,
    /// Alias for up
    Start,
    /// Report stacks, detection, and DHCP status
    Check,
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();
    let mut registry = Registry::new();
    registry.register(Box::new(OmadaStack::new()));

    let paths = Paths::default();
    let defaults = DhcpDefaults::default();

    match cli.command.unwrap_or(Commands::Up) {
        Commands::Up | Commands::Configure | Commands::Start => match run_up(&registry, &paths, &defaults)
        {
            Ok(report) => {
                if report.gate_on {
                    log::info!(
                        "configure-dhcp: gate ON ({}); iface={}; reservations_changed={}",
                        report.reason,
                        report.iface,
                        report.reservations_changed
                    );
                    for d in &report.devices {
                        log::info!(
                            "  detected {} {:?} {:?}",
                            d.stack,
                            d.mac.as_ref().map(ToString::to_string),
                            d.ip
                        );
                    }
                } else {
                    log::info!("configure-dhcp: {}", report.reason);
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                log::error!("configure-dhcp: {e}");
                ExitCode::FAILURE
            }
        },
        Commands::Check => match run_check(&registry, &paths, &defaults) {
            Ok(code) => ExitCode::from(code as u8),
            Err(e) => {
                log::error!("configure-dhcp: {e}");
                ExitCode::FAILURE
            }
        },
    }
}
