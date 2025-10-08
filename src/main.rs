use anyhow::Result;
use clap::Parser;
use env_logger::Env;
use image_janitor::{driver, firmware};
use log::info;
use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging.
    #[arg(short, long, default_value_t = false)]
    verbose: bool,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Cleans up unused kernel drivers.
    DriverCleanup {
        /// Really delete the files.
        #[arg(long)]
        delete: bool,

        /// Directory with kernel modules.
        #[arg(long, default_value = "/lib/modules")]
        module_dir: PathBuf,

        /// Paths to module list configuration files.
        #[arg(long, default_value = "module.list,module.list.extra")]
        config_files: String,
    },
    /// Cleans up unused firmware.
    FwCleanup {
        /// Really delete the files.
        #[arg(long)]
        delete: bool,

        /// Directory with kernel modules.
        #[arg(long, default_value = "/lib/modules")]
        module_dir: PathBuf,

        /// Directory with firmware files.
        #[arg(long, default_value = "/lib/firmware")]
        firmware_dir: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_level = if cli.verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(Env::default().default_filter_or(log_level)).init();

    match &cli.command {
        Commands::DriverCleanup {
            delete,
            module_dir,
            config_files,
        } => {
            info!(
                "Driver cleanup running. Delete: {}, Module Dir: {}",
                delete,
                module_dir.display()
            );
            let config_paths: Vec<&str> = config_files.split(',').collect();
            driver::cleanup_drivers(&config_paths, module_dir, *delete)?;
        }
        Commands::FwCleanup {
            delete,
            module_dir,
            firmware_dir,
        } => {
            info!(
                "Firmware cleanup running. Delete: {}, Module Dir: {}, Firmware Dir: {}",
                delete,
                module_dir.display(),
                firmware_dir.display()
            );
            firmware::cleanup_firmware(module_dir, firmware_dir, *delete)?;
        }
    }

    Ok(())
}