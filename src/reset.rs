//! Reading the counters and resetting them. The rules, agreed in review:
//! nothing is written before the user confirms; only `RESET_MAP` and the
//! commit bit are ever written; every write is read back; the first write
//! that does not verify stops everything; success is only claimed once every
//! target byte reads back as expected.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::model::{COMMIT_ADDR, COMMIT_CLEAR_MASK, RESET_MAP, backup_addresses};
use crate::printer::{Eeprom, WriteAck};

/// Values of every backed up address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot(pub BTreeMap<u16, u8>);

impl Snapshot {
    #[must_use]
    pub fn get(&self, addr: u16) -> Option<u8> {
        self.0.get(&addr).copied()
    }

    /// True when every counter byte already has its reset value and the
    /// commit bit is clear.
    #[must_use]
    pub fn is_reset(&self) -> bool {
        RESET_MAP.iter().all(|(a, v)| self.get(*a) == Some(*v))
            && self
                .get(COMMIT_ADDR)
                .is_some_and(|v| v & COMMIT_CLEAR_MASK == 0)
    }

    /// Bytes that a reset would change.
    #[must_use]
    pub fn pending_writes(&self) -> Vec<(u16, u8, u8)> {
        RESET_MAP
            .iter()
            .filter_map(|(a, v)| {
                let now = self.get(*a)?;
                (now != *v).then_some((*a, now, *v))
            })
            .collect()
    }

    /// Text for the backup file.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::from("address value\r\n");
        for (a, v) in &self.0 {
            let _ = write!(out, "0x{a:03X} 0x{v:02X}\r\n");
        }
        out
    }
}

/// Reads every address twice; both passes must agree, so a reply that landed
/// on the wrong question cannot slip through.
pub fn read_snapshot(p: &mut impl Eeprom) -> Result<Snapshot, String> {
    let addrs = backup_addresses();
    let mut first = BTreeMap::new();
    for a in &addrs {
        first.insert(*a, p.read(*a)?);
    }
    for a in &addrs {
        let again = p.read(*a)?;
        if first[a] != again {
            return Err(format!(
                "address 0x{a:03X} read 0x{:02X} then 0x{again:02X}; the readings are not reliable",
                first[a]
            ));
        }
    }
    Ok(Snapshot(first))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Counters and commit bit were already in the reset state.
    NothingToChange,
    /// The state differs from the snapshot the user saw; nothing written.
    ChangedSinceRead,
    /// The first changed byte did not verify; no further writes attempted.
    FirstWriteNotVerified { addr: u16, detail: String },
    /// A later byte failed; `written` lists what did verify.
    Failed {
        addr: u16,
        detail: String,
        written: Vec<u16>,
    },
    /// Every target byte read back as expected.
    Done {
        counters_written: usize,
        commit_cleared: bool,
    },
}

enum Verify {
    Ok,
    Bad(String),
}

/// Writes one byte and reads it back. An unclear reply is followed by a
/// fresh session and a read, never by a second write.
fn write_verified(p: &mut impl Eeprom, addr: u16, value: u8) -> Result<Verify, String> {
    let ack = p.write(addr, value)?;
    if ack == WriteAck::Refused {
        return Ok(Verify::Bad("the printer refused the write".into()));
    }
    if ack == WriteAck::Unknown {
        p.recover()?;
    }
    let back = p.read(addr)?;
    Ok(if back == value {
        Verify::Ok
    } else if ack == WriteAck::Ok {
        // Some firmware answers OK and discards the write (epson_print_conf).
        Verify::Bad(format!(
            "the printer answered OK but the byte still reads 0x{back:02X}"
        ))
    } else {
        Verify::Bad(format!("no confirmation and the byte reads 0x{back:02X}"))
    })
}

/// Runs the reset against the state the user confirmed.
pub fn reset(p: &mut impl Eeprom, confirmed: &Snapshot) -> Result<Outcome, String> {
    let now = read_snapshot(p)?;
    if &now != confirmed {
        return Ok(Outcome::ChangedSinceRead);
    }
    if now.is_reset() {
        return Ok(Outcome::NothingToChange);
    }

    let mut written = Vec::new();
    for (addr, _, value) in now.pending_writes() {
        match write_verified(p, addr, value) {
            Ok(Verify::Ok) => written.push(addr),
            Ok(Verify::Bad(detail)) | Err(detail) => {
                return Ok(if written.is_empty() {
                    Outcome::FirstWriteNotVerified { addr, detail }
                } else {
                    Outcome::Failed {
                        addr,
                        detail,
                        written,
                    }
                });
            }
        }
    }

    // Re-read right before masking so only bit 0 changes from what the
    // printer holds now.
    let commit = p.read(COMMIT_ADDR)?;
    let mut commit_cleared = false;
    if commit & COMMIT_CLEAR_MASK != 0 {
        let target = commit & !COMMIT_CLEAR_MASK;
        match write_verified(p, COMMIT_ADDR, target) {
            Ok(Verify::Ok) => commit_cleared = true,
            Ok(Verify::Bad(detail)) | Err(detail) => {
                return Ok(if written.is_empty() {
                    Outcome::FirstWriteNotVerified {
                        addr: COMMIT_ADDR,
                        detail,
                    }
                } else {
                    Outcome::Failed {
                        addr: COMMIT_ADDR,
                        detail,
                        written,
                    }
                });
            }
        }
    }

    let after = read_snapshot(p)?;
    if !after.is_reset() {
        let addr = RESET_MAP
            .iter()
            .find(|(a, v)| after.get(*a) != Some(*v))
            .map_or(COMMIT_ADDR, |(a, _)| *a);
        return Ok(Outcome::Failed {
            addr,
            detail: "the final check did not read back the reset values".into(),
            written,
        });
    }
    Ok(Outcome::Done {
        counters_written: written.len(),
        commit_cleared,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory EEPROM with switchable firmware behaviour.
    struct Fake {
        mem: BTreeMap<u16, u8>,
        /// Answer OK but keep the old value.
        discard_writes: bool,
        refuse_writes: bool,
        /// Write lands but the ack is lost, from this write number on.
        lose_ack_from: Option<usize>,
        writes: Vec<(u16, u8)>,
        recovers: usize,
    }

    impl Fake {
        fn full() -> Self {
            let mut mem = BTreeMap::new();
            for (a, _) in RESET_MAP {
                mem.insert(a, 0x7F);
            }
            mem.insert(COMMIT_ADDR, 0xA1);
            Self {
                mem,
                discard_writes: false,
                refuse_writes: false,
                lose_ack_from: None,
                writes: Vec::new(),
                recovers: 0,
            }
        }
    }

    impl Eeprom for Fake {
        fn read(&mut self, addr: u16) -> Result<u8, String> {
            self.mem
                .get(&addr)
                .copied()
                .ok_or_else(|| "no such address".into())
        }
        fn write(&mut self, addr: u16, value: u8) -> Result<WriteAck, String> {
            self.writes.push((addr, value));
            if self.refuse_writes {
                return Ok(WriteAck::Refused);
            }
            if !self.discard_writes {
                self.mem.insert(addr, value);
            }
            if self.lose_ack_from.is_some_and(|n| self.writes.len() > n) {
                return Ok(WriteAck::Unknown);
            }
            Ok(WriteAck::Ok)
        }
        fn recover(&mut self) -> Result<(), String> {
            self.recovers += 1;
            Ok(())
        }
    }

    #[test]
    fn resets_writes_only_the_map_and_one_commit_bit() {
        let mut p = Fake::full();
        let snap = read_snapshot(&mut p).unwrap();
        let out = reset(&mut p, &snap).unwrap();
        assert_eq!(
            out,
            Outcome::Done {
                counters_written: 14,
                commit_cleared: true
            }
        );
        assert_eq!(p.mem[&COMMIT_ADDR], 0xA0);
        let allowed: Vec<u16> = RESET_MAP
            .iter()
            .map(|(a, _)| *a)
            .chain([COMMIT_ADDR])
            .collect();
        assert!(p.writes.iter().all(|(a, _)| allowed.contains(a)));
        assert_eq!(p.writes.len(), 15);
    }

    #[test]
    fn discarded_first_write_stops_everything() {
        let mut p = Fake::full();
        p.discard_writes = true;
        let snap = read_snapshot(&mut p).unwrap();
        let out = reset(&mut p, &snap).unwrap();
        assert!(matches!(
            out,
            Outcome::FirstWriteNotVerified { addr: 0x1C, .. }
        ));
        assert_eq!(p.writes.len(), 1);
    }

    #[test]
    fn refused_write_stops() {
        let mut p = Fake::full();
        p.refuse_writes = true;
        let snap = read_snapshot(&mut p).unwrap();
        assert!(matches!(
            reset(&mut p, &snap).unwrap(),
            Outcome::FirstWriteNotVerified { .. }
        ));
        assert_eq!(p.writes.len(), 1);
    }

    #[test]
    fn lost_ack_recovers_and_reads_instead_of_rewriting() {
        let mut p = Fake::full();
        p.lose_ack_from = Some(3);
        let snap = read_snapshot(&mut p).unwrap();
        let out = reset(&mut p, &snap).unwrap();
        assert!(matches!(out, Outcome::Done { .. }));
        assert_eq!(p.writes.len(), 15, "no write was repeated");
        assert!(p.recovers > 0);
    }

    #[test]
    fn already_reset_writes_nothing() {
        let mut p = Fake::full();
        for (a, v) in RESET_MAP {
            p.mem.insert(a, v);
        }
        p.mem.insert(COMMIT_ADDR, 0xA0);
        let snap = read_snapshot(&mut p).unwrap();
        assert_eq!(reset(&mut p, &snap).unwrap(), Outcome::NothingToChange);
        assert_eq!(p.writes, []);
    }

    #[test]
    fn commit_only_when_bit_set() {
        let mut p = Fake::full();
        p.mem.insert(COMMIT_ADDR, 0xA0);
        let snap = read_snapshot(&mut p).unwrap();
        let out = reset(&mut p, &snap).unwrap();
        assert_eq!(
            out,
            Outcome::Done {
                counters_written: 14,
                commit_cleared: false
            }
        );
        assert!(p.writes.iter().all(|(a, _)| *a != COMMIT_ADDR));
    }

    #[test]
    fn changed_state_writes_nothing() {
        let mut p = Fake::full();
        let snap = read_snapshot(&mut p).unwrap();
        p.mem.insert(0x2F, 0x01);
        assert_eq!(reset(&mut p, &snap).unwrap(), Outcome::ChangedSinceRead);
        assert_eq!(p.writes, []);
    }
}
