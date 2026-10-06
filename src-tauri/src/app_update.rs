//! The app updates itself (v0.1.3; Michael 2026-10-07, from FreeBank's v0.2.4 updater, which he wrote). It trusts
//! exactly what the update check always trusted: a release's SHA256SUMS signed with this app's release key
//! (SHA256SUMS.sig, `update.rs`). So an update is offered only once the release has been signed, and nothing GitHub
//! holds can make one without it. The AppImage and the Mac app are built on GitHub's machines and don't rebuild byte
//! for byte (VERIFY.md), so the signature vouches for the release Michael checked and signed, not for more than that
//! build.
//!
//! - **Check:** SHA256SUMS and SHA256SUMS.sig from the app's latest GitHub release, at start (reused for six hours) and
//!   from Settings. The signature is checked first, then the version is read from the package names in it (they must
//!   all agree). A version newer than this app is offered; an unsigned release isn't, yet.
//! - **AppImage:** the new AppImage is downloaded beside the running one, checked against its signed line, and renamed
//!   over it (the running app keeps the old copy it has open); the app then restarts from the same path.
//! - **Mac:** the new `Truthcoin App.app` (`Truthcoin-App_<v>_universal.app.tar.gz`) is unpacked beside the running
//!   bundle from the very file that was checked, checked again (this app, that version, a macOS this Mac has), and
//!   exchanged with the running bundle in one step where the disk allows it (two renames elsewhere, the old bundle put
//!   back if the second fails); the app then restarts.
//! - **The Debian package** can't replace itself: the release page has the new .deb.
//! - **Anything else** (a developer's build, a Mac app run from the disk image or from where macOS put a download
//!   aside, a folder this user can't change or others could): the release page.
//!
//! The restart waits until nothing holds the node (a download of it, Obliterate), and takes the app's usual way out
//! (`lib.rs`, RunEvent::Exit), which stops the node. Never after Obliterate, which in turn refuses while an update is
//! under way (`updating`).

use crate::state::St;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, State};

/// The app's releases on GitHub.
const RELEASES: &str = "https://github.com/truthcoinapp/truthcoin-app/releases";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// A check is reused for this long, unless asked again from Settings.
const CHECK_AGE: Duration = Duration::from_secs(6 * 3600);
/// No package is anywhere near this big (the AppImage is about 80 MB).
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
/// SHA256SUMS and its signature are a few hundred bytes.
const SMALL_FILE_MAX: u64 = 64 * 1024;
const TRANSLOCATED: &str = "Move Truthcoin App to your Applications folder and open it from there: macOS runs it from a \
                            temporary place now, so it can't update itself.";
const DISK_IMAGE: &str = "Truthcoin App runs from its disk image: drag it to your Applications folder and open it from \
                          there, and it can update itself.";
const IDENTIFIER: &str = "dev.truthcoinapp.desktop";
const BUNDLE: &str = "Truthcoin App.app";

/// An update is under way: Obliterate refuses meanwhile (it sets the node's `removing` and then looks at this; the
/// update sets this and then looks at `removing`, so the two never both go ahead).
static UPDATING: AtomicBool = AtomicBool::new(false);

pub fn updating() -> bool {
    UPDATING.load(Ordering::SeqCst)
}

/// Written in the app's folder just before "Update and restart": the copy that starts then waits for the old copy's
/// lock (lib.rs, single_instance), then removes it. A file rather than the environment, which other threads read.
pub const RESTARTING: &str = "restarting";

/// Ends an update that doesn't restart, a panic included (review L5, re-review N1): UPDATING is cleared first, and
/// only then does the screen see it stopped, so a second "Update and restart" can't start before the flag is clear.
struct Updating {
    restart: bool,
    progress: Arc<Mutex<Progress>>,
}

impl Drop for Updating {
    fn drop(&mut self) {
        if self.restart {
            return;
        }
        UPDATING.store(false, Ordering::SeqCst);
        let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        p.running = false;
        p.note = None;
        if p.error.is_none() {
            p.error = Some("The update stopped before it finished. Nothing was changed.".into());
        }
    }
}

/// Where releases are read from, and the key their SHA256SUMS must be signed with.
#[derive(Clone, Debug)]
pub struct Source {
    /// The latest release's files: `<latest>/SHA256SUMS`.
    latest: String,
    /// A release's files by tag: `<by_tag>/v0.1.3/<name>`.
    by_tag: String,
    key: String,
}

impl Source {
    /// GitHub and the release key. A build with the `update-test` feature (never released) reads TRUTHCOIN_UPDATE_URL
    /// (`<url>/latest/…`, `<url>/v<version>/…`) and TRUTHCOIN_UPDATE_KEY instead, for the end-to-end test.
    pub fn new() -> Self {
        #[cfg(feature = "update-test")]
        if let (Ok(url), Ok(key)) = (std::env::var("TRUTHCOIN_UPDATE_URL"), std::env::var("TRUTHCOIN_UPDATE_KEY")) {
            // Only a server on this computer, even in a test build.
            if loopback(&url) {
                return Source { latest: format!("{url}/latest"), by_tag: url, key };
            }
        }
        Source {
            latest: format!("{RELEASES}/latest/download"),
            by_tag: format!("{RELEASES}/download"),
            key: crate::update::RELEASE_KEY.into(),
        }
    }
}

/// `url` is plain http to this computer, with nothing else in it: the end-to-end test's server.
#[cfg(any(test, feature = "update-test"))]
fn loopback(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    let local = match u.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    u.scheme() == "http"
        && local
        && u.username().is_empty()
        && u.password().is_none()
        && u.path() == "/"
        && u.query().is_none()
        && u.fragment().is_none()
}

/// What Settings and the update notice show.
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub current: String,
    /// The latest release whose SHA256SUMS is signed.
    pub latest: Option<String>,
    pub available: bool,
    /// How this copy updates: "self" (it replaces itself and restarts), "deb" (the Debian package: the release page
    /// has the new .deb), or "download" (anything else: the release page; `why` says why).
    pub how: &'static str,
    pub why: Option<String>,
    /// The latest release's page.
    pub page: Option<String>,
    pub error: Option<String>,
}

/// The update under way: stages signature, download, verify, replace, restart.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Progress {
    pub running: bool,
    pub stage: String,
    pub version: Option<String>,
    pub bytes: u64,
    pub total: Option<u64>,
    /// While the restart waits for the node.
    pub note: Option<String>,
    pub error: Option<String>,
}

pub struct AppUpdater {
    src: Source,
    http: reqwest::Client,
    checked: Mutex<Option<(Instant, Check)>>,
    progress: Arc<Mutex<Progress>>,
}

impl AppUpdater {
    pub fn new() -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("truthcoin-app/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(AppUpdater { src: Source::new(), http, checked: Mutex::new(None), progress: Arc::default() })
    }
}

/// A signed release: its version and the very SHA256SUMS text that was verified.
#[derive(Debug, Clone, PartialEq)]
struct Release {
    version: String,
    sums: String,
}

/// A small file from a release, whole, at most SMALL_FILE_MAX. Ok(None) when the release has no such file.
async fn download_bytes(http: &reqwest::Client, url: &str) -> Result<Option<Vec<u8>>, String> {
    let failed = |e: reqwest::Error| format!("Download failed: {url} ({e})");
    let resp = http.get(url).timeout(Duration::from_secs(60)).send().await.map_err(failed)?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let mut resp = resp.error_for_status().map_err(failed)?;
    let too_big = || format!("{url} is far bigger than a checksums file, so it wasn't read.");
    if resp.content_length().is_some_and(|n| n > SMALL_FILE_MAX) {
        return Err(too_big());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(failed)? {
        if (body.len() + chunk.len()) as u64 > SMALL_FILE_MAX {
            return Err(too_big());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(body))
}

/// The hash listed for `name` in a sha256sum-style file ("<hash>  name" or "<hash> *name").
fn listed_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let hash = it.next()?;
        let file = it.next()?.trim_start_matches('*');
        (file == name).then(|| hash.to_lowercase())
    })
}

/// The latest release, once its SHA256SUMS checks against the release key. Ok(None) while it has no SHA256SUMS.sig:
/// a release is published first and signed after.
async fn latest(http: &reqwest::Client, src: &Source) -> Result<Option<Release>, String> {
    if src.key.is_empty() {
        return Err("This build has no release key, so it can't check for updates.".into());
    }
    let sums = download_bytes(http, &format!("{}/SHA256SUMS", src.latest)).await?;
    let sig = download_bytes(http, &format!("{}/SHA256SUMS.sig", src.latest)).await?;
    let (Some(sums), Some(sig)) = (sums, sig) else { return Ok(None) };
    crate::update::verify_with(&src.key, &sums, &sig).map_err(|why| {
        format!(
            "The latest release's signature didn't check out ({why}), so it isn't offered. If a release came out a moment \
             ago, try again in a minute."
        )
    })?;
    let sums = String::from_utf8(sums).map_err(|_| "The latest release's checksums file isn't plain text.".to_string())?;
    let version = release_version(&sums)?;
    Ok(Some(Release { version, sums }))
}

/// "0.1.3" as numbers: three parts of plain digits.
fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || p.len() > 6 || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    Some((parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?))
}

/// The version in a package's name: truthcoin-app_0.1.3_amd64.deb, Truthcoin-App_0.1.3_amd64.AppImage,
/// Truthcoin-App_0.1.3_universal.dmg, Truthcoin-App_0.1.3_universal.app.tar.gz.
fn package_version(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("Truthcoin-App_").or_else(|| name.strip_prefix("truthcoin-app_"))?;
    let (v, _) = rest.split_once('_')?;
    parse_version(v).map(|_| v)
}

/// The release's version: the one its package names carry, which must all agree.
fn release_version(sums: &str) -> Result<String, String> {
    let mut found: Option<&str> = None;
    for name in sums.lines().filter_map(|l| l.split_whitespace().nth(1)).map(|n| n.trim_start_matches('*')) {
        let Some(v) = package_version(name) else { continue };
        if found.is_some_and(|f| f != v) {
            return Err("The latest release's packages carry different versions, so it isn't offered.".into());
        }
        found = Some(v);
    }
    found.map(str::to_string).ok_or_else(|| "The latest release lists no Truthcoin App packages.".into())
}

fn newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// The package this system updates from.
fn asset(version: &str) -> Option<String> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some(format!("Truthcoin-App_{version}_amd64.AppImage"))
    } else if cfg!(target_os = "macos") {
        Some(format!("Truthcoin-App_{version}_universal.app.tar.gz"))
    } else {
        None
    }
}

/// How this copy of the app was installed.
#[derive(Debug, Clone, PartialEq)]
enum Install {
    /// An AppImage, replaced in place.
    AppImage(PathBuf),
    /// A Mac app bundle (…/Truthcoin App.app), swapped in place.
    Bundle(PathBuf),
    /// The Debian package.
    Deb,
    /// Anything else, and why it can't update itself.
    Other(String),
}

/// `this_install`, off the async threads: it reads the disk and may ask the system's user directory (`safe_place`).
async fn install_kind() -> Install {
    tokio::task::spawn_blocking(this_install)
        .await
        .unwrap_or_else(|_| Install::Other("The app couldn't tell how it was installed.".into()))
}

fn this_install() -> Install {
    let Ok(exe) = std::env::current_exe() else {
        return Install::Other("The app couldn't find its own program.".into());
    };
    if cfg!(target_os = "macos") {
        // The bundle itself, not a link to it.
        let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
        return mac_bundle(&exe);
    }
    if let Some(image) = running_appimage(&exe) {
        return match replaceable(&image) {
            Ok(()) => Install::AppImage(image),
            Err(why) => Install::Other(why),
        };
    }
    if exe == Path::new("/usr/bin/truthcoin-app") {
        return Install::Deb;
    }
    Install::Other("This copy of the app wasn't installed from a release package, so it can't update itself.".into())
}

/// The AppImage this program runs from: APPIMAGE names it, but every program an AppImage starts inherits that, so
/// this program must also run from inside the AppImage's mount (APPDIR, or the runtime's /tmp/.mount_*).
fn running_appimage(current: &Path) -> Option<PathBuf> {
    let mount = std::env::var_os("APPDIR").map(PathBuf::from);
    let from_appimage = mount.as_ref().is_some_and(|m| current.starts_with(m)) || current.to_string_lossy().starts_with("/tmp/.mount_");
    std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|p| p.is_file() && from_appimage)
}

/// The bundle a Mac app runs from (…/Truthcoin App.app/Contents/MacOS/truthcoin-app), if it can be replaced.
fn mac_bundle(exe: &Path) -> Install {
    let bundle = exe.parent().and_then(Path::parent).and_then(Path::parent);
    let Some(bundle) = bundle.filter(|b| b.extension().is_some_and(|x| x == "app")) else {
        return Install::Other("This copy of the app isn't a Mac app, so it can't update itself.".into());
    };
    if bundle.to_string_lossy().contains("/AppTranslocation/") {
        return Install::Other(TRANSLOCATED.into());
    }
    if bundle.starts_with("/Volumes") && writable(bundle).is_err() {
        return Install::Other(DISK_IMAGE.into());
    }
    match replaceable(bundle) {
        Ok(()) => Install::Bundle(bundle.to_path_buf()),
        Err(why) => Install::Other(why),
    }
}

/// This user can replace `p`, and no other user could swap a file in while it is checked and renamed.
fn replaceable(p: &Path) -> Result<(), String> {
    writable(p)?;
    safe_place(p)
}

/// This user can replace `p`: the new copy goes beside it, then a rename, so its folder must take writes. (A disk
/// image is read-only, so a Mac app run from one is refused here.)
fn writable(p: &Path) -> Result<(), String> {
    let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let refused = || {
            format!(
                "Truthcoin App is in {}, which your user can't change, so it can't update itself. Download the new \
                 version instead.",
                dir.display()
            )
        };
        let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).map_err(|_| refused())?;
        // SAFETY: a valid C string; access() only looks.
        if unsafe { libc::access(c.as_ptr(), libc::W_OK) } != 0 {
            return Err(refused());
        }
    }
    Ok(())
}

/// The program, and every folder above it, belongs to this user or the system, and no one else can write to it (a
/// root-owned folder may be group-writable, as macOS's Applications is for admins; so may this user's own files where
/// the group is the user's private one, as Debian and Ubuntu set up: their umask 002 makes a downloaded AppImage
/// group-writable).
fn safe_place(exe: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: getuid() can't fail.
        let me = unsafe { libc::getuid() };
        let mut p = Some(exe);
        while let Some(at) = p {
            let m = std::fs::metadata(at).map_err(|e| format!("Couldn't check {}: {e}", at.display()))?;
            let (uid, mode) = (m.uid(), m.mode());
            let others_write = mode & 0o002 != 0;
            let group_write = mode & 0o020 != 0 && uid != 0 && !(uid == me && own_private_group(m.gid()));
            if (uid != me && uid != 0) || others_write || group_write {
                return Err(format!(
                    "Truthcoin App runs from {}, which other users of this computer could change, so it doesn't update \
                     itself there. Move it to your own folders (or install it) first.",
                    exe.display()
                ));
            }
            p = at.parent();
        }
    }
    Ok(())
}

/// `gid` is this user's private group: the user's primary group, named after the user, with no other members (user
/// private groups, as Debian, Ubuntu and Fedora make them). A shared primary group (macOS's "staff", a "users" group)
/// isn't.
#[cfg(unix)]
fn own_private_group(gid: u32) -> bool {
    use std::ffi::CStr;
    // SAFETY: getpwuid_r and getgrgid_r fill structs that point into buffers owned here, which outlive every read of
    // them; each result pointer is checked before use, and the member list ends with a null pointer.
    unsafe {
        let mut pw: libc::passwd = std::mem::zeroed();
        let mut pw_found: *mut libc::passwd = std::ptr::null_mut();
        let mut pw_buf = vec![0 as libc::c_char; 16 * 1024];
        if libc::getpwuid_r(libc::getuid(), &mut pw, pw_buf.as_mut_ptr(), pw_buf.len(), &mut pw_found) != 0
            || pw_found.is_null()
            || pw.pw_gid != gid
        {
            return false;
        }
        let mut gr: libc::group = std::mem::zeroed();
        let mut gr_found: *mut libc::group = std::ptr::null_mut();
        let mut gr_buf = vec![0 as libc::c_char; 64 * 1024];
        if libc::getgrgid_r(gid, &mut gr, gr_buf.as_mut_ptr(), gr_buf.len(), &mut gr_found) != 0 || gr_found.is_null() {
            return false;
        }
        if pw.pw_name.is_null() || gr.gr_name.is_null() {
            return false;
        }
        let user = CStr::from_ptr(pw.pw_name);
        if CStr::from_ptr(gr.gr_name) != user {
            return false;
        }
        let mut member = gr.gr_mem;
        while !member.is_null() && !(*member).is_null() {
            if CStr::from_ptr(*member) != user {
                return false;
            }
            member = member.add(1);
        }
        true
    }
}

fn page(version: &str) -> String {
    format!("{RELEASES}/tag/v{version}")
}

/// The check Settings and the notice show, for a release (or none) found by `latest`.
fn check_result(found: Result<Option<Release>, String>, how: &Install) -> Check {
    let (mut how_name, mut why) = match how {
        Install::AppImage(_) | Install::Bundle(_) => ("self", None),
        Install::Deb => ("deb", None),
        Install::Other(why) => ("download", Some(why.clone())),
    };
    // A release without this system's update package (a build that failed, say) is offered as a download.
    if let Ok(Some(r)) = &found {
        if how_name == "self" && !asset(&r.version).is_some_and(|a| listed_hash(&r.sums, &a).is_some()) {
            how_name = "download";
            why = Some("This release has no update package for this system: download it from the release page.".into());
        }
    }
    let (latest, error) = match found {
        Ok(r) => (r.map(|r| r.version), None),
        Err(e) => (None, Some(e)),
    };
    Check {
        current: VERSION.into(),
        available: latest.as_deref().is_some_and(|l| newer(l, VERSION)),
        page: latest.as_deref().map(page),
        latest,
        how: how_name,
        why,
        error,
    }
}

#[tauri::command]
pub async fn app_update_check(upd: State<'_, Arc<AppUpdater>>, force: bool) -> Result<Check, String> {
    if !force {
        if let Some((at, c)) = upd.checked.lock().unwrap().as_ref() {
            if at.elapsed() < CHECK_AGE {
                return Ok(c.clone());
            }
        }
    }
    let found = latest(&upd.http, &upd.src).await;
    let c = check_result(found, &install_kind().await);
    *upd.checked.lock().unwrap() = Some((Instant::now(), c.clone()));
    Ok(c)
}

#[tauri::command]
pub fn app_update_progress(upd: State<'_, Arc<AppUpdater>>) -> Progress {
    upd.progress.lock().unwrap().clone()
}

/// The app's files have gone (Obliterate): it neither updates nor restarts.
fn removed() -> Result<(), String> {
    if crate::files::stopped() {
        return Err("Truthcoin App was removed from this computer, so it doesn't update.".into());
    }
    Ok(())
}

/// What keeps the app from updating or restarting just now, if anything.
fn busy(node: &crate::node::Node) -> Option<String> {
    if node.removing.load(Ordering::SeqCst) {
        return Some("Obliterate is under way.".into());
    }
    if node.install.lock().unwrap().running {
        return Some("The Truthcoin node is being downloaded.".into());
    }
    None
}

/// "Update and restart": download, check and put the new version in place, then restart on it.
#[tauri::command]
pub fn app_update_start(app: AppHandle, upd: State<'_, Arc<AppUpdater>>, st: St<'_>) -> Result<(), String> {
    removed()?;
    {
        let mut p = upd.progress.lock().unwrap();
        if p.running {
            return Ok(());
        }
        // Obliterate sets the node's `removing` and then looks at UPDATING; this sets UPDATING and then looks at
        // `removing`. So the two never both go ahead.
        UPDATING.store(true, Ordering::SeqCst);
        if let Some(what) = busy(&st.node).or_else(|| removed().err()) {
            UPDATING.store(false, Ordering::SeqCst);
            return Err(format!("Please wait: {what}"));
        }
        *p = Progress { running: true, stage: "signature".into(), ..Default::default() };
    }
    let (upd, node, dir) = (upd.inner().clone(), st.node.clone(), st.dir.clone());
    tauri::async_runtime::spawn(async move {
        let mut guard = Updating { restart: false, progress: upd.progress.clone() };
        let how = install_kind().await;
        let result = update(&upd.http, &upd.src, &how, &upd.progress, VERSION, &removed).await;
        match result {
            Ok(version) => {
                crate::activity::note(&dir, &format!("Truthcoin App {version} is in place; restarting on it"));
                guard.restart = restart_when_free(&app, &node, &dir, &upd.progress).await;
            }
            Err(e) => {
                crate::activity::note(&dir, &format!("App update failed: {e}"));
                set(&upd.progress, |s| s.error = Some(e));
            }
        }
        // The guard (if it isn't restarting) clears UPDATING and then shows the update as stopped.
        drop(guard);
    });
    Ok(())
}

/// Restart once nothing holds the node (a download of it). It waits as long as that takes: the screen says what for,
/// and closing the app by hand opens the new version too. Never after Obliterate. True once the restart is asked for.
async fn restart_when_free(app: &AppHandle, node: &crate::node::Node, dir: &Path, p: &Mutex<Progress>) -> bool {
    loop {
        if let Err(e) = removed() {
            set(p, |s| s.error = Some(e));
            return false;
        }
        let held = busy(node);
        set(p, |s| {
            s.stage = "restart".into();
            s.note = held.clone();
        });
        if held.is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    // The exit (lib.rs, RunEvent::Exit) stops the node; the new version starts it again, once this copy's lock is free.
    let _ = crate::files::write_private(&dir.join(RESTARTING), std::process::id().to_string().as_bytes());
    app.request_restart();
    true
}

fn set(p: &Mutex<Progress>, f: impl FnOnce(&mut Progress)) {
    f(&mut p.lock().unwrap());
}

/// Fetch the latest signed release and put it in place of this copy; its version. Nothing is changed unless the
/// download matches its line in the signed SHA256SUMS, and `go_on` (the app is still here) still holds just before.
async fn update(
    http: &reqwest::Client,
    src: &Source,
    how: &Install,
    p: &Mutex<Progress>,
    current: &str,
    go_on: &(dyn Fn() -> Result<(), String> + Sync),
) -> Result<String, String> {
    let target = match how {
        Install::AppImage(t) | Install::Bundle(t) => t,
        Install::Deb => return Err("The Debian package is updated with the new .deb from the release page.".into()),
        Install::Other(why) => return Err(why.clone()),
    };
    let release = latest(http, src).await?.ok_or("The latest release isn't signed yet. Try again later.")?;
    if !newer(&release.version, current) {
        return Err(format!("Truthcoin App {current} is already the newest version."));
    }
    let name = asset(&release.version).ok_or("Truthcoin App has no update for this system.")?;
    let want =
        listed_hash(&release.sums, &name).ok_or_else(|| format!("{name} isn't in the release's signed checksums."))?;
    set(p, |s| {
        s.stage = "download".into();
        s.version = Some(release.version.clone());
    });

    let (tmp, mut file) = fresh_beside(target, "")?;
    let url = format!("{}/v{}/{}", src.by_tag, release.version, name);
    let got = download_to(http, &url, &mut file, |bytes, total| {
        set(p, |s| {
            s.bytes = bytes;
            s.total = total;
        })
    })
    .await;
    set(p, |s| s.stage = "verify".into());
    let checked = got.and_then(|got| {
        if got == want {
            Ok(())
        } else {
            Err(format!(
                "The download didn't match the release's signed checksums, so nothing was changed. Please try again. \
                 (expected {want}, got {got})"
            ))
        }
    });
    let result = match checked.and_then(|()| go_on()) {
        Err(e) => Err(e),
        Ok(()) => {
            set(p, |s| s.stage = "replace".into());
            // The checked file goes on by its handle, not by its name; the file work runs off the async threads.
            let (how, new, version) = (how.clone(), tmp.clone(), release.version.clone());
            tokio::task::spawn_blocking(move || match how {
                Install::AppImage(image) => replace_file(file, &new, &image),
                Install::Bundle(bundle) => swap_bundle(file, &bundle, &version, mac_os_version()),
                _ => Err("This copy of the app can't update itself.".into()),
            })
            .await
            .unwrap_or_else(|e| Err(format!("The update stopped: {e}")))
        }
    };
    let _ = std::fs::remove_file(&tmp);
    result.map(|()| release.version)
}

/// A new file beside `target`, under a name nothing else uses, readable only by this user. Open for reading too: the
/// Mac update is unpacked from this very handle.
fn fresh_beside(target: &Path, suffix: &str) -> Result<(PathBuf, std::fs::File), String> {
    let dir = target.parent().unwrap_or(Path::new("."));
    for _ in 0..8 {
        let p = dir.join(format!(".truthcoin-app-update-{:016x}{suffix}", rand::random::<u64>()));
        let mut o = std::fs::OpenOptions::new();
        o.read(true).write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        match o.open(&p) {
            Ok(f) => return Ok((p, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Couldn't write in {}: {e}", dir.display())),
        }
    }
    Err(format!("Couldn't write in {}.", dir.display()))
}

/// Stream `url` into `f`, reporting bytes as they arrive; the SHA-256 of what was written.
async fn download_to(
    http: &reqwest::Client,
    url: &str,
    f: &mut std::fs::File,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<String, String> {
    let mut resp = http
        .get(url)
        .timeout(Duration::from_secs(3600))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Download failed: {url} ({e})"))?;
    let total = resp.content_length();
    progress(0, total);
    let mut h = Sha256::new();
    let mut got = 0u64;
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("Download interrupted: {e}"))? {
        got += chunk.len() as u64;
        if got > MAX_DOWNLOAD {
            return Err("The download is far bigger than any Truthcoin App package, so it was stopped.".into());
        }
        f.write_all(&chunk).map_err(|e| format!("Couldn't save the download: {e}"))?;
        h.update(&chunk);
        progress(got, total);
    }
    f.sync_all().map_err(|e| format!("Couldn't save the download: {e}"))?;
    Ok(hex::encode(h.finalize()))
}

/// The checked AppImage (`file`, at `new`) takes `image`'s place: as executable as the old one, never writable by
/// others.
fn replace_file(file: std::fs::File, new: &Path, image: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let old = std::fs::metadata(image).map(|m| m.permissions().mode()).unwrap_or(0o755);
        let mode = (old | 0o500) & 0o755;
        file.set_permissions(std::fs::Permissions::from_mode(mode))
            .map_err(|e| format!("Couldn't prepare the new version: {e}"))?;
    }
    drop(file);
    std::fs::rename(new, image).map_err(|e| format!("Couldn't put the new version in place: {e}"))
}

/// Unpack the new `Truthcoin App.app` from the checked `archive` (its handle, read from the start) beside `bundle`,
/// check it, and exchange the two; the old bundle then goes with the unpacking folder. `os`: this Mac's macOS version.
fn swap_bundle(mut archive: std::fs::File, bundle: &Path, version: &str, os: Option<(u32, u32, u32)>) -> Result<(), String> {
    use std::io::{Seek, SeekFrom};
    let dir = bundle.parent().unwrap_or(Path::new("."));
    let stage = dir.join(format!(".truthcoin-app-update-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&stage).map_err(|e| format!("Couldn't write in {}: {e}", dir.display()))?;
    let result = (|| {
        archive.seek(SeekFrom::Start(0)).map_err(|e| format!("Couldn't read the download: {e}"))?;
        let mut a = tar::Archive::new(flate2::read::GzDecoder::new(&mut archive));
        // No set-id bits, and nothing writable by group or others.
        a.set_mask(0o022);
        a.unpack(&stage).map_err(|e| format!("Couldn't unpack the new version: {e}"))?;
        let new = stage.join(BUNDLE);
        check_bundle(&new, version, os)?;
        exchange(&new, bundle, true).map_err(|e| moved_refused(bundle, e))?;
        // Finder and the Dock look again at a bundle whose date changed.
        let _ = std::process::Command::new("/usr/bin/touch").arg(bundle).status();
        Ok(())
    })();
    // The old bundle is in there now; if it can't go, the next start sweeps it (`sweep_leftovers`).
    let _ = std::fs::remove_dir_all(&stage);
    result
}

/// Afterwards `a` holds what `b` held, and `b` what `a` held. In one step where the disk allows it (APFS and HFS+ on a
/// Mac, Linux's own file systems), with `atomic`; else `b` steps aside to a hidden name, `a` takes its place (`b` goes
/// back if that fails), and the old `b` moves to `a`'s name.
fn exchange(a: &Path, b: &Path, atomic: bool) -> std::io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if atomic {
        use std::os::unix::ffi::OsStrExt;
        let ca = std::ffi::CString::new(a.as_os_str().as_bytes())?;
        let cb = std::ffi::CString::new(b.as_os_str().as_bytes())?;
        // SAFETY: two valid C strings that outlive the call.
        #[cfg(target_os = "macos")]
        let r = unsafe { libc::renamex_np(ca.as_ptr(), cb.as_ptr(), libc::RENAME_SWAP) };
        #[cfg(target_os = "linux")]
        let r = unsafe { libc::renameat2(libc::AT_FDCWD, ca.as_ptr(), libc::AT_FDCWD, cb.as_ptr(), libc::RENAME_EXCHANGE) };
        if r == 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        // Only "this disk can't" falls back; anything else (a permission, App Management) is the answer.
        let cant = |c: i32| c == libc::ENOTSUP || c == libc::EOPNOTSUPP || c == libc::EINVAL || c == libc::ENOSYS;
        if !e.raw_os_error().is_some_and(cant) {
            return Err(e);
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = atomic;
    let aside = b.with_file_name(format!(".truthcoin-app-old-{:016x}.app", rand::random::<u64>()));
    std::fs::rename(b, &aside)?;
    if let Err(e) = std::fs::rename(a, b) {
        let _ = std::fs::rename(&aside, b);
        return Err(e);
    }
    // If this one fails, the old copy stays under its hidden name until the next start sweeps it.
    let _ = std::fs::rename(&aside, a);
    Ok(())
}

fn moved_refused(bundle: &Path, e: std::io::Error) -> String {
    #[cfg(unix)]
    let not_permitted = e.raw_os_error() == Some(libc::EPERM);
    #[cfg(not(unix))]
    let not_permitted = false;
    let hint = if not_permitted {
        " If macOS asks whether Truthcoin App may change apps, allow it in System Settings → Privacy & Security → App \
         Management, or download the new version."
    } else {
        " Download the new version instead."
    };
    format!("Couldn't replace {} ({e}).{hint}", bundle.display())
}

/// The unpacked bundle is this app, at the version the signed checksums named, with its program, for a macOS this Mac
/// has (`os`; a version that needs a newer one would install and then not open).
fn check_bundle(app: &Path, version: &str, os: Option<(u32, u32, u32)>) -> Result<(), String> {
    let wrong = |what: &str| format!("The new version doesn't look right ({what}), so nothing was changed.");
    if !std::fs::symlink_metadata(app).is_ok_and(|m| m.is_dir()) {
        return Err(wrong("not a folder"));
    }
    let plist = std::fs::read(app.join("Contents/Info.plist")).map_err(|_| wrong("no Info.plist"))?;
    let text = String::from_utf8_lossy(&plist);
    if plist_string(&text, "CFBundleIdentifier") != Some(IDENTIFIER) {
        return Err(wrong("another app's bundle"));
    }
    if plist_string(&text, "CFBundleShortVersionString") != Some(version) {
        return Err(wrong("another version"));
    }
    let exe = plist_string(&text, "CFBundleExecutable").ok_or_else(|| wrong("no program named"))?;
    if exe.contains('/') || !app.join("Contents/MacOS").join(exe).is_file() {
        return Err(wrong("no program"));
    }
    if let (Some(os), Some(min)) = (os, plist_string(&text, "LSMinimumSystemVersion").and_then(loose_version)) {
        if os < min {
            return Err(format!(
                "Truthcoin App {version} needs macOS {}.{} or later, and this Mac has {}.{}.{}, so nothing was changed.",
                min.0, min.1, os.0, os.1, os.2
            ));
        }
    }
    Ok(())
}

/// "14", "14.2" or "14.2.1" as numbers (a missing part is 0).
fn loose_version(v: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = v.trim().split('.').collect();
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let n = |i: usize| parts.get(i).map_or(Some(0), |p| p.parse().ok());
    Some((n(0)?, n(1)?, n(2)?))
}

/// This Mac's macOS version (`sw_vers`); None elsewhere, or when it can't be read.
fn mac_os_version() -> Option<(u32, u32, u32)> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let out = std::process::Command::new("/usr/bin/sw_vers").arg("-productVersion").output().ok()?;
    loose_version(std::str::from_utf8(&out.stdout).ok()?)
}

/// At start: what an interrupted update left beside the app (`.truthcoin-app-update-…`, `.truthcoin-app-old-….app`)
/// goes, once an hour old. Only those names, only this user's, never through a link.
pub fn sweep_leftovers() {
    if let Install::AppImage(target) | Install::Bundle(target) = this_install() {
        if let Some(dir) = target.parent() {
            sweep(dir, Duration::from_secs(3600));
        }
    }
}

fn leftover_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(".truthcoin-app-update-").or_else(|| name.strip_prefix(".truthcoin-app-old-")) else {
        return false;
    };
    let hex = rest.strip_suffix(".app").unwrap_or(rest);
    hex.len() == 16 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// How long since the entry last changed: its ctime where there is one (a rename updates it, so a bundle just set aside
/// counts as new), else its mtime.
fn changed_age(m: &std::fs::Metadata) -> Option<Duration> {
    #[cfg(unix)]
    {
        let ctime = std::time::UNIX_EPOCH + Duration::from_secs(u64::try_from(std::os::unix::fs::MetadataExt::ctime(m)).ok()?);
        return ctime.elapsed().ok();
    }
    #[cfg(not(unix))]
    m.modified().ok()?.elapsed().ok()
}

fn sweep(dir: &Path, min_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if !e.file_name().to_str().is_some_and(leftover_name) {
            continue;
        }
        let path = e.path();
        let Ok(m) = std::fs::symlink_metadata(&path) else { continue };
        #[cfg(unix)]
        // SAFETY: getuid() can't fail.
        if std::os::unix::fs::MetadataExt::uid(&m) != unsafe { libc::getuid() } {
            continue;
        }
        if !changed_age(&m).is_some_and(|age| age >= min_age) {
            continue;
        }
        let _ = if m.is_dir() {
            std::fs::remove_dir_all(&path)
        } else if m.is_file() {
            std::fs::remove_file(&path)
        } else {
            continue;
        };
    }
}

/// The string after `<key>KEY</key>` in an XML property list (Tauri writes Info.plist as XML).
fn plist_string<'a>(plist: &'a str, key: &str) -> Option<&'a str> {
    let at = plist.find(&format!("<key>{key}</key>"))? + key.len() + "<key></key>".len();
    let rest = plist[at..].trim_start().strip_prefix("<string>")?;
    rest.split_once("</string>").map(|(v, _)| v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssh_key::{private::Ed25519Keypair, HashAlg, LineEnding, PrivateKey};

    /// A throwaway release key (fixed seed) and its public half.
    fn test_key() -> (PrivateKey, String) {
        let key = PrivateKey::from(Ed25519Keypair::from_seed(&[7; 32]));
        let public = key.public_key().to_openssh().unwrap();
        (key, public)
    }

    fn sign(key: &PrivateKey, sums: &str) -> Vec<u8> {
        key.sign(crate::update::NAMESPACE, HashAlg::Sha512, sums.as_bytes()).unwrap().to_pem(LineEnding::LF).unwrap().into_bytes()
    }

    fn sha(b: &[u8]) -> String {
        hex::encode(Sha256::digest(b))
    }

    /// A tiny HTTP server for the given paths; the paths asked for, in order.
    fn serve(files: Vec<(String, Vec<u8>)>) -> (String, Arc<Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = asked.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                        break;
                    }
                }
                log.lock().unwrap().push(path.clone());
                let mut s = stream;
                match files.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => {
                        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                        let _ = s.write_all(body);
                    }
                    None => {
                        let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            }
        });
        (base, asked)
    }

    fn source(base: &str, key: &str) -> Source {
        Source { latest: format!("{base}/latest"), by_tag: base.to_string(), key: key.to_string() }
    }

    fn sums_for(v: &str, appimage: &[u8], tarball: &[u8]) -> String {
        format!(
            "{}  truthcoin-app_{v}_amd64.deb\n{}  Truthcoin-App_{v}_amd64.AppImage\n{}  Truthcoin-App_{v}_universal.app.tar.gz\n{}  Truthcoin-App_{v}_universal.dmg\n",
            sha(b"deb"),
            sha(appimage),
            sha(tarball),
            sha(b"dmg")
        )
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("0.2.4"), Some((0, 2, 4)));
        for bad in ["0.2", "0.2.4.1", "0.2.+4", "0.2.-4", "v0.2.4", "0.2.4 ", "", "0..4", "0.2.4a", "0.2.1234567"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
        assert!(newer("0.2.10", "0.2.9"), "numbers, not text");
        assert!(newer("0.3.0", "0.2.99"));
        assert!(!newer("0.2.4", "0.2.4"));
        assert!(!newer("0.2.3", "0.2.4"), "never back to an older version");
        assert!(!newer("junk", "0.2.4"));
    }

    #[test]
    fn the_release_version_comes_from_its_packages() {
        assert_eq!(release_version(&sums_for("0.2.5", b"a", b"t")), Ok("0.2.5".into()));
        // sha256sum's binary mode marks names with '*'.
        assert_eq!(release_version(&format!("{}  *Truthcoin-App_0.2.6_amd64.AppImage\n", sha(b"a"))), Ok("0.2.6".into()));
        // Other files (latest.json, a README) don't count; packages that disagree refuse the release.
        let mixed = format!("{}  README\n{}  Truthcoin-App_0.2.5_amd64.AppImage\n{}  truthcoin-app_0.2.6_amd64.deb\n", sha(b"r"), sha(b"a"), sha(b"d"));
        assert!(release_version(&mixed).is_err());
        assert!(release_version(&format!("{}  truthcoin-0.19.0-x86_64-unknown-linux-gnu\n", sha(b"n"))).is_err(), "a node release isn't an app release");
        assert!(release_version("").is_err());
    }

    #[test]
    fn a_mac_app_runs_from_a_bundle_it_can_replace() {
        assert_eq!(
            mac_bundle(Path::new("/private/var/folders/x/AppTranslocation/ABC/d/Truthcoin App.app/Contents/MacOS/truthcoin-app")),
            Install::Other(TRANSLOCATED.into())
        );
        assert!(matches!(mac_bundle(Path::new("/usr/bin/truthcoin-app")), Install::Other(_)));
        // Beside the source: folders only this user can change (on a Debian or Ubuntu machine, group-writable by the
        // user's own group).
        let src = Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmp = src.join(format!(".tc-bundle-{}", std::process::id()));
        let exe = tmp.join("Truthcoin App.app/Contents/MacOS/truthcoin-app");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        if safe_place(src).is_ok() {
            assert_eq!(mac_bundle(&exe), Install::Bundle(tmp.join("Truthcoin App.app")));
        } else {
            eprintln!("skipped: {} is a place other users could change", src.display());
        }
        std::fs::remove_dir_all(&tmp).unwrap();
        // Under /tmp, which every user can write: no self-update there.
        let shared = std::env::temp_dir().join(format!("tc-bundle-{}", std::process::id()));
        let exe = shared.join("Truthcoin App.app/Contents/MacOS/truthcoin-app");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        assert!(matches!(mac_bundle(&exe), Install::Other(_)));
        std::fs::remove_dir_all(&shared).unwrap();
        // A folder this user can't write (a disk image is read-only): the release page instead.
        assert!(writable(Path::new("/proc/1/Truthcoin App.app")).is_err());
    }

    #[test]
    fn what_the_check_says() {
        let sums = |v: &str| asset(v).map(|a| format!("{}  {a}\n", sha(b"p"))).unwrap_or_default();
        let r = |v: &str| Ok(Some(Release { version: v.into(), sums: sums(v) }));
        let c = check_result(r("9.0.0"), &Install::AppImage("/x".into()));
        assert!(c.available && c.how == "self" && c.error.is_none());
        assert_eq!(c.page.as_deref(), Some("https://github.com/truthcoinapp/truthcoin-app/releases/tag/v9.0.0"));
        assert!(!check_result(r(VERSION), &Install::AppImage("/x".into())).available);
        assert_eq!(check_result(r("9.0.0"), &Install::Deb).how, "deb");
        let other = check_result(r("9.0.0"), &Install::Other("why".into()));
        assert_eq!((other.how, other.why.as_deref()), ("download", Some("why")));
        // A release without this system's package: the release page instead.
        let missing = check_result(Ok(Some(Release { version: "9.0.0".into(), sums: String::new() })), &Install::AppImage("/x".into()));
        assert!(missing.available && missing.how == "download" && missing.why.is_some());
        // Not signed yet: nothing offered, and no error either.
        let unsigned = check_result(Ok(None), &Install::AppImage("/x".into()));
        assert!(!unsigned.available && unsigned.latest.is_none() && unsigned.error.is_none());
        assert!(check_result(Err("bad".into()), &Install::AppImage("/x".into())).error.is_some());
    }

    #[test]
    fn the_plist_is_read_as_tauri_writes_it() {
        let p = "<dict>\n\t<key>CFBundleExecutable</key>\n\t<string>truthcoin-app</string>\n\t<key>CFBundleIdentifier</key>\n\t<string>dev.truthcoinapp.desktop</string>\n</dict>";
        assert_eq!(plist_string(p, "CFBundleExecutable"), Some("truthcoin-app"));
        assert_eq!(plist_string(p, "CFBundleIdentifier"), Some(IDENTIFIER));
        assert_eq!(plist_string(p, "CFBundleShortVersionString"), None);
    }

    /// Run `update` against a served release; the AppImage folder afterwards. `go_on`: the app is still here.
    async fn update_appimage(
        sums: String,
        sig: Vec<u8>,
        served: Vec<u8>,
        key: &str,
        go_on: impl Fn() -> Result<(), String> + Sync,
    ) -> (Result<String, String>, Vec<u8>, usize, Vec<String>) {
        let v = "9.0.0";
        let (base, asked) = serve(vec![
            ("/latest/SHA256SUMS".into(), sums.into_bytes()),
            ("/latest/SHA256SUMS.sig".into(), sig),
            (format!("/v{v}/Truthcoin-App_{v}_amd64.AppImage"), served),
        ]);
        let dir = std::env::temp_dir().join(format!("tc-update-{}-{}", std::process::id(), rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = dir.join("Truthcoin-App_0.1.2_amd64.AppImage");
        std::fs::write(&image, b"old").unwrap();
        let p = Mutex::new(Progress::default());
        let r = update(&reqwest::Client::new(), &source(&base, key), &Install::AppImage(image.clone()), &p, "0.1.2", &go_on).await;
        let now = std::fs::read(&image).unwrap();
        let files = std::fs::read_dir(&dir).unwrap().count();
        std::fs::remove_dir_all(&dir).unwrap();
        let asked = asked.lock().unwrap().clone();
        (r, now, files, asked)
    }

    #[tokio::test]
    async fn an_appimage_is_replaced_only_by_a_signed_matching_download() {
        let (key, public) = test_key();
        let new = b"new appimage".to_vec();
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return; // the AppImage is the Linux package
        }
        let sums = sums_for("9.0.0", &new, b"t");

        // Signed and matching: in place, executable, and no stray file left beside it.
        let (r, now, files, _) = update_appimage(sums.clone(), sign(&key, &sums), new.clone(), &public, || Ok(())).await;
        assert_eq!(r, Ok("9.0.0".into()));
        assert_eq!(now, new);
        assert_eq!(files, 1);

        // Another key's signature: refused before any download.
        let other = PrivateKey::from(Ed25519Keypair::from_seed(&[8; 32]));
        let (r, now, files, asked) = update_appimage(sums.clone(), sign(&other, &sums), new.clone(), &public, || Ok(())).await;
        assert!(r.unwrap_err().contains("signature"));
        assert_eq!((now.as_slice(), files), (&b"old"[..], 1));
        assert_eq!(asked, ["/latest/SHA256SUMS", "/latest/SHA256SUMS.sig"], "the package is never asked for");

        // Signed, but the download differs from its line: nothing changes.
        let (r, now, files, _) = update_appimage(sums.clone(), sign(&key, &sums), b"tampered".to_vec(), &public, || Ok(())).await;
        assert!(r.unwrap_err().contains("didn't match"));
        assert_eq!((now.as_slice(), files), (&b"old"[..], 1));

        // Obliterate ran meanwhile: the checked download isn't put in place.
        let removed = || Err("Truthcoin App was removed from this computer.".to_string());
        let (r, now, files, _) = update_appimage(sums.clone(), sign(&key, &sums), new.clone(), &public, removed).await;
        assert!(r.unwrap_err().contains("removed"));
        assert_eq!((now.as_slice(), files), (&b"old"[..], 1));

        // Not signed yet: not offered.
        let (base, _) = serve(vec![("/latest/SHA256SUMS".into(), sums.clone().into_bytes())]);
        assert_eq!(latest(&reqwest::Client::new(), &source(&base, &public)).await, Ok(None));

        // A checksums file far too big is never read whole.
        let (base, _) = serve(vec![
            ("/latest/SHA256SUMS".into(), vec![b'0'; 100 * 1024]),
            ("/latest/SHA256SUMS.sig".into(), sign(&key, &sums)),
        ]);
        assert!(latest(&reqwest::Client::new(), &source(&base, &public)).await.unwrap_err().contains("far bigger"));
    }

    #[tokio::test]
    async fn an_older_or_same_release_is_never_installed() {
        let (key, public) = test_key();
        let sums = sums_for("0.1.2", b"a", b"t");
        let (base, asked) = serve(vec![
            ("/latest/SHA256SUMS".into(), sums.clone().into_bytes()),
            ("/latest/SHA256SUMS.sig".into(), sign(&key, &sums)),
        ]);
        let p = Mutex::new(Progress::default());
        let r = update(&reqwest::Client::new(), &source(&base, &public), &Install::AppImage("/nonexistent/x".into()), &p, "0.1.2", &|| Ok(()))
            .await;
        assert!(r.unwrap_err().contains("already the newest"));
        assert_eq!(asked.lock().unwrap().len(), 2);
    }

    /// A gzipped tar of a Truthcoin App.app with this Info.plist (needing macOS 14.0).
    fn bundle_tarball(id: &str, version: &str) -> Vec<u8> {
        let plist = format!(
            "<?xml version=\"1.0\"?>\n<plist version=\"1.0\">\n<dict>\n\t<key>CFBundleExecutable</key>\n\t<string>truthcoin-app</string>\n\t<key>CFBundleIdentifier</key>\n\t<string>{id}</string>\n\t<key>CFBundleShortVersionString</key>\n\t<string>{version}</string>\n\t<key>LSMinimumSystemVersion</key>\n\t<string>14.0</string>\n</dict>\n</plist>\n"
        );
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for (path, body, mode) in [
            ("Truthcoin App.app/Contents/Info.plist", plist.as_bytes(), 0o644),
            // Set-id and group-writable in the archive: unpacked without either.
            ("Truthcoin App.app/Contents/MacOS/truthcoin-app", &b"new program"[..], 0o4775),
        ] {
            let mut h = tar::Header::new_gnu();
            h.set_size(body.len() as u64);
            h.set_mode(mode);
            h.set_cksum();
            b.append_data(&mut h, path, body).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn a_mac_bundle_is_swapped_whole_or_not_at_all() {
        let dir = std::env::temp_dir().join(format!("tc-swap-{}", std::process::id()));
        let bundle = dir.join("Truthcoin App.app");
        let reset = || {
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
            std::fs::write(bundle.join("Contents/MacOS/truthcoin-app"), b"old program").unwrap();
        };
        let left = || std::fs::read_dir(&dir).unwrap().count();
        let archive = std::env::temp_dir().join(format!("tc-swap-{}.tar.gz", std::process::id()));
        let open = || std::fs::File::open(&archive).unwrap();

        reset();
        std::fs::write(&archive, bundle_tarball(IDENTIFIER, "9.0.0")).unwrap();
        assert_eq!(swap_bundle(open(), &bundle, "9.0.0", Some((15, 1, 0))), Ok(()));
        assert_eq!(std::fs::read(bundle.join("Contents/MacOS/truthcoin-app")).unwrap(), b"new program");
        assert_eq!(left(), 1, "the old bundle and the unpacked copy are gone");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(bundle.join("Contents/MacOS/truthcoin-app")).unwrap().permissions().mode();
            assert_eq!(mode & 0o7777, 0o755, "executable; no set-id bit, nothing group-writable");
        }

        // Another app, another version than the signed checksums named, or one this Mac's macOS is too old for:
        // the old bundle stays, untouched.
        for (id, v, os, why) in [
            ("com.example.other", "9.0.0", None, "doesn't look right"),
            (IDENTIFIER, "9.0.1", None, "doesn't look right"),
            (IDENTIFIER, "9.0.0", Some((13, 6, 1)), "needs macOS 14.0"),
        ] {
            reset();
            std::fs::write(&archive, bundle_tarball(id, v)).unwrap();
            assert!(swap_bundle(open(), &bundle, "9.0.0", os).unwrap_err().contains(why), "{id} {v} {os:?}");
            assert_eq!(std::fs::read(bundle.join("Contents/MacOS/truthcoin-app")).unwrap(), b"old program");
            assert_eq!(left(), 1);
        }

        // Where the disk can't exchange in one step: two renames, with the same result.
        reset();
        let new = dir.join("new.app");
        std::fs::create_dir_all(new.join("Contents/MacOS")).unwrap();
        std::fs::write(new.join("Contents/MacOS/truthcoin-app"), b"new program").unwrap();
        exchange(&new, &bundle, false).unwrap();
        assert_eq!(std::fs::read(bundle.join("Contents/MacOS/truthcoin-app")).unwrap(), b"new program");
        assert_eq!(std::fs::read(new.join("Contents/MacOS/truthcoin-app")).unwrap(), b"old program");
        assert_eq!(left(), 2, "no hidden copy left beside them");
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_file(&archive).unwrap();
    }

    #[tokio::test]
    async fn a_mac_bundle_updates_from_the_downloaded_file() {
        // The whole Mac path through update(): the tarball is unpacked from the very handle it was downloaded into.
        // (Served under this system's package name, so it runs on Linux too.)
        let (key, public) = test_key();
        let v = "9.0.0";
        let tarball = bundle_tarball(IDENTIFIER, v);
        let name = asset(v).expect("this system has an update package");
        let sums = format!("{}  {name}\n", sha(&tarball));
        let (base, _) = serve(vec![
            ("/latest/SHA256SUMS".into(), sums.clone().into_bytes()),
            ("/latest/SHA256SUMS.sig".into(), sign(&key, &sums)),
            (format!("/v{v}/{name}"), tarball),
        ]);
        let dir = std::env::temp_dir().join(format!("tc-bundle-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bundle = dir.join("Truthcoin App.app");
        std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        std::fs::write(bundle.join("Contents/MacOS/truthcoin-app"), b"old program").unwrap();
        let p = Mutex::new(Progress::default());
        let r = update(&reqwest::Client::new(), &source(&base, &public), &Install::Bundle(bundle.clone()), &p, "0.1.2", &|| Ok(()))
            .await;
        assert_eq!(r, Ok(v.to_string()));
        assert_eq!(std::fs::read(bundle.join("Contents/MacOS/truthcoin-app")).unwrap(), b"new program");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "the download, the unpacking folder and the old bundle are gone");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// One answer with no Content-Length: the body runs until the connection closes.
    fn serve_unsized(body: Vec<u8>) -> String {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                        break;
                    }
                }
                let mut s = stream;
                let _ = write!(s, "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
                let _ = s.write_all(&body);
            }
        });
        base
    }

    #[tokio::test]
    async fn small_files_are_capped_even_without_a_length() {
        let http = reqwest::Client::new();
        let big = serve_unsized(vec![b'0'; 100 * 1024]);
        assert!(download_bytes(&http, &format!("{big}/SHA256SUMS")).await.unwrap_err().contains("far bigger"));
        let small = serve_unsized(b"abc  Truthcoin-App_9.0.0_amd64.AppImage\n".to_vec());
        assert_eq!(
            download_bytes(&http, &format!("{small}/SHA256SUMS")).await,
            Ok(Some(b"abc  Truthcoin-App_9.0.0_amd64.AppImage\n".to_vec()))
        );
    }

    #[test]
    fn the_test_server_must_be_on_this_computer() {
        assert!(loopback("http://127.0.0.1:8765"));
        assert!(loopback("http://[::1]:8765"));
        for far in [
            "https://127.0.0.1:8765",
            "http://10.0.0.1:8765",
            "http://example.com",
            "http://localhost:8765",
            "http://u@127.0.0.1:8765",
            "http://127.0.0.1:8765/x",
            "http://127.0.0.1:8765/?a",
            "not a url",
        ] {
            assert!(!loopback(far), "{far}");
        }
    }

    #[test]
    fn leftovers_go_only_by_name_and_age() {
        let dir = std::env::temp_dir().join(format!("tc-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".truthcoin-app-old-0123456789abcdef.app/Contents")).unwrap();
        std::fs::write(dir.join(".truthcoin-app-update-0123456789abcdef"), b"half a download").unwrap();
        std::fs::write(dir.join(".truthcoin-app-update-xyz"), b"not ours").unwrap();
        std::fs::write(dir.join("Truthcoin-App_0.1.2_amd64.AppImage"), b"the app").unwrap();
        let outside = dir.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep"), b"k").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, dir.join(".truthcoin-app-update-fedcba9876543210")).unwrap();
        // Too young: nothing goes.
        sweep(&dir, Duration::from_secs(3600));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), if cfg!(unix) { 6 } else { 5 });
        sweep(&dir, Duration::ZERO);
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        let mut want = vec![".truthcoin-app-update-xyz", "Truthcoin-App_0.1.2_amd64.AppImage", "outside"];
        if cfg!(unix) {
            want.insert(0, ".truthcoin-app-update-fedcba9876543210");
        }
        assert_eq!(left, want, "only the two leftovers went; never through a link");
        assert!(outside.join("keep").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
