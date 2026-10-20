//! EPSON-CTRL commands and their replies. Command layout from reinkpy
//! `epson.py` (`encode`, `read_eeprom`, `write_eeprom`), checked against
//! `epson_print_conf` (`eeprom_oid_read_address`, `eeprom_oid_write_address`) and
//! Epson-Waste-Reset (`generator.cpp`, `status.cpp`).
#![forbid(unsafe_code)]

use crate::model::{READ_KEY, WRITE_KEY};

/// A two-letter command, a little-endian payload length, then the payload.
#[must_use]
pub fn command(name: [u8; 2], payload: &[u8]) -> Vec<u8> {
    let len = u16::try_from(payload.len()).expect("payload fits a u16 length");
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&name);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// A "||" factory command: read key, the action letter, its complement and
/// its rotation, then the action's data.
fn factory(action: u8, data: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(5 + data.len());
    payload.extend_from_slice(&READ_KEY.to_le_bytes());
    payload.push(action);
    payload.push(!action);
    payload.push(action.rotate_right(1));
    payload.extend_from_slice(data);
    command(*b"||", &payload)
}

/// Read one EEPROM byte (two-byte address, little endian).
#[must_use]
pub fn read_eeprom(addr: u16) -> Vec<u8> {
    factory(b'A', &addr.to_le_bytes())
}

/// Write one EEPROM byte with the family's write key.
#[must_use]
pub fn write_eeprom(addr: u16, value: u8) -> Vec<u8> {
    let mut data = Vec::with_capacity(11);
    data.extend_from_slice(&addr.to_le_bytes());
    data.push(value);
    data.extend_from_slice(&WRITE_KEY);
    factory(b'B', &data)
}

/// Status report request ("st" with payload 01).
#[must_use]
pub fn status() -> Vec<u8> {
    command(*b"st", &[0x01])
}

/// Device id request ("di" with payload 01).
#[must_use]
pub fn device_id() -> Vec<u8> {
    command(*b"di", &[0x01])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadReply {
    Value(u8),
    /// ":41:NA;" the firmware names the read and turns it down.
    Refused,
    /// The reply echoed a different address than the one asked for.
    WrongAddress,
    Other,
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn hex_byte(s: &[u8]) -> Option<u8> {
    let text = std::str::from_utf8(s.get(..2)?).ok()?;
    u8::from_str_radix(text, 16).ok()
}

/// Two reply forms exist (Epson-Waste-Reset `ParseEepromReadReply`): "EE:" with
/// the two-byte address echo and the value, or ":41:" with the value only.
#[must_use]
pub fn parse_read(reply: &[u8], expected_addr: u16) -> ReadReply {
    if find(reply, b":41:NA;").is_some() {
        return ReadReply::Refused;
    }
    if let Some(at) = find(reply, b"EE:") {
        let body = &reply[at + 3..];
        let Some(end) = body.iter().position(|b| *b == b';') else {
            return ReadReply::Other;
        };
        if end != 6 {
            return ReadReply::Other;
        }
        let (Some(hi), Some(lo), Some(value)) = (
            hex_byte(&body[0..]),
            hex_byte(&body[2..]),
            hex_byte(&body[4..]),
        ) else {
            return ReadReply::Other;
        };
        if u16::from_be_bytes([hi, lo]) != expected_addr {
            return ReadReply::WrongAddress;
        }
        return ReadReply::Value(value);
    }
    if let Some(at) = find(reply, b":41:") {
        let body = &reply[at + 4..];
        if body.get(2) == Some(&b';')
            && let Some(value) = hex_byte(body)
        {
            return ReadReply::Value(value);
        }
    }
    ReadReply::Other
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteReply {
    Ok,
    Refused,
    Other,
}

#[must_use]
pub fn parse_write(reply: &[u8]) -> WriteReply {
    if find(reply, b":42:OK;").is_some() {
        WriteReply::Ok
    } else if find(reply, b":42:NA;").is_some() {
        WriteReply::Refused
    } else {
        WriteReply::Other
    }
}

/// The fields of an IEEE 1284 device id that this tool uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceId {
    pub manufacturer: String,
    pub model: String,
}

/// Parses "KEY:value;" pairs. Accepts the "@EJL ID" reply of "di" and the
/// bare string from the USB printer class request alike.
#[must_use]
pub fn parse_device_id(text: &str) -> DeviceId {
    let body = text
        .find("@EJL ID")
        .map_or(text, |at| &text[at + "@EJL ID".len()..]);
    let mut id = DeviceId::default();
    for field in body.split(';') {
        let Some((key, value)) = field.split_once(':') else {
            continue;
        };
        let value = value.trim().to_string();
        match key.trim().to_ascii_uppercase().as_str() {
            "MFG" | "MANUFACTURER" => id.manufacturer = value,
            "MDL" | "MODEL" => id.model = value,
            _ => {}
        }
    }
    id
}

/// Printer errors the status report can carry (Epson-Waste-Reset
/// `PrinterErrorName`).
pub const ERROR_SERVICE_REQUEST: u8 = 0x10;
pub const ERROR_CARTRIDGE_OVERFLOW: u8 = 0x2C;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub state: Option<u8>,
    pub error: Option<u8>,
    /// The report announced more than arrived; a missing error then proves
    /// nothing.
    pub truncated: bool,
}

impl Status {
    /// The waste ink states this tool exists to clear.
    #[must_use]
    pub fn waste_error(&self) -> bool {
        matches!(
            self.error,
            Some(ERROR_SERVICE_REQUEST | ERROR_CARTRIDGE_OVERFLOW)
        )
    }
}

/// "@BDC ST2\r\n", a little-endian body length, then (header, length, data)
/// fields. Layout from Epson-Waste-Reset `ParseStatusReply`.
#[must_use]
pub fn parse_status(reply: &[u8]) -> Option<Status> {
    const PREFIX: &[u8] = b"@BDC ST2\r\n";
    let start = find(reply, PREFIX)? + PREFIX.len();
    let len_bytes = reply.get(start..start + 2)?;
    let declared = usize::from(u16::from_le_bytes([len_bytes[0], len_bytes[1]]));
    let end = (start + 2 + declared).min(reply.len());
    let mut status = Status::default();
    let mut decoded = 0;
    let mut i = start + 2;
    while i + 2 <= end {
        let (header, len) = (reply[i], usize::from(reply[i + 1]));
        i += 2;
        if i + len > end {
            status.truncated = true;
            break;
        }
        let field = &reply[i..i + len];
        if let Some(first) = field.first() {
            match header {
                0x01 => {
                    status.state = Some(*first);
                    decoded += 1;
                }
                0x02 => {
                    status.error = Some(*first);
                    decoded += 1;
                }
                _ => {}
            }
        }
        i += len;
    }
    // Bytes left over that do not make a field mean the report is malformed.
    if declared + start + 2 > reply.len() || i < end {
        status.truncated = true;
    }
    (decoded > 0).then_some(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_frame_matches_the_references() {
        // 7C 7C 07 00 4A 36 41 BE A0 <addr LE16>
        assert_eq!(
            read_eeprom(0x002F),
            [
                0x7C, 0x7C, 0x07, 0x00, 0x4A, 0x36, 0x41, 0xBE, 0xA0, 0x2F, 0x00
            ]
        );
    }

    #[test]
    fn write_frame_matches_the_references() {
        // 7C 7C 10 00 4A 36 42 BD 21 <addr LE16> <value> <8-byte key>
        let mut expected = vec![
            0x7C, 0x7C, 0x10, 0x00, 0x4A, 0x36, 0x42, 0xBD, 0x21, 0x00, 0x01, 0xFE,
        ];
        expected.extend_from_slice(b"Nbsjcbzb");
        assert_eq!(write_eeprom(0x0100, 0xFE), expected);
    }

    #[test]
    fn status_and_id_requests() {
        assert_eq!(status(), b"st\x01\x00\x01");
        assert_eq!(device_id(), b"di\x01\x00\x01");
    }

    #[test]
    fn reads_both_reply_forms() {
        assert_eq!(
            parse_read(b"@BDC PS\r\nEE:002F3A;", 0x2F),
            ReadReply::Value(0x3A)
        );
        assert_eq!(
            parse_read(b"@BDC PS\r\nEE:00303A;", 0x2F),
            ReadReply::WrongAddress
        );
        assert_eq!(parse_read(b"||:41:5E;", 0x36), ReadReply::Value(0x5E));
        assert_eq!(parse_read(b"||:41:NA;", 0x36), ReadReply::Refused);
        assert_eq!(parse_read(b"||:;", 0x36), ReadReply::Other);
        assert_eq!(parse_read(b"@BDC PS\r\nEE:002F3;", 0x2F), ReadReply::Other);
    }

    #[test]
    fn write_acks() {
        assert_eq!(parse_write(b"||:42:OK;"), WriteReply::Ok);
        assert_eq!(parse_write(b"||:42:NA;"), WriteReply::Refused);
        assert_eq!(parse_write(b""), WriteReply::Other);
    }

    #[test]
    fn device_id_fields() {
        let id =
            parse_device_id("@EJL ID\r\nMFG:EPSON;CMD:ESCPL2,BDC;MDL:ET-2870 Series;CLS:PRINTER;");
        assert_eq!(id.manufacturer, "EPSON");
        assert_eq!(id.model, "ET-2870 Series");
    }

    #[test]
    fn status_report() {
        let mut r = b"@BDC ST2\r\n".to_vec();
        r.extend_from_slice(&[0x06, 0x00, 0x01, 0x01, 0x04, 0x02, 0x01, 0x10]);
        let s = parse_status(&r).unwrap();
        assert_eq!(s.state, Some(0x04));
        assert_eq!(s.error, Some(0x10));
        assert!(s.waste_error());
        assert!(!s.truncated);

        r.truncate(r.len() - 1);
        let s = parse_status(&r).unwrap();
        assert!(s.truncated);
        assert_eq!(s.error, None);

        // A declared body that ends in half a field header.
        let mut r = b"@BDC ST2\r\n".to_vec();
        r.extend_from_slice(&[0x04, 0x00, 0x01, 0x01, 0x04, 0x02]);
        assert!(parse_status(&r).unwrap().truncated);
    }
}
