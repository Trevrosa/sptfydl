# sptfydl

A simple and *fast* CLI tool that allows you to download Spotify tracks, albums, and playlists. Requires [yt-dlp](https://github.com/yt-dlp/yt-dlp).

## Features
- Concurrent searching and downloading
- Metadata tagging for supported formats
- Light on spotify api calls
- Customisable, see cli args below

---

```
a tool to download spotify links

Usage: sptfydl.exe [OPTIONS] <URL> [-- <YTDLP_ARGS>...]

Arguments:
  <URL>            The spotify url to download
  [YTDLP_ARGS]...  Additional args for yt-dlp

Options:
      --mp3
          Tell yt-dlp to convert to mp3
  -v, --verbose...
          Be a bit more verbose. Can be applied more than once (-v, -vv)
      --show-ytdlp
          Show the output of ytdlp commands
  -n, --no-interaction
          Skip prompts; always choose the default or first available option
  -d, --downloaders <DOWNLOADERS>
          The number of concurrent downloads [default: 5]
  -s, --searchers <SEARCHERS>
          The number of concurrent searches [default: 3]
      --download-retries <DOWNLOAD_RETRIES>
          The number of retries allowed for downloads [default: 5]
      --search-retries <SEARCH_RETRIES>
          The number of retries allowed for searches [default: 3]
  -h, --help
          Print help
  -V, --version
          Print version
```
