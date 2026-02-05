use crate::spotify::search::{ExternalIds, SpotifyArtist};

#[derive(Debug)]
pub struct Extraction {
    pub tracks: Vec<(usize, Track)>,
    pub name: Option<String>,
    /// guaranteed to be in range of `urls`
    pub warnings: Vec<usize>,
    pub failures: usize,
}

impl Extraction {
    #[must_use]
    pub fn warnings(&self) -> Vec<&(usize, Track)> {
        self.warnings.iter().map(|idx| &self.tracks[*idx]).collect()
    }
}

/// A track, with its `url` and `metadata`.
#[derive(Debug, Clone)]
pub struct Track {
    pub url: String,
    pub metadata: Metadata,
}

impl Track {
    #[must_use]
    pub fn new(url: String, metadata: Metadata) -> Self {
        Self { url, metadata }
    }
}

/// Contains select fields of [`SpotifyTrack`].
#[derive(Debug, Clone)]
pub struct Metadata {
    pub cover_url: String,
    pub disc_number: u32,
    /// genres are found here
    pub artists: Vec<SpotifyArtist>,
    pub spotify_id: String,
    pub name: String,
    pub explicit: bool,
    pub external_ids: ExternalIds,
    pub track_number: u32,
    pub album_name: String,
    /// y-m-d
    pub release_date: String,
}

impl Metadata {
    #[inline]
    pub fn artists_and_genres(artists: Vec<SpotifyArtist>, separator: char) -> (String, String) {
        let (artists, genres): (Vec<_>, Vec<_>) =
            artists.into_iter().map(SpotifyArtist::into_tuple).unzip();

        let mut genres = genres.iter().flatten().fold(String::new(), |mut acc, g| {
            acc.push_str(g);
            acc.push(separator);
            acc
        });
        // remove trailling \0
        genres.pop();

        let mut tmp = [0; 1];
        let separator: &str = separator.encode_utf8(&mut tmp);

        (artists.join(separator), genres)
    }
}

impl SpotifyArtist {
    #[inline]
    #[must_use]
    pub fn into_tuple(self) -> (String, Vec<String>) {
        (self.name, self.genres)
    }
}
