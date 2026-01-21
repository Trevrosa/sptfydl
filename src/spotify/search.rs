use std::borrow::Borrow;

use anyhow::anyhow;
use reqwest::IntoUrl;
use serde::Deserialize;
use tracing::info;

use crate::CLIENT;

/// Parse the track id from `url` and get a list of [`SpotifyTrack`]s, and the name (if playlist or album)
pub fn get_from_url(
    url: impl IntoUrl,
    access_token: impl AsRef<str>,
) -> anyhow::Result<(Vec<SpotifyTrack>, Option<String>)> {
    let url = url.into_url()?;

    // check if url is spotify track url
    if url.domain().is_none_or(|d| !d.ends_with("spotify.com")) {
        return Err(anyhow!("{url} is not a spotify url"));
    }

    let Some(id) = url.path().split('/').nth(2) else {
        return Err(anyhow!("could not parse input url"));
    };

    if url.path().starts_with("/track") {
        Ok((vec![find_track(id, access_token)?], None))
    } else if url.path().starts_with("/playlist") {
        let (tracks, name) = find_playlist_tracks(id, access_token)?;
        Ok((tracks, Some(name)))
    } else if url.path().starts_with("/album") {
        let (tracks, name) = find_album_tracks(id, access_token)?;
        Ok((tracks, Some(name)))
    } else {
        Err(anyhow!("spotify url was not a track, album, or a playlist"))
    }
}

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

impl Borrow<str> for SpotifyArtist {
    fn borrow(&self) -> &str {
        &self.name
    }
}

// we can search youtube music by isrc by just using it as query.
pub fn find_track(
    track_id: impl AsRef<str>,
    access_token: impl AsRef<str>,
) -> anyhow::Result<SpotifyTrack> {
    const TRACK_API: &str = "https://api.spotify.com/v1/tracks";

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

#[derive(Deserialize, Debug)]
struct Album {
    name: String,
    artists: Vec<SpotifyArtist>,
    tracks: AlbumTracks,
}

#[derive(Deserialize, Debug)]
struct AlbumTracks {
    items: Vec<SpotifyTrack>,
}

pub fn find_album_tracks(
    id: impl AsRef<str>,
    access_token: impl AsRef<str>,
) -> anyhow::Result<(Vec<SpotifyTrack>, String)> {
    const ALBUM_API: &str = "https://api.spotify.com/v1/albums";

    let id = id.as_ref();
    let access_token = access_token.as_ref();

    info!("finding album id `{id}`");

    let resp = CLIENT
        .get(format!("{ALBUM_API}/{id}"))
        .bearer_auth(access_token)
        .send()?;

    if !resp.status().is_success() {
        return Err(anyhow!("got {}: {:?}", resp.status(), resp.text()));
    }

    let resp = resp.json::<Album>()?;

    let artists = resp.artists.join(", ");

    Ok((resp.tracks.items, format!("{} - {artists}", resp.name)))
}

#[derive(Deserialize, Debug)]
struct Playlist {
    name: String,
    owner: PlaylistOwner,
    tracks: PlaylistTracks,
}

#[derive(Deserialize, Debug)]
struct PlaylistTracks {
    items: Vec<PlaylistTrack>,
}

#[derive(Deserialize, Debug)]
struct PlaylistTrack {
    track: Option<SpotifyTrack>,
}

#[derive(Deserialize, Debug)]
struct PlaylistOwner {
    display_name: Option<String>,
}

pub fn find_playlist_tracks(
    id: impl AsRef<str>,
    access_token: impl AsRef<str>,
) -> anyhow::Result<(Vec<SpotifyTrack>, String)> {
    const PLAYLIST_API: &str = "https://api.spotify.com/v1/playlists";

    let id = id.as_ref();
    let access_token = access_token.as_ref();

    info!("finding playlist id `{id}`");

    let resp = CLIENT
        .get(format!("{PLAYLIST_API}/{id}"))
        .bearer_auth(access_token)
        .send()?;

    if !resp.status().is_success() {
        return Err(anyhow!("got {}: {:?}", resp.status(), resp.text()));
    }

    let resp = resp.json::<Playlist>()?;

    let tracks = resp
        .tracks
        .items
        .into_iter()
        .filter_map(|p| p.track)
        .collect();

    let owner = resp.owner.display_name.as_deref().unwrap_or("NO OWNER");

    Ok((tracks, format!("{} - {owner}", resp.name)))
}
