//! What the two buttons do, apart from the window and from USB, so the whole
//! flow can be tested against the simulated printer.
#![forbid(unsafe_code)]

use crate::ctrl::{self, Status};
use crate::d4::{Timing, Transport};
use crate::log::Log;
use crate::model::is_supported;
use crate::printer::Printer;
use crate::reset::{self, Outcome, Snapshot};
use crate::text::Text;

/// Access to the USB printers on this PC.
pub trait Bus {
    type Id: Clone + PartialEq + std::fmt::Debug;
    type T: Transport;
    fn candidates(&self) -> Vec<Self::Id>;
    /// Device id string from the USB driver; sends nothing to the printer's
    /// command channel.
    fn device_id(&self, id: &Self::Id) -> Result<String, String>;
    fn open(&self, id: &Self::Id) -> Result<Self::T, String>;
    fn describe(&self, id: &Self::Id) -> String;
    /// What Windows' print queues say about the printer, for advice when it
    /// does not answer.
    fn queue_hint(&self) -> QueueHint {
        QueueHint::default()
    }
}

#[derive(Debug, Clone, Default)]
pub struct QueueHint {
    /// A queue uses the Microsoft IPP Class Driver.
    pub ready_print: bool,
    pub waiting_jobs: u32,
    /// Name, driver and jobs per queue, for the log.
    pub queues: Vec<String>,
}

/// The no-answer message with the advice that fits the print queues.
fn no_answer<B: Bus>(bus: &B, t: &Text, detail: &str, log: &mut Log) -> String {
    let hint = bus.queue_hint();
    for q in &hint.queues {
        log.line(format!("print queue: {q}"));
    }
    let mut message = t.no_answer.to_string();
    if hint.waiting_jobs > 0 {
        message = format!("{message}\r\n\r\n{}", t.jobs_waiting);
    }
    if hint.ready_print {
        message = format!("{message}\r\n\r\n{}", t.ready_print);
    }
    message = format!("{message}\r\n\r\n{}", t.press_driver);
    with_details(t, &message, detail)
}

/// The printer and state the user saw before confirming the reset.
#[derive(Debug, Clone)]
pub struct Confirmed<I> {
    pub id: I,
    pub device_id: String,
    pub snapshot: Snapshot,
}

pub struct Report<I> {
    pub message: String,
    /// Set when a reset may be offered for this read.
    pub confirmed: Option<Confirmed<I>>,
    /// The printer did not answer: offer Epson's driver and connection check.
    pub offer_help: bool,
    pub log: Log,
}

fn with_details(t: &Text, message: &str, detail: &str) -> String {
    format!("{message}\r\n\r\n{}: {detail}", t.details)
}

struct Found<B: Bus> {
    id: B::Id,
    device_id: String,
    printer: Printer<B::T>,
}

enum NotFound {
    NoPrinter,
    Unsupported(String),
    NoAnswer(String),
}

fn open_supported<B: Bus>(bus: &B, timing: Timing, log: &mut Log) -> Result<Found<B>, NotFound> {
    let mut unsupported = None;
    let mut failure = None;
    let candidates = bus.candidates();
    log.line(format!(
        "{} Epson USB printer interface(s)",
        candidates.len()
    ));
    for id in candidates {
        log.line(format!("interface {}", bus.describe(&id)));
        let device_id = match bus.device_id(&id) {
            Ok(d) => d,
            Err(e) => {
                log.line(format!("  device id: {e}"));
                failure.get_or_insert(e);
                continue;
            }
        };
        log.bytes("  device id", device_id.as_bytes());
        let parsed = ctrl::parse_device_id(&device_id);
        if !is_supported(&parsed.manufacturer, &parsed.model) {
            unsupported.get_or_insert(parsed.model);
            continue;
        }
        match bus
            .open(&id)
            .and_then(|tr| Printer::open_logged(tr, timing, log))
        {
            Ok(printer) => {
                return Ok(Found {
                    id,
                    device_id,
                    printer,
                });
            }
            Err(e) => {
                log.line(format!("  open: {e}"));
                failure.get_or_insert(e);
            }
        }
    }
    Err(match (failure, unsupported) {
        (Some(e), _) => NotFound::NoAnswer(e),
        (None, Some(model)) => NotFound::Unsupported(model),
        (None, None) => NotFound::NoPrinter,
    })
}

fn status_line(t: &Text, status: Option<&Status>) -> &'static str {
    match status {
        Some(s) if s.waste_error() => t.pads_full,
        Some(s) if s.error.is_none() && !s.truncated => t.pads_ok,
        _ => t.status_unknown,
    }
}

/// Model check over the command channel, status and the double read. The
/// error is the message for the user.
fn inspect<T: Transport>(
    p: &mut Printer<T>,
    t: &Text,
    log: &mut Log,
) -> Result<(String, Option<Status>, Snapshot), String> {
    let id = p
        .device_id()
        .map_err(|e| with_details(t, t.no_answer, &e))?;
    if !is_supported(&id.manufacturer, &id.model) {
        return Err(t.not_supported.replace("{model}", &id.model));
    }
    let status = p.status().unwrap_or_else(|e| {
        log.line(format!("status: {e}"));
        None
    });
    let snapshot = reset::read_snapshot(p).map_err(|e| with_details(t, t.read_failed, &e))?;
    Ok((id.model, status, snapshot))
}

/// "Read printer status" and "Check again".
pub fn read<B: Bus>(bus: &B, t: &Text, timing: Timing) -> Report<B::Id> {
    let mut log = Log::default();
    log.line(format!(
        "== read printer status (EpsonReset {})",
        env!("CARGO_PKG_VERSION")
    ));
    let report = |message: String, log: Log| Report {
        message,
        confirmed: None,
        offer_help: false,
        log,
    };
    let mut found = match open_supported(bus, timing, &mut log) {
        Ok(f) => f,
        Err(NotFound::Unsupported(model)) => {
            return report(t.not_supported.replace("{model}", &model), log);
        }
        Err(NotFound::NoAnswer(e)) => {
            let message = no_answer(bus, t, &e, &mut log);
            return Report {
                message,
                confirmed: None,
                offer_help: true,
                log,
            };
        }
        Err(NotFound::NoPrinter) => return report(t.no_printer.to_string(), log),
    };
    let inspected = inspect(&mut found.printer, t, &mut log);
    log.append(std::mem::take(&mut found.printer.session.log));
    let (model, status, snapshot) = match inspected {
        Ok(v) => v,
        Err(message) => return report(message, log),
    };
    log.line(format!("status: {status:?}"));
    log.line(format!(
        "snapshot:
{}",
        snapshot.to_text()
    ));

    let heading = t.found.replace("{model}", &model);
    let waste = status.as_ref().is_some_and(Status::waste_error);
    if let Some(code) = status.as_ref().and_then(|s| s.error).filter(|_| !waste) {
        let other = t.other_error.replace("{code}", &format!("0x{code:02X}"));
        return report(
            format!(
                "{heading}

{other}"
            ),
            log,
        );
    }
    if snapshot.is_reset() {
        let message = if waste {
            t.still_error
        } else if status.as_ref().is_some_and(|s| !s.truncated) {
            t.all_good
        } else {
            t.nothing_to_change
        };
        return report(
            format!(
                "{heading}

{message}"
            ),
            log,
        );
    }
    Report {
        message: format!(
            "{heading}

{}
{}",
            status_line(t, status.as_ref()),
            t.press_reset
        ),
        confirmed: Some(Confirmed {
            id: found.id,
            device_id: found.device_id,
            snapshot,
        }),
        offer_help: false,
        log,
    }
}

/// "Reset E-11 error", only after the user confirmed `c`.
pub fn reset<B: Bus>(
    bus: &B,
    t: &Text,
    timing: Timing,
    c: &Confirmed<B::Id>,
) -> (String, Log, bool) {
    let mut log = Log::default();
    log.line(format!(
        "== reset (EpsonReset {})",
        env!("CARGO_PKG_VERSION")
    ));
    // The same physical printer: same interface path and same device id.
    match bus.device_id(&c.id) {
        Ok(now) if now == c.device_id => {}
        Ok(_) | Err(_) => {
            log.line("device id differs from the read; not resetting");
            return (t.changed_since_read.to_string(), log, false);
        }
    }
    let mut p = match bus
        .open(&c.id)
        .and_then(|tr| Printer::open_logged(tr, timing, &mut log))
    {
        Ok(p) => p,
        Err(e) => {
            let message = no_answer(bus, t, &e, &mut log);
            return (message, log, false);
        }
    };
    let ok_model = p
        .device_id()
        .is_ok_and(|d| is_supported(&d.manufacturer, &d.model));
    if !ok_model {
        log.append(std::mem::take(&mut p.session.log));
        return (t.changed_since_read.to_string(), log, false);
    }
    // An error that appeared after the read (paper, cover) is fixed first,
    // as in read(). An unreadable status does not block: every write is
    // read back anyway.
    if let Ok(Some(s)) = p.status()
        && let Some(code) = s.error.filter(|_| !s.waste_error())
    {
        log.append(std::mem::take(&mut p.session.log));
        log.line(format!("status before reset: {s:?}; not resetting"));
        let message = t.other_error.replace("{code}", &format!("0x{code:02X}"));
        return (message, log, false);
    }
    let outcome = reset::reset(&mut p, &c.snapshot);
    log.append(std::mem::take(&mut p.session.log));
    log.line(format!("outcome: {outcome:?}"));
    match outcome {
        Ok(Outcome::Done { .. }) => (t.done.to_string(), log, true),
        Ok(Outcome::NothingToChange) => (t.nothing_to_change.to_string(), log, true),
        Ok(Outcome::ChangedSinceRead) => (t.changed_since_read.to_string(), log, false),
        Ok(Outcome::FirstWriteNotVerified { addr, detail }) => (
            with_details(t, t.first_write_failed, &format!("0x{addr:03X}: {detail}")),
            log,
            false,
        ),
        Ok(Outcome::Failed {
            addr,
            detail,
            written,
        }) => (
            with_details(
                t,
                t.partial,
                &format!(
                    "0x{addr:03X}: {detail}; {} byte(s) verified before",
                    written.len()
                ),
            ),
            log,
            false,
        ),
        Err(e) => (with_details(t, t.partial, &e), log, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::SimPrinter;
    use crate::text::{Lang, text};
    use std::cell::RefCell;
    use std::time::Duration;

    /// One simulated printer whose memory persists across sessions.
    struct SimBus {
        model: &'static str,
        state: RefCell<Option<SimPrinter>>,
    }

    struct Shared<'a>(&'a RefCell<Option<SimPrinter>>);

    impl Transport for Shared<'_> {
        fn send(&mut self, d: &[u8]) -> Result<(), String> {
            self.0.borrow_mut().as_mut().unwrap().send(d)
        }
        fn recv(&mut self, t: Duration) -> Result<Vec<u8>, String> {
            self.0.borrow_mut().as_mut().unwrap().recv(t)
        }
    }

    impl<'a> Bus for &'a SimBus {
        type Id = u8;
        type T = Shared<'a>;
        fn candidates(&self) -> Vec<u8> {
            vec![0]
        }
        fn device_id(&self, _: &u8) -> Result<String, String> {
            Ok(format!("MFG:EPSON;MDL:{};CLS:PRINTER;", self.model))
        }
        fn open(&self, _: &u8) -> Result<Shared<'a>, String> {
            Ok(Shared(&self.state))
        }
        fn describe(&self, _: &u8) -> String {
            "sim".into()
        }
    }

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
    fn read_reset_check() {
        let bus = SimBus {
            model: "ET-2870 Series",
            state: RefCell::new(Some(SimPrinter::full())),
        };
        let t = text(Lang::En);
        let r = read(&&bus, t, fast());
        assert!(r.message.contains(t.pads_full), "{}", r.message);
        let c = r.confirmed.expect("reset offered");
        let (msg, _, ok) = reset(&&bus, t, fast(), &c);
        assert!(ok, "{msg}");
        let again = read(&&bus, t, fast());
        assert!(again.message.contains(t.all_good), "{}", again.message);
        assert!(again.confirmed.is_none());
    }

    #[test]
    fn new_error_after_read_blocks_reset() {
        let bus = SimBus {
            model: "ET-2870 Series",
            state: RefCell::new(Some(SimPrinter::full())),
        };
        let t = text(Lang::En);
        let c = read(&&bus, t, fast()).confirmed.expect("reset offered");
        bus.state.borrow_mut().as_mut().unwrap().error = Some(0x04);
        let (msg, _, ok) = reset(&&bus, t, fast(), &c);
        assert!(!ok);
        assert!(msg.contains("0x04"), "{msg}");
        assert_eq!(bus.state.borrow().as_ref().unwrap().writes.len(), 0);
    }

    #[test]
    fn unsupported_model_is_never_opened() {
        let bus = SimBus {
            model: "ET-2820 Series",
            state: RefCell::new(None),
        };
        let t = text(Lang::En);
        let r = read(&&bus, t, fast());
        assert!(r.message.contains("ET-2820"));
        assert!(r.confirmed.is_none());
    }

    #[test]
    fn log_has_no_serial() {
        let bus = SimBus {
            model: "ET-2870 Series",
            state: RefCell::new(Some(SimPrinter::full())),
        };
        let r = read(&&bus, text(Lang::En), fast());
        // Rebuild the logged bytes from the hex columns; the ASCII column
        // wraps every 16 bytes.
        let mut bytes = Vec::new();
        for line in r.log.text().lines() {
            if line.len() > 8
                && line.starts_with("  ")
                && line[2..6].chars().all(|c| c.is_ascii_hexdigit())
            {
                let hex = &line[8..(8 + 48).min(line.len())];
                bytes.extend(
                    hex.split_whitespace()
                        .filter_map(|h| u8::from_str_radix(h, 16).ok()),
                );
            }
        }
        let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(has(b"MDL:ET-2870 Series;"), "the dump was parsed");
        assert!(!has(b"SIM0001"));
        assert!(has(b"SN:*******;"));
    }
}
