//! USB printer access through `usbprint.sys`, the class driver Windows binds
//! to every USB printer. Nothing is installed or replaced: the tool opens the
//! same device interface the print spooler uses, shared.
//!
//! Enumeration and the overlapped read/write rules follow Epson-Waste-Reset
//! `usb_windows.cpp`: usbprint.sys can complete a read with zero bytes while
//! a reply is on its way, so an empty read means "not yet".

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile,
    WriteFile,
};
use windows_sys::Win32::System::IO::{
    CancelIoEx, DeviceIoControl, GetOverlappedResult, OVERLAPPED,
};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows_sys::core::GUID;

use crate::app::QueueHint;
use crate::d4::Transport;

/// `GUID_DEVINTERFACE_USBPRINT` (usbprint.h).
static USBPRINT: GUID = GUID::from_u128(0x28d7_8fad_5a12_11d1_ae5b_0000_f803_a8c2);
/// `IOCTL_USBPRINT_GET_1284_ID` (usbprint.h).
const GET_1284_ID: u32 = 0x0022_0034;
const WRITE_TIMEOUT: Duration = Duration::from_millis(2000);
const READ_CHUNK: usize = 512;
/// Bounds the reply size taken from one read loop.
const MAX_REPLY: usize = 64 * 1024;

/// One USB printer interface of an Epson device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Device interface path, for example `\\?\usb#vid_04b8&pid_xxxx&mi_00#...`.
    pub path: OsString,
    /// Interface number from the `mi_` part of the path, if composite.
    pub interface: Option<u8>,
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

fn interface_of(lower: &str) -> Option<u8> {
    let at = lower.find("mi_")?;
    u8::from_str_radix(lower.get(at + 3..at + 5)?, 16).ok()
}

/// Present USB printer interfaces with Epson's vendor id, interface 0 first,
/// then other interfaces, then single-function devices.
#[must_use]
pub fn enumerate() -> Vec<Candidate> {
    let mut out = Vec::new();
    // SAFETY: SetupAPI calls with a valid GUID pointer; the returned set is
    // destroyed below on every path.
    let set = unsafe {
        SetupDiGetClassDevsW(
            &raw const USBPRINT,
            null(),
            null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if set == INVALID_HANDLE_VALUE as isize {
        return out;
    }
    for index in 0.. {
        let mut data = SP_DEVICE_INTERFACE_DATA {
            cbSize: u32::try_from(size_of::<SP_DEVICE_INTERFACE_DATA>()).unwrap_or(0),
            ..unsafe { std::mem::zeroed() }
        };
        // SAFETY: `data` is a correctly sized, writable struct.
        if unsafe {
            SetupDiEnumDeviceInterfaces(set, null(), &raw const USBPRINT, index, &raw mut data)
        } == 0
        {
            break;
        }
        let mut needed = 0u32;
        // SAFETY: size query with a null buffer, as documented.
        unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                set,
                &raw const data,
                null_mut(),
                0,
                &raw mut needed,
                null_mut(),
            );
        }
        if !(8..=4096).contains(&needed) {
            continue;
        }
        // u64 storage keeps the struct aligned.
        let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
        let detail = storage
            .as_mut_ptr()
            .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        // SAFETY: the buffer is `needed` bytes and aligned; cbSize is the
        // fixed header size the API expects (8 on x64).
        let ok = unsafe {
            (*detail).cbSize =
                u32::try_from(size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>()).unwrap_or(0);
            SetupDiGetDeviceInterfaceDetailW(
                set,
                &raw const data,
                detail,
                needed,
                null_mut(),
                null_mut(),
            )
        };
        if ok == 0 {
            continue;
        }
        // SAFETY: DevicePath is a NUL-terminated string inside the buffer.
        let path = unsafe {
            let start = (&raw const (*detail).DevicePath).cast::<u16>();
            let max = (needed as usize - 4) / 2;
            let len = (0..max).take_while(|i| *start.add(*i) != 0).count();
            OsString::from_wide(std::slice::from_raw_parts(start, len))
        };
        let lower = path.to_string_lossy().to_ascii_lowercase();
        if lower.contains("vid_04b8") {
            out.push(Candidate {
                interface: interface_of(&lower),
                path,
            });
        }
    }
    // SAFETY: `set` came from SetupDiGetClassDevsW.
    unsafe { SetupDiDestroyDeviceInfoList(set) };
    out.sort_by_key(|c| match c.interface {
        Some(0) => (0, 0),
        Some(n) => (1, n),
        None => (2, 0),
    });
    out
}

fn wide_str(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: a NUL-terminated string owned by the EnumPrinters buffer.
    unsafe {
        let len = (0..4096).take_while(|i| *p.add(*i) != 0).count();
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }
}

/// Print queues for an ET-2870 series printer: whether Windows drives one
/// with its own IPP class driver, under which an XP-332 never answered D4
/// (Epson-Waste-Reset issue #55), and how many jobs are waiting.
#[must_use]
pub fn queue_hint() -> QueueHint {
    use windows_sys::Win32::Graphics::Printing::{
        EnumPrintersW, PRINTER_ENUM_LOCAL, PRINTER_INFO_2W,
    };
    let mut hint = QueueHint::default();
    let (mut needed, mut count) = (0u32, 0u32);
    // SAFETY: size query with a null buffer, as documented.
    unsafe {
        EnumPrintersW(
            PRINTER_ENUM_LOCAL,
            null(),
            2,
            null_mut(),
            0,
            &raw mut needed,
            &raw mut count,
        )
    };
    if needed == 0 || needed > 1 << 20 {
        return hint;
    }
    let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
    let buf = storage.as_mut_ptr().cast::<u8>();
    // SAFETY: the buffer holds `needed` bytes and is aligned for the structs.
    let ok = unsafe {
        EnumPrintersW(
            PRINTER_ENUM_LOCAL,
            null(),
            2,
            buf,
            needed,
            &raw mut needed,
            &raw mut count,
        )
    };
    if ok == 0 {
        return hint;
    }
    let infos = buf.cast::<PRINTER_INFO_2W>();
    for i in 0..count as usize {
        // SAFETY: EnumPrintersW wrote `count` structs at the buffer start.
        let info = unsafe { &*infos.add(i) };
        let name = wide_str(info.pPrinterName);
        let driver = wide_str(info.pDriverName);
        if !name.to_ascii_uppercase().contains("ET-287") {
            continue;
        }
        hint.queues
            .push(format!("{name} / {driver} / {} job(s)", info.cJobs));
        hint.ready_print |= driver
            .trim()
            .eq_ignore_ascii_case("Microsoft IPP Class Driver");
        hint.waiting_jobs += info.cJobs;
    }
    hint
}

/// Owned handle that is closed on drop.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is valid and owned.
        unsafe { CloseHandle(self.0) };
    }
}

fn open(path: &OsString, overlapped: bool) -> Result<Handle, String> {
    let w = wide(path);
    let flags = if overlapped { FILE_FLAG_OVERLAPPED } else { 0 };
    // SAFETY: `w` is NUL-terminated and lives across the call.
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        // SAFETY: plain thread-local error read.
        return Err(format!(
            "could not open the printer (Windows error {})",
            unsafe { GetLastError() }
        ));
    }
    Ok(Handle(h))
}

/// The IEEE 1284 device id the printer reports to the USB driver, read
/// without sending anything to the printer's command channel.
pub fn device_id(c: &Candidate) -> Result<String, String> {
    let h = open(&c.path, false)?;
    let mut buf = vec![0u8; 1024];
    let mut got = 0u32;
    // SAFETY: valid handle and output buffer of the stated size.
    let ok = unsafe {
        DeviceIoControl(
            h.0,
            GET_1284_ID,
            null(),
            0,
            buf.as_mut_ptr().cast(),
            u32::try_from(buf.len()).unwrap_or(0),
            &raw mut got,
            null_mut(),
        )
    };
    if ok == 0 || got < 2 {
        return Err("the printer did not report its device id".into());
    }
    buf.truncate(got as usize);
    id_from_ioctl(&buf)
}

/// The reply normally starts with a two-byte big-endian length that includes
/// itself; some drivers strip it. Realistic ids are far below 8 KiB, so a
/// prefixed reply starts with a byte below 0x20 (Epson-Waste-Reset
/// `ExtractDeviceIdString`).
fn id_from_ioctl(buf: &[u8]) -> Result<String, String> {
    let body = if buf.first().is_some_and(|b| *b < 0x20) {
        let declared = usize::from(u16::from_be_bytes([buf[0], *buf.get(1).unwrap_or(&0)]));
        if declared < 2 || declared > buf.len() {
            return Err("the printer's device id was cut short".into());
        }
        &buf[2..declared]
    } else {
        buf
    };
    // Some drivers return a prefix of other non-text bytes (an ET-2870 under
    // the Microsoft IPP class driver did); the id itself is plain ASCII.
    let start = body
        .iter()
        .position(u8::is_ascii_graphic)
        .unwrap_or(body.len());
    let body = &body[start..];
    let end = body.iter().position(|b| *b == 0).unwrap_or(body.len());
    Ok(String::from_utf8_lossy(&body[..end]).into_owned())
}

/// Overlapped byte pipe to one printer interface.
pub struct UsbPrint {
    handle: Handle,
    event: Handle,
}

impl UsbPrint {
    pub fn open(c: &Candidate) -> Result<Self, String> {
        let handle = open(&c.path, true)?;
        // SAFETY: anonymous manual-reset event.
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        if event.is_null() {
            return Err("could not create an I/O event".into());
        }
        Ok(Self {
            handle,
            event: Handle(event),
        })
    }

    fn overlapped(&self) -> OVERLAPPED {
        // SAFETY: OVERLAPPED is plain data; zeroed is its documented start state.
        let mut o: OVERLAPPED = unsafe { std::mem::zeroed() };
        o.hEvent = self.event.0;
        o
    }

    /// Waits for a pending operation; on timeout cancels it and returns what
    /// was transferred before the cancel landed.
    fn finish(&self, o: &OVERLAPPED, timeout: Duration) -> (u32, bool) {
        let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let mut done = 0u32;
        // SAFETY: `o` is the OVERLAPPED of the operation in flight on this handle.
        unsafe {
            if WaitForSingleObject(self.event.0, ms) == WAIT_OBJECT_0 {
                let ok = GetOverlappedResult(self.handle.0, o, &raw mut done, 0) != 0;
                return (done, ok);
            }
            CancelIoEx(self.handle.0, o);
            let ok = GetOverlappedResult(self.handle.0, o, &raw mut done, 1) != 0;
            (done, ok && done > 0)
        }
    }
}

impl Transport for UsbPrint {
    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        let mut o = self.overlapped();
        let mut written = 0u32;
        let len = u32::try_from(data.len()).map_err(|_| "packet too large")?;
        // SAFETY: `data` and `o` outlive the operation: it completes or is
        // cancelled and waited for in `finish` before this returns.
        let ok = unsafe {
            WriteFile(
                self.handle.0,
                data.as_ptr(),
                len,
                &raw mut written,
                &raw mut o,
            )
        };
        if ok == 0 {
            // SAFETY: plain thread-local error read.
            let err = unsafe { GetLastError() };
            if err != ERROR_IO_PENDING {
                return Err(format!("USB write failed (Windows error {err})"));
            }
            let (n, done) = self.finish(&o, WRITE_TIMEOUT);
            if !done {
                return Err(format!("USB write timed out ({n} of {len} bytes)"));
            }
            written = n;
        }
        if written != len {
            return Err(format!("USB write incomplete ({written} of {len} bytes)"));
        }
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + timeout;
        let mut buf = vec![0u8; READ_CHUNK];
        loop {
            let mut o = self.overlapped();
            let mut got = 0u32;
            // SAFETY: as in `send`; the read finishes or is cancelled and
            // waited for before `buf` and `o` go out of scope.
            let ok = unsafe {
                ReadFile(
                    self.handle.0,
                    buf.as_mut_ptr(),
                    u32::try_from(READ_CHUNK).unwrap_or(0),
                    &raw mut got,
                    &raw mut o,
                )
            };
            if ok == 0 {
                // SAFETY: plain thread-local error read.
                let err = unsafe { GetLastError() };
                if err != ERROR_IO_PENDING {
                    return Err(format!("USB read failed (Windows error {err})"));
                }
                let left = deadline.saturating_duration_since(Instant::now());
                let (n, _) = self.finish(&o, left);
                got = n;
            }
            if got > 0 {
                buf.truncate((got as usize).min(MAX_REPLY));
                return Ok(buf);
            }
            if Instant::now() >= deadline {
                return Ok(Vec::new());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_with_and_without_prefix() {
        let id = b"MFG:EPSON;MDL:ET-2870 Series;";
        let mut prefixed = u16::try_from(id.len() + 2).unwrap().to_be_bytes().to_vec();
        prefixed.extend_from_slice(id);
        assert_eq!(
            id_from_ioctl(&prefixed).unwrap(),
            "MFG:EPSON;MDL:ET-2870 Series;"
        );
        assert_eq!(id_from_ioctl(id).unwrap(), "MFG:EPSON;MDL:ET-2870 Series;");
        prefixed.truncate(10);
        assert!(id_from_ioctl(&prefixed).is_err());
        let mut odd = vec![0xCF, 0xA0];
        odd.extend_from_slice(id);
        assert_eq!(
            id_from_ioctl(&odd).unwrap(),
            "MFG:EPSON;MDL:ET-2870 Series;"
        );
    }

    #[test]
    fn interface_numbers() {
        assert_eq!(
            interface_of(r"\\?\usb#vid_04b8&pid_1234&mi_01#7&abc"),
            Some(1)
        );
        assert_eq!(interface_of(r"\\?\usb#vid_04b8&pid_1234#abc"), None);
    }
}
