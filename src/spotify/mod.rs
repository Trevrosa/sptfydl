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

/// Returns `Vec<(usize, String)>` because some tracks may not be found from ytmusic,
/// so some tracks may be missing,
/// so we return the track number as well
pub fn extract_spotify(
    id: &str,
    secret: &str,
    spotify_url: &str,
    no_interaction: bool,
) -> anyhow::Result<(Vec<(usize, String)>, Option<String>)> {
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

    let urls = get_youtube(name.as_deref(), &spotify_tracks, &auth, no_interaction)?;

    Ok((urls, name))
}

const RETRY_DELAY: Duration = Duration::from_secs(5);

const MAX_RETRIES: usize = 3;

fn get_youtube(
    name: Option<&str>,
    spotify_tracks: &[SpotifyTrack],
    auth: &Browser,
    no_interaction: bool,
) -> anyhow::Result<Vec<(usize, String)>> {
    let mut urls = Vec::with_capacity(spotify_tracks.len());

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

            let results = searched.json()?;

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

            0
        } else {
            Select::new()
                .with_prompt("Choose link to download")
                .default(0)
                .items(&results)
                .interact()?
        };

        let url = results[choice].link().to_string();
        urls.push((i, url));
    }

    if !failed.is_empty() {
        warn!("{} songs failed, check report", failed.len());

        let report = failed.iter().fold(String::new(), |mut report, (n, t)| {
            let _ = write!(report, "track #{n}: {t:#?}\n");
            report
        });
        if let Some(name) = name {
            let _ = fs::write(format!("failed-{name}.txt"), report);
        } else {
            let name = Utc::now().timestamp();
            let _ = fs::write(format!("failed-{name}.txt"), report);
        }
    }

    Ok(urls)
}

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
