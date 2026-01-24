# sptfydl

A simple CLI tool that allows you to download Spotify tracks, albums, and playlists.

```
Usage: sptfydl.exe [OPTIONS] <URL> [-- <YTDLP_ARGS>...]

Arguments:
  <URL>            The spotify url to download
  [YTDLP_ARGS]...  Additional args for yt-dlp

Options:
      --mp3             Tell yt-dlp to convert to mp3
  -v, --verbose...      Be a bit more verbose. Can be applied more than once (-v, -vv)
  -n, --no-interaction  Skip prompts. Always choose the first option
  -h, --help            Print help
  -V, --version         Print version
```
