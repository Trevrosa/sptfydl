use super::ext::ValueExt;
use super::{
    SearchResult,
    SearchResultType::{Album, Artist, Episode, Playlist, Podcast, Profile, Song, Station, Video},
};

use serde_json::Value;
use tracing::{trace, warn};

use super::{
    SearchResultType,
    consts::{MRLIR, NAVIGATION_BROWSE_ID, NAVIGATION_VIDEO_TYPE, PLAY_BUTTON, SUBTITLE},
};

use super::consts::{MENU_PLAYLIST_ID, TITLE_TEXT, WATCH_PID, WATCH_VIDEO_ID};

// https://github.com/sigma67/ytmusicapi//blob/a979691bb03c1cb5e7e39985bbd4014187940d68/ytmusicapi/parsers/search.py#L34
pub fn parse_top_result(data: &Value) -> Option<SearchResult> {
    let result_type = SearchResultType::try_from_str(data.pointer(SUBTITLE)?.as_str()?)?;

    let mut title = String::new();
    let mut video_id = None;
    let mut playlist_id = None;

    match result_type {
        Song | Video => {
            if let Some(on_tap) = data.get("onTap") {
                video_id = on_tap.pointer(WATCH_VIDEO_ID)?.try_to_string();
            }
        }
        Album => {
            title = data.pointer(TITLE_TEXT)?.try_to_string()?;
            let button_command = data.pointer("/buttons/0/buttonRenderer/command")?;
            playlist_id = button_command.pointer(WATCH_PID)?.try_to_string();
        }
        Playlist => {
            title = data.pointer(TITLE_TEXT)?.try_to_string()?;
            playlist_id = data.pointer(MENU_PLAYLIST_ID)?.try_to_string();
        }
        Episode | Profile | Artist | Podcast | Station => {
            warn!("ignoring {result_type:?}");
            return None;
        }
    }

    let result = SearchResult {
        title,
        video_id,
        playlist_id,
    };

    Some(result)
}

// https://github.com/sigma67/ytmusicapi//blob/a979691bb03c1cb5e7e39985bbd4014187940d68/ytmusicapi/parsers/_utils.py#L39
#[inline]
fn get_item_text(item: &Value, index: usize, run_index: Option<usize>) -> Option<&Value> {
    item.get("flexColumns")?
        .get(index)?
        .get("musicResponsiveListItemFlexColumnRenderer")?
        .pointer("/text/runs")?
        .get(run_index.unwrap_or(0))?
        .get("text")
}

// https://github.com/sigma67/ytmusicapi//blob/a979691bb03c1cb5e7e39985bbd4014187940d68/ytmusicapi/parsers/search.py#L74
pub fn parse_search_results(results: &[Value]) -> Option<Vec<SearchResult>> {
    let mut search_results = Vec::new();

    for data in results {
        trace!("parse_search_results");

        let Some(data) = data.pointer(MRLIR) else {
            continue;
        };

        trace!("mrlir");

        let video_type = data.pointer(PLAY_BUTTON).and_then(|v| {
            v.get("playNavigationEndpoint")
                .and_then(|v| v.pointer(NAVIGATION_VIDEO_TYPE))
        });
        let Some(video_type) = video_type else {
            continue;
        };

        let result_type = if let Some(browse_id) = data.pointer(NAVIGATION_BROWSE_ID) {
            SearchResultType::try_from_browse_id(browse_id.as_str())
        } else {
            None
        };

        let result_type = result_type.unwrap_or_else(|| {
            if video_type == "MUSIC_VIDEO_TYPE_ATV" {
                Song
            } else {
                Video
            }
        });

        let title = if result_type == Artist {
            String::new()
        } else {
            get_item_text(data, 0, None)?.try_to_string()?
        };

        let playlist_id = if result_type == Album {
            let play_nav = data.pointer(PLAY_BUTTON)?.get("playNavigationEndpoint")?;
            Some(play_nav.pointer(WATCH_PID)?.try_to_string()?)
        } else {
            None
        };

        let video_id = if matches!(result_type, Song | Video) {
            Some(
                data.pointer(PLAY_BUTTON)?
                    .pointer("/playNavigationEndpoint/watchEndpoint/videoId")?
                    .try_to_string()?,
            )
        } else {
            None
        };

        let result = SearchResult {
            title,
            video_id,
            playlist_id,
        };

        search_results.push(result);
    }

    Some(search_results)
}
