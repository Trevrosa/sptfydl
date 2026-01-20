use std::time::{Duration, Instant};

use base64::{Engine, engine::GeneralPurpose};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::CLIENT;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AccessToken {
    #[allow(clippy::struct_field_names)]
    access_token: String,
    token_type: String,
    /// seconds
    expires_in: u64,

    #[serde(skip)]
    granted: Option<Instant>,
}

impl AsRef<str> for AccessToken {
    fn as_ref(&self) -> &str {
        &self.access_token
    }
}

impl AccessToken {
    /// Get a new [`AccessToken`] with client credentials.
    ///
    /// <https://developer.spotify.com/documentation/web-api/tutorials/client-credentials-flow>
    pub fn get(id: &str, secret: &str) -> Option<Self> {
        const AUTH_REQ: &str = "https://accounts.spotify.com/api/token";
        const BASE64: GeneralPurpose = base64::engine::general_purpose::STANDARD;

        let auth = BASE64.encode(format!("{id}:{secret}"));

        let resp = CLIENT
            .post(AUTH_REQ)
            .header("Authorization", format!("Basic {auth}"))
            .form(&[("grant_type", "client_credentials")])
            .send();
        let resp = match resp {
            Ok(resp) => resp,
            Err(err) => {
                error!("{err}");
                return None;
            }
        };

        if !resp.status().is_success() {
            error!(
                "failed to request access token: `{}`",
                resp.text().unwrap_or("failed to read body".to_string())
            );
            return None;
        }

        let Ok(mut resp) = resp.json::<AccessToken>() else {
            return None;
        };
        resp.granted = Some(Instant::now());

        info!(
            "got access token `{}`, expiring in {} secs",
            resp.token_type, resp.expires_in
        );

        Some(resp)
    }

    #[must_use]
    pub fn expired(&self) -> bool {
        self.granted
            .is_none_or(|g| g.elapsed() > Duration::from_secs(self.expires_in))
    }
}
