mod consts;
mod ext;
mod helpers;

use std::{borrow::Cow, fmt::Display, time::Instant};

use consts::SECTION_LIST;
use helpers::{parse_search_results, parse_top_result};
use serde_json::Value;
use tracing::{debug, trace};

// just the ones I want.
#[derive(Debug)]
pub struct SearchResult {
    pub title: String,
    pub video_id: Option<String>,
    pub playlist_id: Option<String>,
}

/// Parse a raw search response to the more useful [`SearchResult`].
///
/// <https://github.com/sigma67/ytmusicapi//blob/a979691bb03c1cb5e7e39985bbd4014187940d68/ytmusicapi/mixins/search.py#L190>
pub fn parse_results(resp: &Value) -> Option<Vec<SearchResult>> {
    let start = Instant::now();

    let results = resp
        .pointer("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content")
        .map_or_else(|| resp.get("contents"), Some)?;

    // https://github.com/sigma67/ytmusicapi//blob/a979691bb03c1cb5e7e39985bbd4014187940d68/ytmusicapi/navigation.py#L115
    let section_list = results.pointer(SECTION_LIST)?.as_array()?;

    // no results
    if section_list.is_empty() {
        return None;
    }

    let mut search_results = Vec::new();

    for result in section_list {
        trace!("res");
        let shelf_contents = if let Some(music_card_shelf) = result.get("musicCardShelfRenderer") {
            trace!("parsing top result");
            search_results.push(parse_top_result(music_card_shelf)?);

            let Some(shelf_contents) = music_card_shelf.get("contents").and_then(|v| v.as_array())
            else {
                trace!("d");
                continue;
            };

            shelf_contents
        } else if let Some(music_shelf) = result.get("musicShelfRenderer") {
            let shelf_contents = music_shelf.get("contents")?.as_array()?;
            trace!("we got shelf contents");

            shelf_contents
        } else {
            continue;
        };

        let mut parsed = parse_search_results(shelf_contents)?;
        search_results.append(&mut parsed);
    }

    trace!("there are some results!");

    debug!(
        "took {:?} to parse {} results",
        start.elapsed(),
        search_results.len()
    );

    Some(search_results)
}

impl SearchResult {
    #[must_use]
    pub fn link(&self) -> Option<String> {
        self.video_id
            .as_ref()
            .map(|id| format!("https://youtube.com/watch?v={id}"))
    }

    /// Returns the result of [`Self::link`] or "NO LINK FOUND"
    #[must_use]
    pub fn link_or_default(&self) -> Cow<'_, str> {
        if let Some(link) = self.link() {
            Cow::Owned(link)
        } else {
            Cow::Borrowed("NO LINK FOUND")
        }
    }
}

impl Display for SearchResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.title, self.link_or_default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchResultType {
    Album,
    Artist,
    Playlist,
    Song,
    Video,
    Station,
    Profile,
    Podcast,
    Episode,
}

impl SearchResultType {
    /// Try to convert `str` to a [`SearchResultType`], accepting names and browseId prefixes. Case-insensitive.
    fn try_from_str(str: &str) -> Option<Self> {
        use SearchResultType::{
            Album, Artist, Episode, Playlist, Podcast, Profile, Song, Station, Video,
        };
        let str = str.to_lowercase();
        match str.as_str() {
            "album" | "mpre" => Some(Album),
            "artist" | "mpla" | "uc" => Some(Artist),
            "playlist" | "vm" | "rd" | "vl" => Some(Playlist),
            "song" => Some(Song),
            "video" => Some(Video),
            "station" => Some(Station),
            "profile" => Some(Profile),
            "podcast" | "mpsp" => Some(Podcast),
            "episode" | "mped" => Some(Episode),
            _ => None,
        }
    }

    fn try_from_browse_id(id: Option<&str>) -> Option<Self> {
        use SearchResultType::{Artist, Episode, Playlist, Podcast};
        let id = id?;
        if id.starts_with("VM") || id.starts_with("RD") || id.starts_with("VL") {
            Some(Playlist)
        } else if id.starts_with("MPLA") {
            Some(Artist)
        } else if id.starts_with("MPSP") {
            Some(Podcast)
        } else if id.starts_with("MPED") {
            Some(Episode)
        } else if id.starts_with("UC") {
            Some(Artist)
        } else {
            None
        }
    }
}
