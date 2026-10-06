//! The operating systems a plugin section can be limited to.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Windows,
    Macos,
    Linux,
}

impl Os {
    pub const ALL: [Os; 3] = [Os::Windows, Os::Macos, Os::Linux];

    /// The OS this build is running on.
    pub fn current() -> Os {
        match tinkerpad_process::platform::name() {
            "windows" => Os::Windows,
            "macos" => Os::Macos,
            _ => Os::Linux,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::Macos => "macos",
            Os::Linux => "linux",
        }
    }
}

impl std::fmt::Display for Os {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A CPU architecture, for the per-platform files a `[…].acquire` download
/// names (work item 04-08a). Two, because those are the two anyone publishes
/// desktop binaries for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    #[serde(alias = "amd64", alias = "x64")]
    X86_64,
    #[serde(alias = "arm64")]
    Aarch64,
}

impl Arch {
    /// The architecture this build is running on, or `None` on something
    /// neither common nor guessable — where a download must not be offered.
    pub fn current() -> Option<Arch> {
        match std::env::consts::ARCH {
            "x86_64" => Some(Arch::X86_64),
            "aarch64" => Some(Arch::Aarch64),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
        }
    }
}

impl std::fmt::Display for Arch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
