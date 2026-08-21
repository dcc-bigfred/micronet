//! micronet — Ethernet bring-up and DHCP gateway daemon.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use micronet::daemon;
use micronet::datadir;
use micronet::ipc::{self, Request};
use micronet::version;

#[derive(Parser, Debug)]
#[command(
    name = "micronet",
    about = "Ethernet bring-up and DHCP gateway for BigFred OS",
    version = env!("CARGO_PKG_VERSION")
)]
struct Cli {
    /// Override DATA_DIR (absolute path) before start
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

    /// Config file path (default $DATA_DIR/etc/micronet.json)
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Control socket (default $DATA_DIR/run/micronet.sock)
    #[arg(long, global = true)]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the network daemon (default)
    Serve,
    /// Alias for serve
    Run,
    /// One-shot probe + apply
    Apply,
    /// Query daemon status (JSON)
    Status,
    /// Exit 0 when iface is UP with IPv4 (microinit liveness)
    Check,
    /// Re-run DHCP probe + apply
    Reconfigure,
    /// Print build / release metadata
    Info,
}

fn argv0_basename() -> Option<String> {
    std::env::args_os()
        .next()
        .as_ref()
        .map(Path::new)
        .and_then(Path::file_name)
        .and_then(OsStr::to_str)
        .map(str::to_string)
}

fn main() -> ExitCode {
    if let Some(name) = argv0_basename() {
        if name == "configure-ethernet" || name == "configure-dhcp" {
            return alias_main(&name);
        }
    }

    let cli = Cli::parse();
    if let Some(dir) = &cli.data_dir {
        datadir::set_root(dir);
    }
    let config_path = daemon::resolve_config(cli.config.as_ref());
    let socket = daemon::resolve_socket(cli.socket.as_ref());
    dispatch(
        cli.command.unwrap_or(Commands::Serve),
        &config_path,
        &socket,
    )
}

fn alias_main(argv0: &str) -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut data_dir = None;
    let mut config = None;
    let mut socket = None;
    let mut cmd = "apply";
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--data-dir" if i + 1 < args.len() => {
                data_dir = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--config" if i + 1 < args.len() => {
                config = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--socket" if i + 1 < args.len() => {
                socket = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "up" | "configure" | "start" | "apply" => {
                cmd = "apply";
                i += 1;
            }
            "check" => {
                cmd = "check";
                i += 1;
            }
            "serve" | "run" => {
                cmd = "serve";
                i += 1;
            }
            other if other.starts_with('-') => {
                eprintln!("{argv0}: unknown option {other}");
                return ExitCode::FAILURE;
            }
            other => {
                eprintln!("{argv0}: unknown command {other}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Some(dir) = &data_dir {
        datadir::set_root(dir);
    }
    let config_path = daemon::resolve_config(config.as_ref());
    let socket_path = daemon::resolve_socket(socket.as_ref());
    let command = match cmd {
        "serve" => Commands::Serve,
        "check" => Commands::Check,
        _ => Commands::Apply,
    };
    dispatch(command, &config_path, &socket_path)
}

fn dispatch(command: Commands, config_path: &Path, socket: &Path) -> ExitCode {
    match command {
        Commands::Serve | Commands::Run => {
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                .init();
            match daemon::run(config_path, socket) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    log::error!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Commands::Apply => {
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                .init();
            match daemon::apply_once(config_path) {
                Ok(s) => {
                    log::info!("mode {}", s.mode.as_str());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    log::error!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Commands::Status => match ipc::call(socket, &Request::Status) {
            Ok(resp) => match serde_json::to_string_pretty(&resp) {
                Ok(s) => {
                    println!("{s}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            },
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Commands::Check => match daemon::check_liveness(socket) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Commands::Reconfigure => match ipc::call(socket, &Request::Reconfigure) {
            Ok(ipc::Response::Ok) => ExitCode::SUCCESS,
            Ok(ipc::Response::Error { message }) => {
                eprintln!("{message}");
                ExitCode::FAILURE
            }
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Commands::Info => {
            println!("{}", version::format_info(&version::info()));
            ExitCode::SUCCESS
        }
    }
}
