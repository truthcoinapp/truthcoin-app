//! The Truthcoin node this app runs: L2L's `truthcoin_dc`, installed by the app (`install.rs`, pinned by hash in
//! `pins.rs`), started as the app's child with its own data folder and ports, and stopped when the app quits.
//! BitWindow provides the rest of the stack: eCash's node and the enforcer, whose gRPC the node follows.
//!
//! Ports: the read-only RPC on `rpc_port` (16013 by default, so a Truthcoin that BitWindow runs on 6013 never
//! clashes), the wallet and node-control calls on a private port picked at random at each start, P2P on `p2p_addr`.
//! The child is watched by a thread of its own; on Linux it is told to stop if the app dies.
//!
//! Truthcoin 0.20 can't read what 0.19 wrote, the wallet included. Before it first starts in a folder 0.19 used, the
//! app moves 0.19's data into `node/set-aside-0.19/` (`set_aside_old_data`), until Obliterate removes it with the rest
//! of `node/`.

pub mod enforcer;
pub mod install;
pub mod pins;

use crate::rpc::Rpc;
use crate::settings::Settings;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the node is doing, for the screens.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum RunState {
    Stopped,
    Starting,
    Running,
    Stopping,
    /// It stopped by itself, or never answered: the message and the end of its log.
    Failed { message: String },
}

struct Child {
    pid: u32,
    private_host: String,
    private_port: u16,
    rpc_port: u16,
    started: Instant,
    /// Set by the watching thread when the process ends.
    exited: Arc<Mutex<Option<String>>>,
}

pub struct Node {
    pub dir: PathBuf,
    pub http: reqwest::Client,
    pub settings: Mutex<Settings>,
    state: Mutex<RunState>,
    child: Mutex<Option<Child>>,
    pub install: Mutex<install::InstallProgress>,
    /// Start and stop one at a time (review L12).
    op: tokio::sync::Mutex<()>,
    /// "Obliterate" is removing Truthcoin: no start or install until it is done (obliterate.rs).
    pub removing: std::sync::atomic::AtomicBool,
    /// Told when 0.19's data was set aside, so what the app holds about that wallet goes too (lib.rs).
    pub on_set_aside: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>,
}

/// Where 0.19's data goes inside `node/` when 0.20 first starts there.
pub const SET_ASIDE: &str = "set-aside-0.19";
/// Written in `node/` once 0.20 uses it.
const FORMAT_FILE: &str = "app-format";
const FORMAT_V20: &str = "0.20";
/// What the app records about the wallet beside `node/`, which goes with 0.19's data.
const WALLET_RECORDS: &[&str] = &["wallet-ready", "trades.json", "withdrawals.json", "deposits.json"];

/// The node this app started: its pid and program (so a later launch can stop one left by a crash) and its ports
/// (so a developer's scripts can reach it). Owner-only.
const PID_FILE: &str = "node.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct PidFile {
    pid: u32,
    program: String,
    rpc_port: u16,
    #[serde(default)]
    private_host: Option<String>,
    private_port: u16,
}

impl Node {
    pub fn new(dir: PathBuf, http: reqwest::Client) -> Node {
        let settings = Settings::load(&dir);
        Node {
            dir,
            http,
            settings: Mutex::new(settings),
            state: Mutex::new(RunState::Stopped),
            child: Mutex::new(None),
            install: Mutex::new(Default::default()),
            op: tokio::sync::Mutex::new(()),
            removing: std::sync::atomic::AtomicBool::new(false),
            on_set_aside: std::sync::OnceLock::new(),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    pub fn datadir(&self) -> PathBuf {
        self.dir.join("node")
    }

    pub fn log_path(&self) -> PathBuf {
        self.datadir().join("app-node.log")
    }

    /// The node's RPC, while it runs.
    pub fn rpc(&self) -> Option<Rpc> {
        let c = self.child.lock().unwrap();
        c.as_ref().map(|c| Rpc::new(self.http.clone(), c.rpc_port, &c.private_host, c.private_port))
    }

    /// The node's RPC, or the reason there is none, for commands.
    pub fn rpc_or_err(&self) -> Result<Rpc, String> {
        self.rpc().ok_or_else(|| "The Truthcoin node isn't running".to_string())
    }

    pub fn state(&self) -> RunState {
        // A child that ended by itself turns Running/Starting into Failed.
        let ended = {
            let c = self.child.lock().unwrap();
            c.as_ref().and_then(|c| c.exited.lock().unwrap().clone())
        };
        if let Some(msg) = ended {
            let mut s = self.state.lock().unwrap();
            if matches!(*s, RunState::Running | RunState::Starting) {
                *s = RunState::Failed { message: format!("The node stopped: {msg}\n{}", self.log_tail(12)) };
            } else if *s == RunState::Stopping {
                *s = RunState::Stopped;
            }
            *self.child.lock().unwrap() = None;
            let _ = std::fs::remove_file(self.dir.join(PID_FILE));
        }
        self.state.lock().unwrap().clone()
    }

    fn set_state(&self, s: RunState) {
        *self.state.lock().unwrap() = s;
    }

    /// The last lines of the node's log.
    pub fn log_tail(&self, lines: usize) -> String {
        let t = std::fs::read_to_string(self.log_path()).unwrap_or_default();
        let v: Vec<&str> = t.lines().collect();
        v[v.len().saturating_sub(lines)..].join("\n")
    }

    /// The program to run: the developer's own if Settings names one, else the pinned release, checked again.
    pub fn program(&self) -> Result<PathBuf, String> {
        if let Some(p) = self.settings().locked().node_binary {
            return if p.exists() { Ok(p) } else { Err(format!("{} doesn't exist", p.display())) };
        }
        install::check_installed(&self.dir)
    }

    /// 0.19's data, if it was set aside (for Setup's notice).
    pub fn set_aside_path(&self) -> Option<PathBuf> {
        let p = self.datadir().join(SET_ASIDE);
        p.is_dir().then_some(p)
    }

    pub fn installed(&self) -> bool {
        self.settings().locked().node_binary.is_some() || install::installed_path(&self.dir).exists()
    }

    /// Start the node and wait until it answers.
    pub async fn start(self: &Arc<Self>) -> Result<(), String> {
        let _op = self.op.lock().await;
        if self.removing.load(std::sync::atomic::Ordering::SeqCst) || crate::files::stopped() {
            return Err("Obliterate is removing Truthcoin from this computer".into());
        }
        if matches!(self.state(), RunState::Running | RunState::Starting) {
            return Ok(());
        }
        self.set_state(RunState::Starting);
        match self.start_inner().await {
            Ok(()) => {
                self.set_state(RunState::Running);
                crate::activity::note(&self.dir, "node started");
                Ok(())
            }
            Err(e) => {
                self.kill_child();
                self.set_state(RunState::Failed { message: e.clone() });
                crate::activity::note(&self.dir, &format!("node failed to start: {}", e.lines().next().unwrap_or("")));
                Err(e)
            }
        }
    }

    async fn start_inner(self: &Arc<Self>) -> Result<(), String> {
        // A node a crash left on this folder is stopped first, whether or not this start gets far.
        stop_leftover(&self.dir).await;
        let s = self.settings().locked();
        let program = self.program()?;
        let (eh, ep) = s.enforcer_host_port()?;
        enforcer::chain(&s.enforcer).await.map_err(|e| {
            format!("The enforcer isn't answering at {} ({e}). Start eCash in BitWindow first.", s.enforcer)
        })?;
        if crate::settings::BETA_ONLY {
            enforcer::check_ecash_beta(&s.enforcer).await?;
        }
        if !port_free(s.rpc_port) {
            return Err(format!(
                "Port {} is in use, so the node can't take it. Is another copy of this app running? (Settings › \
                 Advanced can move it.)",
                s.rpc_port
            ));
        }
        // The wallet's calls: a random port, and on Linux a random address in 127.0.0.0/8 too, so a web page can't find
        // them by scanning 127.0.0.1 (the node allows any web page; review H1). Never the read-only port (N6).
        let (private_host, private_port) = private_endpoint(s.rpc_port).ok_or("no free port for the node's wallet calls")?;
        if cfg!(target_os = "linux") && private_host == "127.0.0.1" {
            crate::activity::note(&self.dir, "random local addresses refused: the wallet's calls are on 127.0.0.1");
        }
        let datadir = self.datadir();
        if let Some(to) = set_aside_old_data(&self.dir).map_err(|e| format!("Can't set Truthcoin 0.19's data aside: {e}"))? {
            crate::activity::note(&self.dir, &format!("Truthcoin 0.19's data and wallet set aside in {}", to.display()));
            if let Some(f) = self.on_set_aside.get() {
                f();
            }
        }
        crate::files::private_dir(&datadir).map_err(|e| e.to_string())?;
        let log = self.log_path();
        if log.exists() {
            let _ = std::fs::rename(&log, log.with_extension("log.1"));
        }
        let out = std::fs::File::create(&log).map_err(|e| e.to_string())?;
        let err = out.try_clone().map_err(|e| e.to_string())?;
        let mut cmd = std::process::Command::new(&program);
        if !s.node_args.iter().any(|a| a == "--log-level") {
            cmd.args(["--log-level", "info"]);
        }
        cmd.arg("--headless")
            .args(["--network", &s.network])
            .arg("--datadir")
            .arg(&datadir)
            .args(["--mainchain-grpc-url", &grpc_url(&eh, ep)])
            .args(["--rpc-addr", &format!("127.0.0.1:{}", s.rpc_port)])
            .args(["--private-rpc-addr", &format!("{private_host}:{private_port}")])
            .args(["--net-addr", &s.p2p_addr])
            .args(&s.node_args)
            // Info logs only, whatever the environment: at trace level the node logs every RPC request and answer,
            // the recovery words among them (review L4).
            .env_remove("RUST_LOG")
            .stdin(std::process::Stdio::null())
            .stdout(out)
            .stderr(err);
        let exited = Arc::new(Mutex::new(None));
        let pid = spawn_watched(cmd, exited.clone())?;
        let _ = crate::files::write_json(
            &self.dir.join(PID_FILE),
            &PidFile {
                pid,
                program: program.display().to_string(),
                rpc_port: s.rpc_port,
                private_host: Some(private_host.clone()),
                private_port,
            },
        );
        *self.child.lock().unwrap() = Some(Child {
            pid,
            private_host: private_host.clone(),
            private_port,
            rpc_port: s.rpc_port,
            started: Instant::now(),
            exited: exited.clone(),
        });
        let rpc = Rpc::new(self.http.clone(), s.rpc_port, &private_host, private_port);
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if let Some(msg) = exited.lock().unwrap().clone() {
                return Err(format!("The node stopped while starting: {msg}\n{}", self.log_tail(15)));
            }
            if rpc.public::<u64>("getblockcount", json!([])).await.is_ok()
                && rpc.private::<Value>("balance", json!([])).await.is_ok()
            {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(format!("The node didn't answer within 90 seconds.\n{}", self.log_tail(15)));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    fn kill_child(&self) {
        // The pid file goes only with the child it names: one left by a crash stays, so it can still be stopped
        // (security re-review L3).
        if let Some(c) = self.child.lock().unwrap().take() {
            signal(c.pid, Sig::Kill);
            let _ = std::fs::remove_file(self.dir.join(PID_FILE));
        }
    }

    /// Stop the node: its own `stop`, then SIGTERM, then SIGKILL.
    pub async fn stop(&self) {
        let _op = self.op.lock().await;
        let (pid, exited) = {
            let c = self.child.lock().unwrap();
            match c.as_ref() {
                Some(c) => (c.pid, c.exited.clone()),
                None => {
                    self.set_state(RunState::Stopped);
                    return;
                }
            }
        };
        self.set_state(RunState::Stopping);
        if let Some(rpc) = self.rpc() {
            let _ = rpc.private::<Value>("stop", json!([])).await;
        }
        let done = || exited.lock().unwrap().is_some();
        for step in 0..3 {
            let wait = if step == 0 { 20 } else { 10 };
            for _ in 0..wait * 4 {
                if done() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            if done() {
                break;
            }
            signal(pid, if step == 0 { Sig::Term } else { Sig::Kill });
        }
        *self.child.lock().unwrap() = None;
        let _ = std::fs::remove_file(self.dir.join(PID_FILE));
        self.set_state(RunState::Stopped);
        crate::activity::note(&self.dir, "node stopped");
    }

    /// Stop at once, for the app's exit (blocking).
    pub fn stop_blocking(&self) {
        let pid = self.child.lock().unwrap().as_ref().map(|c| c.pid);
        let Some(pid) = pid else { return };
        if let Some(rpc) = self.rpc() {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build();
            if let Ok(rt) = rt {
                let _ = rt.block_on(async {
                    tokio::time::timeout(Duration::from_secs(5), rpc.private::<Value>("stop", json!([]))).await
                });
            }
        }
        let exited = self.child.lock().unwrap().as_ref().map(|c| c.exited.clone());
        let gone = || exited.as_ref().map(|e| e.lock().unwrap().is_some()).unwrap_or(true);
        for _ in 0..60 {
            if gone() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if !gone() {
            signal(pid, Sig::Term);
            for _ in 0..40 {
                if gone() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            if !gone() {
                signal(pid, Sig::Kill);
            }
        }
        let _ = std::fs::remove_file(self.dir.join(PID_FILE));
    }

    pub fn wallet_host(&self) -> Option<String> {
        self.child.lock().unwrap().as_ref().map(|c| c.private_host.clone())
    }

    pub fn uptime_secs(&self) -> Option<u64> {
        self.child.lock().unwrap().as_ref().map(|c| c.started.elapsed().as_secs())
    }
}

enum Sig {
    Term,
    Kill,
}

fn signal(pid: u32, s: Sig) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, match s {
            Sig::Term => libc::SIGTERM,
            Sig::Kill => libc::SIGKILL,
        });
    }
    #[cfg(not(unix))]
    let _ = (pid, s);
}

/// Start `cmd` from a thread that lives as long as the child and waits for it, recording how it ended. On Linux the
/// child gets SIGTERM if that thread (and so the app) goes away.
fn spawn_watched(mut cmd: std::process::Command, exited: Arc<Mutex<Option<String>>>) -> Result<u32, String> {
    #[cfg(target_os = "linux")]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("truthcoin-node".into())
        .spawn(move || match cmd.spawn() {
            Ok(mut child) => {
                let _ = tx.send(Ok(child.id()));
                let how = match child.wait() {
                    Ok(st) => st.to_string(),
                    Err(e) => e.to_string(),
                };
                *exited.lock().unwrap() = Some(how);
            }
            Err(e) => {
                let _ = tx.send(Err(format!("can't start the node program: {e}")));
            }
        })
        .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

/// For Obliterate: stop a node a crash left behind on this data folder.
/// The enforcer's gRPC as a URL (`--mainchain-grpc-url`), with an IPv6 address in brackets.
fn grpc_url(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("http://[{host}]:{port}")
    } else {
        format!("http://{host}:{port}")
    }
}

/// Before 0.20 first uses `<dir>/node`: anything 0.19 left there (the chain and the wallet, which
/// 0.20 refuses to open) goes into `node/set-aside-0.19/`, and so do the app's records of that wallet. Nothing is
/// deleted. Returns where it went, when anything did. Once 0.20 has the folder, `app-format` says so.
fn set_aside_old_data(dir: &Path) -> std::io::Result<Option<PathBuf>> {
    let node = dir.join("node");
    let format = node.join(FORMAT_FILE);
    if std::fs::read_to_string(&format).is_ok_and(|f| f.trim() == FORMAT_V20) {
        return Ok(None);
    }
    let keep = |n: &str| n == SET_ASIDE || n == FORMAT_FILE || n.starts_with("app-node.log");
    let mut old: Vec<PathBuf> = match std::fs::read_dir(&node) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| !keep(&e.file_name().to_string_lossy()))
            .map(|e| e.path())
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(e) => return Err(e),
    };
    let to = node.join(SET_ASIDE);
    if !old.is_empty() {
        old.extend(WALLET_RECORDS.iter().map(|r| dir.join(r)).filter(|r| r.symlink_metadata().is_ok()));
        crate::files::private_dir(&to)?;
        for from in &old {
            let name = from.file_name().unwrap_or_default();
            let mut dest = to.join(name);
            let mut n = 2;
            while dest.symlink_metadata().is_ok() {
                dest = to.join(format!("{}-{n}", name.to_string_lossy()));
                n += 1;
            }
            std::fs::rename(from, &dest)?;
        }
    }
    crate::files::private_dir(&node)?;
    crate::files::write_private(&format, FORMAT_V20.as_bytes())?;
    Ok((!old.is_empty()).then_some(to))
}

pub async fn stop_leftover_in(dir: &Path) {
    stop_leftover(dir).await
}

/// A node an earlier run of this app started and left behind (a crash): stop it if it is still our program.
async fn stop_leftover(dir: &Path) {
    let p = dir.join(PID_FILE);
    let Ok(Some(f)) = crate::files::read_json::<PidFile>(&p) else {
        let _ = std::fs::remove_file(&p);
        return;
    };
    let (pid, prog) = (f.pid, f.program.as_str());
    if is_our_program(pid, prog) {
        signal(pid, Sig::Term);
        for _ in 0..80 {
            if !is_our_program(pid, prog) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if is_our_program(pid, prog) {
            signal(pid, Sig::Kill);
        }
    }
    let _ = std::fs::remove_file(&p);
}

#[cfg(target_os = "linux")]
fn is_our_program(pid: u32, prog: &str) -> bool {
    std::fs::read_link(format!("/proc/{pid}/exe")).map(|e| e.to_string_lossy() == prog).unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn is_our_program(pid: u32, prog: &str) -> bool {
    // macOS: ask ps for the process's command; it must be our program.
    std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == prog)
        .unwrap_or(false)
}

/// Where the node's wallet calls listen: (address, port). On Linux all of 127.0.0.0/8 is this computer, so the address
/// is picked at random there too; elsewhere it is 127.0.0.1. The port is never `not`.
fn private_endpoint(not: u16) -> Option<(String, u16)> {
    #[cfg(target_os = "linux")]
    for _ in 0..8 {
        let [a, b, c]: [u8; 3] = rand::random();
        if a == 0 || b == 0 || c == 0 || c == 255 {
            continue;
        }
        let host = format!("127.{a}.{b}.{c}");
        if let Ok(l) = std::net::TcpListener::bind((host.as_str(), 0)) {
            let port = l.local_addr().ok()?.port();
            if port != not {
                return Some((host, port));
            }
        }
    }
    (0..8).filter_map(|_| free_port()).find(|p| *p != not).map(|p| ("127.0.0.1".to_string(), p))
}

pub fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// A free port on 127.0.0.1, picked by the system.
pub fn free_port() -> Option<u16> {
    std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?.local_addr().ok().map(|a| a.port())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_watched_child_reports_how_it_ended() {
        let exited = Arc::new(Mutex::new(None));
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.args(["-c", "exit 3"]);
        let pid = spawn_watched(cmd, exited.clone()).unwrap();
        assert!(pid > 0);
        for _ in 0..100 {
            if exited.lock().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(exited.lock().unwrap().as_deref().unwrap().contains('3'));
    }

    #[test]
    fn the_wallet_endpoint_is_local_and_not_the_rpc_port() {
        let (host, port) = private_endpoint(16013).unwrap();
        assert!(host.starts_with("127.") && port != 16013);
        #[cfg(target_os = "linux")]
        assert_ne!(host, "127.0.0.1", "a random loopback address on Linux");
        // Something may take a port between picking and binding (tests run in parallel): one of a few must bind.
        assert!((0..5).any(|_| {
            let (h, p) = private_endpoint(16013).unwrap();
            std::net::TcpListener::bind((h.as_str(), p)).is_ok()
        }));
    }

    #[test]
    fn free_ports_are_free() {
        // Tests run in parallel and also take ports, so one may grab the port between the two calls: try a few.
        assert!((0..5).any(|_| port_free(free_port().unwrap())));
        let held = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        assert!(!port_free(held.local_addr().unwrap().port()));
    }

    #[test]
    fn the_enforcer_is_given_as_a_url() {
        assert_eq!(grpc_url("127.0.0.1", 50051), "http://127.0.0.1:50051");
        assert_eq!(grpc_url("fd7a:115c::1", 50051), "http://[fd7a:115c::1]:50051");
    }

    #[test]
    fn data_from_019_is_set_aside_once_and_nothing_is_deleted() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path();
        // A new install: nothing to set aside; the folder is marked as 0.20's.
        let fresh = dir.join("fresh");
        assert_eq!(set_aside_old_data(&fresh).unwrap(), None);
        assert_eq!(std::fs::read_to_string(fresh.join("node").join(FORMAT_FILE)).unwrap(), FORMAT_V20);

        // A folder 0.19 used: its data and the wallet's records move into node/set-aside-0.19/; the node's log stays.
        let node = dir.join("node");
        std::fs::create_dir_all(node.join("data.mdb")).unwrap();
        std::fs::write(node.join("data.mdb").join("data.mdb"), b"chain").unwrap();
        std::fs::create_dir_all(node.join("wallet.mdb")).unwrap();
        std::fs::write(node.join("app-node.log"), b"log").unwrap();
        for r in ["wallet-ready", "trades.json", "withdrawals.json"] {
            std::fs::write(dir.join(r), r.as_bytes()).unwrap();
        }
        std::fs::write(dir.join("settings.json"), b"{}").unwrap();
        let to = set_aside_old_data(dir).unwrap().expect("set aside");
        assert_eq!(to, node.join(SET_ASIDE));
        assert_eq!(std::fs::read(to.join("data.mdb").join("data.mdb")).unwrap(), b"chain");
        assert!(to.join("wallet.mdb").is_dir() && to.join("wallet-ready").is_file() && to.join("trades.json").is_file());
        assert!(!node.join("data.mdb").exists() && !dir.join("wallet-ready").exists() && !dir.join("trades.json").exists());
        assert!(node.join("app-node.log").is_file() && dir.join("settings.json").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&to).unwrap().permissions().mode() & 0o777, 0o700);
        }

        // Once 0.20 has the folder, what it writes stays where it is.
        std::fs::create_dir_all(node.join("data.mdb")).unwrap();
        std::fs::write(dir.join("wallet-ready"), b"").unwrap();
        assert_eq!(set_aside_old_data(dir).unwrap(), None);
        assert!(node.join("data.mdb").is_dir() && dir.join("wallet-ready").is_file());

        // Interrupted before the mark: a second pass adds to the same folder, renaming what would clash.
        std::fs::remove_file(node.join(FORMAT_FILE)).unwrap();
        assert_eq!(set_aside_old_data(dir).unwrap(), Some(to.clone()));
        assert!(to.join("data.mdb").is_dir() && to.join("data.mdb-2").is_dir() && to.join("wallet-ready-2").is_file());
    }
}
