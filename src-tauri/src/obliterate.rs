//! "Obliterate": remove what this app put on this computer, in two parts the user ticks. **Truthcoin**: the node
//! program the app downloaded, the node's data folder (the chain, and the wallet with its seed) and the wallet's records
//! (trades, withdrawals). **The app**: its settings, activity log and phone link, what its window stored, and, where it
//! can, its own program when it closes. The list is worked out here, and `run` acts only when a fresh list matches the
//! one the screen showed: never on paths from the screen. Never on the list: BitWindow, eCash's node and the enforcer,
//! and a node program chosen under Advanced.
//!
//! What the running window still uses (its caches, and on Linux the app's folder) and the running program go when the
//! app closes (`wipe_at_exit`). Once the app part has gone, nothing is written to disk again (`files::stop_writing`).

use crate::state::St;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// Truthcoin's entries in the app's folder: the node program (`bin/`), the node's data (`node/`), the node's pid file,
/// and the wallet's records.
const TRUTHCOIN_ENTRIES: &[&str] =
    &["bin", "node", "node.json", "wallet-ready", "trades.json", "withdrawals.json", "deposits.json"];
/// The app's own entries: settings, the activity log, the phone link (this computer's key and the paired phones), and
/// the lock that keeps a second copy from starting.
const APP_ENTRIES: &[&str] = &["settings.json", "activity.log", "phone", "lock"];

/// What WebKitGTK keeps in a folder: its cache, storage and HSTS list. A folder named after the program (not the app's
/// identifier) is listed only if it holds nothing else, and these alone are removed from the app's folder on Linux when
/// Truthcoin stays.
const WEBKIT_NAMES: &[&str] = &[
    "WebKitCache", "CacheStorage", "hsts-storage.sqlite", "hsts-storage.sqlite-journal", "hsts-storage.sqlite-wal",
    "hsts-storage.sqlite-shm", "mediakeys", "storage", "databases", "localstorage", "indexeddb", "serviceworkers",
    "cookies", "deviceidhashsalts", "itp",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Part {
    Truthcoin,
    App,
}

/// A folder or file the window (the system webview) may have written.
#[derive(Debug, Clone)]
pub struct Cache {
    pub id: &'static str,
    pub path: PathBuf,
    /// Named after the program, not the app's identifier: listed only when it holds nothing but WebKit's files.
    pub program_named: bool,
}

/// Everywhere the plan looks. The app fills this from Tauri's paths; tests pass temporary folders.
#[derive(Debug, Clone)]
pub struct Places {
    pub home: PathBuf,
    pub app_dir: PathBuf,
    /// Set with TRUTHCOIN_APP_DIR (developers, tests): only the app's own entries in it go, never the folder.
    pub app_dir_from_env: bool,
    /// WebKitGTK keeps the window's data in the app's folder (Linux), so that folder can only go when the app closes.
    pub screen_uses_app_dir: bool,
    pub caches: Vec<Cache>,
    /// The app's own program, when it can be removed as the app closes (a Mac's .app, an AppImage).
    pub program: Option<PathBuf>,
}

/// One line of the list.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Item {
    /// "node-program", "node-data", "wallet-records", "app-files", "cache:<which>" or "program".
    pub id: String,
    pub part: Part,
    pub label: String,
    pub path: String,
    /// Bytes, links not followed.
    pub size: u64,
    pub note: String,
    /// Goes when the app closes.
    pub at_exit: bool,
}

/// A line as the screen showed it.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Shown {
    pub id: String,
    pub path: String,
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Bytes on disk under `path`, never following links (a link counts as nothing). On disk, not the files' lengths: the
/// node's databases are sparse files hundreds of gigabytes long that use a few megabytes.
fn size_of(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if !meta.is_dir() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            return meta.blocks() * 512;
        }
        #[cfg(not(unix))]
        return meta.len();
    }
    std::fs::read_dir(path).into_iter().flatten().filter_map(|e| e.ok()).map(|e| size_of(&e.path())).sum()
}

/// Where `path` really is: its folder resolved, the last part not followed, so a link stays a link.
fn real_location(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?;
    let parent = path.parent()?.canonicalize().ok()?;
    Some(parent.join(name))
}

/// The guard every deletion passes: never a relative path, "/", the home folder or anything above it, nor a link that
/// leads there. Returns where the entry really is (see `real_location`).
pub fn check_deletable(path: &Path, home: &Path) -> Result<PathBuf, String> {
    let refuse = |why: &str| Err(format!("The app never deletes {}: {}.", path.display(), why));
    if !path.is_absolute() {
        return refuse("it isn't a full path");
    }
    let Some(full) = real_location(path) else {
        return refuse("it can't tell where that is");
    };
    let Ok(home_real) = home.canonicalize() else {
        return refuse("it can't find your home folder");
    };
    let above_home = |p: &Path| home_real.starts_with(p) || home.starts_with(p);
    if above_home(&full) || above_home(path) || full.canonicalize().is_ok_and(|c| above_home(&c)) {
        return refuse("it holds your home folder");
    }
    Ok(full)
}

/// Delete one entry: a folder with everything in it, or a file or link (never followed). False when it was gone.
fn remove_item(path: &Path, home: &Path) -> Result<bool, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("Couldn't look at {}: {}", path.display(), e)),
    };
    let full = check_deletable(path, home)?;
    // symlink_metadata never calls a link a folder, so a link is removed as a file: only the link.
    if meta.is_dir() { std::fs::remove_dir_all(&full) } else { std::fs::remove_file(&full) }
        .map_err(|e| format!("Couldn't remove {}: {}", full.display(), e))?;
    Ok(true)
}

/// The part an entry of the app's folder belongs to, if it is the app's at all: one of the names above, or one the
/// app writes beside a record (`<stem>.tmp` while writing it, `<stem>.bad-<time>` and `<stem>.seen-<time>` for one that
/// couldn't be read).
pub fn part_of(name: &str) -> Option<Part> {
    let ours = |entries: &[&str]| {
        entries.iter().any(|e| {
            let stem = e.split_once('.').map_or(*e, |(s, _)| s);
            *e == name
                || name
                    .strip_prefix(stem)
                    .and_then(|r| r.strip_prefix('.'))
                    .is_some_and(|r| r == "tmp" || r.starts_with("bad-") || r.starts_with("seen-"))
        })
    };
    if ours(TRUTHCOIN_ENTRIES) {
        Some(Part::Truthcoin)
    } else if ours(APP_ENTRIES) {
        Some(Part::App)
    } else {
        None
    }
}

/// The entries of the app's folder in `part`, except the node's two folders (listed apart).
fn entries(app_dir: &Path, part: Part) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(app_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n != "bin" && n != "node" && part_of(&n) == Some(part)
        })
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

/// A bin/ that holds only what the installer writes: truthcoin_dc-<version> and its download.
fn only_node_programs(bin: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(bin) else {
        return false;
    };
    rd.filter_map(|e| e.ok()).all(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        n.starts_with("truthcoin_dc-") || n == ".download"
    })
}

/// A folder named after the program that holds nothing but WebKit's files.
fn only_webkit(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    rd.filter_map(|e| e.ok()).all(|e| WEBKIT_NAMES.contains(&e.file_name().to_string_lossy().as_ref()))
}

fn cache_label(id: &str) -> &'static str {
    match id {
        "cache" => "The window's cache",
        "local-data" => "The window's stored data",
        "webkit-cache" => "The window's web cache",
        "webkit-data" => "The window's site settings",
        "webkit" => "The window's web storage",
        "caches" => "The window's caches",
        "http-storage" => "The window's network storage",
        "cookies" => "The window's cookies",
        "saved-state" => "The window's saved position",
        _ => "What the window stored",
    }
}

fn item(id: &str, part: Part, label: &str, path: &Path, note: String, at_exit: bool) -> Item {
    Item { id: id.into(), part, label: label.into(), path: path.display().to_string(), size: size_of(path), note, at_exit }
}

fn names(paths: &[PathBuf]) -> String {
    paths.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ")
}

/// Everything this app put on this computer that still exists, one line each.
pub fn plan_items(p: &Places) -> Result<Vec<Item>, String> {
    check_deletable(&p.app_dir, &p.home).map_err(|e| format!("{e} Remove what you want by hand."))?;
    let mut items = Vec::new();
    let bin = p.app_dir.join("bin");
    // A folder set with TRUTHCOIN_APP_DIR may be anything (~/.local, say): its bin/ and node/ are listed only when they
    // hold what this app puts there (security review L1).
    if exists(&bin) && (!p.app_dir_from_env || only_node_programs(&bin)) {
        let note = format!(
            "truthcoin_dc, downloaded from L2L's GitHub release (this app runs {}).",
            crate::node::pins::NODE_VERSION
        );
        items.push(item("node-program", Part::Truthcoin, "The Truthcoin node program", &bin, note, false));
    }
    let node = p.app_dir.join("node");
    if exists(&node) && (!p.app_dir_from_env || node.join("app-node.log").is_file()) {
        let note = "The chain it downloaded, and your Truthcoin wallet with its seed.".to_string();
        items.push(item("node-data", Part::Truthcoin, "The Truthcoin node's data", &node, note, false));
    }
    // The node's pid file comes and goes as the node starts and stops, so it never makes a line of its own (it goes
    // with the rest): stopping the node mustn't change the list between the screen and the deletion (re-review N1).
    let records: Vec<PathBuf> = entries(&p.app_dir, Part::Truthcoin)
        .into_iter()
        .filter(|f| !f.file_name().is_some_and(|n| n.to_string_lossy().split('.').next() == Some("node")))
        .collect();
    if !records.is_empty() {
        let mut it = item(
            "wallet-records",
            Part::Truthcoin,
            "The wallet's records",
            &p.app_dir,
            format!("What the app recorded about this wallet: {}.", names(&records)),
            false,
        );
        it.size = records.iter().map(|r| size_of(r)).sum();
        items.push(it);
    }
    let own = entries(&p.app_dir, Part::App);
    let mut note = if own.is_empty() {
        String::from("Nothing of the app's own is left in it.")
    } else {
        format!("Settings, the activity log and the phone link ({}). Paired phones stop working.", names(&own))
    };
    if p.app_dir_from_env {
        note.push_str(" This folder was set with TRUTHCOIN_APP_DIR, so it stays: only the app's own files in it go.");
    } else if p.screen_uses_app_dir {
        note.push_str(" What the window stored in this folder goes when the app closes.");
    }
    let mut it = item("app-files", Part::App, "The app's settings and phone link", &p.app_dir, note, false);
    it.size = own.iter().map(|r| size_of(r)).sum();
    items.push(it);
    let mut seen: Vec<PathBuf> = vec![p.app_dir.clone()];
    for c in &p.caches {
        // Inside something already listed, or holding it (then it isn't a cache): left out.
        let overlaps = seen.iter().any(|s| c.path.starts_with(s) || s.starts_with(&c.path));
        if !exists(&c.path) || overlaps || (c.program_named && !only_webkit(&c.path)) || check_deletable(&c.path, &p.home).is_err() {
            continue;
        }
        seen.push(c.path.clone());
        let note = "What the app's window stored. It goes when the app closes.".to_string();
        items.push(item(&format!("cache:{}", c.id), Part::App, cache_label(c.id), &c.path, note, true));
    }
    if let Some(prog) = &p.program {
        if exists(prog) && check_deletable(prog, &p.home).is_ok() {
            let note = "The app itself. It goes when the app closes.".to_string();
            items.push(item("program", Part::App, "The Truthcoin App program", prog, note, true));
        }
    }
    Ok(items)
}

/// What `execute` did.
#[derive(Debug, Default)]
pub struct Done {
    pub removed: Vec<PathBuf>,
    pub at_exit: AtExit,
    /// What couldn't be removed, and why.
    pub errors: Vec<String>,
}

/// What waits for the app to close.
#[derive(Debug, Default, Clone)]
pub struct AtExit {
    pub home: PathBuf,
    pub paths: Vec<PathBuf>,
    /// Of `paths`, the folders named after the program: removed only if they still hold nothing but WebKit's files.
    pub webkit_only: Vec<PathBuf>,
    /// The app's folder, once the app part has gone: its lock (held until the app exits, so no second copy can start
    /// on a folder that is going; security review M1), WebKit's entries when the window keeps its data there, and
    /// then the folder itself if nothing else is left in it.
    pub app_dir: Option<AppDirAtExit>,
}

#[derive(Debug, Clone)]
pub struct AppDirAtExit {
    pub path: PathBuf,
    pub webkit: bool,
    pub remove_if_empty: bool,
}

/// The lines whose path is fixed by what they are: these must be exactly the lines the screen showed. (Caches the
/// window creates while the list is open are added as they come; they go at exit only.)
fn fixed(items: &[Item]) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> =
        items.iter().filter(|i| !i.id.starts_with("cache:")).map(|i| (i.id.clone(), i.path.clone())).collect();
    v.sort();
    v
}

/// Check the screen's list against a fresh one. Nothing is deleted.
pub fn check(p: &Places, parts: &[Part], shown: &[Shown]) -> Result<Vec<Item>, String> {
    if parts.is_empty() {
        return Err("Nothing is ticked, so nothing was removed.".into());
    }
    let plan = plan_items(p)?;
    let mut showed: Vec<(String, String)> =
        shown.iter().filter(|s| !s.id.starts_with("cache:")).map(|s| (s.id.clone(), s.path.clone())).collect();
    showed.sort();
    if fixed(&plan) != showed || shown.iter().any(|s| s.id.starts_with("cache:") && !plan.iter().any(|i| i.id == s.id && i.path == s.path)) {
        return Err("The list has changed since it was shown, so nothing was removed. Please look at it again.".into());
    }
    Ok(plan)
}

/// Delete the ticked parts of a fresh plan that matches the screen's (the command does the same in two steps, with the
/// node stopped between them).
#[cfg(test)]
pub fn execute(p: &Places, parts: &[Part], shown: &[Shown]) -> Result<Done, String> {
    let plan = check(p, parts, shown)?;
    Ok(delete(p, parts, &plan))
}

/// Delete the ticked parts of `plan`, which `check` has just returned.
fn delete(p: &Places, parts: &[Part], plan: &[Item]) -> Done {
    let truthcoin = parts.contains(&Part::Truthcoin);
    let app = parts.contains(&Part::App);
    let mut done = Done { at_exit: AtExit { home: p.home.clone(), ..Default::default() }, ..Default::default() };
    let gone = |path: &Path, done: &mut Done| match remove_item(path, &p.home) {
        Ok(true) => done.removed.push(path.to_path_buf()),
        Ok(false) => {}
        Err(e) => done.errors.push(e),
    };
    if truthcoin {
        // The wallet's data first, then its records (and the node's pid file), then the program.
        if let Some(it) = plan.iter().find(|i| i.id == "node-data") {
            gone(Path::new(&it.path), &mut done);
        }
        for f in entries(&p.app_dir, Part::Truthcoin) {
            gone(&f, &mut done);
        }
        if let Some(it) = plan.iter().find(|i| i.id == "node-program") {
            gone(Path::new(&it.path), &mut done);
        }
    }
    if app {
        for f in entries(&p.app_dir, Part::App) {
            if f.file_name().is_some_and(|n| n == "lock") {
                continue;
            }
            gone(&f, &mut done);
        }
        done.at_exit.app_dir = Some(AppDirAtExit {
            path: p.app_dir.clone(),
            webkit: p.screen_uses_app_dir && !p.app_dir_from_env,
            remove_if_empty: !p.app_dir_from_env,
        });
        for it in plan.iter().filter(|i| i.at_exit) {
            let path = PathBuf::from(&it.path);
            if p.caches.iter().any(|c| c.program_named && it.id == format!("cache:{}", c.id)) {
                done.at_exit.webkit_only.push(path.clone());
            }
            done.at_exit.paths.push(path);
        }
    }
    done
}

/// Delete what waited for the app to close, through the same guard. Returns what couldn't be removed.
pub fn wipe(later: &AtExit) -> Vec<String> {
    let mut errors = Vec::new();
    for p in &later.paths {
        // A folder named after the program may have become another program's meanwhile: looked at again.
        if later.webkit_only.contains(p) && exists(p) && !only_webkit(p) {
            errors.push(format!("Left {}: something other than the window's files is in it now.", p.display()));
            continue;
        }
        if let Err(e) = remove_item(p, &later.home) {
            errors.push(e);
        }
    }
    if let Some(a) = &later.app_dir {
        let names: &[&str] = if a.webkit { WEBKIT_NAMES } else { &[] };
        // The lock, and the Finder's folder notes (a Mac): neither is reason to keep the folder.
        for name in names.iter().chain(["lock", ".DS_Store"].iter()) {
            if let Err(e) = remove_item(&a.path.join(name), &later.home) {
                errors.push(e);
            }
        }
        let empty = std::fs::read_dir(&a.path).is_ok_and(|mut d| d.next().is_none());
        if a.remove_if_empty && empty && check_deletable(&a.path, &later.home).is_ok() {
            if let Err(e) = std::fs::remove_dir(&a.path) {
                errors.push(format!("Couldn't remove {}: {}", a.path.display(), e));
            }
        }
    }
    errors
}

static AT_EXIT: Mutex<Option<AtExit>> = Mutex::new(None);

/// Called as the app exits, after the node has stopped.
pub fn wipe_at_exit() {
    let later = AT_EXIT.lock().unwrap().take();
    if let Some(later) = later {
        for e in wipe(&later) {
            eprintln!("{e}");
        }
    }
}

/// The string after `<key>KEY</key>` in an XML property list (Tauri writes Info.plist as XML).
fn plist_string<'a>(plist: &'a str, key: &str) -> Option<&'a str> {
    let at = plist.find(&format!("<key>{key}</key>"))? + key.len() + "<key></key>".len();
    let rest = plist[at..].trim_start().strip_prefix("<string>")?;
    rest.split_once("</string>").map(|(v, _)| v)
}

/// Can this user write in `dir` (so a file in it can be removed)?
fn writable(dir: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else { return false };
        // SAFETY: access() reads a NUL-terminated path we own.
        unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        false
    }
}

/// How the app itself can go. "mac": the .app (removed as the app closes when `path` is set; otherwise drag it to the
/// Trash). "appimage": the AppImage file at `path`, likewise. "deb": `sudo apt remove truthcoin-app`. "other": the
/// program at `path`, by hand.
#[derive(Debug, Clone, Serialize)]
pub struct RemoveApp {
    pub kind: &'static str,
    pub path: Option<String>,
    /// The app removes it as it closes.
    pub at_exit: bool,
}

/// The app's own program, and whether the app can remove it as it closes. Only a program that is certainly this app:
/// on a Mac the .app it runs from, with this app's identifier, not run from a read-only copy macOS made of a download
/// (App Translocation); on Linux the AppImage it runs from (APPIMAGE is inherited by any program an AppImage starts, so
/// this program must run inside APPDIR too). Its folder must be writable.
pub fn own_program(identifier: &str) -> RemoveApp {
    let exe = std::env::current_exe().ok();
    let show = |p: &Path| Some(p.display().to_string());
    if cfg!(target_os = "macos") {
        let bundle = exe.as_deref().and_then(|e| e.ancestors().find(|a| a.extension().is_some_and(|x| x == "app")));
        let ok = bundle.is_some_and(|b| {
            let ours = std::fs::read_to_string(b.join("Contents/Info.plist"))
                .is_ok_and(|t| plist_string(&t, "CFBundleIdentifier") == Some(identifier));
            ours && !b.to_string_lossy().contains("/AppTranslocation/") && b.parent().is_some_and(writable)
        });
        return RemoveApp { kind: "mac", path: bundle.and_then(show), at_exit: ok };
    }
    if let (Some(image), Some(appdir)) = (std::env::var_os("APPIMAGE"), std::env::var_os("APPDIR")) {
        let image = PathBuf::from(image);
        let inside = exe.as_deref().is_some_and(|e| e.starts_with(&appdir));
        let named = image.file_name().is_some_and(|n| {
            let n = n.to_string_lossy().to_lowercase();
            n.contains("truthcoin") && n.ends_with(".appimage")
        });
        let file = std::fs::symlink_metadata(&image).is_ok_and(|m| m.is_file());
        if inside {
            let ok = named && file && image.parent().is_some_and(writable);
            return RemoveApp { kind: "appimage", path: show(&image), at_exit: ok };
        }
    }
    match exe {
        Some(e) if e.starts_with("/usr") => RemoveApp { kind: "deb", path: None, at_exit: false },
        e => RemoveApp { kind: "other", path: e.as_deref().and_then(show), at_exit: false },
    }
}

/// The folders the window (the system webview) writes to. With TRUTHCOIN_APP_DIR a developer's copy shares them with
/// the installed app, so then they are left alone.
fn screen_caches(app: &AppHandle, from_env: bool) -> Vec<Cache> {
    if from_env {
        return Vec::new();
    }
    let path = app.path();
    let named = |id, p: Option<PathBuf>| p.map(|path| Cache { id, path, program_named: false });
    let mut caches = vec![named("cache", path.app_cache_dir().ok()), named("local-data", path.app_local_data_dir().ok())];
    // WebKitGTK names its cache and HSTS list after the program: ~/.cache/truthcoin-app, ~/.local/share/truthcoin-app.
    #[cfg(target_os = "linux")]
    if let Some(program) = std::env::current_exe().ok().and_then(|e| e.file_name().map(|n| n.to_owned())) {
        for (id, dir) in [("webkit-cache", path.cache_dir().ok()), ("webkit-data", path.data_dir().ok())] {
            caches.push(dir.map(|d| Cache { id, path: d.join(&program), program_named: true }));
        }
    }
    #[cfg(target_os = "macos")]
    if let Ok(home) = path.home_dir() {
        let id = &app.config().identifier;
        let lib = home.join("Library");
        caches.extend([
            named("webkit", Some(lib.join("WebKit").join(id))),
            named("caches", Some(lib.join("Caches").join(id))),
            named("http-storage", Some(lib.join("HTTPStorages").join(id))),
            named("cookies", Some(lib.join("HTTPStorages").join(format!("{id}.binarycookies")))),
            named("saved-state", Some(lib.join("Saved Application State").join(format!("{id}.savedState")))),
        ]);
    }
    caches.into_iter().flatten().collect()
}

fn places(app: &AppHandle, dir: &Path) -> Result<Places, String> {
    let path = app.path();
    let from_env = std::env::var_os("TRUTHCOIN_APP_DIR").is_some();
    let prog = own_program(&app.config().identifier);
    // Without the home folder the guard can't tell what holds it: nothing is planned (security review L2).
    let home = path.home_dir().map_err(|_| "The app can't find your home folder, so it removes nothing.".to_string())?;
    Ok(Places {
        home,
        app_dir: dir.to_path_buf(),
        app_dir_from_env: from_env,
        // WebKitGTK keeps the window's data (localstorage/, storage/) in the app's own folder.
        screen_uses_app_dir: cfg!(target_os = "linux") && path.app_local_data_dir().ok().as_deref() == Some(dir),
        caches: screen_caches(app, from_env),
        program: if prog.at_exit { prog.path.map(PathBuf::from) } else { None },
    })
}

/// The list the screen shows.
#[derive(Debug, Serialize)]
pub struct Plan {
    pub items: Vec<Item>,
    /// Why Obliterate can't run now.
    pub blocked: Option<String>,
    pub remove_app: RemoveApp,
    /// A node program chosen under Advanced: never removed.
    pub own_node_program: Option<String>,
}

fn blocked(st: &crate::state::AppState) -> Option<String> {
    if crate::files::stopped() {
        return Some("The app's files have been removed. Close the app.".into());
    }
    if st.node.install.lock().unwrap().running {
        return Some("The Truthcoin node is being downloaded. Let it finish first.".into());
    }
    None
}

#[tauri::command]
pub async fn obliterate_plan(app: AppHandle, st: St<'_>) -> Result<Plan, String> {
    let p = places(&app, &st.dir)?;
    let items = tauri::async_runtime::spawn_blocking(move || plan_items(&p)).await.map_err(|e| e.to_string())??;
    Ok(Plan {
        items,
        blocked: blocked(&st),
        remove_app: own_program(&app.config().identifier),
        own_node_program: st.node.settings().node_binary.map(|p| p.display().to_string()),
    })
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub removed: Vec<String>,
    /// Removed when the app closes.
    pub at_exit: Vec<String>,
    pub errors: Vec<String>,
    /// The app part went: the app must close.
    pub app_removed: bool,
    pub remove_app: RemoveApp,
}

/// Clears the node's "removing" flag when Obliterate ends, unless the app itself went (then nothing starts again).
struct Removing<'a> {
    node: &'a crate::node::Node,
    keep: bool,
}

impl Drop for Removing<'_> {
    fn drop(&mut self) {
        if !self.keep {
            self.node.removing.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// "Obliterate": check the list, stop what uses it, delete the ticked parts, and leave for the app's exit what the
/// window and the running program still use. `words`: the user ticked that they have the wallet's recovery words,
/// required whenever the wallet goes (security review H1: what a wallet holds can't be read reliably enough to excuse it).
#[tauri::command]
pub async fn obliterate(
    app: AppHandle,
    st: St<'_>,
    truthcoin: bool,
    the_app: bool,
    words: bool,
    shown: Vec<Shown>,
) -> Result<Outcome, String> {
    if let Some(why) = blocked(&st) {
        return Err(format!("{why} Nothing was removed."));
    }
    // From here no node starts and no install begins (security review M2). The install flag is read under its lock,
    // as node_install sets it.
    st.node.removing.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut guard = Removing { node: &st.node, keep: false };
    if st.node.install.lock().unwrap().running {
        return Err("The Truthcoin node is being downloaded. Let it finish first. Nothing was removed.".into());
    }
    let parts: Vec<Part> = [(truthcoin, Part::Truthcoin), (the_app, Part::App)]
        .into_iter()
        .filter_map(|(on, p)| on.then_some(p))
        .collect();
    let p = places(&app, &st.dir)?;
    // Checked before anything stops, then again as it is deleted.
    let (p2, parts2, shown2) = (p.clone(), parts.clone(), shown.clone());
    let plan = tauri::async_runtime::spawn_blocking(move || check(&p2, &parts2, &shown2))
        .await
        .map_err(|e| e.to_string())??;
    if truthcoin && !words && plan.iter().any(|i| i.id == "node-data") {
        return Err("Tick that you have this wallet's recovery words first. Nothing was removed.".into());
    }
    crate::activity::note(
        &st.dir,
        &format!("Obliterate: removing {}", if truthcoin && the_app { "Truthcoin and the app" } else if truthcoin { "Truthcoin" } else { "the app" }),
    );
    st.node.stop().await;
    // A node a crash left running on this folder (macOS has no parent-death signal) is stopped too (review L3).
    crate::node::stop_leftover_in(&st.dir).await;
    // Checked again now nothing runs on it; only then does writing stop and the deleting start (re-review N1).
    let (p2, parts2, shown2) = (p.clone(), parts.clone(), shown.clone());
    let plan = tauri::async_runtime::spawn_blocking(move || check(&p2, &parts2, &shown2))
        .await
        .map_err(|e| e.to_string())??;
    if the_app {
        // Nothing is written to disk from here, so the phones' link can't come back (review L5); then it ends.
        guard.keep = true;
        crate::files::stop_writing();
        st.phone.shutdown();
    }
    let done = tauri::async_runtime::spawn_blocking(move || delete(&p, &parts, &plan))
        .await
        .map_err(|e| e.to_string())?;
    if truthcoin {
        st.trades.forget_all();
        st.phone.forget_wallet();
        *st.new_words.lock().unwrap() = None;
    }
    if truthcoin && !the_app {
        crate::activity::note(&st.dir, "Obliterate: Truthcoin removed (the node program, its data and the wallet)");
    }
    let strings = |v: &[PathBuf]| v.iter().map(|p| p.display().to_string()).collect::<Vec<_>>();
    let mut at_exit = strings(&done.at_exit.paths);
    if let Some(a) = &done.at_exit.app_dir {
        if a.webkit {
            at_exit.push(format!("What the window stored in {}", a.path.display()));
        }
        if a.remove_if_empty && truthcoin && done.errors.is_empty() {
            at_exit.push(a.path.display().to_string());
        }
    }
    let out = Outcome {
        removed: strings(&done.removed),
        at_exit,
        errors: done.errors.clone(),
        app_removed: the_app,
        remove_app: own_program(&app.config().identifier),
    };
    *AT_EXIT.lock().unwrap() = Some(done.at_exit);
    drop(guard);
    Ok(out)
}

/// "Close the app", on the last screen: exiting stops the node and removes what waited for it.
#[tauri::command]
pub fn app_close(app: AppHandle) {
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct T {
        _d: tempfile::TempDir,
        home: PathBuf,
        app: PathBuf,
    }

    /// A home folder with an app folder holding everything the app writes, and some things it doesn't.
    fn setup() -> T {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().canonicalize().unwrap().join("home");
        let app = home.join(".local/share/dev.truthcoinapp.desktop");
        for f in ["bin/truthcoin_dc-0.19.0", "node/wallet.mdb/data.mdb", "node/app-node.log", "phone/keys.json"] {
            std::fs::create_dir_all(app.join(f).parent().unwrap()).unwrap();
            std::fs::write(app.join(f), b"x").unwrap();
        }
        for f in [
            "settings.json", "activity.log", "lock", "trades.json", "trades.bad-17", "withdrawals.json", "wallet-ready",
            "node.json", "settings.tmp", "localstorage",
        ] {
            std::fs::write(app.join(f), b"x").unwrap();
        }
        T { _d: d, home, app }
    }

    fn places(t: &T, screen: bool, env: bool) -> Places {
        Places {
            home: t.home.clone(),
            app_dir: t.app.clone(),
            app_dir_from_env: env,
            screen_uses_app_dir: screen,
            caches: Vec::new(),
            program: None,
        }
    }

    fn shown(items: &[Item]) -> Vec<Shown> {
        items.iter().map(|i| Shown { id: i.id.clone(), path: i.path.clone() }).collect()
    }

    #[test]
    fn entries_are_sorted_into_their_parts() {
        for n in ["bin", "node", "node.json", "node.tmp", "trades.json", "trades.tmp", "trades.bad-1", "trades.seen-2", "wallet-ready", "withdrawals.json", "deposits.json"] {
            assert_eq!(part_of(n), Some(Part::Truthcoin), "{n}");
        }
        for n in ["settings.json", "settings.tmp", "activity.log", "activity.tmp", "phone", "lock"] {
            assert_eq!(part_of(n), Some(Part::App), "{n}");
        }
        for n in ["localstorage", "notes.txt", "trades", "settings.json.bak", "nodes", "trades.jsonx", "binx"] {
            assert_eq!(part_of(n), None, "{n}");
        }
    }

    #[test]
    fn both_parts_take_the_whole_folder() {
        let t = setup();
        // A Mac: the window keeps nothing in the app's folder.
        std::fs::remove_file(t.app.join("localstorage")).unwrap();
        let p = places(&t, false, false);
        let plan = plan_items(&p).unwrap();
        let ids: Vec<&str> = plan.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["node-program", "node-data", "wallet-records", "app-files"]);
        let done = execute(&p, &[Part::Truthcoin, Part::App], &shown(&plan)).unwrap();
        assert!(done.errors.is_empty(), "{:?}", done.errors);
        let left: Vec<String> =
            std::fs::read_dir(&t.app).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["lock"], "the lock stays until the app exits, so no second copy starts");
        assert!(wipe(&done.at_exit).is_empty());
        assert!(!t.app.exists());
        assert!(t.home.exists());
    }

    #[test]
    fn the_nodes_pid_file_never_changes_the_list() {
        let t = setup();
        for f in ["trades.json", "trades.bad-17", "withdrawals.json", "wallet-ready"] {
            std::fs::remove_file(t.app.join(f)).unwrap();
        }
        let p = places(&t, false, false);
        let plan = plan_items(&p).unwrap();
        assert!(!plan.iter().any(|i| i.id == "wallet-records"), "node.json alone makes no line");
        // The node stops (its pid file goes) between the screen's list and the deletion: the list still matches.
        std::fs::remove_file(t.app.join("node.json")).unwrap();
        assert!(check(&p, &[Part::Truthcoin], &shown(&plan)).is_ok());
        std::fs::write(t.app.join("node.json"), b"x").unwrap();
        execute(&p, &[Part::Truthcoin], &shown(&plan)).unwrap();
        assert!(!t.app.join("node.json").exists(), "it goes with Truthcoin");
    }

    #[test]
    fn something_unknown_keeps_the_folder() {
        let t = setup();
        let p = places(&t, false, false);
        let plan = plan_items(&p).unwrap();
        let done = execute(&p, &[Part::Truthcoin, Part::App], &shown(&plan)).unwrap();
        assert!(wipe(&done.at_exit).is_empty());
        let left: Vec<String> =
            std::fs::read_dir(&t.app).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["localstorage"]);
    }

    #[test]
    fn truthcoin_alone_leaves_the_app() {
        let t = setup();
        let p = places(&t, true, false);
        let plan = plan_items(&p).unwrap();
        let done = execute(&p, &[Part::Truthcoin], &shown(&plan)).unwrap();
        assert!(done.errors.is_empty());
        for gone in ["bin", "node", "node.json", "trades.json", "trades.bad-17", "withdrawals.json", "wallet-ready"] {
            assert!(!t.app.join(gone).exists(), "{gone}");
        }
        for kept in ["settings.json", "settings.tmp", "activity.log", "lock", "phone/keys.json", "localstorage"] {
            assert!(t.app.join(kept).exists(), "{kept}");
        }
        assert!(done.at_exit.paths.is_empty() && done.at_exit.app_dir.is_none());
    }

    #[test]
    fn the_app_alone_leaves_truthcoin_and_takes_the_windows_data_at_exit() {
        let t = setup();
        let p = places(&t, true, false);
        let plan = plan_items(&p).unwrap();
        let done = execute(&p, &[Part::App], &shown(&plan)).unwrap();
        for gone in ["settings.json", "settings.tmp", "activity.log", "phone"] {
            assert!(!t.app.join(gone).exists(), "{gone}");
        }
        assert!(t.app.join("localstorage").exists() && t.app.join("lock").exists(), "they wait for exit");
        assert!(wipe(&done.at_exit).is_empty());
        assert!(!t.app.join("localstorage").exists() && !t.app.join("lock").exists());
        for kept in ["bin/truthcoin_dc-0.19.0", "node/wallet.mdb/data.mdb", "trades.json", "wallet-ready"] {
            assert!(t.app.join(kept).exists(), "{kept}");
        }
    }

    #[test]
    fn on_linux_the_folder_goes_at_exit() {
        let t = setup();
        let p = places(&t, true, false);
        let plan = plan_items(&p).unwrap();
        let done = execute(&p, &[Part::Truthcoin, Part::App], &shown(&plan)).unwrap();
        assert!(t.app.join("localstorage").exists());
        assert!(wipe(&done.at_exit).is_empty());
        assert!(!t.app.exists());
    }

    #[test]
    fn a_folder_set_by_hand_keeps_what_isnt_the_apps() {
        let t = setup();
        std::fs::write(t.app.join("notes.txt"), b"mine").unwrap();
        let p = places(&t, false, true);
        let plan = plan_items(&p).unwrap();
        let done = execute(&p, &[Part::Truthcoin, Part::App], &shown(&plan)).unwrap();
        assert!(done.errors.is_empty());
        assert!(wipe(&done.at_exit).is_empty());
        assert!(t.app.join("notes.txt").exists());
        assert!(t.app.join("localstorage").exists());
        assert!(!t.app.join("node").exists() && !t.app.join("settings.json").exists() && !t.app.join("lock").exists());
    }

    #[test]
    fn a_folder_set_by_hand_keeps_a_bin_and_node_that_arent_the_apps() {
        let t = setup();
        std::fs::write(t.app.join("bin/my-tool"), b"x").unwrap();
        std::fs::remove_file(t.app.join("node/app-node.log")).unwrap();
        let p = places(&t, false, true);
        let plan = plan_items(&p).unwrap();
        assert!(!plan.iter().any(|i| i.id == "node-program" || i.id == "node-data"));
        execute(&p, &[Part::Truthcoin, Part::App], &shown(&plan)).unwrap();
        assert!(t.app.join("bin/my-tool").exists() && t.app.join("node/wallet.mdb/data.mdb").exists());
        assert!(!t.app.join("trades.json").exists());
    }

    #[test]
    fn a_changed_list_removes_nothing() {
        let t = setup();
        let p = places(&t, false, false);
        let plan = plan_items(&p).unwrap();
        let mut s = shown(&plan);
        s.retain(|x| x.id != "node-data");
        assert!(execute(&p, &[Part::Truthcoin, Part::App], &s).unwrap_err().contains("changed"));
        let mut s = shown(&plan);
        s[0].path = t.home.display().to_string();
        assert!(execute(&p, &[Part::Truthcoin], &s).is_err());
        assert!(execute(&p, &[], &shown(&plan)).unwrap_err().contains("Nothing is ticked"));
        assert!(t.app.join("node/wallet.mdb/data.mdb").exists() && t.app.join("settings.json").exists());
    }

    #[test]
    fn the_home_folder_is_never_the_apps() {
        let t = setup();
        let mut p = places(&t, false, true);
        p.app_dir = t.home.clone();
        assert!(plan_items(&p).is_err());
        p.app_dir = t.home.parent().unwrap().to_path_buf();
        assert!(plan_items(&p).is_err());
        p.app_dir = PathBuf::from("relative/dir");
        assert!(plan_items(&p).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_goes_alone() {
        let t = setup();
        let elsewhere = t.home.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("keep"), b"x").unwrap();
        std::fs::remove_dir_all(t.app.join("bin")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, t.app.join("bin")).unwrap();
        let p = places(&t, true, false);
        let plan = plan_items(&p).unwrap();
        let done = execute(&p, &[Part::Truthcoin, Part::App], &shown(&plan)).unwrap();
        assert!(done.errors.is_empty(), "{:?}", done.errors);
        assert!(wipe(&done.at_exit).is_empty());
        assert!(elsewhere.join("keep").exists());
        assert!(!t.app.exists());
    }

    #[test]
    fn a_program_named_folder_is_listed_only_when_it_holds_only_webkits_files() {
        let t = setup();
        let ours = t.home.join(".cache/truthcoin-app");
        let theirs = t.home.join(".local/share/truthcoin-app");
        for d in [&ours, &theirs] {
            std::fs::create_dir_all(d.join("WebKitCache")).unwrap();
        }
        std::fs::write(theirs.join("wallet.dat"), b"x").unwrap();
        let mut p = places(&t, true, false);
        p.caches = vec![
            Cache { id: "webkit-cache", path: ours.clone(), program_named: true },
            Cache { id: "webkit-data", path: theirs.clone(), program_named: true },
            Cache { id: "local-data", path: t.app.clone(), program_named: false },
        ];
        let plan = plan_items(&p).unwrap();
        let caches: Vec<&str> = plan.iter().filter(|i| i.at_exit).map(|i| i.path.as_str()).collect();
        assert_eq!(caches, [ours.display().to_string()]);
        let done = execute(&p, &[Part::App], &shown(&plan)).unwrap();
        // Something else moved in before the app closed: it stays.
        std::fs::write(ours.join("other"), b"x").unwrap();
        assert_eq!(wipe(&done.at_exit).len(), 1);
        assert!(ours.join("other").exists() && theirs.join("wallet.dat").exists());
    }

    #[test]
    fn plist_identifier() {
        let p = "<dict>\n\t<key>CFBundleIdentifier</key>\n\t<string>dev.truthcoinapp.desktop</string>\n</dict>";
        assert_eq!(plist_string(p, "CFBundleIdentifier"), Some("dev.truthcoinapp.desktop"));
        assert_eq!(plist_string(p, "CFBundleName"), None);
    }
}
