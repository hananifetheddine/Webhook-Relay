use std::sync::Arc;
use std::time::Duration;

use sqlx::MySqlPool;

use crate::config::Config;
use crate::error::AppError;

#[derive(Clone)]
pub struct AppState {
    pub pool: MySqlPool,
    pub http: reqwest::Client,
    pub config: Arc<Config>,
}

impl AppState {
    pub fn new(pool: MySqlPool, config: Config) -> Result<Self, AppError> {
        let timeout = Duration::from_secs(config.http_timeout_secs);
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("webhook-relay/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(AppError::internal)?;
        Ok(Self {
            pool,
            http,
            config: Arc::new(config),
        })
    }
}
