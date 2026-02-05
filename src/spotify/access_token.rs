use std::sync::Arc;

use base64::{Engine, engine::GeneralPurpose};
use chrono::{TimeDelta, Utc};
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
    granted: Option<chrono::DateTime<Utc>>,
}

impl From<AccessToken> for Arc<str> {
    fn from(val: AccessToken) -> Self {
        Arc::from(val.access_token)
    }
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
    pub async fn get(id: &str, secret: &str) -> Option<Self> {
        const AUTH_REQ: &str = "https://accounts.spotify.com/api/token";
        const BASE64: GeneralPurpose = base64::engine::general_purpose::STANDARD;

        let auth = BASE64.encode(format!("{id}:{secret}"));

        let resp = CLIENT
            .post(AUTH_REQ)
            .header("Authorization", format!("Basic {auth}"))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await;
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
                resp.text()
                    .await
                    .as_deref()
                    .unwrap_or("failed to read body")
            );
            return None;
        }

        let Ok(mut resp) = resp.json::<AccessToken>().await else {
            return None;
        };
        resp.granted = Some(Utc::now());

        info!(
            "got access token `{}`, expiring in {} secs",
            resp.token_type, resp.expires_in
        );

        Some(resp)
    }

    #[must_use]
    pub fn expired(&self) -> bool {
        self.granted
            .is_none_or(|g| Utc::now() - g > TimeDelta::seconds(self.expires_in.cast_signed()))
    }
}
