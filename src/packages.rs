//! Epson programs the app can fetch for the user. They are Epson's own,
//! proprietary and not ours to redistribute, so they are never bundled: the
//! app downloads them from Epson's server on request and runs a file only
//! when it is byte for byte the one recorded here and carries a valid
//! signature from Seiko Epson.
#![forbid(unsafe_code)]

/// The only host the downloader talks to.
pub const HOST: &str = "download.epson-europe.com";
/// Organisation (O) of the code signing certificate both files carry.
pub const SIGNER_ORG: &str = "SEIKO EPSON CORPORATION";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Package {
    pub file: &'static str,
    pub path: &'static str,
    pub size: u64,
    /// SHA-256, lower-case hex.
    pub sha256: &'static str,
}

/// Epson's ET-2870 printer driver (package `ET2870_STD_WW_38005_W64`,
/// 64-bit Windows). Recorded 2026-10-20: Authenticode valid, signed by
/// `SEIKO EPSON CORPORATION` (`DigiCert` Trusted G4 Code Signing RSA4096 SHA384
/// 2021 CA1).
pub const DRIVER: Package = Package {
    file: "epson680525eu.exe",
    path: "/pub/download/6805/epson680525eu.exe",
    size: 72_138_664,
    sha256: "5175b9b0d3701fd29ffe844b4b4652c35c986a3b0494a531a596553f1aba99e8",
};

/// Epson Connection Checker. Recorded 2026-10-20, same signer.
pub const CHECKER: Package = Package {
    file: "epson677705eu.exe",
    path: "/pub/download/6777/epson677705eu.exe",
    size: 8_237_384,
    sha256: "b30f5a7157eb05d3af7919765bb25d90a99cdb7054452494fb237fb1ffc2a64c",
};

impl Package {
    #[must_use]
    pub fn url(&self) -> String {
        format!("https://{HOST}{}", self.path)
    }

    /// True when `digest` is this package's recorded SHA-256.
    #[must_use]
    pub fn matches(&self, digest: &[u8; 32]) -> bool {
        hex(digest) == self.sha256
    }
}

#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packages_are_well_formed() {
        for p in [DRIVER, CHECKER] {
            assert_eq!(p.sha256.len(), 64);
            assert!(p.sha256.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
            assert!(p.path.ends_with(p.file));
            assert!(
                p.url()
                    .starts_with("https://download.epson-europe.com/pub/download/")
            );
        }
    }

    #[test]
    fn digest_comparison() {
        let mut d = [0u8; 32];
        assert!(!DRIVER.matches(&d));
        let want: Vec<u8> = (0..32)
            .map(|i| u8::from_str_radix(&DRIVER.sha256[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        d.copy_from_slice(&want);
        assert!(DRIVER.matches(&d));
        assert!(!CHECKER.matches(&d));
    }
}
