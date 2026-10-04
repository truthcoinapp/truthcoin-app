//! The app's own files: owner-only, written whole through a temporary file so a crash never leaves half a file.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Create `dir` (and its parents) and make it readable by this user only.
pub fn private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Write `bytes` to `path`, readable by this user only: a temporary file beside it, synced, then renamed over it.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut o = fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// Read a JSON file the app wrote; None when it doesn't exist.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn write_json<T: serde::Serialize>(path: &Path, v: &T) -> io::Result<()> {
    let b = serde_json::to_vec_pretty(v).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_private(path, &b)
}

#[cfg(test)]
mod tests {
    #[test]
    fn written_files_are_owner_only() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.json");
        super::write_json(&p, &serde_json::json!({"a": 1})).unwrap();
        let v: serde_json::Value = super::read_json(&p).unwrap().unwrap();
        assert_eq!(v["a"], 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(super::read_json::<serde_json::Value>(&d.path().join("none")).unwrap().is_none());
    }
}
