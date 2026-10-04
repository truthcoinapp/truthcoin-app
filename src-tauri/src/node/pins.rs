//! The Truthcoin node releases this app runs, pinned by hash. L2L publishes its `truthcoin_dc` builds on GitHub
//! without checksums or signatures, so each build this app accepts is downloaded once by us, hashed, and pinned here.
//! This app's own signed release then vouches for the pin: a download that doesn't match is never run. A newer node
//! comes with a newer app.

pub const NODE_VERSION: &str = "0.19.0";

pub struct Pin {
    pub target: &'static str,
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// github.com/LayerTwo-Labs/truthcoin-dc/releases/tag/v0.19.0 (published 2026-09-29), hashed 2026-10-04.
pub const PINS: &[Pin] = &[
    Pin {
        target: "x86_64-unknown-linux-gnu",
        file: "truthcoin-0.19.0-x86_64-unknown-linux-gnu",
        size: 56_331_096,
        sha256: "31604f5e306bca15b38df27c8ca454f87acc4fb435350f0e16bf22347b47a838",
    },
    Pin {
        target: "x86_64-apple-darwin",
        file: "truthcoin-0.19.0-x86_64-apple-darwin",
        size: 42_063_712,
        sha256: "7aa3f04f87189bf61c0c5e98f5ff482b33e5b9f5f9e47d72c40020e00ae83764",
    },
    Pin {
        target: "aarch64-apple-darwin",
        file: "truthcoin-0.19.0-aarch64-apple-darwin",
        size: 40_957_792,
        sha256: "3763adf22257e6a022f89509bc62c50c583a984bdf02ecd05e0268724a5693f0",
    },
];

/// The pin for this computer, if L2L builds for it.
pub fn this_computer() -> Option<&'static Pin> {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => return None,
    };
    PINS.iter().find(|p| p.target == target)
}

pub fn url(p: &Pin) -> String {
    format!("https://github.com/LayerTwo-Labs/truthcoin-dc/releases/download/v{NODE_VERSION}/{}", p.file)
}

#[cfg(test)]
mod tests {
    #[test]
    fn pins_are_well_formed() {
        for p in super::PINS {
            assert_eq!(p.sha256.len(), 64);
            assert!(hex::decode(p.sha256).is_ok());
            assert!(p.file.contains(super::NODE_VERSION) && p.file.ends_with(p.target));
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert!(super::this_computer().is_some());
    }
}
