//! The app's shared state, handed to every command.

use crate::node::Node;
use crate::trades::Trades;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use zeroize::Zeroizing;

static APP_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The app's data folder (set once at start).
pub fn app_dir() -> &'static Path {
    APP_DIR.get().map(|p| p.as_path()).unwrap_or(Path::new("."))
}

pub fn set_app_dir(p: PathBuf) {
    let _ = APP_DIR.set(p);
}

/// New recovery words waiting for the user to write them down and give three back.
pub struct NewWords {
    pub words: Zeroizing<String>,
    pub ask: Vec<usize>,
    pub made: std::time::Instant,
}

pub struct AppState {
    pub dir: PathBuf,
    pub node: Arc<Node>,
    pub trades: Arc<Trades>,
    pub new_words: Mutex<Option<NewWords>>,
    pub phone: Arc<crate::phone::Phone>,
}

pub type St<'a> = tauri::State<'a, Arc<AppState>>;
