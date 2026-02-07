use serde::Deserialize;
use tracing::{debug, info};

use super::{ExternalIds, SimplifiedArtist, SpotifyTrack, get_resp};

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
    items: Vec<PlaylistItem>,
}

#[derive(Deserialize, Debug)]
struct PlaylistItem {
    track: Option<PlaylistTrack>,
}

/// playlists may contain custom tracks that have less data
#[derive(Deserialize, Debug)]
struct PlaylistTrack {
    pub name: String,
    #[serde(default)]
    pub id: Option<String>,
    /// used to find its image
    #[serde(default)]
    pub album: Option<serde_json::Value>,
    /// use [`get_artists`] to get the actual artist info
    #[serde(default)]
    pub artists: Option<Vec<SimplifiedArtist>>,
    #[serde(default)]
    pub disc_number: Option<u32>,
    #[serde(default)]
    pub explicit: Option<bool>,
    #[serde(default)]
    pub external_ids: Option<ExternalIds>,
    #[serde(default)]
    pub track_number: Option<u32>,
}

impl PlaylistTrack {
    fn into(self) -> Option<SpotifyTrack> {
        Some(SpotifyTrack {
            name: self.name,
            id: self.id?,
            album: self.album,
            artists: self.artists?,
            disc_number: self.disc_number?,
            explicit: self.explicit?,
            external_ids: self.external_ids,
            track_number: self.track_number?,
        })
    }
}

#[derive(Deserialize, Debug)]
struct PlaylistOwner {
    display_name: Option<String>,
}

#[derive(Deserialize, Debug)]
struct PlaylistPagination {
    next: Option<String>,
    items: Vec<PlaylistItem>,
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

    let mut tracks: Vec<SpotifyTrack> = Vec::with_capacity(resp.tracks.total as usize);

    tracks.extend(
        resp.tracks
            .items
            .into_iter()
            .filter_map(|a| a.track?.into()),
    );

    // if `next_page` is set, we need to go to next pagination
    let mut next_page = resp.tracks.next;
    while let Some(cur_page) = next_page {
        info!("getting next page of tracks");

        let cur_page: PlaylistPagination = get_resp(&cur_page, access_token).await?;
        debug!("got {} tracks", cur_page.items.len());
        tracks.extend(cur_page.items.into_iter().filter_map(|p| p.track?.into()));
        next_page = cur_page.next;
    }

    let owner = resp.owner.display_name.as_deref().unwrap_or("NO OWNER");

    Ok((tracks, format!("{} - {owner}", resp.name)))
}
