use std::{borrow::Borrow, fmt::Debug};

use anyhow::anyhow;
use reqwest::IntoUrl;
use serde::Deserialize;
use tracing::{debug, info};

use crate::CLIENT;

/// Parse the spotify id from `url` and get a list of [`SpotifyTrack`]s and the name (of the playlist or album, if `url` is one.)
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
#[derive(Deserialize)]
pub struct SpotifyTrack {
    pub name: String,
    pub id: String,
    pub artists: Vec<SpotifyArtist>,
}

impl Debug for SpotifyTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let url = format!("https://open.spotify.com/track/{}", self.id);
        f.debug_struct("SpotifyTrack")
            .field("name", &self.name)
            .field("url", &url)
            .field("artists", &self.artists)
            .finish()
    }
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

    let resp: SpotifyTrack = get_resp(&format!("{TRACK_API}/{track_id}"), access_token)?;

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

    let resp: Album = get_resp(&format!("{ALBUM_API}/{id}"), access_token)?;

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
    total: u32,
    next: Option<String>,
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

#[derive(Deserialize, Debug)]
struct PlaylistPagination {
    next: Option<String>,
    items: Vec<PlaylistTrack>,
}

pub fn find_playlist_tracks(
    id: impl AsRef<str>,
    access_token: impl AsRef<str>,
) -> anyhow::Result<(Vec<SpotifyTrack>, String)> {
    const PLAYLIST_API: &str = "https://api.spotify.com/v1/playlists";

    let id = id.as_ref();
    let access_token = access_token.as_ref();

    info!("finding playlist id `{id}`");

    let resp: Playlist = get_resp(&format!("{PLAYLIST_API}/{id}"), access_token)?;

    let mut tracks = Vec::with_capacity(resp.tracks.total as usize);

    tracks.extend(resp.tracks.items.into_iter().filter_map(|p| p.track));

    // if `next_page` is set, we need to go to next pagination
    let mut next_page = resp.tracks.next;
    while let Some(cur_page) = next_page {
        debug!("getting next page of results");

        let cur_page: PlaylistPagination = get_resp(&cur_page, access_token)?;
        debug!("got {} tracks", cur_page.items.len());
        tracks.extend(cur_page.items.into_iter().filter_map(|p| p.track));
        next_page = cur_page.next;
    }

    let owner = resp.owner.display_name.as_deref().unwrap_or("NO OWNER");

    Ok((tracks, format!("{} - {owner}", resp.name)))
}

fn get_resp<T: for<'a> Deserialize<'a>>(url: &str, access_token: &str) -> anyhow::Result<T> {
    let resp = CLIENT.get(url).bearer_auth(access_token).send()?;

    if !resp.status().is_success() {
        return Err(anyhow!("got {}: {:?}", resp.status(), resp.text()));
    }

    Ok(resp.json::<T>()?)
}
