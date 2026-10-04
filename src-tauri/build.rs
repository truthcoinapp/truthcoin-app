fn main() {
    // The release key the app's update check trusts: release/truthcoinapp-release.pub, when it exists. A build without
    // it has no update check (it says so) rather than trusting anything else.
    let key = "../release/truthcoinapp-release.pub";
    println!("cargo:rerun-if-changed={key}");
    let k = std::fs::read_to_string(key).unwrap_or_default();
    let k = k.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
    println!("cargo:rustc-env=TRUTHCOINAPP_RELEASE_KEY={k}");
    tauri_build::build()
}
