pub mod album;
pub mod playlist;
pub mod track;

pub use album::find_album_tracks;
pub use playlist::find_playlist_tracks;
pub use track::find_track;

use std::{
    fmt::Debug,
    sync::atomic::{AtomicU16, Ordering},
};

use anyhow::anyhow;
use reqwest::IntoUrl;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{CLIENT, IterExt};

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

/// Only some of the fields.
///
/// <https://developer.spotify.com/documentation/web-api/reference/get-track>
#[derive(Deserialize)]
pub struct SpotifyTrack {
    pub name: String,
    pub id: String,
    /// used to find its image
    pub album: Option<serde_json::Value>,
    /// use [`get_artists`] to get the actual artist info
    pub artists: Vec<SimplifiedArtist>,
    pub disc_number: u32,
    pub explicit: bool,
    pub external_ids: Option<ExternalIds>,
    pub track_number: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Image {
    url: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ExternalIds {
    #[serde(default)]
    pub isrc: Option<String>,
    // pub ean: String,
    // pub upc: String,
}

// has `None`s because tracks/episodes/shows from playlists don't always have the fields.
#[derive(Deserialize, Debug, Clone)]
pub struct SimplifiedArtist {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct SpotifyArtist {
    pub name: String,
    pub genres: Vec<String>,
    id: String,
}

impl Debug for SpotifyArtist {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

impl SpotifyTrack {
    // is an associated function to allow partial moves
    /// Returns (`album_name`, `cover_url`, `release_date`, `total_tracks`).
    ///
    /// Will be `None` if `album` is `None`.
    ///
    /// # Panics
    ///
    /// Will panic if `album` is not a valid `Album` as defined in the function.
    #[must_use]
    pub fn extract_album(
        album: Option<serde_json::Value>,
    ) -> Option<(String, String, String, u32)> {
        // used here only to find the image url
        #[derive(Deserialize)]
        struct Album {
            name: String,
            images: Vec<Image>,
            release_date: String,
            total_tracks: u32,
        }

        let mut album: Album = serde_json::from_value(album?).ok()?;
        let cover_url = album.images.swap_remove(0).url;

        Some((
            album.name,
            cover_url,
            album.release_date,
            album.total_tracks,
        ))
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

/// Turn [`SimplifiedArtist`]s into [`SpotifyArtist`]s. Does bulk requests, chunking by 50.
///
/// # Errors
///
/// Will fail if any artist could not be found, or if any request fails to be sent.
///
/// # Panics
///
/// Should never panic.
pub async fn bulk_artists(
    from: &[SimplifiedArtist],
    access_token: &str,
) -> anyhow::Result<Vec<SpotifyArtist>> {
    let mut artists = bulk_many_artists(&[&from.to_vec()], access_token).await?;
    Ok(artists.pop().unwrap())
}

impl PartialEq<SimplifiedArtist> for SpotifyArtist {
    fn eq(&self, other: &SimplifiedArtist) -> bool {
        Some(&self.id) == other.id.as_ref()
    }
}

/// Turn multiple [`SimplifiedArtist`]s into [`SpotifyArtist`]s. Does bulk requests, chunking by 50.
///
/// Make sure all passed [`SimplifiedArtist`]s have an id.
///
/// Order is preserved.
///
/// # Errors
///
/// Will fail if any artist could not be found, or if any request fails to be sent.
pub async fn bulk_many_artists(
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
            let ids = chunk
                .iter()
                // if `name` is None even if `id` is Some, something is wrong.
                .filter(|a| a.name.is_some())
                .filter_map(|a| a.id.as_ref())
                .join(",");
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
            // accept that some artists won't be found
            .filter_map(|wanted| all_artists.iter().find(|artist| *artist == wanted).cloned())
            .collect();
        // `artists` will at least be vec![]
        result.push(artists);
    }

    Ok(result)
}

async fn bulk_tracks(
    tracks: &[SpotifyTrack],
    access_token: &str,
) -> anyhow::Result<Vec<SpotifyTrack>> {
    const TRACK_API: &str = "https://api.spotify.com/v1/tracks";

    #[derive(Deserialize)]
    struct Tracks {
        tracks: Vec<SpotifyTrack>,
    }

    let mut full_tracks = Vec::with_capacity(tracks.len());
    for track in tracks.chunks(50) {
        let ids = track.iter().map(|t| &t.id).join(",");
        let resp: Tracks = get_resp(&format!("{TRACK_API}/?ids={ids}"), access_token).await?;
        full_tracks.extend(resp.tracks);
    }

    Ok(full_tracks)
}

pub static REQUESTS: AtomicU16 = AtomicU16::new(0);

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
    REQUESTS.fetch_add(1, Ordering::Relaxed);

    if !resp.status().is_success() {
        return Err(anyhow!("got {}: {:?}", resp.status(), resp.text().await));
    }

    // let string = resp.text().await?;
    // std::fs::write("a", &string);
    // let jd = &mut serde_json::Deserializer::from_str(&string);

    // Ok(serde_path_to_error::deserialize(jd)?)

    Ok(resp.json::<T>().await?)
}
