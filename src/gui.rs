//! The window: a heading, the pad warning, a status box and three buttons.
//! Plain Win32 controls, so the program stays one small file with nothing to
//! install. Printer work runs on a worker thread; the window only shows
//! results, and it cannot be closed while a reset is writing.

use std::cell::RefCell;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, GlobalFree};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, SYSTEMTIME, WPARAM};
use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
use windows_sys::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_WINDOW, CreateFontW, DEFAULT_CHARSET,
    DeleteObject, FF_DONTCARE, FW_BOLD, FW_NORMAL, GetSysColor, GetSysColorBrush, HDC, HFONT,
    OUT_DEFAULT_PRECIS, SetBkColor, SetTextColor,
};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
    SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BS_PUSHBUTTON, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, FindWindowW, GetMessageW, GetSystemMetrics,
    IDC_ARROW, IDYES, KillTimer, LoadCursorW, MB_DEFBUTTON2, MB_ICONINFORMATION, MB_ICONWARNING,
    MB_OK, MB_YESNO, MSG, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassExW,
    SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_RESTORE, SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOZORDER,
    SendMessageW, SetForegroundWindow, SetTimer, SetWindowPos, SetWindowTextW, ShowWindow,
    TranslateMessage, WM_APP, WM_CLOSE, WM_COMMAND, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED,
    WM_SETFONT, WM_TIMER, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_MINIMIZEBOX,
    WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::app::{self, Bus, Confirmed};
use crate::ctrl;
use crate::d4::Timing;
use crate::model::is_supported;
use crate::store::Store;
use crate::text::{Lang, Text, text};
use crate::usb::{self, Candidate, UsbPrint};

const ID_READ: usize = 101;
const ID_RESET: usize = 102;
const ID_COPY: usize = 103;
const ID_DRIVER: usize = 104;
const ID_CHECKER: usize = 105;
/// `CF_UNICODETEXT` (winuser.h).
const CF_UNICODETEXT: u32 = 13;
const WM_WORKER: u32 = WM_APP + 1;
const TIMER_DETECT: usize = 1;
/// Retries worker results that arrived while a dialog was open.
const TIMER_WORKER: usize = 2;
/// While nothing has been read, look for a newly plugged printer this often.
const DETECT_MS: u32 = 2000;
/// Warning text colour, dark red.
const WARNING_RGB: u32 = 0x0012_2A9A;

#[cfg(not(feature = "demo"))]
type ActiveBus = WinBus;
#[cfg(feature = "demo")]
type ActiveBus = demo::DemoBus;
type BusId = <ActiveBus as Bus>::Id;

/// Screenshot build: one simulated printer shared by every session.
#[cfg(feature = "demo")]
mod demo {
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use crate::app::Bus;
    use crate::d4::Transport;
    use crate::sim::SimPrinter;

    fn printer() -> &'static Mutex<SimPrinter> {
        static P: OnceLock<Mutex<SimPrinter>> = OnceLock::new();
        P.get_or_init(|| {
            let mut sim = SimPrinter::full();
            // Screenshot of the "no answer" state.
            if std::env::var_os("EPSONRESET_DEMO_SILENT").is_some() {
                sim.quirk = crate::sim::Quirk::Silent;
            }
            Mutex::new(sim)
        })
    }

    pub struct DemoBus;
    pub struct DemoLink;

    impl Transport for DemoLink {
        fn send(&mut self, d: &[u8]) -> Result<(), String> {
            std::thread::sleep(Duration::from_millis(15));
            printer().lock().map_err(|e| e.to_string())?.send(d)
        }
        fn recv(&mut self, t: Duration) -> Result<Vec<u8>, String> {
            printer().lock().map_err(|e| e.to_string())?.recv(t)
        }
    }

    impl Bus for DemoBus {
        type Id = u8;
        type T = DemoLink;
        fn candidates(&self) -> Vec<u8> {
            vec![0]
        }
        fn device_id(&self, _: &u8) -> Result<String, String> {
            Ok("MFG:EPSON;CMD:ESCPL2,BDC,D4;MDL:ET-2870 Series;CLS:PRINTER;".into())
        }
        fn open(&self, _: &u8) -> Result<DemoLink, String> {
            Ok(DemoLink)
        }
        fn describe(&self, _: &u8) -> String {
            "demo".into()
        }
        fn queue_hint(&self) -> crate::app::QueueHint {
            crate::app::QueueHint {
                ready_print: std::env::var_os("EPSONRESET_DEMO_SILENT").is_some(),
                ..crate::app::QueueHint::default()
            }
        }
    }
}

#[cfg_attr(feature = "demo", allow(dead_code))]
struct WinBus;

impl Bus for WinBus {
    type Id = Candidate;
    type T = UsbPrint;
    fn candidates(&self) -> Vec<Candidate> {
        usb::enumerate()
    }
    fn device_id(&self, id: &Candidate) -> Result<String, String> {
        usb::device_id(id)
    }
    fn open(&self, id: &Candidate) -> Result<UsbPrint, String> {
        UsbPrint::open(id)
    }
    fn queue_hint(&self) -> app::QueueHint {
        usb::queue_hint()
    }
    fn describe(&self, id: &Candidate) -> String {
        match id.interface {
            Some(n) => format!("USB printer interface {n}"),
            None => "USB printer interface".into(),
        }
    }
}

enum Msg {
    Detected(String),
    Read(app::Report<BusId>),
    Reset(String, crate::log::Log, bool),
    Progress(String),
    /// Message, log, and the package file name when it was started.
    Fetched(String, crate::log::Log, Option<&'static str>),
}

#[allow(clippy::struct_excessive_bools, reason = "independent UI flags")]
struct Ui {
    hwnd: HWND,
    heading: HWND,
    warning: HWND,
    status: HWND,
    read: HWND,
    reset: HWND,
    logs: HWND,
    driver: HWND,
    checker: HWND,
    /// The driver and connection check buttons are shown.
    help: bool,
    dpi: u32,
    /// Epson programs started in this run.
    launched: Vec<&'static str>,
    fonts: Vec<HFONT>,
    t: &'static Text,
    store: Store,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    busy: bool,
    resetting: bool,
    detecting: bool,
    /// Set once a full read happened; stops the background detection.
    read_once: bool,
    confirmed: Option<Confirmed<BusId>>,
    /// Everything logged this run, for the copy button.
    log_text: String,
}

/// The warning label, for painting it red.
static WARNING: AtomicIsize = AtomicIsize::new(0);

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

fn stamp() -> String {
    // SAFETY: GetLocalTime fills the struct.
    let t: SYSTEMTIME = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&raw mut t);
        t
    };
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}-{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

fn set_text(hwnd: HWND, s: &str) {
    // The multi-line edit control only breaks lines at CR LF.
    let w = wide(&s.replace("\r\n", "\n").replace('\n', "\r\n"));
    // SAFETY: valid window and NUL-terminated text.
    unsafe { SetWindowTextW(hwnd, w.as_ptr()) };
}

fn enable(hwnd: HWND, on: bool) {
    // SAFETY: valid window handle.
    unsafe { EnableWindow(hwnd, i32::from(on)) };
}

fn font(points: i32, bold: bool, dpi: u32) -> HFONT {
    let face = wide("Segoe UI");
    let height = -(points * i32::try_from(dpi).unwrap_or(96) / 72);
    // SAFETY: plain GDI font creation; freed in `layout` or on destroy.
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            i32::try_from(if bold { FW_BOLD } else { FW_NORMAL }).unwrap_or(400),
            0,
            0,
            0,
            u32::from(DEFAULT_CHARSET),
            u32::from(OUT_DEFAULT_PRECIS),
            u32::from(CLIP_DEFAULT_PRECIS),
            u32::from(CLEARTYPE_QUALITY),
            u32::from(FF_DONTCARE),
            face.as_ptr(),
        )
    }
}

/// Client size in 96 dpi units.
const CLIENT_W: i32 = 560;
const CLIENT_H: i32 = 430;
const STYLE: u32 = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;

fn scale(v: i32, dpi: u32) -> i32 {
    v * i32::try_from(dpi).unwrap_or(96) / 96
}

/// Positions controls and recreates fonts for `dpi`.
fn layout(ui: &mut Ui, dpi: u32) {
    let s = |v| scale(v, dpi);
    let place = |h: HWND, x, y, w, ht| {
        // SAFETY: valid child window.
        unsafe {
            SetWindowPos(
                h,
                null_mut(),
                s(x),
                s(y),
                s(w),
                s(ht),
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
    };
    place(ui.heading, 24, 18, 512, 34);
    place(ui.warning, 24, 58, 512, 60);
    if ui.help {
        place(ui.status, 24, 126, 512, 164);
        place(ui.driver, 24, 302, 250, 46);
        place(ui.checker, 286, 302, 250, 46);
    } else {
        place(ui.status, 24, 126, 512, 220);
    }
    let shown = if ui.help { SW_SHOWNORMAL } else { SW_HIDE };
    for h in [ui.driver, ui.checker] {
        // SAFETY: valid child windows.
        unsafe { ShowWindow(h, shown) };
    }
    ui.dpi = dpi;
    place(ui.read, 24, 364, 180, 42);
    place(ui.reset, 216, 364, 160, 42);
    place(ui.logs, 388, 364, 148, 42);

    for f in ui.fonts.drain(..) {
        // SAFETY: fonts created by `font` and no longer selected.
        unsafe { DeleteObject(f) };
    }
    let big = font(16, true, dpi);
    let body = font(10, false, dpi);
    let small = font(9, false, dpi);
    let set = |h: HWND, f: HFONT| {
        // SAFETY: valid window and font.
        unsafe { SendMessageW(h, WM_SETFONT, f as WPARAM, 1) };
    };
    set(ui.heading, big);
    set(ui.warning, small);
    for h in [ui.status, ui.read, ui.reset, ui.logs, ui.driver, ui.checker] {
        set(h, body);
    }
    ui.fonts = vec![big, body, small];
}

fn child(parent: HWND, class: &str, text: &str, style: u32, id: usize) -> HWND {
    let c = wide(class);
    let t = wide(text);
    // SAFETY: standard child control creation with a valid parent.
    unsafe {
        CreateWindowExW(
            0,
            c.as_ptr(),
            t.as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            10,
            10,
            parent,
            id as _,
            GetModuleHandleW(null()),
            null(),
        )
    }
}

fn post(hwnd: usize) {
    // SAFETY: posting to our own window; ignored if it is gone.
    unsafe { PostMessageW(hwnd as HWND, WM_WORKER, 0, 0) };
}

fn start_detect(ui: &mut Ui) {
    if ui.busy || ui.detecting || ui.read_once {
        return;
    }
    ui.detecting = true;
    let (tx, hwnd, t) = (ui.tx.clone(), ui.hwnd as usize, ui.t);
    std::thread::spawn(move || {
        // Only the USB driver's device id: nothing goes to the printer.
        let mut message = t.start.to_string();
        let bus = ActiveBus {};
        for c in bus.candidates() {
            if let Ok(id) = bus.device_id(&c) {
                let d = ctrl::parse_device_id(&id);
                if is_supported(&d.manufacturer, &d.model) {
                    message = format!(
                        "{}\r\n\r\n{}",
                        t.found.replace("{model}", &d.model),
                        t.press_read
                    );
                    break;
                }
                if d.manufacturer.eq_ignore_ascii_case("EPSON") {
                    message = t.not_supported.replace("{model}", &d.model);
                }
            }
        }
        let _ = tx.send(Msg::Detected(message));
        post(hwnd);
    });
}

/// Downloads, verifies and starts one of Epson's programs on a worker.
fn start_fetch(ui: &mut Ui, package: &'static crate::packages::Package) {
    let is_driver = package.file == crate::packages::DRIVER.file;
    let body = wide(if is_driver {
        ui.t.confirm_driver
    } else {
        ui.t.confirm_checker
    });
    let title = wide(if is_driver {
        ui.t.driver_button
    } else {
        ui.t.checker_button
    });
    // SAFETY: modal message box owned by our window.
    let answer = unsafe {
        MessageBoxW(
            ui.hwnd,
            body.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONINFORMATION,
        )
    };
    if answer != IDYES {
        return;
    }
    ui.busy = true;
    for h in [ui.read, ui.reset, ui.driver, ui.checker] {
        enable(h, false);
    }
    set_text(ui.status, &ui.t.downloading.replace("{pct}", "0"));
    let (tx, hwnd, t) = (ui.tx.clone(), ui.hwnd as usize, ui.t);
    let base = ui.store.dir().to_path_buf();
    let run_stamp = stamp();
    std::thread::spawn(move || {
        let mut log = crate::log::Log::default();
        log.line(format!("== fetch {}", package.file));
        let mut last = u64::MAX;
        let progress_tx = tx.clone();
        let mut progress = |done: u64| {
            let pct = done * 100 / package.size.max(1);
            if pct != last {
                last = pct;
                let _ = progress_tx.send(Msg::Progress(
                    t.downloading.replace("{pct}", &pct.to_string()),
                ));
                post(hwnd);
            }
        };
        let verify_tx = tx.clone();
        let mut verifying = || {
            let _ = verify_tx.send(Msg::Progress(t.verifying.to_string()));
            post(hwnd);
        };
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let result = crate::fetch::fetch_and_launch(
            package,
            &base,
            &run_stamp,
            &mut progress,
            &mut verifying,
            &cancel,
            &mut log,
        );
        log.line(format!("fetch result: {result:?}"));
        let started = result.is_ok().then_some(package.file);
        let message = match result {
            Ok(()) if is_driver => t.driver_started.to_string(),
            Ok(()) => t.checker_started.to_string(),
            Err(crate::fetch::FetchError::Declined) => t.declined.to_string(),
            Err(crate::fetch::FetchError::WrongFile(_)) => t.wrong_file.to_string(),
            Err(e) => format!("{}\r\n\r\n{}: {e:?}", t.fetch_failed, t.details),
        };
        let _ = tx.send(Msg::Fetched(message, log, started));
        post(hwnd);
    });
}

fn start_read(ui: &mut Ui) {
    ui.busy = true;
    ui.read_once = true;
    ui.confirmed = None;
    for h in [ui.read, ui.reset] {
        enable(h, false);
    }
    set_text(ui.status, ui.t.reading);
    let (tx, hwnd, t) = (ui.tx.clone(), ui.hwnd as usize, ui.t);
    std::thread::spawn(move || {
        let report = app::read(&ActiveBus {}, t, Timing::default());
        let _ = tx.send(Msg::Read(report));
        post(hwnd);
    });
}

fn start_reset(ui: &mut Ui) {
    let Some(c) = ui.confirmed.clone() else {
        return;
    };
    let body = wide(ui.t.confirm);
    let title = wide(ui.t.confirm_title);
    // SAFETY: modal message box owned by our window.
    let answer = unsafe {
        MessageBoxW(
            ui.hwnd,
            body.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        )
    };
    if answer != IDYES {
        return;
    }
    ui.busy = true;
    ui.resetting = true;
    ui.confirmed = None;
    for h in [ui.read, ui.reset, ui.logs] {
        enable(h, false);
    }
    set_text(ui.status, ui.t.resetting);
    let (tx, hwnd, t) = (ui.tx.clone(), ui.hwnd as usize, ui.t);
    std::thread::spawn(move || {
        let (message, log, ok) = app::reset(&ActiveBus {}, t, Timing::default(), &c);
        let _ = tx.send(Msg::Reset(message, log, ok));
        post(hwnd);
    });
}

/// Keeps a log entry for the copy button and writes it to the log file.
fn record(ui: &mut Ui, text: &str) {
    let _ = ui.store.append_log(text);
    ui.log_text.push_str(text);
}

/// Puts `text` on the clipboard as Unicode text.
fn to_clipboard(hwnd: HWND, text: &str) -> bool {
    let w = wide(&text.replace("\r\n", "\n").replace('\n', "\r\n"));
    let bytes = w.len() * 2;
    // SAFETY: the clipboard owns the memory once SetClipboardData succeeds;
    // otherwise it is freed here. The copy stays within the allocation.
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return false;
        }
        EmptyClipboard();
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes);
        let mut ok = false;
        if !mem.is_null() {
            let dst = GlobalLock(mem).cast::<u16>();
            if !dst.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), dst, w.len());
                GlobalUnlock(mem);
                ok = !SetClipboardData(CF_UNICODETEXT, mem).is_null();
            }
            if !ok {
                GlobalFree(mem);
            }
        }
        CloseClipboard();
        ok
    }
}

fn copy_log(ui: &Ui) {
    let text = if ui.log_text.is_empty() {
        format!(
            "EpsonReset {}: nothing logged yet\r\n",
            env!("CARGO_PKG_VERSION")
        )
    } else {
        ui.log_text.clone()
    };
    let note = if to_clipboard(ui.hwnd, &text) {
        ui.t.copied
    } else {
        ui.t.copy_failed
    };
    let body = wide(note);
    let title = wide(ui.t.title);
    // SAFETY: modal message box owned by our window.
    unsafe {
        MessageBoxW(
            ui.hwnd,
            body.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        )
    };
}

fn on_worker(ui: &mut Ui) {
    while let Ok(msg) = ui.rx.try_recv() {
        match msg {
            Msg::Detected(message) => {
                ui.detecting = false;
                if !ui.read_once && !ui.busy {
                    set_text(ui.status, &message);
                }
            }
            Msg::Read(report) => {
                ui.busy = false;
                let mut message = report.message;
                record(ui, &report.log.text());
                if let Some(c) = report.confirmed {
                    let backup = format!(
                        "device id: {}\r\n{}",
                        crate::log::redact_text(&c.device_id),
                        c.snapshot.to_text()
                    );
                    match ui.store.write_backup(&stamp(), &backup) {
                        Ok(_) => ui.confirmed = Some(c),
                        Err(e) => {
                            record(ui, &format!("backup failed: {e}\r\n"));
                            message = format!("{message}\r\n\r\n{}", ui.t.backup_failed);
                        }
                    }
                }
                record(ui, &format!("shown: {message}\r\n\r\n"));
                set_text(ui.status, &message);
                enable(ui.read, true);
                enable(ui.reset, ui.confirmed.is_some());
                if report.offer_help != ui.help {
                    ui.help = report.offer_help;
                    let dpi = ui.dpi;
                    layout(ui, dpi);
                }
            }
            Msg::Progress(text) => set_text(ui.status, &text),
            Msg::Fetched(message, log, started) => {
                ui.busy = false;
                record(ui, &log.text());
                record(ui, &format!("shown: {message}\r\n\r\n"));
                set_text(ui.status, &message);
                enable(ui.read, true);
                // A started installer may still need its files: no second
                // download of the same program in this run.
                if let Some(which) = started {
                    ui.launched.push(which);
                }
                enable(
                    ui.driver,
                    !ui.launched.contains(&crate::packages::DRIVER.file),
                );
                enable(
                    ui.checker,
                    !ui.launched.contains(&crate::packages::CHECKER.file),
                );
            }
            Msg::Reset(message, log, ok) => {
                ui.busy = false;
                ui.resetting = false;
                record(ui, &log.text());
                record(ui, &format!("shown: {message}\r\n\r\n"));
                set_text(ui.status, &message);
                if ok {
                    set_text(ui.read, ui.t.check);
                }
                enable(ui.read, true);
                enable(ui.logs, true);
                enable(ui.reset, false);
            }
        }
    }
}

/// Runs `f` on the UI state; false when a dialog's own message loop is
/// holding it.
fn with_ui(f: impl FnOnce(&mut Ui)) -> bool {
    UI.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut()
            && let Some(ui) = guard.as_mut()
        {
            f(ui);
            true
        } else {
            false
        }
    })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = wp & 0xFFFF;
            with_ui(|ui| match id {
                ID_READ if !ui.busy => start_read(ui),
                ID_RESET if !ui.busy => start_reset(ui),
                ID_COPY => copy_log(ui),
                ID_DRIVER if !ui.busy => start_fetch(ui, &crate::packages::DRIVER),
                ID_CHECKER if !ui.busy => start_fetch(ui, &crate::packages::CHECKER),
                _ => {}
            });
            0
        }
        WM_WORKER => {
            if !with_ui(on_worker) {
                // A dialog is open: try again shortly so no result is lost.
                // SAFETY: timer on our own window.
                unsafe { SetTimer(hwnd, TIMER_WORKER, 200, None) };
            }
            0
        }
        WM_TIMER if wp == TIMER_WORKER => {
            if with_ui(on_worker) {
                // SAFETY: our own timer.
                unsafe { KillTimer(hwnd, TIMER_WORKER) };
            }
            0
        }
        WM_TIMER => {
            with_ui(start_detect);
            0
        }
        WM_CTLCOLORSTATIC => {
            let hdc = wp as HDC;
            // Read without borrowing `UI`, which is held while a message box
            // runs its own loop.
            let is_warning = lp != 0 && lp == WARNING.load(Ordering::Relaxed);
            // SAFETY: the DC belongs to the control being painted.
            unsafe {
                SetBkColor(hdc, GetSysColor(COLOR_WINDOW));
                if is_warning {
                    SetTextColor(hdc, WARNING_RGB);
                }
                GetSysColorBrush(COLOR_WINDOW) as LRESULT
            }
        }
        WM_DPICHANGED => {
            let dpi = u32::try_from(wp & 0xFFFF).unwrap_or(96);
            // SAFETY: lParam points at the suggested window rectangle.
            let r = unsafe { *(lp as *const RECT) };
            unsafe {
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            with_ui(|ui| layout(ui, dpi));
            0
        }
        WM_CLOSE => {
            let mut writing = false;
            with_ui(|ui| writing = ui.resetting);
            if !writing {
                // SAFETY: our own top-level window.
                unsafe { DestroyWindow(hwnd) };
            }
            0
        }
        WM_DESTROY => {
            with_ui(|ui| {
                for f in ui.fonts.drain(..) {
                    // SAFETY: fonts created by `font`.
                    unsafe { DeleteObject(f) };
                }
            });
            // SAFETY: ends the message loop.
            unsafe { PostQuitMessage(0) };
            0
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn build_ui(hwnd: HWND, t: &'static Text) -> Ui {
    let edit_style = (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32 | WS_BORDER | WS_VSCROLL;
    let button = BS_PUSHBUTTON as u32 | WS_TABSTOP;
    let (tx, rx) = channel();
    Ui {
        hwnd,
        heading: child(hwnd, "STATIC", t.heading, 0, 0),
        warning: child(hwnd, "STATIC", t.warning, 0, 0),
        status: child(hwnd, "EDIT", t.start, edit_style, 0),
        read: child(hwnd, "BUTTON", t.read, button, ID_READ),
        reset: child(hwnd, "BUTTON", t.reset, button, ID_RESET),
        logs: child(hwnd, "BUTTON", t.logs, button, ID_COPY),
        driver: child(hwnd, "BUTTON", t.driver_button, button, ID_DRIVER),
        checker: child(hwnd, "BUTTON", t.checker_button, button, ID_CHECKER),
        help: false,
        dpi: 96,
        launched: Vec::new(),
        fonts: Vec::new(),
        t,
        store: Store::new(&stamp()),
        tx,
        rx,
        busy: false,
        resetting: false,
        detecting: false,
        read_once: false,
        confirmed: None,
        log_text: String::new(),
    }
}

/// Sizes the window for its client area at `dpi` and centres it.
fn place_window(hwnd: HWND, dpi: u32) {
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: scale(CLIENT_W, dpi),
        bottom: scale(CLIENT_H, dpi),
    };
    // SAFETY: valid window and rectangle.
    unsafe {
        AdjustWindowRectExForDpi(&raw mut frame, STYLE, 0, 0, dpi);
        let (width, height) = (frame.right - frame.left, frame.bottom - frame.top);
        let left = (GetSystemMetrics(SM_CXSCREEN) - width) / 2;
        let top = (GetSystemMetrics(SM_CYSCREEN) - height) / 2;
        SetWindowPos(
            hwnd,
            null_mut(),
            left.max(0),
            top.max(0),
            width,
            height,
            SWP_NOZORDER,
        );
    }
}

pub fn run() {
    // SAFETY: process-wide setting made before any window exists.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    // SAFETY: plain query.
    let lang = Lang::from_langid(unsafe { GetUserDefaultUILanguage() });
    let t = text(lang);
    let class = wide("EpsonResetWindow");
    // A second copy would share the USB interface and could restart the D4
    // session in the middle of a reset. Global covers other Windows sessions
    // too; a mutex another user created is access denied, also "already".
    // The mutex lives until the process ends.
    let mutex_name = wide(r"Global\EpsonReset.Instance");
    // SAFETY: named mutex with a NUL-terminated name; GetLastError is read
    // right after the call that set it.
    let already = unsafe {
        CreateMutexW(null(), 0, mutex_name.as_ptr()).is_null()
            || GetLastError() == ERROR_ALREADY_EXISTS
    };
    if already {
        // SAFETY: lookup by our own class name; a null result is skipped.
        unsafe {
            let other = FindWindowW(class.as_ptr(), null());
            if !other.is_null() {
                ShowWindow(other, SW_RESTORE);
                SetForegroundWindow(other);
            }
        }
        return;
    }
    let title = wide(t.title);
    // SAFETY: class registration and window creation with valid arguments;
    // the message loop runs on this thread until the window is destroyed.
    unsafe {
        let instance = GetModuleHandleW(null());
        let wc = WNDCLASSEXW {
            cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).unwrap_or(0),
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&raw const wc);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            STYLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CLIENT_W,
            CLIENT_H,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        let mut ui = build_ui(hwnd, t);
        enable(ui.reset, false);
        let dpi = GetDpiForWindow(hwnd).max(96);
        layout(&mut ui, dpi);
        place_window(hwnd, dpi);

        WARNING.store(ui.warning as isize, Ordering::Relaxed);
        UI.with(|cell| *cell.borrow_mut() = Some(ui));
        with_ui(start_detect);
        SetTimer(hwnd, TIMER_DETECT, DETECT_MS, None);
        ShowWindow(hwnd, SW_SHOWNORMAL);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&raw mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
        KillTimer(hwnd, TIMER_DETECT);
    }
}
