use chrono::Utc;
use sha1::{Digest, Sha1};

pub struct Browser {
    auth: String,
}

impl Browser {
    /// Takes a "__Secure-3PAPISID" cookie and converts into the corresponding authorization header value.
    ///
    /// <https://github.com/sigma67/ytmusicapi/blob/21445ca6f3bff83fc4f4f4546fc316710f517731/ytmusicapi/helpers.py#L60>
    #[must_use]
    pub fn new(sapisid: &str) -> Self {
        let timestamp = Utc::now().timestamp();

        let hash = Sha1::digest(format!("{timestamp} {sapisid} https://music.youtube.com"));

        Self {
            auth: format!("SAPISIDHASH {timestamp}_{hash:x}"),
        }
    }
}

impl AsRef<str> for Browser {
    fn as_ref(&self) -> &str {
        &self.auth
    }
}

/// Takes a list of headers (or just the Cookie header) and finds the "__Secure-3PAPISID" cookie.
#[must_use]
pub fn parse_cookie(input: &str) -> Option<&str> {
    let cookies = if input.starts_with("Cookie: ") {
        input
    } else {
        input.lines().find(|l| l.starts_with("Cookie: "))?
    };
    let cookie = cookies
        .split(';')
        .find(|cookie| cookie.trim_ascii_start().starts_with("__Secure-3PAPISID"))?;

    cookie.split('=').nth(1)
}
