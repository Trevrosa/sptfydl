pub mod access_token;
pub use access_token::AccessToken;

pub mod search;
use anyhow::anyhow;
use chrono::Utc;
use dialoguer::Select;
pub use search::get_from_url;

use std::{
    fmt::Write as FmtWrite,
    fs,
    io::{Write, stdin, stdout},
    thread,
    time::Duration,
};

use tracing::{debug, info, warn};

use crate::{
    load, load_str, save, save_str,
    spotify::search::SpotifyTrack,
    ytmusic::auth::{Browser, parse_cookie},
};

use super::ytmusic;

const SPOTIFY_TOKEN_CONFIG_NAME: &str = "spotify_token.yaml";
const YTM_DATA_CONFIG_NAME: &str = "ytm_browser_data";

#[derive(Debug)]
pub struct Extraction {
    pub urls: Vec<(usize, String)>,
    pub name: Option<String>,
    /// guaranteed to be in range of `urls`
    pub warnings: Vec<usize>,
    pub failures: usize,
}

impl Extraction {
    #[must_use]
    pub fn warning_urls(&self) -> Vec<&String> {
        self.warnings.iter().map(|idx| &self.urls[*idx].1).collect()
    }
}

/// Returns `Vec<(usize, String)>` because some tracks may not be found from ytmusic,
/// so some tracks may be missing,
/// so we return the track number as well
///
/// # Errors
///
/// This function fails if:
/// - We could not get a new [`AccessToken`], and one is not cached.
/// - Cookies were required to be prompted and `no_interaction` was true.
/// - We got no urls from ytmusic.
///
/// # Panics
///
/// This function panics if we could not get the cookies from `stdin`.
pub fn extract_spotify(
    id: &str,
    secret: &str,
    spotify_url: &str,
    no_interaction: bool,
) -> anyhow::Result<Extraction> {
    let token = load::<AccessToken>(SPOTIFY_TOKEN_CONFIG_NAME);

    let token = if let Ok(token) = token {
        debug!("got spotify token from cache");
        token
    } else {
        request_token_and_save(id, secret)?
    };

    let token = if token.expired() {
        request_token_and_save(id, secret)?
    } else {
        token
    };

    let (spotify_tracks, name) = get_from_url(spotify_url, token)?;

    info!("got {} tracks", spotify_tracks.len());

    let raw_cookie = if let Ok(cookie) = load_str(YTM_DATA_CONFIG_NAME) {
        cookie
    } else {
        info!("no saved ytm cookie, need input.");
        info!(
            "please go to https://music.youtube.com and copy-paste your headers to an authenticatied POST request. (https://ytmusicapi.readthedocs.io/en/stable/setup/browser.html#copy-authentication-headers)"
        );

        // FIXME: could use dialoguer::Editor

        if no_interaction {
            return Err(anyhow!(
                "required interaction to set cookie but --no-interaction was set."
            ));
        }

        print!("input: ");
        let _ = stdout().flush();
        let cookie = stdin()
            .lines()
            .find(|l| {
                l.as_ref().is_ok_and(|l| {
                    l.starts_with("Cookie: ") || l.trim_ascii_start().starts_with("\"cookie:")
                })
            })
            .expect("waiting forever for line")?;

        if let Err(err) = save_str(&cookie, YTM_DATA_CONFIG_NAME) {
            warn!("failed to save yt cookie: {err}");
        }

        cookie
    };

    let cookie = parse_cookie(&raw_cookie).ok_or(anyhow!("failed to parse cookie"))?;
    let auth = Browser::new(cookie);

    let (urls, warnings, failed) = get_youtube(&spotify_tracks, &auth, no_interaction);

    if !failed.is_empty() {
        if !no_interaction {
            warn!("{} songs failed, check report", failed.len());
        }

        let report = failed.iter().fold(String::new(), |mut report, (n, t)| {
            let _ = writeln!(report, "track #{n}: {t:#?}");
            report
        });
        if let Some(ref name) = name {
            let _ = fs::write(format!("failed-{name}.txt"), report);
        } else {
            let name = Utc::now().timestamp();
            let _ = fs::write(format!("failed-{name}.txt"), report);
        }
    }

    if urls.is_empty() {
        Err(anyhow!("got no urls"))
    } else {
        Ok(Extraction {
            urls,
            name,
            warnings,
            failures: failed.len(),
        })
    }
}

const RETRY_DELAY: Duration = Duration::from_secs(5);

const MAX_RETRIES: usize = 3;

fn get_youtube<'a>(
    spotify_tracks: &'a [SpotifyTrack],
    auth: &Browser,
    no_interaction: bool,
) -> (
    Vec<(usize, String)>,
    Vec<usize>,
    Vec<(usize, &'a SpotifyTrack)>,
) {
    let mut urls = Vec::with_capacity(spotify_tracks.len());

    let mut warnings = Vec::new();
    let mut failed = Vec::new();

    'tracks: for (i, spotify_track) in spotify_tracks.iter().enumerate() {
        debug!("metadata: {spotify_track:#?}");
        let spotify_artists: Vec<&str> = spotify_track
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect();

        info!("finding track {}: {}", i + 1, spotify_track.name);

        let query = format!("{} {}", spotify_track.name, spotify_artists.join(" "));

        let mut tries = 0;
        let mut results = loop {
            tries += 1;

            if tries > MAX_RETRIES {
                failed.push((i + 1, spotify_track));
                warn!("reached max retries, skipping");
                continue 'tracks;
            }

            let searched = ytmusic::search(query.as_str(), None, auth.as_ref());

            let searched = match searched {
                Ok(resp) => resp,
                Err(err) => {
                    warn!("{err}, retrying in {RETRY_DELAY:?}");
                    thread::sleep(RETRY_DELAY);
                    continue;
                }
            };

            if !searched.status().is_success() {
                warn!(
                    "ytm api search endpoint failed with {}: {:?}",
                    searched.status(),
                    searched.text()
                );

                warn!("retrying in {RETRY_DELAY:?}");
                thread::sleep(RETRY_DELAY);
                continue;
            }

            let Ok(results) = searched.json() else {
                warn!("couldnt deserialize response as json, retrying in {RETRY_DELAY:?}");
                thread::sleep(RETRY_DELAY);
                continue;
            };

            let Some(results) = ytmusic::parse_results(&results) else {
                warn!("couldnt parse search results, retrying in {RETRY_DELAY:?}");
                thread::sleep(RETRY_DELAY);
                continue;
            };

            if results.is_empty() {
                warn!("search results was empty, retrying in {RETRY_DELAY:?}");
                thread::sleep(RETRY_DELAY);
                continue;
            }

            break results;
        };

        results[0].title.push_str("Best Result");

        debug!("got {} results", results.len());

        let choice = if no_interaction {
            debug!("choosing first result");

            results.iter().position(|r| r.video_id.is_some())
        } else {
            Select::new()
                .with_prompt("Choose link to download")
                .default(0)
                .items(&results)
                .interact()
                .ok()
        }
        .unwrap_or(0);

        if no_interaction && choice != 0 {
            warn!("--no-interaction was set but the best result was not available");
            warnings.push(i);
        }

        let url = results[choice].link_or_default().to_string();

        urls.push((i, url));
    }

    (urls, warnings, failed)
}

/// # Errors
///
/// This function fails if we could not get a new [`AccessToken`].
pub fn request_token_and_save(id: &str, secret: &str) -> anyhow::Result<AccessToken> {
    debug!("requesting new spotify access token");
    let Some(access_token) = AccessToken::get(id, secret) else {
        return Err(anyhow!("could not get access token"));
    };

    if let Err(err) = save(&access_token, SPOTIFY_TOKEN_CONFIG_NAME) {
        warn!("failed to save new access token: {err}");
    } else {
        debug!("saved new access token");
    }

    Ok(access_token)
}
