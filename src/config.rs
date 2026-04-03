use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::error::{Error, Result};

#[cfg(target_os = "macos")]
const DEFAULT_XDG_CONFIG_DIRS: &str = "/opt/homebrew/etc/xdg:/usr/local/etc/xdg:/etc/xdg";
#[cfg(target_os = "linux")]
const DEFAULT_XDG_CONFIG_DIRS: &str = "/etc/xdg";

#[derive(Debug, Clone, Default)]
pub enum ConfigMode {
    #[default]
    Default,
    None,
    Explicit(PathBuf),
}

#[derive(Debug, Default)]
pub struct WriteConfig {
    pub required: Vec<PathBuf>,
    pub optional: Vec<PathBuf>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    write: Vec<PathBuf>,
    #[serde(default)]
    optional_write: Vec<PathBuf>,
}

pub fn load(mode: &ConfigMode, home: Option<&Path>) -> Result<WriteConfig> {
    let mut config = WriteConfig::default();
    for path in config_paths(mode, home) {
        if !path.exists() {
            if matches!(mode, ConfigMode::Explicit(_)) {
                return Err(Error::io_path(
                    "read config file",
                    &path,
                    std::io::Error::from(std::io::ErrorKind::NotFound),
                ));
            }
            continue;
        }
        let raw = fs::read_to_string(&path)
            .map_err(|err| Error::io_path("read config file", &path, err))?;
        let parsed: RawConfig = toml::from_str(&raw).map_err(|err| Error::ConfigParse {
            path: path.display().to_string(),
            source: err,
        })?;
        if parsed.version.unwrap_or(1) != 1 {
            return Err(Error::UnsupportedConfigVersion {
                path: path.display().to_string(),
            });
        }
        validate_config_paths(&path, &parsed.write)?;
        validate_config_paths(&path, &parsed.optional_write)?;
        config.required.extend(parsed.write);
        config.optional.extend(parsed.optional_write);
    }
    Ok(config)
}

fn config_paths(mode: &ConfigMode, home: Option<&Path>) -> Vec<PathBuf> {
    match mode {
        ConfigMode::None => Vec::new(),
        ConfigMode::Explicit(path) => vec![path.clone()],
        ConfigMode::Default => {
            let mut out = Vec::new();
            let roots = env::var_os("XDG_CONFIG_DIRS")
                .and_then(|value| value.into_string().ok())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_XDG_CONFIG_DIRS.to_string());
            for root in roots.split(':').filter(|root| root.starts_with('/')) {
                out.push(PathBuf::from(root).join("sbrun").join("config.toml"));
            }
            if let Some(path) = user_config_path(home) {
                out.push(path);
            }
            out
        }
    }
}

fn user_config_path(home: Option<&Path>) -> Option<PathBuf> {
    if let Some(root) = env::var_os("XDG_CONFIG_HOME")
        && Path::new(&root).is_absolute()
    {
        return Some(PathBuf::from(root).join("sbrun").join("config.toml"));
    }
    home.map(|home| home.join(".config").join("sbrun").join("config.toml"))
}

fn validate_config_paths(config_path: &Path, entries: &[PathBuf]) -> Result<()> {
    for entry in entries {
        if entry.is_absolute() || starts_with_tilde(entry) {
            continue;
        }
        return Err(Error::RelativeConfigPath {
            path: config_path.display().to_string(),
            entry: entry.display().to_string(),
        });
    }
    Ok(())
}

fn starts_with_tilde(path: &Path) -> bool {
    path.as_os_str().as_encoded_bytes().starts_with(b"~")
}
