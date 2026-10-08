use crate::model::{Chat, ChatSummary, JobStatus, Media, new_id, now, valid_id};
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use tokio::sync::Mutex;

pub const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;
pub const MAX_CHAT_BYTES: u64 = 8 * 1024 * 1024;

/// One instance owns the directory. JSON edits are serialized; chat bodies live on disk.
pub struct Store {
    pub root: PathBuf,
    lock: Mutex<()>,
    _process_lock: File,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root.join("chats"))?;
        fs::create_dir_all(root.join("workspaces"))?;
        let root = fs::canonicalize(root)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join(".server.lock"))?;
        file.try_lock_exclusive()
            .context("another server is already using this data directory")?;
        let store = Self {
            root,
            lock: Mutex::new(()),
            _process_lock: file,
        };
        // Never silently replay interrupted prompts on restart: they may spend usage.
        for id in store.ids()? {
            let mut chat = store.read_sync(&id)?;
            let mut changed = false;
            for job in &mut chat.jobs {
                if matches!(job.status, JobStatus::Queued | JobStatus::Running) {
                    job.status = JobStatus::Interrupted;
                    job.error =
                        Some("The laptop server stopped. Send a new message to continue.".into());
                    job.finished_at = Some(now());
                    changed = true;
                }
            }
            if changed {
                store.write_sync(&chat)?;
            }
        }
        Ok(store)
    }

    fn ids(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(self.root.join("chats"))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if valid_id(&name) && entry.file_type()?.is_dir() {
                ids.push(name);
            }
        }
        Ok(ids)
    }

    pub fn chat_dir(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            bail!("invalid chat id");
        }
        Ok(self.root.join("chats").join(id))
    }

    pub fn workspace(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            bail!("invalid chat id");
        }
        Ok(self.root.join("workspaces").join(id))
    }

    fn read_sync(&self, id: &str) -> Result<Chat> {
        let path = self.chat_dir(id)?.join("chat.json");
        if fs::metadata(&path)?.len() > MAX_CHAT_BYTES {
            bail!("chat file is too large");
        }
        let chat: Chat = serde_json::from_slice(&fs::read(&path)?).context("invalid chat JSON")?;
        if chat.version != 1 || chat.id != id {
            bail!("unsupported or mismatched chat file");
        }
        Ok(chat)
    }

    fn write_sync(&self, chat: &Chat) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(chat)?;
        if bytes.len() as u64 > MAX_CHAT_BYTES {
            bail!("chat is full; start a new conversation");
        }
        atomic_write(&self.chat_dir(&chat.id)?.join("chat.json"), &bytes)
    }

    pub async fn create(&self, title: String) -> Result<Chat> {
        let _guard = self.lock.lock().await;
        if self.ids()?.len() >= 500 {
            bail!("chat limit reached (500); archive old data locally");
        }
        let chat = Chat::new(title);
        fs::create_dir_all(self.chat_dir(&chat.id)?.join("media"))?;
        fs::create_dir_all(self.workspace(&chat.id)?)?;
        self.write_sync(&chat)?;
        Ok(chat)
    }

    pub async fn read(&self, id: &str) -> Result<Chat> {
        let _guard = self.lock.lock().await;
        self.read_sync(id)
    }

    pub async fn list(&self) -> Result<Vec<ChatSummary>> {
        let _guard = self.lock.lock().await;
        let mut summaries = self
            .ids()?
            .into_iter()
            .map(|id| self.read_sync(&id).map(|chat| ChatSummary::from(&chat)))
            .collect::<Result<Vec<_>>>()?;
        summaries.sort_by_key(|chat| std::cmp::Reverse(chat.updated_at));
        Ok(summaries)
    }

    /// Read → edit → atomic replace under the same lock prevents lost updates.
    pub async fn update<T>(
        &self,
        id: &str,
        edit: impl FnOnce(&mut Chat) -> Result<T>,
    ) -> Result<T> {
        let _guard = self.lock.lock().await;
        let mut chat = self.read_sync(id)?;
        let result = edit(&mut chat)?;
        chat.updated_at = now();
        self.write_sync(&chat)?;
        Ok(result)
    }

    pub async fn add_media(&self, chat_id: &str, bytes: &[u8], mut media: Media) -> Result<Media> {
        if bytes.len() > MAX_IMAGE_BYTES {
            bail!("image exceeds 12 MB");
        }
        let (mime, extension) = image_type(bytes).context("use a PNG, JPEG, or WebP image")?;
        media.mime = mime.into();
        media.bytes = bytes.len() as u64;
        media.filename = format!("{}.{}", media.id, extension);
        let _guard = self.lock.lock().await;
        let mut chat = self.read_sync(chat_id)?;
        if chat.media.len() >= 500 {
            bail!("image limit reached for this chat");
        }
        let path = self.chat_dir(chat_id)?.join("media").join(&media.filename);
        atomic_write(&path, bytes)?;
        chat.media.push(media.clone());
        chat.updated_at = now();
        if let Err(error) = self.write_sync(&chat) {
            let _ = fs::remove_file(path);
            return Err(error);
        }
        Ok(media)
    }

    pub async fn media_path(&self, chat_id: &str, media_id: &str) -> Result<(Media, PathBuf)> {
        if !valid_id(media_id) {
            bail!("invalid image id");
        }
        let chat = self.read(chat_id).await?;
        let media = chat
            .media
            .into_iter()
            .find(|m| m.id == media_id)
            .context("image not found")?;
        let directory = fs::canonicalize(self.chat_dir(chat_id)?.join("media"))?;
        let path = fs::canonicalize(directory.join(&media.filename))?;
        if !path.starts_with(directory) {
            bail!("image path is outside its directory");
        }
        Ok((media, path))
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("file has no parent directory")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub fn image_type(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some(("image/jpeg", "jpg"))
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else {
        None
    }
}

pub fn media_record(
    name: String,
    kind: &str,
    job_id: Option<String>,
    prompt: Option<String>,
) -> Media {
    Media {
        id: new_id(),
        name,
        filename: String::new(),
        mime: String::new(),
        bytes: 0,
        kind: kind.into(),
        created_at: now(),
        job_id,
        prompt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Job;

    #[tokio::test]
    async fn json_survives_restart_and_pending_jobs_are_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let chat = store.create("A poster".into()).await.unwrap();
        store
            .update(&chat.id, |c| {
                c.jobs.push(Job {
                    id: new_id(),
                    status: JobStatus::Running,
                    mode: "chat".into(),
                    message_id: new_id(),
                    created_at: now(),
                    finished_at: None,
                    error: None,
                });
                Ok(())
            })
            .await
            .unwrap();
        assert!(
            Store::open(dir.path()).is_err(),
            "two writers must not share a directory"
        );
        drop(store);
        let reopened = Store::open(dir.path()).unwrap();
        let recovered = reopened.read(&chat.id).await.unwrap();
        assert_eq!(recovered.jobs[0].status, JobStatus::Interrupted);
        assert!(!recovered.busy());
        assert!(reopened.chat_dir("../private").is_err());
    }

    #[tokio::test]
    async fn concurrent_edits_are_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(Store::open(dir.path()).unwrap());
        let chat = store.create("Start".into()).await.unwrap();
        let mut tasks = Vec::new();
        for _ in 0..10 {
            let store = store.clone();
            let id = chat.id.clone();
            tasks.push(tokio::spawn(async move {
                store
                    .update(&id, |chat| {
                        chat.title.push('!');
                        Ok(())
                    })
                    .await
                    .unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(store.read(&chat.id).await.unwrap().title, "Start!!!!!!!!!!");
    }
}
