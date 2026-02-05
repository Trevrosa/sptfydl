use std::{borrow::Borrow, fmt::Debug};

use anyhow::anyhow;
use reqwest::IntoUrl;
use serde::Deserialize;
use tracing::{debug, info};

use crate::{CLIENT, spotify::Metadata};

/// Parse the spotify id from `url` and get a list of [`SpotifyTrack`]s and the name (of the playlist or album, if `url` is one.)
///
/// # Errors
///
/// This function fails if:
/// - `url` was not a spotify url.
/// - We failed to find an id from `url`.
/// - We failed to run [`find_track`], [`find_playlist_tracks`], or [`find_album_tracks`].
pub async fn get_from_url(
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
        Ok((vec![find_track(id, access_token).await?], None))
    } else if url.path().starts_with("/playlist") {
        let (tracks, name) = find_playlist_tracks(id, access_token).await?;
        Ok((tracks, Some(name)))
    } else if url.path().starts_with("/album") {
        let (tracks, name) = find_album_tracks(id, access_token).await?;
        Ok((tracks, Some(name)))
    } else {
        Err(anyhow!("spotify url was not a track, album, or a playlist"))
    }
}

#[derive(Deserialize, Debug)]
pub struct Image {
    url: String,
}

/// Only some of the fields.
///
/// <https://developer.spotify.com/documentation/web-api/reference/get-track>
#[derive(Deserialize)]
pub struct SpotifyTrack {
    pub name: String,
    pub id: String,
    /// used to find its image
    pub album: serde_json::Value,
    /// use [`get_artists`] to get the actual artist info
    pub artists: Vec<SimplifiedArtist>,
    pub disc_number: u32,
    pub explicit: bool,
    pub external_ids: ExternalIds,
    pub track_number: u32,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ExternalIds {
    pub isrc: String,
    // pub ean: String,
    // pub upc: String,
}

impl SpotifyTrack {
    /// Turns `self` into [`Metadata`] with `artists`.
    #[must_use]
    pub fn into_metadata(self, artists: Vec<SpotifyArtist>) -> Metadata {
        let (album_name, cover_url, release_date) = SpotifyTrack::extract_album(self.album);
        Metadata {
            artists,
            disc_number: self.disc_number,
            name: self.name,
            spotify_id: self.id,
            explicit: self.explicit,
            external_ids: self.external_ids,
            track_number: self.track_number,
            release_date,
            cover_url,
            album_name,
        }
    }

    // is an associated function to allow partial moves
    /// Returns (`album_name`, `cover_url`, `release_date`).
    #[must_use]
    #[allow(clippy::missing_panics_doc)]
    pub fn extract_album(album: serde_json::Value) -> (String, String, String) {
        // used here only to find the image url
        #[derive(Deserialize)]
        struct Album {
            name: String,
            images: Vec<Image>,
            release_date: String,
        }

        let mut album: Album = serde_json::from_value(album).expect("must exist");
        let cover_url = album.images.swap_remove(0).url;

        (album.name, cover_url, album.release_date)
    }
}

impl Debug for SpotifyTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let url = format_args!("https://open.spotify.com/track/{}", self.id);
        let album = self.album.clone();
        f.debug_struct("SpotifyTrack")
            .field("name", &self.name)
            .field("url", &url)
            .field("artists", &self.artists)
            .field("album_info", &SpotifyTrack::extract_album(album))
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct SimplifiedArtist {
    pub name: String,
    id: String,
}

impl Borrow<str> for SimplifiedArtist {
    fn borrow(&self) -> &str {
        &self.id
    }
}

impl Borrow<str> for &SimplifiedArtist {
    fn borrow(&self) -> &str {
        &self.id
    }
}

/// Turn [`SimplifiedArtist`]s into [`SpotifyArtist`]s. Does bulk requests, chunking by 50.
pub async fn get_artists(
    from: &[SimplifiedArtist],
    access_token: &str,
) -> anyhow::Result<Vec<SpotifyArtist>> {
    let mut artists = get_many_artists(&[&from.to_vec()], access_token).await?;
    Ok(artists.pop().expect("must exist"))
}

// TODO: cleanup some of this code?

/// Turn multiple [`SimplifiedArtist`]s into [`SpotifyArtist`]s. Does bulk requests, chunking by 50.
///
/// Order is preserved.
///
/// # Errors
///
/// Will fail if any artist could not be found, or if the request fails to be sent.
pub async fn get_many_artists(
    artist_arrays: &[&Vec<SimplifiedArtist>],
    access_token: &str,
) -> anyhow::Result<Vec<Vec<SpotifyArtist>>> {
    const ARTIST_API: &str = "https://api.spotify.com/v1/artists";

    #[derive(Deserialize)]
    struct SpotifyArtists {
        artists: Vec<SpotifyArtist>,
    }

    let mut all_artists = Vec::with_capacity(artist_arrays.len());

    {
        let artists: Vec<&SimplifiedArtist> = artist_arrays.iter().copied().flatten().collect();
        for chunk in artists.chunks(50) {
            let ids = chunk.join(",");
            let resp: SpotifyArtists =
                get_resp(&format!("{ARTIST_API}/?ids={ids}"), access_token).await?;
            all_artists.extend(resp.artists);
        }
    }

    debug!("got {} total artists", all_artists.len());

    let mut result = Vec::with_capacity(artist_arrays.len());
    for array in artist_arrays {
        let artists = array
            .iter()
            .map(|want| {
                all_artists
                    .iter()
                    .find(|a| a.name == want.name)
                    .expect("must exist")
                    .clone()
            })
            .collect();
        result.push(artists);
    }

    Ok(result)
}

#[derive(Deserialize, Debug, Clone)]
pub struct SpotifyArtist {
    pub name: String,
    pub genres: Vec<String>,
}

impl Borrow<str> for SpotifyArtist {
    fn borrow(&self) -> &str {
        &self.name
    }
}

/// Find a track by its `id` using `access_token` for authorization.
///
/// # Errors
///
/// See [`get_resp`].
pub async fn find_track(
    id: impl AsRef<str>,
    access_token: impl AsRef<str>,
) -> anyhow::Result<SpotifyTrack> {
    const TRACK_API: &str = "https://api.spotify.com/v1/tracks";

    let track_id = id.as_ref();
    let access_token = access_token.as_ref();

    info!("finding track id `{track_id}`");

    let resp: SpotifyTrack = get_resp(&format!("{TRACK_API}/{track_id}"), access_token).await?;

    Ok(resp)
}

#[derive(Deserialize, Debug)]
struct Album {
    name: String,
    artists: Vec<SimplifiedArtist>,
    tracks: AlbumTracks,
}

#[derive(Deserialize, Debug)]
struct AlbumTracks {
    items: Vec<SpotifyTrack>,
}

/// Find an album's tracks by its `id` using `access_token` for authorization.
///
/// # Errors
///
/// See [`get_resp`].
pub async fn find_album_tracks(
    id: impl AsRef<str>,
    access_token: impl AsRef<str>,
) -> anyhow::Result<(Vec<SpotifyTrack>, String)> {
    const ALBUM_API: &str = "https://api.spotify.com/v1/albums";

    let id = id.as_ref();
    let access_token = access_token.as_ref();

    info!("finding album id `{id}`");

    let resp: Album = get_resp(&format!("{ALBUM_API}/{id}"), access_token).await?;

    let artists = get_artists(&resp.artists, access_token).await?.join(", ");

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

/// Find a playlist's tracks by its `id` using `access_token` for authorization.
///
/// # Errors
///
/// See [`get_resp`].
pub async fn find_playlist_tracks(
    id: impl AsRef<str>,
    access_token: impl AsRef<str>,
) -> anyhow::Result<(Vec<SpotifyTrack>, String)> {
    const PLAYLIST_API: &str = "https://api.spotify.com/v1/playlists";

    let id = id.as_ref();
    let access_token = access_token.as_ref();

    info!("finding playlist id `{id}`");

    let resp: Playlist = get_resp(&format!("{PLAYLIST_API}/{id}"), access_token).await?;

    let mut tracks = Vec::with_capacity(resp.tracks.total as usize);

    tracks.extend(resp.tracks.items.into_iter().filter_map(|p| p.track));

    // if `next_page` is set, we need to go to next pagination
    let mut next_page = resp.tracks.next;
    while let Some(cur_page) = next_page {
        debug!("getting next page of results");

        let cur_page: PlaylistPagination = get_resp(&cur_page, access_token).await?;
        debug!("got {} tracks", cur_page.items.len());
        tracks.extend(cur_page.items.into_iter().filter_map(|p| p.track));
        next_page = cur_page.next;
    }

    let owner = resp.owner.display_name.as_deref().unwrap_or("NO OWNER");

    Ok((tracks, format!("{} - {owner}", resp.name)))
}

/// Get `url`, parsing as json to `T`, using `access_token` for authorization.
///
/// # Errors
///
/// This function fails if:
/// - We could not send the request to `url`.
/// - The request was not successful.
/// - We could not deserialize the response as json to `T`.
async fn get_resp<T: for<'a> Deserialize<'a>>(url: &str, access_token: &str) -> anyhow::Result<T> {
    let resp = CLIENT.get(url).bearer_auth(access_token).send().await?;

    if !resp.status().is_success() {
        return Err(anyhow!("got {}: {:?}", resp.status(), resp.text().await));
    }

    Ok(resp.json::<T>().await?)
}
