//! The one printer family this tool supports, with every value traced to the
//! open source projects it comes from. Three projects agree on the keys and
//! the reset map; none of them reports a test on an ET-2871 itself, so the
//! first run on real hardware is the confirmation.
//!
//! Sources (read October 2026):
//! - reinkpy `epson.toml`, family with rkey 0x364A (lists ET-2870/2871/2873/2875).
//! - Epson-Waste-Reset `database.json`, models ET-2870 and ET-1811-family.
//! - `epson_print_conf` `ET-2800` and `ET-2812`/`ET-2814` entries.
#![forbid(unsafe_code)]

/// Epson's USB vendor id.
pub const VENDOR_EPSON: u16 = 0x04B8;

/// Read key (model code) sent with every EEPROM command, little endian on the
/// wire as `4A 36`. reinkpy `rkey = 0x364A`; `epson_print_conf` `read_key [74, 54]`.
pub const READ_KEY: u16 = 0x364A;

/// Write key as transmitted: "Maribaya" with every byte shifted up by one.
/// reinkpy `wkey "Nbsjcbzb"` (sent as stored); `epson_print_conf` sends
/// caesar(b"Maribaya"); Epson-Waste-Reset sends wkey "Nbsjcbzb".
pub const WRITE_KEY: [u8; 8] = *b"Nbsjcbzb";

/// Highest valid EEPROM address for the family (all three sources: 0x7FF).
pub const MEM_HIGH: u16 = 0x07FF;

/// Waste counter bytes and the value each is reset to. All three sources list
/// the same fourteen bytes and values (`epson_print_conf` `raw_waste_reset`,
/// reinkpy `mem` reset map plus zeroed counters, Epson-Waste-Reset pad groups).
pub const RESET_MAP: [(u16, u8); 14] = [
    (0x1C, 0x00),
    (0x2F, 0x00),
    (0x30, 0x00),
    (0x31, 0x00),
    (0x32, 0x00),
    (0x33, 0x00),
    (0x34, 0x00),
    (0x35, 0x00),
    (0x36, 0x5E),
    (0x37, 0x5E),
    (0xFC, 0x00),
    (0xFD, 0x00),
    (0xFE, 0x00),
    (0xFF, 0x5E),
];

/// State byte whose bit 0 keeps the overflow error latched after the counters
/// are zeroed. Epson-Waste-Reset ET-2870 `close: {addr 256, and 254}`;
/// `epson_print_conf` ET-2812/2814 `waste_reset_commit {0x100: 0xFE}`, verified
/// there on an ET-2814 (firmware XF26P8). Unverified on the ET-2871.
pub const COMMIT_ADDR: u16 = 0x100;
/// Only this bit is cleared; the other bits belong to unrelated state.
pub const COMMIT_CLEAR_MASK: u8 = 0x01;

/// Every address the tool reads and backs up. Writes are limited to
/// `RESET_MAP` and `COMMIT_ADDR`.
#[must_use]
pub fn backup_addresses() -> Vec<u16> {
    let mut all: Vec<u16> = RESET_MAP.iter().map(|(a, _)| *a).collect();
    all.push(COMMIT_ADDR);
    all
}

/// True when the IEEE 1284 device id names an Epson ET-2870 series printer.
/// The ET-2871 is sold under the ET-2870 series name; the model list in the
/// sources is ET-2870, ET-2871, ET-2873 and ET-2875.
#[must_use]
pub fn is_supported(manufacturer: &str, model: &str) -> bool {
    // Only Epson's vendor id is ever enumerated, so a missing MFG field is
    // not a reason to refuse; a different one is.
    let maker = manufacturer.trim();
    if !maker.is_empty() && !maker.eq_ignore_ascii_case("EPSON") {
        return false;
    }
    // Spellings vary between firmware and driver: "EPSON ET-2870 Series",
    // "ET-2870 Series", "ET2870". Compare letters and digits only.
    let mut key: String = model
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if let Some(rest) = key.strip_prefix("EPSON") {
        key = rest.to_string();
    }
    if let Some(rest) = key.strip_suffix("SERIES") {
        key = rest.to_string();
    }
    matches!(key.as_str(), "ET2870" | "ET2871" | "ET2873" | "ET2875")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_only_the_2870_series() {
        assert!(is_supported("EPSON", "ET-2870 Series"));
        assert!(is_supported("EPSON", "ET-2871"));
        assert!(!is_supported("EPSON", "ET-2820 Series"));
        assert!(!is_supported("EPSON", "ET-28700"));
        assert!(!is_supported("Canon", "ET-2870 Series"));
        assert!(is_supported("EPSON", "EPSON ET-2870 Series"));
        assert!(is_supported("EPSON", "ET2870 Series"));
        assert!(is_supported("EPSON", "et-2870 series\r\n"));
        assert!(is_supported("", "ET-2870 Series"));
        assert!(!is_supported("EPSON", "ET-2850 Series"));
        assert!(!is_supported("EPSON", "L3270 Series"));
    }

    #[test]
    fn reset_map_is_in_range_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for (addr, _) in RESET_MAP {
            assert!(addr <= MEM_HIGH);
            assert!(seen.insert(addr));
        }
        assert!(!seen.contains(&COMMIT_ADDR));
    }

    #[test]
    fn write_key_is_shifted_maribaya() {
        let shifted: Vec<u8> = b"Maribaya".iter().map(|b| b + 1).collect();
        assert_eq!(shifted, WRITE_KEY);
    }
}
