//! Test-only printer that speaks D4 and EPSON-CTRL the way the reference
//! implementations expect. It exists to exercise the whole stack; it says
//! nothing about how a real ET-2871 answers.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use crate::d4::{EJL_ENTER, Transport, encode};
use crate::model::{COMMIT_ADDR, READ_KEY, RESET_MAP, WRITE_KEY};

#[allow(
    clippy::struct_excessive_bools,
    reason = "independent firmware switches"
)]
pub struct SimPrinter {
    pub revision: u8,
    pub socket: u8,
    pub mem: BTreeMap<u16, u8>,
    pub discard_writes: bool,
    pub accept_read_key: bool,
    /// Bytes on the wire towards the host.
    out: VecDeque<u8>,
    inbuf: Vec<u8>,
    printer_credit: u32,
    held: VecDeque<Vec<u8>>,
    pub writes: Vec<(u16, u8)>,
    pub data_without_credit: usize,
    host_credit: u32,
    /// Hand held replies over before the `CreditReply`.
    pub held_first: bool,
    /// Cut this many replies after one full packet, never sending the rest.
    pub cut_replies: u32,
    /// Real behaviour recorded in Epson-Waste-Reset's tests, replayed here.
    pub quirk: Quirk,
    /// Report this error code in the status instead of the counter state.
    pub error: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quirk {
    None,
    /// ET-2803 interface 0: a stray NAK byte ahead of the first packet.
    StrayNak,
    /// R220: `OpenChannel` reply of six payload bytes, captured byte for byte.
    ShortOpenReply,
    /// ET-2800 IPP-over-USB interface: the 57-byte HTTP 500 answer.
    Http,
    /// ET-2803 interface 1: never answers.
    Silent,
    /// Answers Init 0x20 only; Init 0x10 gets no reply at all.
    IgnoresRev10,
}

impl SimPrinter {
    #[must_use]
    pub fn full() -> Self {
        let mut mem = BTreeMap::new();
        for (a, _) in RESET_MAP {
            mem.insert(a, 0x7F);
        }
        mem.insert(COMMIT_ADDR, 0x11);
        Self {
            revision: 0x10,
            socket: 0x02,
            mem,
            discard_writes: false,
            accept_read_key: true,
            out: VecDeque::new(),
            inbuf: Vec::new(),
            printer_credit: 0,
            held: VecDeque::new(),
            writes: Vec::new(),
            data_without_credit: 0,
            host_credit: 0,
            held_first: false,
            cut_replies: 0,
            error: None,
            quirk: Quirk::None,
        }
    }

    fn reset_state(&self) -> bool {
        RESET_MAP.iter().all(|(a, v)| self.mem.get(a) == Some(v))
            && self.mem.get(&COMMIT_ADDR).is_some_and(|v| v & 1 == 0)
    }

    fn reply_tx(&mut self, payload: &[u8]) {
        self.out.extend(encode(0, 0, 1, payload));
    }

    fn reply_data(&mut self, payload: Vec<u8>) {
        if self.cut_replies > 0 && self.printer_credit > 0 {
            self.cut_replies -= 1;
            // A full 250-byte payload without end of message, then silence.
            self.printer_credit -= 1;
            let mut part = payload;
            part.resize(250, b'.');
            let s = self.socket;
            self.out.extend(encode(s, s, 0, &part));
            return;
        }
        self.held.push_back(payload);
        self.flush_held();
    }

    fn flush_held(&mut self) {
        while self.printer_credit > 0 {
            let Some(p) = self.held.pop_front() else {
                break;
            };
            self.printer_credit -= 1;
            let s = self.socket;
            let mut packet = encode(s, s, 0, &p);
            packet[5] = 0x02; // end of message
            self.out.extend(packet);
        }
    }

    fn on_transaction(&mut self, p: &[u8]) {
        let s = self.socket;
        match p[0] {
            0x00 if self.quirk == Quirk::IgnoresRev10 && p[1] == 0x10 => {}
            0x00 => {
                if p[1] == self.revision {
                    self.reply_tx(&[0x80, 0x00, self.revision]);
                } else {
                    self.reply_tx(&[0x80, 0x02, self.revision]);
                }
            }
            0x09 => {
                let mut r = vec![0x89, 0x00, s];
                r.extend_from_slice(b"EPSON-CTRL");
                self.reply_tx(&r);
            }
            0x01 if self.quirk == Quirk::ShortOpenReply => {
                // 00 00 00 0c 01 00 81 00 02 02 00 01 (Epson-Waste-Reset tests).
                self.reply_tx(&[0x81, 0x00, s, s, 0x00, 0x01]);
            }
            0x01 => {
                let expected = if self.revision == 0x10 { 11 } else { 9 };
                let result = u8::from(p.len() != expected);
                self.reply_tx(&[
                    0x81, result, s, s, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
                ]);
            }
            0x03 => {
                self.printer_credit += u32::from(u16::from_be_bytes([p[3], p[4]]));
                if self.held_first {
                    self.flush_held();
                    self.reply_tx(&[0x83, 0x00, s, s]);
                } else {
                    self.reply_tx(&[0x83, 0x00, s, s]);
                    self.flush_held();
                }
            }
            0x04 => {
                self.host_credit += 1;
                self.reply_tx(&[0x84, 0x00, s, s, 0x00, 0x01]);
            }
            0x02 => self.reply_tx(&[0x82, 0x00, s, s]),
            0x08 => self.reply_tx(&[0x88, 0x00]),
            _ => self.reply_tx(&[0x7F, 0, 0, 0x87]),
        }
    }

    fn on_ctrl(&mut self, m: &[u8]) {
        let name = &m[..2];
        let body = &m[4..];
        let reply = match name {
            b"di" => {
                b"@EJL ID\r\nMFG:EPSON;CMD:ESCPL2,BDC;MDL:ET-2870 Series;SN:SIM0001;CLS:PRINTER;"
                    .to_vec()
            }
            b"st" => {
                let mut r = b"@BDC ST2\r\n".to_vec();
                if let Some(code) = self.error {
                    r.extend_from_slice(&[0x06, 0x00, 0x01, 0x01, 0x04, 0x02, 0x01, code]);
                } else if self.reset_state() {
                    r.extend_from_slice(&[0x03, 0x00, 0x01, 0x01, 0x04]);
                } else {
                    r.extend_from_slice(&[0x06, 0x00, 0x01, 0x01, 0x04, 0x02, 0x01, 0x10]);
                }
                r
            }
            b"||" => self.on_factory(body),
            _ => b"XX:;".to_vec(),
        };
        self.reply_data(reply);
    }

    fn on_factory(&mut self, body: &[u8]) -> Vec<u8> {
        let key_ok = self.accept_read_key && body[..2] == READ_KEY.to_le_bytes();
        let addr = u16::from_le_bytes([body[5], body[6]]);
        match body[2] {
            b'A' if key_ok => {
                let v = self.mem.get(&addr).copied().unwrap_or(0);
                format!("@BDC PS\r\nEE:{addr:04X}{v:02X};").into_bytes()
            }
            b'A' => b"||:41:NA;".to_vec(),
            b'B' if key_ok && body[8..16] == WRITE_KEY => {
                self.writes.push((addr, body[7]));
                if !self.discard_writes {
                    self.mem.insert(addr, body[7]);
                }
                b"||:42:OK;".to_vec()
            }
            _ => b"||:42:NA;".to_vec(),
        }
    }

    fn process(&mut self) {
        match self.quirk {
            Quirk::Http => {
                self.inbuf.clear();
                self.out
                    .extend(b"HTTP/1.0 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n");
                return;
            }
            Quirk::Silent => {
                self.inbuf.clear();
                return;
            }
            Quirk::StrayNak if self.inbuf.starts_with(EJL_ENTER) => self.out.push_back(0x15),
            _ => {}
        }
        loop {
            if self.inbuf.starts_with(EJL_ENTER) {
                self.inbuf.drain(..EJL_ENTER.len());
                self.out
                    .extend([0x00, 0x00, 0x00, 0x08, 0x01, 0x00, 0xC5, 0x00]);
                continue;
            }
            if self.inbuf.len() < 6 {
                return;
            }
            let total = usize::from(u16::from_be_bytes([self.inbuf[2], self.inbuf[3]]));
            if self.inbuf.len() < total {
                return;
            }
            let packet: Vec<u8> = self.inbuf.drain(..total).collect();
            let payload = &packet[6..];
            if packet[0] == 0 && packet[1] == 0 {
                self.on_transaction(payload);
            } else {
                if self.host_credit == 0 {
                    self.data_without_credit += 1;
                    continue;
                }
                self.host_credit -= 1;
                self.on_ctrl(payload);
            }
        }
    }
}

impl Transport for SimPrinter {
    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        self.inbuf.extend_from_slice(data);
        self.process();
        Ok(())
    }

    fn recv(&mut self, _timeout: Duration) -> Result<Vec<u8>, String> {
        // Hand over replies in small pieces to exercise reassembly.
        let n = self.out.len().min(5);
        Ok(self.out.drain(..n).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d4::Timing;
    use crate::model::is_supported;
    use crate::printer::Printer;
    use crate::reset::{Outcome, read_snapshot, reset};

    fn fast() -> Timing {
        Timing {
            reply: Duration::from_millis(50),
            data: Duration::from_millis(50),
            fragment: Duration::from_millis(20),
            held_reply: Duration::from_millis(20),
            between_packets: Duration::ZERO,
        }
    }

    #[test]
    fn full_stack_reset() {
        let mut p = Printer::open(SimPrinter::full(), fast()).unwrap();
        let id = p.device_id().unwrap();
        assert!(is_supported(&id.manufacturer, &id.model));
        assert!(p.status().unwrap().unwrap().waste_error());

        let snap = read_snapshot(&mut p).unwrap();
        assert_eq!(snap.get(0x2F), Some(0x7F));
        let out = reset(&mut p, &snap).unwrap();
        assert_eq!(
            out,
            Outcome::Done {
                counters_written: 14,
                commit_cleared: true
            }
        );
        assert!(!p.status().unwrap().unwrap().waste_error());
        let sim = p.session.transport_mut();
        assert_eq!(sim.mem[&COMMIT_ADDR], 0x10);
        assert_eq!(sim.data_without_credit, 0);
    }

    #[test]
    fn revision_0x20_is_negotiated() {
        let mut sim = SimPrinter::full();
        sim.revision = 0x20;
        let mut p = Printer::open(sim, fast()).unwrap();
        assert_eq!(p.device_id().unwrap().model, "ET-2870 Series");
    }

    #[test]
    fn wrong_read_key_is_an_error_not_a_value() {
        let mut sim = SimPrinter::full();
        sim.accept_read_key = false;
        let mut p = Printer::open(sim, fast()).unwrap();
        let err = read_snapshot(&mut p).unwrap_err();
        assert!(err.contains("refused"), "{err}");
    }

    #[test]
    fn discarded_writes_stop_after_one() {
        let mut sim = SimPrinter::full();
        sim.discard_writes = true;
        let mut p = Printer::open(sim, fast()).unwrap();
        let snap = read_snapshot(&mut p).unwrap();
        assert!(matches!(
            reset(&mut p, &snap).unwrap(),
            Outcome::FirstWriteNotVerified { .. }
        ));
        assert_eq!(p.session.transport_mut().writes.len(), 1);
    }

    #[test]
    fn cut_reply_is_not_used_and_the_session_restarts() {
        let mut p = Printer::open(SimPrinter::full(), fast()).unwrap();
        p.session.transport_mut().cut_replies = 1;
        let id = p.device_id().unwrap();
        assert_eq!(id.model, "ET-2870 Series");
        assert!(p.session.log.text().contains("incomplete reply"));
        let snap = read_snapshot(&mut p).unwrap();
        assert_eq!(snap.get(0x2F), Some(0x7F));
    }

    #[test]
    fn after_a_failed_retry_the_next_command_starts_fresh() {
        let mut p = Printer::open(SimPrinter::full(), fast()).unwrap();
        p.session.transport_mut().cut_replies = 2;
        assert!(p.device_id().is_err());
        let snap = read_snapshot(&mut p).unwrap();
        assert_eq!(snap.get(0x2F), Some(0x7F));
        assert!(
            p.session
                .log
                .text()
                .contains("previous exchange was incomplete")
        );
    }

    #[test]
    fn held_reply_before_credit_reply_keeps_credit_in_step() {
        let mut sim = SimPrinter::full();
        sim.held_first = true;
        sim.held.push_back(b"@BDC PS\r\nEE:002F00;".to_vec());
        let mut p = Printer::open(sim, fast()).unwrap();
        let snap = read_snapshot(&mut p).unwrap();
        assert_eq!(snap.get(0x2F), Some(0x7F));
        let out = reset(&mut p, &snap).unwrap();
        assert!(matches!(out, Outcome::Done { .. }), "{out:?}");
    }

    fn with(quirk: Quirk) -> SimPrinter {
        let mut sim = SimPrinter::full();
        sim.quirk = quirk;
        sim
    }

    #[test]
    fn recorded_quirks_that_still_work() {
        for quirk in [Quirk::StrayNak, Quirk::ShortOpenReply, Quirk::IgnoresRev10] {
            let mut sim = with(quirk);
            if quirk == Quirk::IgnoresRev10 {
                sim.revision = 0x20;
            }
            let mut p = Printer::open(sim, fast()).unwrap();
            let snap = read_snapshot(&mut p).unwrap();
            assert_eq!(snap.get(0x2F), Some(0x7F), "{quirk:?}");
            assert!(
                matches!(reset(&mut p, &snap).unwrap(), Outcome::Done { .. }),
                "{quirk:?}"
            );
        }
    }

    #[test]
    fn recorded_wrong_interfaces_fail_without_writing() {
        for quirk in [Quirk::Http, Quirk::Silent] {
            assert!(Printer::open(with(quirk), fast()).is_err(), "{quirk:?}");
        }
    }

    #[test]
    fn real_et2800_device_id_is_read_and_not_supported() {
        // Captured from an ET-2800 (Epson-Waste-Reset tests): a sibling, not
        // a model this tool resets.
        let id = crate::ctrl::parse_device_id(
            "MFG:EPSON;CMD:ESCPL2,BDC,D4;MDL:ET-2800 Series;CLS:PRINTER;DES:EPSON ET-2800 Series;",
        );
        assert_eq!(id.model, "ET-2800 Series");
        assert!(!is_supported(&id.manufacturer, &id.model));
    }

    #[test]
    fn held_reply_from_an_old_session_is_discarded() {
        let mut sim = SimPrinter::full();
        sim.held.push_back(b"@BDC PS\r\nEE:002F00;".to_vec());
        let mut p = Printer::open(sim, fast()).unwrap();
        let snap = read_snapshot(&mut p).unwrap();
        assert_eq!(snap.get(0x2F), Some(0x7F));
    }
}
