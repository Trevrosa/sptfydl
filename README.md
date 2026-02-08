# sptfydl

A simple and *fast* CLI tool that allows you to download Spotify tracks, albums, and playlists. Requires [yt-dlp](https://github.com/yt-dlp/yt-dlp).

## Features
- Concurrent searching and downloading
- Metadata tagging for supported formats
- Light on spotify api calls (~1 request per 100 playlist tracks, +1 request per 50 total artists, +1 request per 50 tracks (only for album downloads))
- Customisable, see cli args below

## Usage

Install via `cargo install sptfydl` or from [releases](https://github.com/Trevrosa/sptfydl/releases/latest).

```
a tool to download spotify links

Usage: sptfydl [OPTIONS] <URL> [-- <YTDLP_ARGS>...]

Arguments:
  <URL>            The spotify url to download
  [YTDLP_ARGS]...  Additional args for yt-dlp

Options:
  -f, --format <FORMAT>
          The format to download songs to [default: mp3] [possible values: mp3, flac, original]
  -P, --path <PATH>
          The path to output to
  -d, --downloaders <DOWNLOADERS>
          The number of concurrent downloads [default: 5]
  -s, --searchers <SEARCHERS>
          The number of concurrent searches [default: 3]
      --isrc
          Prefer isrc for searches. Useful for when you want a specific recording of a song
      --no-metadata
          Disable tagging of mp3 files
  -n, --no-interaction
          Skip prompts; always choose the default or first available option
      --download-retries <DOWNLOAD_RETRIES>
          The number of retries allowed for downloads [default: 5]
      --search-retries <SEARCH_RETRIES>
          The number of retries allowed for searches [default: 3]
      --show-ytdlp
          Show the output of ytdlp commands
  -v, --verbose...
          Be a bit more verbose. Can be applied more than once (-v, -vv)
  -h, --help
          Print help
  -V, --version
          Print version
```
