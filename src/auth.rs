use crate::model::new_id;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::Path,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;

const SESSION_LIFE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub const COOKIE: &str = "pocket_session";

pub struct Auth {
    key_hash: [u8; 32],
    sessions: Mutex<HashMap<[u8; 32], Instant>>,
    failures: Mutex<Vec<Instant>>,
}

fn digest(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

pub fn load_key(root: &Path) -> Result<String> {
    let path = root.join(".access-key");
    match fs::read_to_string(&path) {
        Ok(key) => {
            let key = key.trim().to_owned();
            if key.len() < 32 {
                anyhow::bail!("access key is too short; remove .access-key to regenerate it");
            }
            Ok(key)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            use std::io::Write;
            let key = format!("{}{}", new_id().replace('-', ""), new_id().replace('-', ""));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(path)
                .context("could not create local access key")?;
            file.write_all(key.as_bytes())?;
            file.sync_all()?;
            Ok(key)
        }
        Err(error) => Err(error.into()),
    }
}

impl Auth {
    pub fn new(key: &str) -> Self {
        Self {
            key_hash: digest(key),
            sessions: Mutex::new(HashMap::new()),
            failures: Mutex::new(Vec::new()),
        }
    }

    pub async fn login(&self, key: &str) -> Result<String, &'static str> {
        let mut failures = self.failures.lock().await;
        failures.retain(|time| time.elapsed() < Duration::from_secs(60));
        if failures.len() >= 10 {
            return Err("too many attempts; wait one minute");
        }
        if key.len() > 256 || !bool::from(self.key_hash.ct_eq(&digest(key))) {
            failures.push(Instant::now());
            return Err("incorrect access key");
        }
        failures.clear();
        let token = format!("{}{}", new_id().replace('-', ""), new_id().replace('-', ""));
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, expires| *expires > Instant::now());
        if sessions.len() >= 8
            && let Some(oldest) = sessions.iter().min_by_key(|(_, t)| *t).map(|(k, _)| *k)
        {
            sessions.remove(&oldest);
        }
        sessions.insert(digest(&token), Instant::now() + SESSION_LIFE);
        Ok(token)
    }

    pub async fn valid(&self, token: &str) -> bool {
        if token.len() != 64 {
            return false;
        }
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, expires| *expires > Instant::now());
        sessions.contains_key(&digest(token))
    }

    pub async fn logout(&self, token: &str) {
        self.sessions.lock().await.remove(&digest(token));
    }
}

pub fn cookie_token(header: &str) -> Option<&str> {
    header.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == COOKIE).then_some(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tokens_expire_on_logout_and_login_is_rate_limited() {
        let auth = Auth::new("a sufficiently long test access key");
        assert!(auth.login("wrong").await.is_err());
        let token = auth
            .login("a sufficiently long test access key")
            .await
            .unwrap();
        assert!(auth.valid(&token).await);
        auth.logout(&token).await;
        assert!(!auth.valid(&token).await);
        for _ in 0..10 {
            let _ = auth.login("wrong").await;
        }
        assert_eq!(
            auth.login("wrong").await.unwrap_err(),
            "too many attempts; wait one minute"
        );
        assert_eq!(cookie_token("other=abc; pocket_session=xyz"), Some("xyz"));
    }
}
