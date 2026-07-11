use std::env;
use std::num::ParseIntError;

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_ttl_secs: i64,
    pub bind_addr: String,
    pub worker_poll_interval_ms: u64,
    pub worker_batch_size: i64,
    pub worker_concurrency: usize,
    pub http_timeout_secs: u64,
    pub max_delivery_attempts: u32,
    pub claim_lease_secs: i64,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing environment variable {0}")]
    Missing(String),
    #[error("invalid environment variable {0}")]
    Invalid(String),
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let database_url = required("DATABASE_URL")?;
        if !database_url.starts_with("mysql://") {
            return Err(ConfigError::Invalid("DATABASE_URL".into()));
        }

        let jwt_secret = required("JWT_SECRET")?;
        if jwt_secret.len() < 32 {
            return Err(ConfigError::Invalid("JWT_SECRET".into()));
        }

        let jwt_ttl_secs = optional("JWT_TTL_SECS", 86_400)?;
        let worker_poll_interval_ms = optional("WORKER_POLL_INTERVAL_MS", 1_000)?;
        let worker_batch_size: i64 = optional("WORKER_BATCH_SIZE", 50)?;
        let worker_concurrency: usize = optional("WORKER_CONCURRENCY", 20)?;
        let http_timeout_secs = optional("HTTP_TIMEOUT_SECS", 10)?;
        let max_delivery_attempts: u32 = optional("MAX_DELIVERY_ATTEMPTS", 6)?;
        let claim_lease_secs: i64 = optional("CLAIM_LEASE_SECS", 60)?;
        let bind_addr = optional_string("BIND_ADDR", "0.0.0.0:8080")?;

        let config = Self {
            database_url,
            jwt_secret,
            jwt_ttl_secs,
            bind_addr,
            worker_poll_interval_ms,
            worker_batch_size,
            worker_concurrency,
            http_timeout_secs,
            max_delivery_attempts,
            claim_lease_secs,
        };
        config.validate()?;
        Ok(config)
    }

    /// Configuration fixe pour les tests d'intégration. Seule l'URL MySQL vient de l'environnement.
    pub fn for_tests(
        database_url: impl Into<String>,
        max_delivery_attempts: u32,
        http_timeout_secs: u64,
    ) -> Self {
        Self {
            database_url: database_url.into(),
            jwt_secret: "test-secret-test-secret-test-secret".to_string(),
            jwt_ttl_secs: 3_600,
            bind_addr: "127.0.0.1:0".to_string(),
            worker_poll_interval_ms: 1_000,
            worker_batch_size: 50,
            worker_concurrency: 8,
            http_timeout_secs,
            max_delivery_attempts,
            claim_lease_secs: 30,
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.jwt_ttl_secs <= 0 {
            return Err(ConfigError::Invalid("JWT_TTL_SECS".into()));
        }
        if !(100..=60_000).contains(&self.worker_poll_interval_ms) {
            return Err(ConfigError::Invalid("WORKER_POLL_INTERVAL_MS".into()));
        }
        if !(1..=500).contains(&self.worker_batch_size) {
            return Err(ConfigError::Invalid("WORKER_BATCH_SIZE".into()));
        }
        if !(1..=100).contains(&self.worker_concurrency) {
            return Err(ConfigError::Invalid("WORKER_CONCURRENCY".into()));
        }
        if !(1..=60).contains(&self.http_timeout_secs) {
            return Err(ConfigError::Invalid("HTTP_TIMEOUT_SECS".into()));
        }
        if !(1..=20).contains(&self.max_delivery_attempts) {
            return Err(ConfigError::Invalid("MAX_DELIVERY_ATTEMPTS".into()));
        }
        if self.claim_lease_secs <= i64::try_from(self.http_timeout_secs).unwrap_or(i64::MAX) {
            return Err(ConfigError::Invalid("CLAIM_LEASE_SECS".into()));
        }
        if self.bind_addr.is_empty() {
            return Err(ConfigError::Invalid("BIND_ADDR".into()));
        }
        Ok(())
    }
}

fn required(name: &str) -> Result<String, ConfigError> {
    env::var(name).map_err(|_| ConfigError::Missing(name.to_string()))
}

fn optional_string(name: &str, default: &str) -> Result<String, ConfigError> {
    match env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(ConfigError::Invalid(name.to_string())),
        Err(_) => Ok(default.to_string()),
    }
}

fn optional<T>(name: &str, default: T) -> Result<T, ConfigError>
where
    T: std::str::FromStr<Err = ParseIntError>,
{
    match env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|_| ConfigError::Invalid(name.to_string())),
        Err(_) => Ok(default),
    }
}
