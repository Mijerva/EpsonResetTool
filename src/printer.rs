//! EPSON-CTRL commands over a D4 session, with the retry rules the reset
//! depends on: a missed reply restarts the session, and a write is never
//! repeated blindly.
#![forbid(unsafe_code)]

use crate::ctrl::{self, DeviceId, ReadReply, Status, WriteReply};
use crate::d4::{Session, Timing, Transport};

/// What a write did, as far as the printer said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteAck {
    Ok,
    Refused,
    /// No reply or an unexpected one: the byte may or may not have changed.
    Unknown,
}

/// The EEPROM operations the reset uses, so its rules can be tested apart
/// from USB.
pub trait Eeprom {
    fn read(&mut self, addr: u16) -> Result<u8, String>;
    fn write(&mut self, addr: u16, value: u8) -> Result<WriteAck, String>;
    /// Fresh session after an unknown outcome.
    fn recover(&mut self) -> Result<(), String>;
}

pub struct Printer<T: Transport> {
    pub session: Session<T>,
}

impl<T: Transport> Printer<T> {
    pub fn open(transport: T, timing: Timing) -> Result<Self, String> {
        Self::open_logged(transport, timing, &mut crate::log::Log::default())
    }

    /// As `open`, but a failed handshake leaves its transcript in `log`:
    /// that is the evidence of what the interface answered.
    pub fn open_logged(
        transport: T,
        timing: Timing,
        log: &mut crate::log::Log,
    ) -> Result<Self, String> {
        let mut session = Session::new(transport, timing);
        match session.start() {
            Ok(()) => Ok(Self { session }),
            Err(e) => {
                log.append(std::mem::take(&mut session.log));
                Err(e)
            }
        }
    }

    fn ask(&mut self, message: &[u8]) -> Result<Vec<u8>, String> {
        if let Some(reply) = self.session.exchange(message)? {
            return Ok(reply);
        }
        self.session
            .log
            .line("no reply; restarting the session once");
        self.session.restart()?;
        self.session
            .exchange(message)?
            .ok_or_else(|| "the printer stopped answering".to_string())
    }

    pub fn device_id(&mut self) -> Result<DeviceId, String> {
        let reply = self.ask(&ctrl::device_id())?;
        Ok(ctrl::parse_device_id(&String::from_utf8_lossy(&reply)))
    }

    pub fn status(&mut self) -> Result<Option<Status>, String> {
        let reply = self.ask(&ctrl::status())?;
        Ok(ctrl::parse_status(&reply))
    }
}

impl<T: Transport> Eeprom for Printer<T> {
    fn read(&mut self, addr: u16) -> Result<u8, String> {
        for attempt in 0..2 {
            let reply = self.ask(&ctrl::read_eeprom(addr))?;
            match ctrl::parse_read(&reply, addr) {
                ReadReply::Value(v) => return Ok(v),
                ReadReply::Refused => {
                    return Err(format!(
                        "the printer refused to read address 0x{addr:03X} (read key not accepted)"
                    ));
                }
                ReadReply::WrongAddress | ReadReply::Other if attempt == 0 => {
                    self.session.log.line(format!(
                        "unexpected reply reading 0x{addr:03X}; restarting the session"
                    ));
                    self.session.restart()?;
                }
                _ => {}
            }
        }
        Err(format!("no valid reply reading address 0x{addr:03X}"))
    }

    fn write(&mut self, addr: u16, value: u8) -> Result<WriteAck, String> {
        // A transport error after sending may still have changed the byte;
        // the caller recovers and reads back.
        let reply = match self.session.exchange(&ctrl::write_eeprom(addr, value)) {
            Ok(Some(reply)) => reply,
            Ok(None) => return Ok(WriteAck::Unknown),
            Err(e) => {
                self.session.log.line(format!("write to 0x{addr:03X}: {e}"));
                return Ok(WriteAck::Unknown);
            }
        };
        Ok(match ctrl::parse_write(&reply) {
            WriteReply::Ok => WriteAck::Ok,
            WriteReply::Refused => WriteAck::Refused,
            WriteReply::Other => WriteAck::Unknown,
        })
    }

    fn recover(&mut self) -> Result<(), String> {
        self.session.restart()
    }
}
