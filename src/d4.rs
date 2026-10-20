//! IEEE 1284.4 ("D4") session to the printer's EPSON-CTRL service.
//!
//! Packet layout, handshake and credit flow follow reinkpy `d4.py` and
//! Epson-Waste-Reset `d4session.cpp`. Revision 0x10 is asked for first, as
//! Epson-Waste-Reset does; a printer that answers "use 0x20" is re-initialised
//! with the 0x20 layouts from reinkpy.
#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::log::Log;

/// Raw byte pipe to one USB printer interface.
pub trait Transport {
    /// Sends all bytes or fails.
    fn send(&mut self, data: &[u8]) -> Result<(), String>;
    /// Returns what arrived within `timeout`; empty when nothing did.
    fn recv(&mut self, timeout: Duration) -> Result<Vec<u8>, String>;
}

/// Leading NULs flush the parser, then EJL switches to 1284.4 packet mode.
pub const EJL_ENTER: &[u8] = b"\x00\x00\x00\x1b\x01@EJL 1284.4\n@EJL\n@EJL\n";
pub const SERVICE: &[u8] = b"EPSON-CTRL";
/// Used when `GetSocketID` is not answered (both references).
pub const DEFAULT_SOCKET: u8 = 0x02;
const NO_INIT_REPLY: &str = "the printer did not answer the D4 Init";

const CMD_INIT: u8 = 0x00;
const CMD_OPEN_CHANNEL: u8 = 0x01;
const CMD_CLOSE_CHANNEL: u8 = 0x02;
const CMD_CREDIT: u8 = 0x03;
const CMD_CREDIT_REQUEST: u8 = 0x04;
const CMD_EXIT: u8 = 0x08;
const CMD_GET_SOCKET_ID: u8 = 0x09;
const CMD_ERROR: u8 = 0x7F;
const REPLY: u8 = 0x80;
/// Control bit marking the last packet of a message.
const END_OF_MESSAGE: u8 = 0x02;

/// Transaction channel payloads, per D4 revision.
pub mod tx {
    use super::{
        CMD_CLOSE_CHANNEL, CMD_CREDIT, CMD_CREDIT_REQUEST, CMD_EXIT, CMD_GET_SOCKET_ID, CMD_INIT,
        CMD_OPEN_CHANNEL, SERVICE,
    };

    #[must_use]
    pub fn init(revision: u8) -> Vec<u8> {
        vec![CMD_INIT, revision]
    }

    #[must_use]
    pub fn get_socket_id() -> Vec<u8> {
        let mut v = vec![CMD_GET_SOCKET_ID];
        v.extend_from_slice(SERVICE);
        v
    }

    /// Proposed packet size 256 each way and zero initial credit. Revision
    /// 0x10 adds a second credit field (reinkpy `protocol_0x10`).
    #[must_use]
    pub fn open_channel(revision: u8, s: u8) -> Vec<u8> {
        let mut v = vec![CMD_OPEN_CHANNEL, s, s, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00];
        if revision == 0x10 {
            v.extend_from_slice(&[0x00, 0x00]);
        }
        v
    }

    /// One credit for the printer to answer with.
    #[must_use]
    pub fn credit(s: u8) -> Vec<u8> {
        vec![CMD_CREDIT, s, s, 0x00, 0x01]
    }

    /// Revision 0x10 layout from reinkpy (d4lib): 0x0080, 0xFFFF.
    #[must_use]
    pub fn credit_request(revision: u8, s: u8) -> Vec<u8> {
        if revision == 0x10 {
            vec![CMD_CREDIT_REQUEST, s, s, 0x00, 0x80, 0xFF, 0xFF]
        } else {
            vec![CMD_CREDIT_REQUEST, s, s, 0x00, 0x00]
        }
    }

    /// Revision 0x10 carries one more byte (reinkpy `protocol_0x10`).
    #[must_use]
    pub fn close_channel(revision: u8, s: u8) -> Vec<u8> {
        if revision == 0x10 {
            vec![CMD_CLOSE_CHANNEL, s, s, 0x00]
        } else {
            vec![CMD_CLOSE_CHANNEL, s, s]
        }
    }

    #[must_use]
    pub fn exit() -> Vec<u8> {
        vec![CMD_EXIT]
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub reply: Duration,
    pub data: Duration,
    pub fragment: Duration,
    pub held_reply: Duration,
    pub between_packets: Duration,
}

impl Default for Timing {
    /// Epson-Waste-Reset defaults: 2 s replies, 250 ms fragments, 100 ms
    /// between packets. Data replies get 2 s too: a slow answer is only a
    /// wait, a missed one is an unknown state.
    fn default() -> Self {
        Self {
            reply: Duration::from_millis(2000),
            data: Duration::from_millis(2000),
            fragment: Duration::from_millis(250),
            held_reply: Duration::from_millis(300),
            between_packets: Duration::from_millis(100),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub psid: u8,
    pub ssid: u8,
    pub credit: u8,
    pub control: u8,
    pub payload: Vec<u8>,
}

impl Packet {
    fn is_transaction(&self) -> bool {
        self.psid == 0 && self.ssid == 0
    }
}

#[must_use]
pub fn encode(psid: u8, ssid: u8, credit: u8, payload: &[u8]) -> Vec<u8> {
    let total = u16::try_from(payload.len() + 6).expect("packet fits a u16 length");
    let mut out = vec![psid, ssid];
    out.extend_from_slice(&total.to_be_bytes());
    out.push(credit);
    out.push(0);
    out.extend_from_slice(payload);
    out
}

/// Offset of the first bytes that can start a packet: equal socket ids and a
/// length of at least the header. Anything before is driver noise or a
/// partial packet (Epson-Waste-Reset `FindPacketStart`).
fn packet_start(buf: &[u8]) -> usize {
    for i in 0..buf.len().saturating_sub(3) {
        let total = usize::from(u16::from_be_bytes([buf[i + 2], buf[i + 3]]));
        if buf[i] == buf[i + 1] && total >= 6 {
            return i;
        }
    }
    buf.len().saturating_sub(3)
}

pub struct Session<T: Transport> {
    transport: T,
    timing: Timing,
    buf: Vec<u8>,
    socket: u8,
    revision: u8,
    send_credit: u32,
    printer_credit: u32,
    /// Credit granted by `OpenChannel`, kept as a fallback.
    initial_credit: u32,
    /// The last exchange did not end with a complete reply.
    stale: bool,
    mtu_to_printer: usize,
    mtu_to_host: usize,
    pending: VecDeque<Packet>,
    open: bool,
    started: bool,
    pub log: Log,
}

impl<T: Transport> Session<T> {
    pub fn new(transport: T, timing: Timing) -> Self {
        Self {
            transport,
            timing,
            buf: Vec::new(),
            socket: DEFAULT_SOCKET,
            revision: 0x10,
            send_credit: 0,
            printer_credit: 0,
            initial_credit: 0,
            stale: false,
            mtu_to_printer: 256,
            mtu_to_host: 256,
            pending: VecDeque::new(),
            open: false,
            started: false,
            log: Log::default(),
        }
    }

    fn send_raw(&mut self, label: &str, data: &[u8]) -> Result<(), String> {
        self.log.bytes(&format!("OUT {label}"), data);
        self.transport.send(data)?;
        if !self.timing.between_packets.is_zero() {
            std::thread::sleep(self.timing.between_packets);
        }
        Ok(())
    }

    fn send_tx(&mut self, payload: &[u8]) -> Result<(), String> {
        let packet = encode(0, 0, 1, payload);
        self.send_raw("transaction", &packet)
    }

    fn read_packet(&mut self, deadline: Instant) -> Result<Option<Packet>, String> {
        loop {
            let start = packet_start(&self.buf);
            if start > 0 {
                // Before the channel opens nothing carries a serial number, and
                // these bytes show what the interface really speaks. Later, a
                // fragment could hold part of one, so only the length is kept.
                if self.open {
                    self.log.line(format!(
                        "dropped {start} byte(s) that do not start a packet"
                    ));
                } else {
                    self.log
                        .bytes("dropped (not a D4 packet)", &self.buf[..start]);
                }
                self.buf.drain(..start);
            }
            if self.buf.len() >= 6 {
                let total = usize::from(u16::from_be_bytes([self.buf[2], self.buf[3]]));
                if self.buf.len() >= total {
                    let raw: Vec<u8> = self.buf.drain(..total).collect();
                    if raw[0] == 0 && raw[1] == 0 {
                        self.log.bytes("IN transaction", &raw);
                    } else {
                        // Data is logged once reassembled, in `exchange`.
                        self.log.line(format!(
                            "IN data packet: socket {}, {} byte(s), credit {}, control 0x{:02X}",
                            raw[0],
                            raw.len(),
                            raw[4],
                            raw[5]
                        ));
                    }
                    return Ok(Some(Packet {
                        psid: raw[0],
                        ssid: raw[1],
                        credit: raw[4],
                        control: raw[5],
                        payload: raw[6..].to_vec(),
                    }));
                }
            }
            let now = Instant::now();
            if now >= deadline {
                if !self.buf.is_empty() {
                    if self.open {
                        self.log.line(format!(
                            "dropped {} byte(s) of an incomplete packet",
                            self.buf.len()
                        ));
                    } else {
                        self.log.bytes("dropped (incomplete)", &self.buf);
                    }
                    self.buf.clear();
                }
                return Ok(None);
            }
            let chunk = self.transport.recv(deadline - now)?;
            self.buf.extend_from_slice(&chunk);
        }
    }

    /// Books credit carried by any packet.
    fn absorb(&mut self, p: &Packet) {
        let s = self.socket;
        if !p.is_transaction() {
            if p.psid == s {
                self.printer_credit = self.printer_credit.saturating_sub(1);
                self.send_credit += u32::from(p.credit);
            }
            return;
        }
        let q = &p.payload;
        match q.first().copied() {
            // Result, both sockets, then the granted credit.
            Some(c) if c == CMD_CREDIT_REQUEST | REPLY && q.len() >= 6 => {
                if q[1] == 0x00 && q[2] == s && q[3] == s {
                    self.send_credit += u32::from(u16::from_be_bytes([q[4], q[5]]));
                } else {
                    self.log
                        .line("credit request reply ignored (refused or other socket)");
                }
            }
            Some(CMD_CREDIT) if q.len() >= 5 && q[1] == s && q[2] == s => {
                self.send_credit += u32::from(u16::from_be_bytes([q[3], q[4]]));
            }
            Some(CMD_ERROR) => {
                let code = p.payload.get(3).copied().unwrap_or(0xFF);
                self.log.line(format!("printer sent D4 error 0x{code:02X}"));
            }
            _ => {}
        }
    }

    fn wait_reply(&mut self, cmd: u8, timeout: Duration) -> Result<Option<Packet>, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let Some(p) = self.read_packet(deadline)? else {
                return Ok(None);
            };
            self.absorb(&p);
            if p.is_transaction() {
                match p.payload.first().copied() {
                    Some(c) if c == cmd | REPLY => return Ok(Some(p)),
                    Some(CMD_ERROR) => return Ok(None),
                    _ => {}
                }
            } else if p.psid == self.socket {
                self.pending.push_back(p);
            }
        }
    }

    fn wait_data(&mut self, timeout: Duration) -> Result<Option<Packet>, String> {
        if let Some(p) = self.pending.pop_front() {
            return Ok(Some(p));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let Some(p) = self.read_packet(deadline)? else {
                return Ok(None);
            };
            self.absorb(&p);
            if !p.is_transaction() && p.psid == self.socket {
                return Ok(Some(p));
            }
        }
    }

    fn init(&mut self, revision: u8, retry: bool) -> Result<(), String> {
        self.send_tx(&tx::init(revision))?;
        let reply = self
            .wait_reply(CMD_INIT, self.timing.reply)?
            .ok_or(NO_INIT_REPLY)?;
        let result = reply.payload.get(1).copied().unwrap_or(0xFF);
        let offered = reply.payload.get(2).copied();
        match (result, offered) {
            (0x00, _) => {
                self.revision = revision;
                Ok(())
            }
            (0x02, Some(other @ (0x10 | 0x20))) if retry && other != revision => {
                self.log
                    .line(format!("printer asks for D4 revision 0x{other:02X}"));
                self.init(other, false)
            }
            _ => Err(format!("D4 Init refused (result 0x{result:02X})")),
        }
    }

    /// EJL switch, `Init`, socket lookup and `OpenChannel`, then any reply held
    /// over from an earlier session is drained.
    pub fn start(&mut self) -> Result<(), String> {
        self.send_raw("EJL enter 1284.4", EJL_ENTER)?;
        let deadline = Instant::now() + self.timing.reply;
        if let Some(p) = self.read_packet(deadline)? {
            self.absorb(&p);
        }
        // Epson-Waste-Reset asks for revision 0x10, reinkpy for 0x20; the
        // only ET-28xx reported working (an ET-2820) went through reinkpy.
        // A printer that ignores one is asked with the other.
        match self.init(0x10, true) {
            Err(e) if e == NO_INIT_REPLY => {
                self.log
                    .line("no answer to Init 0x10; trying revision 0x20");
                self.init(0x20, true)?;
            }
            other => other?,
        }
        self.started = true;

        self.send_tx(&tx::get_socket_id())?;
        self.socket = match self.wait_reply(CMD_GET_SOCKET_ID, self.timing.reply)? {
            Some(p) if p.payload.len() >= 3 && p.payload[1] == 0 && p.payload[2] != 0 => {
                p.payload[2]
            }
            _ => {
                self.log.line("GetSocketID not answered; using socket 2");
                DEFAULT_SOCKET
            }
        };
        let s = self.socket;

        self.send_tx(&tx::open_channel(self.revision, s))?;
        let reply = self
            .wait_reply(CMD_OPEN_CHANNEL, self.timing.reply)?
            .ok_or("the printer did not open the EPSON-CTRL channel")?;
        if reply.payload.get(1).copied() != Some(0x00) {
            return Err(format!(
                "OpenChannel refused (result 0x{:02X})",
                reply.payload.get(1).copied().unwrap_or(0xFF)
            ));
        }
        if reply.payload.len() >= 8 {
            let to_printer = usize::from(u16::from_be_bytes([reply.payload[4], reply.payload[5]]));
            let to_host = usize::from(u16::from_be_bytes([reply.payload[6], reply.payload[7]]));
            if to_printer >= 7 {
                self.mtu_to_printer = to_printer;
            }
            if to_host >= 7 {
                self.mtu_to_host = to_host;
            }
        }
        // Granted credit sits in bytes 10-11 (reinkpy `OpenChannelReply`);
        // bytes 8-9 are the maximum, so a shorter reply grants nothing usable.
        // It is only a fallback for a credit request that grants nothing.
        self.initial_credit = if reply.payload.len() >= 12 {
            u32::from(u16::from_be_bytes([reply.payload[10], reply.payload[11]]))
        } else {
            0
        };
        self.open = true;
        self.stale = false;
        self.pending.clear();
        self.discard_held_replies();
        Ok(())
    }

    /// A printer can keep an unsent reply across sessions and hand it over
    /// once credit arrives, where it would pass for the next answer
    /// (Epson-Waste-Reset `DiscardHeldReplies`).
    fn discard_held_replies(&mut self) {
        for _ in 0..32 {
            if self.ensure_printer_credit().is_err() {
                return;
            }
            match self.wait_data(self.timing.held_reply) {
                Ok(Some(p)) => self.log.line(format!(
                    "discarded a held reply packet ({} byte(s))",
                    p.payload.len()
                )),
                _ => return,
            }
        }
    }

    fn ensure_printer_credit(&mut self) -> Result<(), String> {
        if self.printer_credit > 0 {
            return Ok(());
        }
        let s = self.socket;
        // Booked before sending: a held reply can arrive ahead of the
        // CreditReply and spend this credit while we wait.
        self.printer_credit += 1;
        let granted = self
            .send_tx(&tx::credit(s))
            .and_then(|()| self.wait_reply(CMD_CREDIT, self.timing.reply));
        let result = match granted {
            Ok(Some(reply)) if reply.payload.get(1).copied() == Some(0x00) => return Ok(()),
            Ok(Some(_)) => Err("the printer refused credit".to_string()),
            Ok(None) => Err("the printer did not accept credit".to_string()),
            Err(e) => Err(e),
        };
        self.printer_credit = 0;
        result
    }

    fn ensure_send_credit(&mut self) -> Result<(), String> {
        if self.send_credit > 0 {
            return Ok(());
        }
        let s = self.socket;
        self.send_tx(&tx::credit_request(self.revision, s))?;
        let reply = self
            .wait_reply(CMD_CREDIT_REQUEST, self.timing.reply)?
            .ok_or("the printer did not answer a credit request")?;
        // The fallback needs a valid answer for our channel that grants zero;
        // a refused or foreign reply is a failure.
        let q = &reply.payload;
        if q.len() < 6 || q[1] != 0x00 || q[2] != s || q[3] != s {
            return Err("the printer refused the credit request".into());
        }
        if self.send_credit == 0 && self.initial_credit > 0 {
            self.log.line(format!(
                "credit request granted nothing; using the {} credit(s) from OpenChannel",
                self.initial_credit
            ));
            self.send_credit = std::mem::take(&mut self.initial_credit);
        }
        if self.send_credit == 0 {
            return Err("the printer granted no credit".into());
        }
        Ok(())
    }

    /// Sends one EPSON-CTRL message and returns the whole reply. `None` means
    /// nothing came back; after a write that is an unknown state.
    pub fn exchange(&mut self, message: &[u8]) -> Result<Option<Vec<u8>>, String> {
        if self.stale {
            self.log
                .line("previous exchange was incomplete; starting a fresh session");
            self.restart()?;
        }
        if !self.open {
            return Err("the EPSON-CTRL channel is not open".into());
        }
        // Cleared only by a complete reply: any early return leaves the
        // session marked, so the next command starts afresh.
        self.stale = true;
        self.ensure_printer_credit()?;
        let chunk = self.mtu_to_printer.saturating_sub(6).max(1);
        for part in message.chunks(chunk) {
            self.ensure_send_credit()?;
            let packet = encode(self.socket, self.socket, 0, part);
            self.send_raw("data", &packet)?;
            self.send_credit -= 1;
        }
        let capacity = self.mtu_to_host.saturating_sub(6).max(1);
        let mut reply = Vec::new();
        for fragment in 0..16 {
            let timeout = if fragment == 0 {
                self.timing.data
            } else {
                self.timing.fragment
            };
            let Some(p) = self.wait_data(timeout)? else {
                break;
            };
            reply.extend_from_slice(&p.payload);
            if p.control & END_OF_MESSAGE != 0 || p.payload.len() < capacity {
                self.log.bytes("IN reply", &reply);
                self.stale = false;
                return Ok(Some(reply));
            }
            self.ensure_printer_credit()?;
        }
        // Nothing, or a reply whose end never came: its late parts could
        // answer the next question, so the caller starts a fresh session.
        if !reply.is_empty() {
            self.log.line(format!(
                "incomplete reply ({} byte(s)) discarded",
                reply.len()
            ));
        }
        Ok(None)
    }

    /// `CloseChannel` and `Exit`, best effort; the printer returns to normal
    /// printing either way.
    pub fn close(&mut self) {
        if !self.started {
            return;
        }
        let s = self.socket;
        if self.open {
            self.open = false;
            if self.send_tx(&tx::close_channel(self.revision, s)).is_ok() {
                let _ = self.wait_reply(CMD_CLOSE_CHANNEL, Duration::from_millis(500));
            }
        }
        if self.send_tx(&tx::exit()).is_ok() {
            let _ = self.wait_reply(CMD_EXIT, Duration::from_millis(500));
        }
        self.started = false;
    }

    /// Closes and starts afresh: after a missed reply neither side's credit
    /// count can be trusted, and Init resets both.
    pub fn restart(&mut self) -> Result<(), String> {
        self.close();
        self.buf.clear();
        self.pending.clear();
        self.send_credit = 0;
        self.printer_credit = 0;
        self.initial_credit = 0;
        self.start()
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }
}

impl<T: Transport> Drop for Session<T> {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_header() {
        assert_eq!(
            encode(0, 0, 1, &[0x00, 0x10]),
            [0, 0, 0, 8, 1, 0, 0x00, 0x10]
        );
    }

    #[test]
    fn finds_packet_after_noise() {
        assert_eq!(packet_start(&[0x55, 0x02, 0x02, 0x00, 0x08]), 1);
        assert_eq!(packet_start(&[0x02, 0x02, 0x00, 0x08]), 0);
        // Not enough bytes for a header: keep the tail for the next read.
        assert_eq!(packet_start(&[0x01, 0x02, 0x03, 0x04, 0x05]), 2);
    }
}
