use anyhow::Context;
use clap::{ArgAction, Parser};
use console::Term;
use dialoguer::{Confirm, Input, Password};
use serde::{Deserialize, Serialize};
use sptfydl::{load, save, spotify::extract_spotify};
use tokio::process::Command;
use tracing::{Instrument, Level, debug, info, info_span, warn};
use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::{filter::Targets, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use std::{
    process::{Stdio, exit},
    sync::Arc,
};

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

    /// Skip prompts. Always choose the default or first available option.
    #[arg(short, long)]
    no_interaction: bool,

    #[arg(long, default_value_t = 5)]
    downloaders: usize,

    #[arg(long, default_value_t = 3)]
    searchers: usize,

    /// Additional args for yt-dlp.
    #[arg(last = true)]
    ytdlp_args: Vec<String>,
}

const RETRY_LIMIT: usize = 5;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let filter = match args.verbose {
        0 => Level::INFO,
        1 => Level::DEBUG,
        2..=u8::MAX => Level::TRACE,
    };

    let indicatif_layer = IndicatifLayer::new();

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

    let oauth: SpotifyOauth = if let Ok(oauth) = load(SPOTIFY_CONFIG_NAME) {
        oauth
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

        oauth
    };

    let extraction = extract_spotify(
        &oauth.client_id,
        &oauth.client_secret,
        &args.url,
        args.no_interaction,
    )
    .await
    .context("extracting youtube url from spotify")?;

    let mut ytdlp_args = args.ytdlp_args;

    if args.mp3 {
        ytdlp_args.extend(["--extract-audio", "--audio-format", "mp3"].map(ToString::to_string));
    }

    if let Some(path) = extraction.name.clone() {
        ytdlp_args.extend(["-P".to_string(), path]);
    }

    let ytdlp_args = Arc::new(ytdlp_args);

    let urls_len = extraction.urls.len();
    let single = urls_len == 1;

    let (urls_tx, urls_rx) = async_channel::bounded(args.downloaders);

    let urls = extraction.urls.clone();
    tokio::spawn(async move {
        for url in urls {
            urls_tx.send(url).await.expect("channel should be open");
        }
    });

    let (failed_tx, failed_rx) = async_channel::bounded(urls_len);

    info!("downloading with {} downloaders", args.downloaders);

    let mut downloaders = Vec::with_capacity(args.downloaders);
    for task in 0..args.downloaders {
        let failed_tx = failed_tx.clone();
        let failed_rx = failed_rx.clone();
        let urls_rx = urls_rx.clone();
        let args = ytdlp_args.clone();

        let handle = tokio::spawn(
            async move {
                loop {
                    debug!("waiting for url");
                    let result = match urls_rx.recv().await {
                        Ok((i, url)) => Ok((0, i + 1, url)),
                        Err(_) => failed_rx.try_recv(),
                    };

                    let Ok((try_num, i, url)) = result else {
                        debug!("no more urls");
                        return;
                    };

                    if try_num > RETRY_LIMIT + 1 {
                        warn!("track {i}: {url} reached retry limit");
                        continue;
                    }

                    info!("track {i}: {url}");
                    ytdlp(url, i, single, &args, try_num, &failed_tx).await;
                }
            }
            .instrument(info_span!("downloader", id = task + 1)),
        );
        downloaders.push(handle);
    }

    for handle in downloaders {
        if let Err(err) = handle.await {
            warn!("a downloader failed: {err}");
        };
    }

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

fn handle_exit() {
    let term = Term::stdout();
    if let Err(err) = term.show_cursor() {
        warn!("failed to show cursor: {err}");
    }
    exit(1);
}

/// returns `true` on success
#[inline]
async fn ytdlp(
    url: String,
    track_num: usize,
    single: bool,
    args: &[String],
    try_num: usize,
    failed: &async_channel::Sender<(usize, usize, String)>,
) -> bool {
    let mut ytdlp = Command::new("yt-dlp");

    ytdlp.arg(&url);
    if !single {
        // yt-dlp output template
        ytdlp.args(["-o", &format!("{track_num}. %(title)s [%(id)s].%(ext)s")]);
    };

    let ytdlp = ytdlp.args(["-f", "ba"]).args(args).output().await;

    if let Ok(output) = ytdlp {
        let status = output.status;

        if status.success() {
            return true;
        }

        let stderr = str::from_utf8(&output.stderr);

        if stderr.is_ok_and(|err| err.contains("Interrupted by user")) {
            warn!("ctrl-c detected");
            handle_exit();
        } else {
            warn!("yt-dlp terminated with {status}");
            failed
                .send((try_num + 1, track_num, url))
                .await
                .expect("channel should be open");
        }
    }

    false
}

const SPOTIFY_CONFIG_NAME: &str = "spotify_oauth.yaml";

#[derive(Serialize, Deserialize)]
struct SpotifyOauth {
    client_id: String,
    client_secret: String,
}
