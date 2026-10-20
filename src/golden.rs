//! Byte-for-byte comparison with frames produced by running reinkpy's own
//! encoders for the ET-2871 (`tests/fixtures/reinkpy_frames.txt`).
#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use crate::ctrl;
use crate::d4::{self, EJL_ENTER, tx};
use crate::model::{RESET_MAP, backup_addresses};

const FRAMES: &str = include_str!("../tests/fixtures/reinkpy_frames.txt");

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn num(s: &str) -> u16 {
    u16::from_str_radix(s.trim_start_matches("0x"), 16).expect("number")
}

#[test]
fn every_frame_matches_reinkpy() {
    let mut reads = BTreeSet::new();
    let mut writes = BTreeSet::new();
    let mut checked = 0;
    for line in FRAMES
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let (name, expected) = line.split_once('\t').expect("name and hex");
        let words: Vec<&str> = name.split(' ').collect();
        let ours = match words.as_slice() {
            ["read", a] => {
                reads.insert(num(a));
                ctrl::read_eeprom(num(a))
            }
            ["write", a, v] => {
                let value = u8::try_from(num(v)).expect("byte");
                writes.insert((num(a), value));
                ctrl::write_eeprom(num(a), value)
            }
            ["status"] => ctrl::status(),
            ["device_id"] => ctrl::device_id(),
            ["d4_0x10", what] => {
                let payload = match *what {
                    "Init" => tx::init(0x10),
                    "OpenChannel" => tx::open_channel(0x10, 2),
                    "Credit" => tx::credit(2),
                    "CreditRequest" => tx::credit_request(0x10, 2),
                    "CloseChannel" => tx::close_channel(0x10, 2),
                    "Exit" => tx::exit(),
                    "GetSocketID" => tx::get_socket_id(),
                    other => panic!("unknown D4 frame {other}"),
                };
                d4::encode(0, 0, 1, &payload)
            }
            ["ejl_enter"] => EJL_ENTER.to_vec(),
            _ => panic!("unknown frame {name}"),
        };
        assert_eq!(ours, hex(expected), "{name}");
        checked += 1;
    }
    assert_eq!(checked, 39);
    // reinkpy's own reset map for the ET-2871 is exactly ours.
    assert_eq!(writes, RESET_MAP.iter().copied().collect::<BTreeSet<_>>());
    assert_eq!(
        reads,
        backup_addresses().into_iter().collect::<BTreeSet<_>>()
    );
}
