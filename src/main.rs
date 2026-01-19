use anyhow::Context;
use clap::Parser;
use dialoguer::{Input, Password};
use serde::{Deserialize, Serialize};
use sptfydl::{load, save, spotify::extract_spotify};
use tracing::{Level, error};
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

    /// Be a bit more verbose
    #[arg(short, long)]
    verbose: bool,

    /// Skip prompts. Always choose the first option.
    #[arg(short, long)]
    no_interaction: bool,

    /// Additional args for yt-dlp.
    #[arg(last = true)]
    ytdlp_args: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let filter = if args.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };

    tracing_subscriber::registry()
        .with(fmt::layer().without_time().compact())
        .with(Targets::new().with_target("sptfydl", filter))
        .init();

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

    let url = extract_spotify(
        &oauth.client_id,
        &oauth.client_secret,
        &args.url,
        args.no_interaction,
    )
    .context("extracting youtube url from spotify")?;

    let mp3: &[&str] = if args.mp3 {
        &["--extract-audio", "--audio-format", "mp3"]
    } else {
        &[]
    };

    let ytdlp = Command::new("yt-dlp")
        .arg(url)
        .args(["-f", "ba"])
        .args(mp3)
        .args(args.ytdlp_args)
        .status();

    if let Ok(status) = ytdlp
        && !status.success() {
            error!("yt-dlp terminated with status code {status}");
            exit(1);
        }

    Ok(())
}

const SPOTIFY_CONFIG_NAME: &str = "spotify_oauth.yaml";

#[derive(Serialize, Deserialize)]
struct SpotifyOauth {
    client_id: String,
    client_secret: String,
}
