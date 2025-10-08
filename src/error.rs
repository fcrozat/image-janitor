use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum JanitorError {
    #[error("I/O error")]
    Io(#[from] std::io::Error),

    #[error("Regex error")]
    Regex(#[from] regex::Error),

    #[error("Walkdir error")]
    Walkdir(#[from] walkdir::Error),

    #[error("Command failed: {0}")]
    Command(String),

    #[error("No kernel modules directory found in {0}")]
    NoKernelDir(PathBuf),

    #[error("Path does not have a string representation: {0}")]
    InvalidPath(PathBuf),

    #[error("Could not read config file '{0}': {1}")]
    ConfigRead(String, std::io::Error),
}
