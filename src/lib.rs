use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::{LazyLock, OnceLock},
};
use tracing::warn;

pub mod spotify;
pub mod ytmusic;

pub static CLIENT: LazyLock<Client> = LazyLock::new(Client::new);

fn save_dir<'a>() -> &'a Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();

    let config_dir = if let Ok(config) = env::var("XDG_CONFIG_HOME") {
        Path::new(&config).to_path_buf()
    } else if let Ok(home) = env::var("HOME") {
        Path::new(&home).join(".config")
    } else if let Ok(userprofile) = env::var("USERPROFILE") {
        Path::new(&userprofile).join(".config")
    } else {
        warn!("could not find home directory, using cwd");
        Path::new("./").to_path_buf()
    };

    let config_dir = config_dir.join("sptfydl");

    if !config_dir.exists() {
        fs::create_dir_all(&config_dir).expect("failed to create config dir");
    }

    DIR.get_or_init(|| config_dir)
}

/// # Errors
///
/// - See [`fs::write`].
/// - [`serde_yaml`] serialization failed.
pub fn save<T: Serialize>(obj: &T, name: &str) -> anyhow::Result<()> {
    fs::write(save_dir().join(name), serde_yaml::to_string(obj)?)?;
    Ok(())
}

/// # Errors
///
/// - See [`fs::write`].
/// - [`serde_yaml`] deserialization failed.
pub fn load<T: for<'a> Deserialize<'a>>(name: &str) -> anyhow::Result<T> {
    let file = fs::read_to_string(save_dir().join(name))?;
    Ok(serde_yaml::from_str(&file)?)
}

/// # Errors
///
/// See [`fs::write`].
pub fn save_str(contents: &str, name: &str) -> io::Result<()> {
    fs::write(save_dir().join(name), contents)
}

/// # Errors
///
/// See [`fs::write`].
pub fn load_str(name: &str) -> io::Result<String> {
    let file = fs::read_to_string(save_dir().join(name))?;
    Ok(file)
}
