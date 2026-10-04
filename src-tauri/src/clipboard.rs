//! Copy buttons (addresses, the pairing link): the clipboard is written from Rust, because the webview's own clipboard
//! can't be relied on (UX re-check N3). On Linux the app serves the copied text only while its clipboard object lives,
//! so a thread keeps it for two minutes.

use std::time::Duration;

#[tauri::command]
pub async fn copy_text(text: String) -> Result<(), String> {
    if text.is_empty() || text.len() > 4096 {
        return Err("nothing to copy".into());
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut cb = match arboard::Clipboard::new() {
            Ok(cb) => cb,
            Err(e) => {
                let _ = tx.send(Err(format!("Couldn't reach the clipboard ({e})")));
                return;
            }
        };
        let r = cb.set_text(text).map_err(|e| format!("Couldn't copy ({e})"));
        let ok = r.is_ok();
        let _ = tx.send(r);
        if ok {
            std::thread::sleep(Duration::from_secs(120));
        }
    });
    rx.await.map_err(|_| "Couldn't copy".to_string())?
}
