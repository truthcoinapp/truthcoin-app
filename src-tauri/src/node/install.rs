//! Installing the Truthcoin node: download L2L's release for this computer from GitHub, hashing as it arrives, and
//! keep it only if its size and SHA-256 match the pin (`pins.rs`). It goes to `<app data>/bin/truthcoin_dc-<version>`.

use super::pins::{self, Pin};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone, Default, Debug)]
pub struct InstallProgress {
    pub running: bool,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub error: Option<String>,
    pub finished: bool,
}

pub fn installed_path(dir: &Path) -> PathBuf {
    dir.join("bin").join(format!("truthcoin_dc-{}", pins::NODE_VERSION))
}

/// Hash a file the way the pin was made.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex::encode(h.finalize()))
}

/// Is the installed node the pinned one? (Checked again before every start: a changed file isn't run.)
pub fn check_installed(dir: &Path) -> Result<PathBuf, String> {
    let pin = pins::this_computer().ok_or("L2L doesn't publish a Truthcoin node for this kind of computer")?;
    let p = installed_path(dir);
    if !p.exists() {
        return Err("not installed".into());
    }
    let got = sha256_file(&p).map_err(|e| format!("can't read the node program: {e}"))?;
    if got != pin.sha256 {
        return Err(format!("the node program at {} isn't the release this app checked (sha256 {got})", p.display()));
    }
    Ok(p)
}

/// Download, check and install. `report` gets the progress as it goes.
pub async fn install(
    http: &reqwest::Client,
    dir: &Path,
    mut report: impl FnMut(InstallProgress),
) -> Result<PathBuf, String> {
    let pin: &Pin = pins::this_computer().ok_or("L2L doesn't publish a Truthcoin node for this kind of computer")?;
    let bin = dir.join("bin");
    crate::files::private_dir(&bin).map_err(|e| format!("can't make {}: {e}", bin.display()))?;
    let part = bin.join(".download");
    let mut prog = InstallProgress { running: true, total_bytes: pin.size, ..Default::default() };
    report(prog.clone());
    let mut resp = http
        .get(pins::url(pin))
        .timeout(std::time::Duration::from_secs(1800))
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: GitHub answered {}", resp.status()));
    }
    let mut f = std::fs::File::create(&part).map_err(|e| format!("can't write {}: {e}", part.display()))?;
    let mut h = Sha256::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("download failed: {e}"))? {
        prog.done_bytes += chunk.len() as u64;
        if prog.done_bytes > pin.size {
            let _ = std::fs::remove_file(&part);
            return Err("the download is bigger than the release this app checked; stopped".into());
        }
        h.update(&chunk);
        f.write_all(&chunk).map_err(|e| format!("can't write the download: {e}"))?;
        report(prog.clone());
    }
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    let got = hex::encode(h.finalize());
    if prog.done_bytes != pin.size || got != pin.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(format!(
            "the download isn't the release this app checked (got {} bytes, sha256 {got}); nothing was installed",
            prog.done_bytes
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let dest = installed_path(dir);
    std::fs::rename(&part, &dest).map_err(|e| format!("can't install the node program: {e}"))?;
    prog.finished = true;
    prog.running = false;
    report(prog);
    Ok(dest)
}
