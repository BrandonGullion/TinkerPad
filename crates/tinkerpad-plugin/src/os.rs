//! The operating systems a plugin section can be limited to.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
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
