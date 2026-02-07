pub mod access_token;
pub use access_token::AccessToken;

pub mod search;
pub use search::get_from_url;

pub mod types;
pub use types::{Extraction, Metadata, Track};

use std::{
    fmt::Write as FmtWrite,
    fs,
    io::{Write, stdin, stdout},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::anyhow;
use dialoguer::Select;
use indicatif::ProgressStyle;
use tokio::{sync::mpsc, time::sleep};
use tracing::{Instrument, Span, debug, info, info_span, trace, warn};
use tracing_indicatif::span_ext::IndicatifSpanExt;

use crate::{
    load, load_str, save, save_str,
    spotify::search::{SimplifiedArtist, SpotifyTrack, bulk_artists, bulk_many_artists},
    ytmusic::{
        SearchResult as YtSearchResult,
        auth::{Browser, parse_cookie},
    },
};

use super::ytmusic;

const SPOTIFY_TOKEN_CONFIG_NAME: &str = "spotify_token.yaml";
const YTM_DATA_CONFIG_NAME: &str = "ytm_browser_data";

/// Returns `Vec<(usize, String)>` because some tracks may not be found from ytmusic,
/// so some tracks may be missing,
/// so we return the track number as well
///
/// # Errors
///
/// This function fails if:
/// - We could not get a new [`AccessToken`], and one is not cached.
/// - Cookies were required to be prompted and `no_interaction` was true.
/// - We got no urls from ytmusic.
///
/// # Panics
///
/// This function panics if we could not get the cookies from `stdin`.
pub async fn extract_spotify(
    id: &str,
    secret: &str,
    spotify_url: &str,
    searchers: usize,
    no_interaction: bool,
    retries: usize,
) -> anyhow::Result<Extraction> {
    let token = load::<AccessToken>(SPOTIFY_TOKEN_CONFIG_NAME);

    let token = if let Ok(token) = token {
        debug!("got spotify token from cache");
        token
    } else {
        request_token_and_save(id, secret).await?
    };

    let token = if token.expired() {
        request_token_and_save(id, secret).await?
    } else {
        token
    };

    let (mut spotify_tracks, name) = get_from_url(spotify_url, token.as_ref()).await?;
    let first_name = spotify_tracks[0].name.clone();

    info!("got {} tracks", spotify_tracks.len(),);

    let raw_cookie = get_cookies(no_interaction)?;

    let cookie = parse_cookie(&raw_cookie).ok_or(anyhow!("failed to parse cookie"))?;
    let auth = Browser::new(cookie);

    let (tracks, warnings, failed) = if spotify_tracks.len() == 1 {
        let track = spotify_tracks.pop().expect("len is 1");
        debug!("metadata: {track:#?}");
        info!("searching for {}", track.name);
        search_one(
            track,
            auth.as_ref(),
            token.as_ref(),
            no_interaction,
            retries,
        )
        .await
    } else {
        search_many(
            spotify_tracks,
            Arc::from(auth.into_inner()),
            token.as_ref(),
            searchers,
            retries,
        )
        .await
    };

    if !failed.is_empty() {
        warn!("{} songs failed, check report", failed.len());

        let report = failed.iter().fold(String::new(), |mut report, (n, t)| {
            let _ = writeln!(report, "track #{n}: {t:#?}");
            report
        });

        let name = name.as_deref().unwrap_or(&first_name);
        let path = format!("failed-{name}.txt");

        let _ = fs::write(path, report);
    }

    if tracks.is_empty() {
        Err(anyhow!("got no urls"))
    } else {
        Ok(Extraction {
            tracks,
            name,
            warnings,
            failures: failed.len(),
        })
    }
}

const RETRY_DELAY: Duration = Duration::from_secs(3);

/// (`urls`, `warns`, `fails`)
type SearchResult = (Vec<(usize, Track)>, Vec<usize>, Vec<(usize, SpotifyTrack)>);

/// search ytmusic for the `spotify_tracks`
///
/// # Panics
///
/// Will panic if any internal `Arc` or `Mutex` poisons, or any internal `channel` closes prematurely.
#[inline]
async fn search_many(
    spotify_tracks: Vec<SpotifyTrack>,
    yt_auth: Arc<str>,
    spotify_auth: &str,
    searchers: usize,
    retries: usize,
) -> SearchResult {
    let start = Instant::now();
    let expected_tracks = spotify_tracks.len();

    let (tracks_tx, tracks_rx) = async_channel::bounded(searchers);

    let (results_tx, mut results_rx) = mpsc::channel(expected_tracks);
    let (warns_tx, mut warns_rx) = mpsc::channel(expected_tracks);
    let (fails_tx, mut failed_rx) = mpsc::channel(expected_tracks);

    debug!("channel setup took {:?}", start.elapsed());

    tokio::spawn(async move {
        for track in spotify_tracks.into_iter().enumerate() {
            tracks_tx.send(track).await.expect("channel should be open");
        }
    });

    let pb_span = info_span!("pb");

    pb_span.pb_set_style(
        &ProgressStyle::with_template("{wide_bar} {pos}/{len} ({elapsed}) (eta: {eta})")
            .expect("valid template"),
    );

    pb_span.pb_set_length(expected_tracks as u64);
    pb_span.pb_start();

    let mut searcher_handles = Vec::with_capacity(searchers);
    for task in 0..searchers {
        let output = results_tx.clone();
        let tracks = tracks_rx.clone();
        let warns = warns_tx.clone();
        let failed = fails_tx.clone();
        let yt_auth = yt_auth.clone();

        let handle = tokio::spawn(
            async move {
                loop {
                    trace!("waiting for tracks");

                    let Ok((i, track)) = tracks.recv().await else {
                        debug!("no more tracks");
                        return;
                    };

                    debug!("metadata: {track:#?}");
                    info!("{:?}", track.name);

                    let artists: Vec<&str> = track
                        .artists
                        .iter()
                        .filter_map(|a| a.name.as_deref())
                        .collect();
                    let query = format!("{} {}", track.name, artists.join(" "));

                    let Some(mut results) = search_retrying(&query, &yt_auth, retries).await else {
                        failed
                            .send((i + 1, track))
                            .await
                            .expect("shouldnt be closed");
                        continue;
                    };

                    results[0].title.push_str("Best Result");

                    debug!("got {} results", results.len());

                    let choice = {
                        let choice = results.iter().position(|r| r.video_id.is_some());
                        debug!("default choice was {choice:?}");
                        choice.unwrap_or(0)
                    };

                    if choice != 0 {
                        warn!("the best result was not available");
                        warns.send(i).await.expect("shouldnt be closed");
                    }

                    let url = results[choice].link_or_default().to_string();
                    output
                        .send((i, (url, track)))
                        .await
                        .expect("shouldnt be closed");
                }
            }
            .instrument(info_span!("searcher", id = task + 1)),
        );

        searcher_handles.push(handle);
    }

    // ensure channels close so `recv_many()` doesn't poll forever
    drop((results_tx, warns_tx, fails_tx));

    debug!("total setup took {:?}", start.elapsed());

    let mut tracks = Vec::with_capacity(expected_tracks);
    async {
        while let Some(track) = results_rx.recv().await {
            Span::current().pb_inc(1);
            tracks.push(track);
        }
    }
    .instrument(pb_span)
    .await;

    debug!("getting artists in bulk");

    let tracks = promote(tracks, spotify_auth).await;

    for handle in searcher_handles {
        if let Err(err) = handle.await {
            warn!("a downloader failed: {err}");
        }
    }

    debug!("collecting fails / warns from channels");

    let mut warnings = Vec::new();
    warns_rx.recv_many(&mut warnings, expected_tracks).await;
    let mut failed = Vec::new();
    failed_rx.recv_many(&mut failed, expected_tracks).await;

    (tracks, warnings, failed)
}

/// Converts a tuple (`track_num`, (`url`, `spotify_track`)) into a `Vec<(usize, Track)>` by requesting for full [`search::SpotifyArtist`]s in bulk.
#[inline]
async fn promote(
    urls: Vec<(usize, (String, SpotifyTrack))>,
    spotify_auth: &str,
) -> Vec<(usize, Track)> {
    let artists: Vec<&Vec<SimplifiedArtist>> = urls.iter().map(|t| &t.1.1.artists).collect();
    let artists = bulk_many_artists(&artists, spotify_auth)
        .await
        .expect("failed to get artists");

    assert_eq!(urls.len(), artists.len());

    let mut tracks = Vec::with_capacity(urls.len());
    for ((track_num, (url, track)), artists) in urls.into_iter().zip(artists) {
        let metadata = track.into_metadata(artists);
        tracks.push((track_num, Track::new(url, metadata)));
    }

    tracks
}

#[inline]
async fn search_one(
    track: SpotifyTrack,
    yt_auth: &str,
    spotify_auth: &str,
    no_interaction: bool,
    retries: usize,
) -> SearchResult {
    let artists = bulk_artists(&track.artists, spotify_auth).await.unwrap();
    let artist_strs: Vec<&str> = artists.iter().map(|a| a.name.as_str()).collect();
    let query = format!("{} {}", track.name, artist_strs.join(" "));
    if let Some(mut results) = search_retrying(&query, yt_auth, retries).await {
        results[0].title.push_str("Best Result");

        debug!("got {} results", results.len());

        let choice = if no_interaction {
            let choice = results.iter().position(|r| r.video_id.is_some());
            debug!("default choice was {choice:?}");
            choice
        } else {
            Select::new()
                .with_prompt("Choose link to download")
                .default(0)
                .items(&results)
                .interact()
                .ok()
        }
        .unwrap_or(0);

        let mut warnings = Vec::with_capacity(1);
        if choice != 0 {
            warn!("the best result was not available");
            warnings.push(0);
        }

        let url = results[choice].link_or_default().to_string();
        (
            vec![(0, Track::new(url, track.into_metadata(artists)))],
            warnings,
            vec![],
        )
    } else {
        (vec![], vec![], vec![(0, track)])
    }
}

/// Search `query` with `auth`, retrying `retries` times. Returns `None` if no results could be found after `retries` retries.
#[inline]
async fn search_retrying(query: &str, auth: &str, retries: usize) -> Option<Vec<YtSearchResult>> {
    for attempt in 0..retries {
        if attempt > 0 {
            sleep(RETRY_DELAY).await;
        }

        let searched = match ytmusic::search(query, None, auth).await {
            Ok(resp) => resp,
            Err(err) => {
                if retries > 0 {
                    warn!("{err}, retrying in {RETRY_DELAY:?}");
                }
                continue;
            }
        };

        if !searched.status().is_success() {
            warn!(
                "ytm api search endpoint failed with {}: {:?}",
                searched.status(),
                searched.text().await
            );
            continue;
        }

        let Ok(results) = searched.json().await else {
            if retries > 0 {
                warn!("couldnt deserialize response as json, retrying in {RETRY_DELAY:?}");
            }
            continue;
        };

        let Some(results) = ytmusic::parse_results(&results) else {
            if retries > 0 {
                warn!("couldnt parse search results, retrying in {RETRY_DELAY:?}");
            }
            continue;
        };

        if results.is_empty() {
            if retries > 0 {
                warn!("search results were empty, retrying in {RETRY_DELAY:?}");
            }
            continue;
        }

        return Some(results);
    }

    None
}

#[inline]
fn get_cookies(no_interaction: bool) -> anyhow::Result<String> {
    if let Ok(cookie) = load_str(YTM_DATA_CONFIG_NAME) {
        Ok(cookie)
    } else {
        info!("no saved ytm cookie, need input.");
        info!(
            "please go to https://music.youtube.com and copy-paste your headers to an authenticatied POST request. (https://ytmusicapi.readthedocs.io/en/stable/setup/browser.html#copy-authentication-headers)"
        );

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

        Ok(cookie)
    }
}

/// # Errors
///
/// This function fails if we could not get a new [`AccessToken`].
#[inline]
pub async fn request_token_and_save(id: &str, secret: &str) -> anyhow::Result<AccessToken> {
    debug!("requesting new spotify access token");
    let Some(access_token) = AccessToken::get(id, secret).await else {
        return Err(anyhow!("could not get access token"));
    };

    if let Err(err) = save(&access_token, SPOTIFY_TOKEN_CONFIG_NAME) {
        warn!("failed to save new access token: {err}");
    } else {
        debug!("saved new access token");
    }

    Ok(access_token)
}
