pub mod access_token;
pub use access_token::AccessToken;

pub mod search;
use anyhow::anyhow;
use dialoguer::Select;
pub use search::get_from_url;

use std::io::{Write, stdin, stdout};

use tracing::{debug, info, warn};

use crate::{
    load, load_str, save, save_str,
    spotify::search::SpotifyTrack,
    ytmusic::auth::{Browser, parse_cookie},
};

use super::ytmusic;

const SPOTIFY_TOKEN_CONFIG_NAME: &str = "spotify_token.yaml";
const YTM_DATA_CONFIG_NAME: &str = "ytm_browser_data";

pub fn extract_spotify(
    id: &str,
    secret: &str,
    spotify_url: &str,
    no_interaction: bool,
) -> anyhow::Result<(Vec<String>, Option<String>)> {
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
            .find(|l| l.as_ref().is_ok_and(|l| l.starts_with("Cookie: ")))
            .expect("waiting forever for line")?;

        if let Err(err) = save_str(&cookie, YTM_DATA_CONFIG_NAME) {
            warn!("failed to save yt cookie: {err}");
        }

        cookie
    };
    let cookie = parse_cookie(&raw_cookie).ok_or(anyhow!("failed to parse cookie"))?;

    let auth = Browser::new(cookie);

    let urls = get_youtube(spotify_tracks, auth, no_interaction)?;

    Ok((urls, name))
}

fn get_youtube(
    spotify_tracks: Vec<SpotifyTrack>,
    auth: Browser,
    no_interaction: bool,
) -> anyhow::Result<Vec<String>> {
    let mut urls = Vec::with_capacity(spotify_tracks.len());

    for (i, spotify_track) in spotify_tracks.iter().enumerate() {
        debug!("extracted metadata: {spotify_track:#?}");
        let spotify_artists = spotify_track
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>();

        info!("finding track {}", i + 1);

        let query = format!("{} {}", spotify_track.name, spotify_artists.join(" "));
        let searched = ytmusic::search(query.as_str(), None, auth.as_ref())?;

        if !searched.status().is_success() {
            let err = anyhow!(
                "ytm api search endpoint failed with {}: {:?}",
                searched.status(),
                searched.text()
            );
            return Err(err);
        }

        let results = searched.json()?;

        let Some(mut results) = ytmusic::parse_results(&results) else {
            return Err(anyhow!("couldnt parse search results"));
        };

        if results.is_empty() {
            return Err(anyhow!("search results was empty"));
        }

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
        urls.push(url);
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
