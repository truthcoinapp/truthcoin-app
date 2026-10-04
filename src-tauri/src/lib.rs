mod activity;
mod commands;
mod create;
mod files;
mod markets;
mod node;
mod phone;
mod rpc;
mod settings;
mod state;
mod trades;
mod update;
mod wallet;
#[cfg(test)]
mod realnode;

use state::AppState;
use std::sync::{Arc, Mutex};
use tauri::Manager;

/// Files the app was started with, other than stdin, stdout and stderr, stay out of the programs it starts. An
/// AppImage's runtime hands the app its mount (fd 1023) and a keep-alive pipe; a node that inherited them would keep
/// the closed app's AppImage mounted. Called first in main; Linux only.
pub fn keep_inherited_files_from_children() {
    #[cfg(target_os = "linux")]
    {
        let Ok(dir) = std::fs::read_dir("/proc/self/fd") else { return };
        let fds: Vec<i32> =
            dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok()).filter(|fd| *fd > 2).collect();
        for fd in fds {
            // SAFETY: fcntl on a number that is no longer open fails with EBADF, and is then skipped.
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags >= 0 {
                    libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
                }
            }
        }
    }
}

/// Hold an exclusive lock on `<dir>/lock` for the app's life. False if another copy holds it.
fn single_instance(dir: &std::path::Path) -> bool {
    static LOCK: std::sync::OnceLock<std::fs::File> = std::sync::OnceLock::new();
    let Ok(f) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("lock")) else {
        return true;
    };
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY: flock on an open descriptor we own.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return false;
        }
    }
    let _ = LOCK.set(f);
    true
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // TRUTHCOIN_APP_DIR runs the app on a folder of its own (developers, tests).
            let dir = match std::env::var_os("TRUTHCOIN_APP_DIR") {
                Some(d) => d.into(),
                None => app.path().app_data_dir()?,
            };
            files::private_dir(&dir)?;
            // One copy of the app per data folder: a second would stop the first one's node and answer its phones
            // (review L12). It exits; the first copy's window stays.
            if !single_instance(&dir) {
                eprintln!("Truthcoin App is already running on {}", dir.display());
                std::process::exit(0);
            }
            state::set_app_dir(dir.clone());
            activity::note(&dir, &format!("Truthcoin App {} started", env!("CARGO_PKG_VERSION")));
            // The node's RPC is on this computer: never through a proxy.
            let local = reqwest::Client::builder().no_proxy().build()?;
            let node = Arc::new(node::Node::new(dir.clone(), local));
            let trades = Arc::new(trades::Trades::load(&dir));
            let phone = Arc::new(phone::Phone::new(&dir, node.clone(), trades.clone())?);
            let st = Arc::new(AppState { dir: dir.clone(), node: node.clone(), trades, new_words: Mutex::new(None), phone });
            {
                // Phones paired earlier are served from the start.
                let p = st.phone.clone();
                tauri::async_runtime::spawn(async move { p.ensure_running() });
            }
            app.manage(st);
            // A node already installed starts with the app.
            if node.installed() {
                tauri::async_runtime::spawn(async move {
                    let _ = node.start().await;
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::node_status,
            commands::node_install,
            commands::install_progress,
            commands::node_start,
            commands::node_stop,
            commands::node_log,
            commands::settings_advanced,
            commands::settings_advanced_set,
            commands::markets,
            commands::market,
            commands::positions,
            commands::trade_quote,
            commands::trade_place,
            commands::trades,
            commands::trade_cancel,
            wallet::wallet_status,
            wallet::wallet_new_words,
            wallet::wallet_confirm_words,
            wallet::wallet_restore,
            wallet::wallet_receive,
            wallet::deposit_info,
            wallet::deposit,
            wallet::withdraw,
            wallet::wallet_split,
            create::create_info,
            create::create_cost,
            create::create_market,
            phone::commands::phone_info,
            phone::commands::phone_pair_start,
            phone::commands::phone_pair_state,
            phone::commands::phone_pair_answer,
            phone::commands::phone_pair_cancel,
            phone::commands::phone_revoke,
            phone::commands::phone_records_seen,
            commands::trade_clear,
            phone::commands::phone_set_limit,
            phone::commands::phone_held_answer,
            phone::commands::phone_set_relays,
            update::update_check,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the app");

    app.run(|handle, event| {
        // The node goes down with the app.
        if let tauri::RunEvent::Exit = event {
            if let Some(st) = handle.try_state::<Arc<AppState>>() {
                st.node.stop_blocking();
            }
        }
    });
}

#[cfg(all(test, target_os = "linux"))]
mod fd_tests {
    /// A file the app was started with, like an AppImage's keep-alive pipe, doesn't reach a program it starts.
    #[test]
    fn inherited_files_stay_out_of_children() {
        let mut p = [0; 2];
        assert_eq!(unsafe { libc::pipe(p.as_mut_ptr()) }, 0);
        let sees = |fd: i32| {
            let out = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("[ -e /proc/self/fd/{fd} ] && echo yes || echo no"))
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        assert_eq!(sees(p[0]), "yes", "the test's pipe reaches a child before");
        super::keep_inherited_files_from_children();
        assert_eq!(sees(p[0]), "no");
        for fd in p {
            unsafe { libc::close(fd) };
        }
    }
}
