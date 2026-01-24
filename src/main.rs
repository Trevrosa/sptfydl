use anyhow::Context;
use clap::{ArgAction, Parser};
use console::Term;
use dialoguer::{Confirm, Input, Password};
use serde::{Deserialize, Serialize};
use sptfydl::{load, save, spotify::extract_spotify};
use tracing::{Level, debug, info, warn};
use tracing_subscriber::{filter::Targets, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use std::process::{Command, Stdio, exit};

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

    /// Additional args for yt-dlp.
    #[arg(last = true)]
    ytdlp_args: Vec<String>,
}

const RETRY_LIMIT: u32 = 6;

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let filter = match args.verbose {
        0 => Level::INFO,
        1 => Level::DEBUG,
        2..=u8::MAX => Level::TRACE,
    };

    tracing_subscriber::registry()
        .with(fmt::layer().without_time().compact())
        .with(Targets::new().with_target("sptfydl", filter))
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
    .context("extracting youtube url from spotify")?;

    let mut ytdlp_args = args.ytdlp_args;

    if args.mp3 {
        ytdlp_args.extend(["--extract-audio", "--audio-format", "mp3"].map(ToString::to_string));
    }

    if let Some(path) = extraction.name.clone() {
        ytdlp_args.extend(["-P".to_string(), path]);
    }

    let single = extraction.urls.len() == 1;

    let mut failed = Vec::new();
    for (i, url) in &extraction.urls {
        ytdlp(url, i + 1, single, &ytdlp_args, Some(&mut failed));
    }

    // gets reset to 0 on success!
    let mut tries = 0;
    while !failed.is_empty() {
        tries += 1;

        info!("these urls failed to download: {failed:?}");

        let len = failed.len();

        let mut new_failed = Vec::with_capacity(len);

        let retry_urls = || {
            for (i, url) in failed {
                // we dont +1 to i because we already did in previous call to `ytdlp`, and we are using its output
                if ytdlp(url, i, single, &ytdlp_args, Some(&mut new_failed)) {
                    tries = 0;
                }
            }
        };

        if args.no_interaction {
            debug!("retrying because --no-interaction was set");
            retry_urls();
        } else {
            let retry = Confirm::new()
                .with_prompt("retry urls?")
                .default(true)
                .interact();
            if retry.is_ok_and(|r| r) {
                retry_urls();
            }
        }

        if tries == RETRY_LIMIT && new_failed.len() == len {
            warn!("failed urls not succeeding after {RETRY_LIMIT} tries, stopping");
            break;
        }

        failed = new_failed;
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
fn ytdlp<'a>(
    url: &'a str,
    track_num: usize,
    single: bool,
    args: &[String],
    failed: Option<&mut Vec<(usize, &'a str)>>,
) -> bool {
    let mut ytdlp = Command::new("yt-dlp");

    ytdlp.arg(url);
    if !single {
        // yt-dlp output template
        ytdlp.args(["-o", &format!("{track_num}. %(title)s [%(id)s].%(ext)s")]);
    };

    let ytdlp = ytdlp
        .args(["-f", "ba"])
        .args(args)
        .stdout(Stdio::inherit())
        .output();

    if let Ok(output) = ytdlp {
        let status = output.status;

        if status.success() {
            return true;
        }

        let stderr = str::from_utf8(&output.stderr);

        if stderr.is_ok_and(|err| err.contains("Interrupted by user")) {
            warn!("ctrl-c detected");
            exit(1);
        } else {
            warn!("yt-dlp terminated with {status}");
            if let Some(failed) = failed {
                failed.push((track_num, url));
            }
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
