use tracing::info;

use super::{SpotifyTrack, get_resp};

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
