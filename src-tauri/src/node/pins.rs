//! The Truthcoin node releases this app runs, pinned by hash. L2L publishes its `truthcoin_dc` builds on GitHub
//! without checksums or signatures, so each build this app accepts is downloaded once by us, hashed, and pinned here.
//! This app's own signed release then vouches for the pin: a download that doesn't match is never run. A newer node
//! comes with a newer app.

pub const NODE_VERSION: &str = "0.20.0";

pub struct Pin {
    pub target: &'static str,
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// github.com/LayerTwo-Labs/truthcoin-dc/releases/tag/v0.20.0 (published 2026-10-08), hashed 2026-10-08.
pub const PINS: &[Pin] = &[
    Pin {
        target: "x86_64-unknown-linux-gnu",
        file: "truthcoin-0.20.0-x86_64-unknown-linux-gnu",
        size: 55_623_304,
        sha256: "c69c7f50dec18e4bf6ee798e64d9d0a4baae2090a0f93474cd4530cf20b19d1e",
    },
    Pin {
        target: "x86_64-apple-darwin",
        file: "truthcoin-0.20.0-x86_64-apple-darwin",
        size: 41_803_668,
        sha256: "ddf9220013b40c92d3385b9e0d3bcf875dc57c4e399101c6684bcf37aa88f4d7",
    },
    Pin {
        target: "aarch64-apple-darwin",
        file: "truthcoin-0.20.0-aarch64-apple-darwin",
        size: 40_450_336,
        sha256: "a7e2bc8636d6967d2a1a18ffcedcd7aabd82bc265e346324d5079994d4fc605f",
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
