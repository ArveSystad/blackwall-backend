use std::net::SocketAddr;

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";

/// Runtime configuration, read from the process environment.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: SocketAddr,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url = std::env::var("DATABASE_URL")
            .ok()
            .filter(|v| !v.is_empty())
            .ok_or("DATABASE_URL must be set, e.g. postgres://user:pass@host:5432/blackwall")?;

        let bind_addr = std::env::var("BIND_ADDR")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_BIND_ADDR.to_owned());
        let bind_addr = bind_addr
            .parse()
            .map_err(|e| format!("BIND_ADDR {bind_addr:?} is not a valid socket address: {e}"))?;

        Ok(Self {
            database_url,
            bind_addr,
        })
    }
}
