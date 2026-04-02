use std::{io, path::Path};

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Usage(String),
    #[error("invalid environment variable name {0}")]
    InvalidEnvName(String),
    #[error("cannot use --env-dir and --unset-env for the same variable {0}")]
    ConflictingEnv(String),
    #[error("cannot unset reserved environment variable {0}")]
    ReservedUnsetEnv(String),
    #[error("cannot expand {path} without a home directory")]
    MissingHomeDirectory { path: String },
    #[error("unsupported home expansion in path {0}")]
    UnsupportedHomeExpansion(String),
    #[error("config file {path}: {source}")]
    ConfigParse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("config file {path}: version must be 1")]
    UnsupportedConfigVersion { path: String },
    #[error("config file {path}: write entry {entry} must be absolute or start with ~/")]
    RelativeConfigPath { path: String, entry: String },
    #[error("{action}: {source}")]
    Io {
        action: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("{action} {path}: {source}")]
    IoPath {
        action: &'static str,
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("{0}")]
    Sandbox(String),
    #[error("path contains a newline: {0}")]
    PathContainsNewline(String),
}

impl Error {
    pub fn io(action: &'static str, source: io::Error) -> Self {
        Self::Io { action, source }
    }

    pub fn io_path(action: &'static str, path: &Path, source: io::Error) -> Self {
        Self::IoPath {
            action,
            path: path.display().to_string(),
            source,
        }
    }
}
