use anyhow::anyhow;
use reqwest::IntoUrl;
use serde::Deserialize;
use tracing::info;

use crate::CLIENT;

/// Only some of the fields.
///
/// <https://developer.spotify.com/documentation/web-api/reference/get-track>
#[derive(Deserialize, Debug)]
pub struct SpotifyTrack {
    pub name: String,
    pub artists: Vec<SpotifyArtist>,
}

/// I only want the name.
#[derive(Deserialize, Debug)]
pub struct SpotifyArtist {
    pub name: String,
}

const TRACK_API: &str = "https://api.spotify.com/v1/tracks";

/// Parse the track id from `url` and find its isrc.
pub fn find_track_from_url(
    url: impl IntoUrl,
    access_token: impl AsRef<str>,
) -> anyhow::Result<SpotifyTrack> {
    let url = url.into_url()?;

    // check if url is spotify track url
    if url.domain().is_none_or(|d| d != "spotify.com") && !url.as_str().contains("track") {
        return Err(anyhow!("{url} is not a spotify track url"));
    }

    let Some(track_id) = url.path().split('/').nth(2) else {
        return Err(anyhow!("could not parse input url"));
    };

    find_track(track_id, access_token)
}

// we can search youtube music by isrc by just using it as query.
pub fn find_track(
    track_id: impl AsRef<str>,
    access_token: impl AsRef<str>,
) -> anyhow::Result<SpotifyTrack> {
    let track_id = track_id.as_ref();
    let access_token = access_token.as_ref();

    info!("finding track id `{track_id}`");

    let resp = CLIENT
        .get(format!("{TRACK_API}/{track_id}"))
        .bearer_auth(access_token)
        .send()?;

    if !resp.status().is_success() {
        return Err(anyhow!("got {}: {:?}", resp.status(), resp.text()));
    }

    let resp = resp.json::<SpotifyTrack>()?;

    Ok(resp)
}
