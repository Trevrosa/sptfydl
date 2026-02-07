use serde::Deserialize;
use serde_json::json;
use tracing::info;

use crate::IterExt;

use super::{Image, SimplifiedArtist, SpotifyTrack, bulk_tracks, get_resp};

#[derive(Deserialize, Debug)]
struct Album {
    name: String,
    artists: Vec<SimplifiedArtist>,
    tracks: AlbumTracks,
    total_tracks: u32,
    release_date: String,
    images: Vec<Image>,
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
///
/// # Panics
///
/// Will panic if the `external_id` of any track could not be found.
pub async fn find_album_tracks(
    id: impl AsRef<str>,
    access_token: impl AsRef<str>,
) -> anyhow::Result<(Vec<SpotifyTrack>, String)> {
    const ALBUM_API: &str = "https://api.spotify.com/v1/albums";

    let id = id.as_ref();
    let access_token = access_token.as_ref();

    info!("finding album id `{id}`");

    let resp: Album = get_resp(&format!("{ALBUM_API}/{id}"), access_token).await?;

    let album_data = json!({
        "total_tracks": resp.total_tracks,
        "release_date": resp.release_date,
        "images": resp.images,
        "name": resp.name,
    });

    let mut tracks = resp.tracks.items;

    let full_tracks = bulk_tracks(&tracks, access_token).await?;

    assert_eq!(tracks.len(), full_tracks.len());

    for (track, full) in tracks.iter_mut().zip(full_tracks) {
        track.album = Some(album_data.clone());
        track.external_ids = full.external_ids;
    }

    let artists = resp
        .artists
        .iter()
        .filter_map(|a| a.name.as_deref())
        .join(", ");

    Ok((tracks, format!("{} - {artists}", resp.name)))
}
