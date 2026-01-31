use anyhow::Context;
use clap::{ArgAction, Parser};
use console::Term;
use dialoguer::{Input, Password};
use indicatif::ProgressStyle;
use serde::{Deserialize, Serialize};
use sptfydl::{load, save, spotify::extract_spotify};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{ChildStderr, ChildStdout, Command},
    sync::mpsc,
};
use tracing::{Instrument, Level, Span, debug, info, info_span, warn};
use tracing_indicatif::{IndicatifLayer, span_ext::IndicatifSpanExt};
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

    /// Show the output of ytdlp commands.
    #[arg(long)]
    show_ytdlp: bool,

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

    let oauth = get_spotify_oauth()?;

    let mut extraction = extract_spotify(
        &oauth.client_id,
        &oauth.client_secret,
        &args.url,
        args.searchers,
        args.no_interaction,
        args.search_retries,
    )
    .await
    .context("extracting youtube urls from spotify")?;

    let mut ytdlp_args = args.ytdlp_args;

    ytdlp_args.push("--no-playlist".to_string());

    if args.mp3 {
        ytdlp_args.extend(["--extract-audio", "--audio-format", "mp3"].map(ToString::to_string));
    }

    if let Some(path) = extraction.name.take() {
        ytdlp_args.extend(["-P".to_string(), path]);
    }

    if extraction.urls.len() == 1 {
        let url = extraction.urls[0].1.clone();
        info!("downloading {url}");
        ytdlp(url, 1, 0, &ytdlp_args, args.show_ytdlp, None).await;
    } else {
        download_many(
            extraction.urls.clone(),
            Arc::from(ytdlp_args),
            args.downloaders,
            args.download_retries,
            args.show_ytdlp,
        )
        .await;
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

async fn download_many(
    urls: Vec<(usize, String)>,
    args: Arc<[String]>,
    downloaders: usize,
    retries: usize,
    show_ytdlp: bool,
) {
    let urls_len = urls.len();

    let (urls_tx, urls_rx) = async_channel::bounded(downloaders);
    // we dont want this channel to block on `send`s
    let (failed_tx, failed_rx) = async_channel::bounded(urls_len);
    let (results_tx, mut results_rx) = mpsc::channel(urls_len);

    tokio::spawn(async move {
        for url in urls {
            urls_tx.send(url).await.expect("channel should be open");
        }
    });

    let retry_limit = retries;

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
        let urls = urls_rx.clone();
        let results = results_tx.clone();
        let args = args.clone();

        let handle = tokio::spawn(
            async move {
                loop {
                    debug!("waiting for url");

                    // the `urls` channel will be dropped once all urls are sent,
                    // meaning that eventually `recv()` will return an error,
                    // letting the task end.
                    //
                    // conversely, the `failed` channel has multiple cloned senders,
                    // meaning the channel will not close until all tasks end:
                    // using `try_recv()` ensures that the task will end instead of waiting forever.
                    let result = match urls.recv().await {
                        Ok((i, url)) => Ok((0, i + 1, url)),
                        Err(_) => failed_rx.try_recv(),
                    };

                    let Ok((try_num, track_num, url)) = result else {
                        debug!("no more urls");
                        return;
                    };

                    Span::current().record("try", try_num);

                    if try_num > retry_limit + 1 {
                        warn!("track {track_num}: {url} reached retry limit");
                        continue;
                    }

                    info!("track {track_num}: {url}");
                    let success =
                        ytdlp(url, track_num, try_num, &args, show_ytdlp, Some(&failed_tx)).await;
                    results.send(success).await.expect("shouldnt be closed");
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

/// returns `true` on success
#[inline]
async fn ytdlp(
    url: String,
    track_num: usize,
    try_num: usize,
    args: &[String],
    show_output: bool,
    // (try_num, track_num, url)
    failed: Option<&async_channel::Sender<(usize, usize, String)>>,
) -> bool {
    let stdout = if show_output {
        Stdio::piped()
    } else {
        Stdio::null()
    };

    let ytdlp = Command::new("yt-dlp")
        .arg(&url)
        // yt-dlp output template
        .args(["-o", &format!("{track_num}. %(title)s [%(id)s].%(ext)s")])
        .args(["-f", "ba"])
        .args(args)
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut ytdlp) = ytdlp else {
        return false;
    };

    redir_output(
        ytdlp.stdout.take(),
        ytdlp.stderr.take().expect("stderr is always captured"),
    );

    let status = ytdlp.wait().await;

    if let Ok(status) = status {
        if status.success() {
            return true;
        }

        warn!("yt-dlp terminated with {status}");
        if let Some(failed) = failed {
            failed
                .send((try_num + 1, track_num, url))
                .await
                .expect("channel should be open");
        }
    }

    false
}

fn redir_output(stdout: Option<ChildStdout>, stderr: ChildStderr) {
    let mut stderr = BufReader::new(stderr).lines();

    tokio::spawn(async move {
        while let Ok(Some(line)) = stderr.next_line().await {
            warn!("{line}");
        }
    });

    if let Some(stdout) = stdout {
        let mut stdout = BufReader::new(stdout).lines();
        tokio::spawn(async move {
            while let Ok(Some(line)) = stdout.next_line().await {
                info!("{line}");
            }
        });
    }
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
