use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use uuid::Uuid;

use tokio::fs as tfs;
use tokio::io::AsyncWriteExt;

use crate::backend::auth::Session;
use crate::backend::data::Storage;

pub type Shared<T> = Arc<RwLock<T>>;

pub struct AppState {
    pub storage: Shared<Storage>,
    pub sessions: Shared<HashMap<Uuid, Session>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        // Initialize storage WITHOUT any default admin.
        let storage = Storage::new();
        AppState {
            storage: Arc::new(RwLock::new(storage)),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn with_storage(storage: Storage) -> Self {
        AppState {
            storage: Arc::new(RwLock::new(storage)),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load state from a JSON file or create a new one if not present.
    /// If there are no admin accounts yet, a secure initial admin password
    /// is generated and written to a local file.
    pub fn load_or_new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref();
        let mut storage = if path.exists() {
            let data = fs::read_to_string(path)?;
            match serde_json::from_str::<Storage>(&data) {
                Ok(storage) => storage,
                Err(e) => {
                    eprintln!("Failed to parse state file '{}': {}. Falling back to new state.", path.display(), e);
                    Storage::new()
                }
            }
        } else {
            Storage::new()
        };

        // If this is the first startup (no admins exist), generate secure credentials.
        if storage.admins.is_empty() {
            if let Err(e) = Self::generate_initial_admin(&mut storage, path) {
                eprintln!("Failed to generate initial admin credentials: {}", e);
            }
        }

        Ok(AppState::with_storage(storage))
    }

    pub async fn save_to_async<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() { tfs::create_dir_all(parent).await?; }
        // Build JSON while holding read lock, then drop it before any await
        let json = {
            let storage = self.storage.read().expect("storage poisoned");
            serde_json::to_string_pretty(&*storage)?
        };
        // write atomically
        let tmp_path = path.with_extension("json.tmp");
        {
            let mut tmp = tfs::File::create(&tmp_path).await?;
            tmp.write_all(json.as_bytes()).await?;
            tmp.sync_all().await?;
        }
        tfs::rename(&tmp_path, path).await?;
        Ok(())
    }

    pub fn start_autosave_async<P: Into<PathBuf>>(&self, path: P, interval: Duration) -> tokio::task::JoinHandle<()> {
        let storage = self.storage.clone();
        let path: PathBuf = path.into();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                // Serialize under read lock, then drop guard before any await.
                let json_opt = {
                    if let Ok(guard) = storage.read() {
                        serde_json::to_string_pretty(&*guard).ok()
                    } else {
                        None
                    }
                };
                if let Some(json) = json_opt {
                    let tmp_path = path.with_extension("json.tmp");
                    if let Some(parent) = path.parent() { let _ = tfs::create_dir_all(parent).await; }
                    if let Ok(mut f) = tfs::File::create(&tmp_path).await {
                        let _ = f.write_all(json.as_bytes()).await;
                        let _ = f.sync_all().await;
                        let _ = tfs::rename(&tmp_path, &path).await;
                    }
                }
            }
        })
    }

    /// Generate a secure initial admin password, store its hash, persist storage,
    /// and only print the credentials to the console (no sidecar file is written).
    ///
    /// This function performs a one-time synchronous write using std::fs before Rocket/Tokio start.
    fn generate_initial_admin(state: &mut Storage, state_path: &Path) -> io::Result<()> {
        // Create a long random-looking password using two UUID v4 values (64 hex chars)
        let password = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let username = "admin";

        // Insert admin account with hashed password
        // Ignore existing admin silently (race-safe if called once at startup)
        let _ = state.add_admin(username, &password);

        // Serialize storage now; we will write it synchronously (no Tokio runtime involved)
        let json = serde_json::to_string_pretty(&state)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Persist the updated storage immediately to avoid losing credentials, atomically.
        if let Some(parent) = state_path.parent() { fs::create_dir_all(parent)?; }
        let tmp_path = state_path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut tmp = fs::File::create(&tmp_path)?;
            tmp.write_all(json.as_bytes())?;
            tmp.sync_all()?;
        }
        fs::rename(&tmp_path, state_path)?;

        // Only print to stderr as a one-time notice (no sidecar file)
        eprintln!(
            "Initial admin credentials generated.\nUsername: {}\nPassword: {}\nNOTE: Store this password securely, it will not be printed again.\n",
            username,
            password
        );

        Ok(())
    }
}
