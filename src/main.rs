use anyhow::Context;
use clap::{ArgAction, Parser};
use console::Term;
use dialoguer::{Input, Password};
use indicatif::{HumanDuration, ProgressStyle};
use lofty::{
    config::WriteOptions,
    file::{AudioFile, TaggedFileExt},
    picture::{MimeType, Picture, PictureType},
    probe::Probe,
    tag::{Accessor, ItemKey, Tag},
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{ChildStderr, Command},
    sync::mpsc,
};
use tracing::{Instrument, Level, Span, debug, info, info_span, instrument, warn};
use tracing_indicatif::{IndicatifLayer, span_ext::IndicatifSpanExt};
use tracing_subscriber::{filter::Targets, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use sptfydl::{
    CLIENT, load, save,
    spotify::{Metadata, Track, extract_spotify, search::REQUESTS},
};

use std::{
    path::Path,
    process::{Stdio, exit},
    sync::Arc,
    time::Instant,
};

#[allow(clippy::struct_excessive_bools, clippy::struct_field_names)]
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The spotify url to download.
    url: String,

    /// Tell yt-dlp to convert to mp3.
    #[arg(long)]
    mp3: bool,

    /// Be a bit more verbose. Can be applied more than once (-v, -vv)
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,

    /// Show the output of ytdlp commands.
    #[arg(long)]
    show_ytdlp: bool,

    /// Disable tagging of mp3 files.
    #[arg(long)]
    no_metadata: bool,

    /// Skip prompts; always choose the default or first available option.
    #[arg(short, long)]
    no_interaction: bool,

    /// The number of concurrent downloads.
    #[arg(short, long, default_value_t = 5)]
    downloaders: usize,

    /// The number of concurrent searches.
    #[arg(short, long, default_value_t = 3)]
    searchers: usize,

    /// The number of retries allowed for downloads.
    #[arg(long, default_value_t = 5)]
    download_retries: usize,

    /// The number of retries allowed for searches.
    #[arg(long, default_value_t = 3)]
    search_retries: usize,

    /// Additional args for yt-dlp.
    #[arg(last = true)]
    ytdlp_args: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let filter = match args.verbose {
        0 => Level::INFO,
        1 => Level::DEBUG,
        2..=u8::MAX => Level::TRACE,
    };

    let indicatif_layer = IndicatifLayer::new().with_max_progress_bars(
        11, // 5 downloaders default, + 5 for each ytdlp subspan, + 1 for progress bar
        Some(
            ProgressStyle::with_template("...and {pending_progress_bars} more not shown above.")
                .expect("valid template"),
        ),
    );

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .without_time()
                .compact()
                .with_writer(indicatif_layer.get_stderr_writer()),
        )
        .with(Targets::new().with_target("sptfydl", filter))
        .with(indicatif_layer)
        .init();

    ctrlc::set_handler(handle_exit)?;

    let mut ytdlp_args = args.ytdlp_args;

    ytdlp_args.push("--no-playlist".to_string());

    if args.mp3 {
        ytdlp_args.extend(["--extract-audio", "--audio-format", "mp3"].map(ToString::to_string));
    }

    let oauth = get_spotify_oauth()?;

    let start = Instant::now();
    let extraction = extract_spotify(
        &oauth.client_id,
        &oauth.client_secret,
        &args.url,
        args.searchers,
        args.no_interaction,
        args.search_retries,
    )
    .await
    .context("extracting youtube urls from spotify")?;
    let search_time = start.elapsed();

    if let Some(path) = extraction.name.as_ref() {
        ytdlp_args.extend(["-P".to_string(), path.clone()]);
    }

    let start = Instant::now();
    if extraction.tracks.len() == 1 {
        let (_, track) = extraction.tracks[0].clone();
        let Track { mut url, metadata } = track;
        info!("downloading {url}");
        for attempt in 0..=args.download_retries {
            let (output_file, new_url) =
                ytdlp(url, None, attempt, 0, args.show_ytdlp, &ytdlp_args).await;

            url = new_url;

            if let Some(path) = output_file {
                run_tagger(path.as_ref(), metadata, &url, !args.no_metadata, args.mp3).await;
                break;
            }
        }
    } else {
        download_many(
            extraction.tracks.clone(),
            Arc::from(ytdlp_args),
            args.downloaders,
            args.download_retries,
            args.show_ytdlp,
            !args.no_metadata,
            args.mp3,
        )
        .await;
    }
    let download_time = start.elapsed();

    info!(
        "took {} to download {} tracks ({search_time:?} to search, {download_time:?} to download)",
        HumanDuration(search_time + download_time),
        extraction.tracks.len()
    );

    info!("used {REQUESTS:?} spotify api calls in total");

    if !extraction.warnings.is_empty() {
        warn!(
            "these tracks could be incorrect: {:#?}",
            extraction.warnings()
        );
    }

    if extraction.failures > 0 {
        warn!(
            "{} songs failed to search, check report named `failed-...txt`",
            extraction.failures
        );
    }

    Ok(())
}

async fn download_many(
    urls: Vec<(usize, Track)>,
    args: Arc<[String]>,
    downloaders: usize,
    retry_limit: usize,
    show_ytdlp: bool,
    tag_metadata: bool,
    convert_mp3: bool,
) {
    let urls_len = urls.len();

    let (tracks_tx, tracks_rx) = async_channel::bounded(downloaders);
    // we dont want this channel to block on `send`s
    let (failed_tx, failed_rx) = async_channel::bounded(urls_len);
    let (results_tx, mut results_rx) = mpsc::channel(urls_len);

    let track_padding = urls.last().unwrap().0.to_string().len();

    tokio::spawn(async move {
        for url in urls {
            tracks_tx.send(url).await.expect("channel should be open");
        }
    });

    let pb_span = info_span!("pb");

    pb_span.pb_set_style(
        &ProgressStyle::with_template("{wide_bar} {pos}/{len} ({elapsed})")
            .expect("valid template"),
    );
    pb_span.pb_set_length(urls_len as u64);

    pb_span.pb_start();

    let mut downloader_handles = Vec::with_capacity(downloaders);
    for task in 0..downloaders {
        let failed_tx = failed_tx.clone();
        let failed_rx = failed_rx.clone();
        let tracks = tracks_rx.clone();
        let results = results_tx.clone();
        let args = args.clone();

        let handle = tokio::spawn(
            async move {
                loop {
                    debug!("waiting for url");

                    // `urls_tx` will be dropped once all urls are sent,
                    // closing the channel, meaning that eventually
                    // `recv()` will return an error, letting the task end.
                    //
                    // conversely, the `failed` channel has multiple cloned senders,
                    // meaning the channel will not close until all tasks end:
                    // using `try_recv()` ensures that the task will end instead of waiting forever.
                    let result = match tracks.recv().await {
                        Ok((i, url)) => Ok((0, i + 1, url)),
                        Err(_) => failed_rx.try_recv(),
                    };

                    let Ok((retry, track_num, track)) = result else {
                        debug!("no more urls");
                        return;
                    };

                    let Track { url, metadata } = track;

                    if retry > retry_limit.saturating_sub(1) {
                        warn!("track {track_num}: {url} reached retry limit");
                        continue;
                    }

                    info!("track {track_num}: {url}");
                    let (output_file, url) = ytdlp(
                        url,
                        Some(track_num),
                        retry,
                        track_padding,
                        show_ytdlp,
                        &args,
                    )
                    .await;
                    results
                        .send(output_file.is_some())
                        .await
                        .expect("shouldnt be closed");

                    if let Some(path) = output_file {
                        run_tagger(path.as_ref(), metadata, &url, tag_metadata, convert_mp3).await;
                    } else {
                        failed_tx
                            .send((retry + 1, track_num, Track::new(url, metadata)))
                            .await
                            .expect("channel should be open");
                    }
                }
            }
            .instrument(info_span!("downloader", id = task + 1)),
        );
        downloader_handles.push(handle);
    }

    drop(results_tx);

    async {
        while let Some(success) = results_rx.recv().await {
            if success {
                Span::current().pb_inc(1);
            }
        }
    }
    .instrument(pb_span)
    .await;

    for handle in downloader_handles {
        if let Err(err) = handle.await {
            warn!("a downloader failed: {err}");
        }
    }
}

/// returns a (`output_file`, `url`). `output_file` will always be `Some` on success.
#[inline]
#[instrument(skip(url, args, retry, track_padding, show_output), fields(try = retry + 1))]
async fn ytdlp(
    url: String,
    track: Option<usize>,
    retry: usize,
    track_padding: usize,
    show_output: bool,
    args: &[String],
) -> (Option<String>, String) {
    let mut ytdlp = Command::new("yt-dlp");
    ytdlp.arg(&url);
    if let Some(track) = track {
        // yt-dlp output template
        ytdlp.args([
            "-o",
            &format!("{track:0>track_padding$} - %(title)s [%(id)s].%(ext)s"),
        ]);
    }
    if show_output {
        ytdlp.arg("--verbose");
    }
    let ytdlp = ytdlp
        .args(["-f", "ba"])
        .args(["--quiet", "--print", "after_move:filepath"])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut ytdlp) = ytdlp else {
        return (None, url);
    };

    redir_output(
        Span::current(),
        ytdlp.stderr.take().expect("stderr is always captured"),
        !show_output,
    );

    let result = ytdlp.wait_with_output().await;

    if let Ok(result) = result {
        let status = result.status;

        if status.success() {
            let path = String::from_utf8(result.stdout[0..result.stdout.len() - 1].to_vec())
                .expect("should be utf8");
            return (Some(path), url);
        }

        warn!("yt-dlp terminated with {status}");
    }

    (None, url)
}

/// only warns if user set --mp3, but still tag in case user converts file to a different but supported format.
async fn run_tagger(path: &Path, metadata: Metadata, url: &str, should_tag: bool, mp3: bool) {
    if should_tag
        && let Err(err) = tagger(path, metadata, url).await
        && mp3
    {
        warn!("failed to tag file {path:?}: {err}");
    }
}

#[instrument(skip(metadata, url))]
async fn tagger(path: &Path, metadata: Metadata, url: &str) -> anyhow::Result<()> {
    let mut file = Probe::open(path)?.guess_file_type()?.read()?;

    debug!("tagging file {path:?}");

    let mut tag = Tag::new(file.primary_tag_type());

    // album & cover
    tag.set_album(metadata.album_name);
    let cover = CLIENT.get(metadata.cover_url).send().await?;
    let mime_type: Option<MimeType> = cover
        .headers()
        .get("content-type")
        .iter()
        .find_map(|h| h.to_str().map(MimeType::from_str).ok());
    let image = cover.bytes().await?;
    let picture = Picture::new_unchecked(PictureType::CoverFront, mime_type, None, image.to_vec());
    tag.push_picture(picture);

    // artists & genres
    let (artists, genres) = Metadata::to_tag_values(metadata.artists, '\0');
    tag.set_artist(artists);
    tag.set_genre(genres);

    // all other
    tag.set_title(metadata.name);
    tag.set_disk(metadata.disc_number);

    if metadata.explicit {
        // 1 is explicit
        tag.insert_text(ItemKey::ParentalAdvisory, "1".to_string());
    }

    tag.insert_text(ItemKey::Isrc, metadata.external_ids.isrc);

    let year = metadata
        .release_date
        .split('-')
        .next()
        .unwrap_or(&metadata.release_date);
    if let Ok(year) = year.parse() {
        tag.set_year(year);
    }

    tag.set_track(metadata.track_number);
    tag.set_track_total(metadata.album_tracks);

    tag.set_comment(format!(
        r"
    original spotify url: https://open.spotify.com/track/{}
    downloaded from: {url}

    by sptfydl!
    ",
        metadata.spotify_id
    ));

    file.insert_tag(tag);

    let start = Instant::now();
    // TODO: measure time with spawn_blocking and none
    file.save_to_path(path, WriteOptions::default())?;
    debug!("save took {:?}", start.elapsed());

    Ok(())
}

fn redir_output(span: Span, stderr: ChildStderr, warn: bool) {
    let mut stderr = BufReader::new(stderr).lines();

    tokio::spawn(
        async move {
            while let Ok(Some(line)) = stderr.next_line().await {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if warn {
                    warn!("{line}");
                } else {
                    info!("{line}");
                }
            }
        }
        .instrument(span),
    );
}

fn handle_exit() {
    let term = Term::stdout();
    if let Err(err) = term.show_cursor() {
        warn!("failed to show cursor: {err}");
    }
    exit(1);
}

const SPOTIFY_CONFIG_NAME: &str = "spotify_oauth.yaml";

#[derive(Serialize, Deserialize)]
struct SpotifyOauth {
    client_id: String,
    client_secret: String,
}

#[inline]
fn get_spotify_oauth() -> anyhow::Result<SpotifyOauth> {
    if let Ok(oauth) = load(SPOTIFY_CONFIG_NAME) {
        Ok(oauth)
    } else {
        let client_id = Input::new()
            .with_prompt("spotify client_id?")
            .interact_text()?;
        let client_secret = Password::new()
            .with_prompt("spotify client_secret?")
            .interact()?;

        let oauth = SpotifyOauth {
            client_id,
            client_secret,
        };
        save(&oauth, SPOTIFY_CONFIG_NAME)?;

        Ok(oauth)
    }
}
