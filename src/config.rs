use clap::Parser;
use std::{net::SocketAddr, path::PathBuf};

/// CLI flags also have environment equivalents, so no .env loader is needed.
#[derive(Clone, Debug, Parser)]
#[command(version, about)]
pub struct Config {
    #[arg(long, env = "POCKET_BIND", default_value = "127.0.0.1:8787")]
    pub bind: SocketAddr,
    #[arg(long, env = "POCKET_DATA", default_value = "data")]
    pub data: PathBuf,
    /// Lock browser requests to this origin when serving through ngrok.
    #[arg(long, env = "POCKET_PUBLIC_ORIGIN")]
    pub public_origin: Option<String>,
    /// Defaults to the installed CLI on PATH; Windows desktop installs are detected.
    #[arg(long, env = "POCKET_CODEX_BIN")]
    pub codex_bin: Option<PathBuf>,
    /// Use a separate, already-authenticated Codex config directory if desired.
    #[arg(long, env = "POCKET_CODEX_HOME")]
    pub codex_home: Option<PathBuf>,
    /// Omit to use your CLI's configured default model.
    #[arg(long, env = "POCKET_MODEL")]
    pub model: Option<String>,
    #[arg(long, env = "POCKET_MOCK", default_value_t = false)]
    pub mock: bool,
    /// Maximum duration of a turn, including image generation.
    #[arg(long, default_value_t = 900)]
    pub turn_timeout_secs: u64,
    /// Print the locally stored login key and exit. Never put it in a URL.
    #[arg(long)]
    pub show_key: bool,
}

impl Config {
    pub fn validate(&mut self) -> anyhow::Result<()> {
        if !self.bind.ip().is_loopback() {
            anyhow::bail!("bind must be a loopback address; expose the app through ngrok");
        }
        if self.turn_timeout_secs < 10 {
            anyhow::bail!("turn timeout must be at least 10 seconds");
        }
        if let Some(origin) = &mut self.public_origin {
            *origin = origin.trim_end_matches('/').to_owned();
            let authority = origin.strip_prefix("https://").or_else(|| {
                origin.strip_prefix("http://").filter(|_| {
                    origin.starts_with("http://localhost:")
                        || origin.starts_with("http://127.0.0.1:")
                        || origin.starts_with("http://[::1]:")
                })
            });
            if authority.is_none_or(|s| {
                s.is_empty() || s.chars().any(|c| c.is_whitespace() || "/?#@\\".contains(c))
            }) {
                anyhow::bail!("public origin must be an HTTPS origin without a path");
            }
        }
        Ok(())
    }

    pub fn executable(&self) -> PathBuf {
        if let Some(bin) = &self.codex_bin {
            return bin.clone();
        }
        // The Windows desktop app may install Codex without adding it to PATH.
        if cfg!(windows)
            && let Some(local) = std::env::var_os("LOCALAPPDATA")
        {
            let bin = PathBuf::from(local).join("Programs/OpenAI/Codex/bin/codex.exe");
            if bin.is_file() {
                return bin;
            }
        }
        PathBuf::from("codex")
    }
}
