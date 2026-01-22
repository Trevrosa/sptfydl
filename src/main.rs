use anyhow::Context;
use clap::{ArgAction, Parser};
use console::Term;
use dialoguer::{Input, Password};
use serde::{Deserialize, Serialize};
use sptfydl::{load, save, spotify::extract_spotify};
use tracing::{Level, error, info};
use tracing_subscriber::{filter::Targets, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use std::process::{Command, exit};

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

    /// Skip prompts. Always choose the first option.
    #[arg(short, long)]
    no_interaction: bool,

    /// Additional args for yt-dlp.
    #[arg(last = true)]
    ytdlp_args: Vec<String>,
}

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

    let (urls, download_path) = extract_spotify(
        &oauth.client_id,
        &oauth.client_secret,
        &args.url,
        args.no_interaction,
    )
    .context("extracting youtube url from spotify")?;

    info!("found {} tracks", urls.len());

    let mut ytdlp_args = args.ytdlp_args;

    if args.mp3 {
        ytdlp_args.extend(["--extract-audio", "--audio-format", "mp3"].map(ToString::to_string));
    }

    if let Some(path) = download_path {
        ytdlp_args.extend(["-P".to_string(), path]);
    }

    for url in urls {
        let ytdlp = Command::new("yt-dlp")
            .arg(url)
            .args(["-f", "ba"])
            .args(&ytdlp_args)
            .status();

        if let Ok(status) = ytdlp
            && !status.success()
        {
            error!("yt-dlp terminated with status code {status}");
            exit(1);
        }
    }

    Ok(())
}

fn handle_exit() {
    let term = Term::stdout();
    if let Err(err) = term.show_cursor() {
        warn!("failed to show cursor: {err}");
    }
}

const SPOTIFY_CONFIG_NAME: &str = "spotify_oauth.yaml";

#[derive(Serialize, Deserialize)]
struct SpotifyOauth {
    client_id: String,
    client_secret: String,
}
