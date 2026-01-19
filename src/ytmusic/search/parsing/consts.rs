//! JSON paths to navigate youtube results.
//!
//! Only the ones I use.

pub const SECTION_LIST: &str = "/sectionListRenderer/contents";
pub const PLAY_BUTTON: &str =
    "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer";
pub const NAVIGATION_BROWSE_ID: &str = "/navigationEndpoint/browseEndpoint/browseId";
pub const WATCH_VIDEO_ID: &str = "/watchEndpoint/videoId";
/// Always used when authenticated.
pub const WATCH_PID: &str = "/watchPlaylistEndpoint/playlistId";
pub const NAVIGATION_VIDEO_TYPE: &str =
    "/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType";
pub const TITLE_TEXT: &str = "/title/runs/0/text";
pub const SUBTITLE: &str = "/subtitle/runs/0/text";
pub const MRLIR: &str = "/musicResponsiveListItemRenderer";
pub const MENU_PLAYLIST_ID: &str = "/menu/menuRenderer/items/0/menuNavigationItemRenderer/navigationEndpoint/watchPlaylistEndpoint/playlistId";
