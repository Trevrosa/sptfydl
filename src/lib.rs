use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::{LazyLock, OnceLock},
};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::warn;

pub mod spotify;
pub mod ytmusic;

pub static CLIENT: LazyLock<Client> = LazyLock::new(Client::new);

/// Find the config directory, creating it if it doesn't exist. If all env vars are not found, defaults to `./`
///
/// # Linux/Unix
///
/// defaults to `XDG_CONFIG_HOME`, falling back to `HOME/.config`
///
/// # Windows
///
/// defaults to `USERPROFILE/.config`
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

/// Save `T` to file `name` at the config dir found by [`save_dir`].
///
/// # Errors
///
/// - See [`fs::write`].
/// - [`serde_yaml`] serialization failed.
pub fn save<T: Serialize>(obj: &T, name: &str) -> anyhow::Result<()> {
    fs::write(save_dir().join(name), serde_yaml::to_string(obj)?)?;
    Ok(())
}

/// Load file `name` as `T` from the config dir found by [`save_dir`].
///
/// # Errors
///
/// - See [`fs::write`].
/// - [`serde_yaml`] deserialization failed.
pub fn load<T: for<'a> Deserialize<'a>>(name: &str) -> anyhow::Result<T> {
    let file = fs::read_to_string(save_dir().join(name))?;
    Ok(serde_yaml::from_str(&file)?)
}

/// Save `contents` to file `name` at the config dir found by [`save_dir`].
///
/// # Errors
///
/// See [`fs::write`].
pub fn save_str(contents: &str, name: &str) -> io::Result<()> {
    fs::write(save_dir().join(name), contents)
}

/// Load `file` as a string from the config dir found by [`save_dir`].
///
/// # Errors
///
/// See [`fs::write`].
pub fn load_str(name: &str) -> io::Result<String> {
    let file = fs::read_to_string(save_dir().join(name))?;
    Ok(file)
}

pub trait IterExt<I: Iterator> {
    /// Joins the iterator to a string with separator `sep`.
    fn join(self, sep: &str) -> String;
}

impl<I: Iterator> IterExt<I> for I
where
    I::Item: AsRef<str>,
{
    fn join(self, sep: &str) -> String {
        crate::join(self, sep)
    }
}

/// Joins an iterator `I` to a string with separator `sep`.
pub fn join<I: Iterator>(mut iter: I, sep: &str) -> String
where
    I::Item: AsRef<str>,
{
    let Some(first) = iter.next() else {
        return String::new();
    };

    iter.fold(first.as_ref().to_string(), |mut acc, cur| {
        acc.push_str(sep);
        acc.push_str(cur.as_ref());
        acc
    })
}

#[cfg(test)]
mod tests {
    use crate::IterExt;

    #[test]
    fn join() {
        let list = vec!["1", "2", "3"];

        assert_eq!(list.iter().join(","), "1,2,3");
        assert_eq!(list.iter().join(", "), "1, 2, 3");
        assert_eq!(list.iter().map(ToString::to_string).join(", "), "1, 2, 3");

        let list = ["one", "two", "three"].map(ToString::to_string);

        assert_eq!(list.iter().join(","), "one,two,three");
        assert_eq!(list.iter().join(", "), "one, two, three");

        let list = ["h", "i", "!"].repeat(500);

        assert_ne!(list.iter().join(",").pop(), Some(','));
    }
}
