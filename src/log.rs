//! Diagnostic transcript. Every byte sent and received is kept, except serial
//! numbers, which are blanked before a line is stored.
#![forbid(unsafe_code)]

use std::fmt::Write as _;

#[derive(Debug, Default)]
pub struct Log {
    lines: Vec<String>,
}

impl Log {
    pub fn line(&mut self, text: impl Into<String>) {
        self.lines.push(text.into());
    }

    pub fn bytes(&mut self, label: &str, data: &[u8]) {
        let clean = redact(data);
        self.lines.push(format!("{label} ({} bytes)", clean.len()));
        self.lines.push(hexdump(&clean));
    }

    #[must_use]
    pub fn text(&self) -> String {
        let mut out = self.lines.join("\r\n");
        out.push_str("\r\n");
        out
    }

    pub fn append(&mut self, other: Log) {
        self.lines.extend(other.lines);
    }
}

/// Hex and ASCII, 16 bytes per row.
#[must_use]
pub fn hexdump(data: &[u8]) -> String {
    let mut out = String::new();
    for (row, chunk) in data.chunks(16).enumerate() {
        let _ = write!(out, "  {:04X}  ", row * 16);
        for i in 0..16 {
            match chunk.get(i) {
                Some(b) => {
                    let _ = write!(out, "{b:02X} ");
                }
                None => out.push_str("   "),
            }
        }
        out.push(' ');
        for b in chunk {
            out.push(if b.is_ascii_graphic() || *b == b' ' {
                char::from(*b)
            } else {
                '.'
            });
        }
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Blanks serial numbers: device id fields (SN, SERN, SERIALNUMBER) and the
/// serial field (header 0x40) of a "@BDC ST2" status report.
#[must_use]
pub fn redact(data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    for key in [&b"SN:"[..], b"SERN:", b"SERIALNUMBER:"] {
        let mut from = 0;
        while let Some(pos) = find_field(&out[from..], key) {
            let start = from + pos + key.len();
            let mut i = start;
            while i < out.len() && out[i] != b';' {
                out[i] = b'*';
                i += 1;
            }
            from = i;
        }
    }
    redact_status_serial(&mut out);
    out
}

/// `redact` for text.
#[must_use]
pub fn redact_text(s: &str) -> String {
    String::from_utf8_lossy(&redact(s.as_bytes())).into_owned()
}

/// A key at the start of the buffer or right after a field separator, so
/// "SN:" does not match inside "ISN:".
fn find_field(hay: &[u8], key: &[u8]) -> Option<usize> {
    hay.windows(key.len()).enumerate().find_map(|(i, w)| {
        let boundary = i == 0 || matches!(hay[i - 1], b';' | b'\n' | b'\r' | b' ');
        (w.eq_ignore_ascii_case(key) && boundary).then_some(i)
    })
}

fn redact_status_serial(out: &mut [u8]) {
    const PREFIX: &[u8] = b"@BDC ST2\r\n";
    let Some(at) = out.windows(PREFIX.len()).position(|w| w == PREFIX) else {
        return;
    };
    let start = at + PREFIX.len();
    if start + 2 > out.len() {
        return;
    }
    let declared = usize::from(u16::from_le_bytes([out[start], out[start + 1]]));
    let end = (start + 2 + declared).min(out.len());
    let mut i = start + 2;
    while i + 2 <= end {
        let (header, len) = (out[i], usize::from(out[i + 1]));
        i += 2;
        let stop = (i + len).min(end);
        if header == 0x40 {
            out[i..stop].fill(b'*');
        }
        i = stop;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blanks_device_id_serials() {
        let r = redact(b"MFG:EPSON;MDL:ET-2870 Series;SN:X5AB012345;CLS:PRINTER;");
        assert_eq!(
            r,
            b"MFG:EPSON;MDL:ET-2870 Series;SN:**********;CLS:PRINTER;"
        );
        assert_eq!(redact(b"CMD:ISN:1;"), b"CMD:ISN:1;");
    }

    #[test]
    fn blanks_status_serial() {
        let mut r = b"@BDC ST2\r\n".to_vec();
        r.extend_from_slice(&[0x07, 0x00, 0x01, 0x01, 0x04, 0x40, 0x02, b'A', b'B']);
        let clean = redact(&r);
        assert_eq!(&clean[clean.len() - 2..], b"**");
        assert_eq!(clean[clean.len() - 5], 0x04);
    }

    #[test]
    fn hexdump_rows() {
        assert_eq!(
            hexdump(b"AB"),
            "  0000  41 42                                            AB"
        );
    }
}
