pub mod access_token;
pub use access_token::AccessToken;

pub mod search;
use anyhow::anyhow;
use dialoguer::Select;
use indicatif::ProgressStyle;
pub use search::get_from_url;
use tokio::{sync::mpsc, time::sleep};
use tracing_indicatif::span_ext::IndicatifSpanExt;

use std::{
    fmt::Write as FmtWrite,
    fs,
    io::{Write, stdin, stdout},
    sync::{Arc, Mutex},
    time::Duration,
};

use tracing::{Instrument, Span, debug, info, info_span, warn};

use crate::{
    load, load_str, save, save_str,
    spotify::search::SpotifyTrack,
    ytmusic::auth::{Browser, parse_cookie},
};

use super::ytmusic;

const SPOTIFY_TOKEN_CONFIG_NAME: &str = "spotify_token.yaml";
const YTM_DATA_CONFIG_NAME: &str = "ytm_browser_data";

#[derive(Debug)]
pub struct Extraction {
    pub urls: Vec<(usize, String)>,
    pub name: Option<String>,
    /// guaranteed to be in range of `urls`
    pub warnings: Vec<usize>,
    pub failures: usize,
}

impl Extraction {
    #[must_use]
    pub fn warnings(&self) -> Vec<&(usize, String)> {
        self.warnings.iter().map(|idx| &self.urls[*idx]).collect()
    }
}

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

    let (spotify_tracks, name) = get_from_url(spotify_url, token).await?;
    let first_name = spotify_tracks[0].name.clone();

    info!("got {} tracks", spotify_tracks.len());

    let raw_cookie = get_cookies(no_interaction)?;

    let cookie = parse_cookie(&raw_cookie).ok_or(anyhow!("failed to parse cookie"))?;
    let auth = Browser::new(cookie);

    let (urls, warnings, failed) = get_youtube(
        spotify_tracks,
        Arc::from(auth.inner()),
        searchers,
        no_interaction,
        retries,
    )
    .await;

    if !failed.is_empty() {
        if !no_interaction {
            warn!("{} songs failed, check report", failed.len());
        }

        let report = failed.iter().fold(String::new(), |mut report, (n, t)| {
            let _ = writeln!(report, "track #{n}: {t:#?}");
            report
        });

        let name = name.as_deref().unwrap_or(&first_name);
        let path = format!("failed-{name}.txt");

        let _ = fs::write(path, report);
    }

    if urls.is_empty() {
        Err(anyhow!("got no urls"))
    } else {
        Ok(Extraction {
            urls,
            name,
            warnings,
            failures: failed.len(),
        })
    }
}

const RETRY_DELAY: Duration = Duration::from_secs(5);

/// search ytmusic for the `spotify_tracks`
///
/// # Panics
///
/// Will panic if any internal `Arc` or `Mutex` poisons, or any internal `channel` closes prematurely.
async fn get_youtube(
    spotify_tracks: Vec<SpotifyTrack>,
    auth: Arc<str>,
    searchers: usize,
    no_interaction: bool,
    retries: usize,
) -> (Vec<(usize, String)>, Vec<usize>, Vec<(usize, SpotifyTrack)>) {
    // fails should be uncommon
    let failed = Arc::new(Mutex::new(Vec::new()));
    let (warns_tx, mut warns_rx) = mpsc::channel(spotify_tracks.len());

    let (tracks_tx, tracks_rx) = async_channel::bounded(searchers);
    let (urls_tx, mut urls_rx) = mpsc::channel(spotify_tracks.len());

    let expected_tracks = spotify_tracks.len();

    tokio::spawn(async move {
        for track in spotify_tracks.into_iter().enumerate() {
            tracks_tx.send(track).await.expect("channel should be open");
        }
    });

    let pb_span = info_span!("header");

    pb_span.pb_set_style(
        &ProgressStyle::with_template("{wide_bar} {pos}/{len}").expect("valid template"),
    );
    pb_span.pb_set_length(expected_tracks as u64);
    pb_span.pb_start();

    let mut searcher_handles = Vec::with_capacity(searchers);
    for task in 0..searchers {
        let urls = urls_tx.clone();
        let tracks = tracks_rx.clone();
        let warns = warns_tx.clone();
        let failed = failed.clone();
        let auth = auth.clone();

        let handle = tokio::spawn(async move {
            'tracks: loop {
                debug!("waiting for tracks");

                let Ok((i, track)) = tracks.recv().await else {
                    debug!("no more tracks");
                    return;
                };

                debug!("metadata: {track:#?}");
                info!("track {}: {}", i + 1, track.name);

                let artists: Vec<&str> = track.artists.iter().map(|a| a.name.as_str()).collect();
                let query = format!("{} {}", track.name, artists.join(" "));

                let mut tries = 0;
                let mut results = loop {
                    tries += 1;

                    if tries > retries {
                        failed
                            .lock()
                            .expect("shouldnt be poisoned")
                            .push((i + 1, track));
                        continue 'tracks;
                    }

                    let searched = match ytmusic::search(query.as_str(), None, &auth).await {
                        Ok(resp) => resp,
                        Err(err) => {
                            warn!("{err}, retrying in {RETRY_DELAY:?}");
                            sleep(RETRY_DELAY).await;
                            continue;
                        }
                    };

                    if !searched.status().is_success() {
                        warn!(
                            "ytm api search endpoint failed with {}: {:?}",
                            searched.status(),
                            searched.text().await
                        );

                        warn!("retrying in {RETRY_DELAY:?}");
                        sleep(RETRY_DELAY).await;
                        continue;
                    }

                    let Ok(results) = searched.json().await else {
                        warn!("couldnt deserialize response as json, retrying in {RETRY_DELAY:?}");
                        sleep(RETRY_DELAY).await;
                        continue;
                    };

                    let Some(results) = ytmusic::parse_results(&results) else {
                        warn!("couldnt parse search results, retrying in {RETRY_DELAY:?}");
                        sleep(RETRY_DELAY).await;
                        continue;
                    };

                    if results.is_empty() {
                        warn!("search results was empty, retrying in {RETRY_DELAY:?}");
                        sleep(RETRY_DELAY).await;
                        continue;
                    }

                    break results;
                };

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

                if no_interaction && choice != 0 {
                    warn!("the best result was not available and --no-interaction was set.");
                    warns.send(i).await.expect("shouldnt be closed");
                }

                let url = results[choice].link_or_default().to_string();

                Span::current().pb_inc(1);
                urls.send((i, url)).await.expect("shouldnt be closed");
            }
        })
        .instrument(info_span!("searcher"));
        searcher_handles.push(handle);
    }

    // FIXME: doesnt end
    let mut urls = Vec::with_capacity(expected_tracks);
    async {
        while let Some(url) = urls_rx.recv().await {
            Span::current().pb_inc(1);
            urls.push(url);
        }
    }
    .instrument(pb_span)
    .await;

    for handle in searcher_handles {
        if let Err(err) = handle.await {
            warn!("a downloader failed: {err}");
        };
    }

    let mut warnings = Vec::new();
    warns_rx.recv_many(&mut warnings, expected_tracks).await;

    let failed = Arc::into_inner(failed)
        .expect("should not be poisoned")
        .into_inner()
        .expect("should not be poisoned");
    (urls, warnings, failed)
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
