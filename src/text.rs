//! Everything the window says, in Swedish and English. The language follows
//! the Windows display language.
#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Sv,
    En,
}

impl Lang {
    /// From a Windows LANGID: the low ten bits are the primary language.
    #[must_use]
    pub fn from_langid(id: u16) -> Self {
        if id & 0x3FF == 0x1D {
            Self::Sv
        } else {
            Self::En
        }
    }
}

pub struct Text {
    pub title: &'static str,
    pub heading: &'static str,
    pub warning: &'static str,
    pub start: &'static str,
    pub read: &'static str,
    pub check: &'static str,
    pub reset: &'static str,
    pub logs: &'static str,
    pub reading: &'static str,
    pub resetting: &'static str,
    pub no_printer: &'static str,
    pub confirm_title: &'static str,
    pub confirm: &'static str,
    pub done: &'static str,
    pub nothing_to_change: &'static str,
    pub changed_since_read: &'static str,
    pub all_good: &'static str,
    pub still_error: &'static str,
    pub pads_full: &'static str,
    pub pads_ok: &'static str,
    pub status_unknown: &'static str,
    pub press_reset: &'static str,
    pub press_read: &'static str,
    pub other_error: &'static str,
    pub found: &'static str,
    pub not_supported: &'static str,
    pub no_answer: &'static str,
    pub read_failed: &'static str,
    pub first_write_failed: &'static str,
    pub partial: &'static str,
    pub details: &'static str,
    pub backup_failed: &'static str,
    pub ready_print: &'static str,
    pub jobs_waiting: &'static str,
    pub copied: &'static str,
    pub press_driver: &'static str,
    pub driver_button: &'static str,
    pub checker_button: &'static str,
    pub confirm_driver: &'static str,
    pub confirm_checker: &'static str,
    pub downloading: &'static str,
    pub verifying: &'static str,
    pub driver_started: &'static str,
    pub checker_started: &'static str,
    pub fetch_failed: &'static str,
    pub wrong_file: &'static str,
    pub declined: &'static str,
    pub copy_failed: &'static str,
}

const SV: Text = Text {
    title: "Epson E-11 återställning",
    heading: "Återställ E-11 på Epson ET-2871",
    warning: "Obs: detta nollställer bara räknaren. Bläckdynorna inuti skrivaren är fortfarande fulla, och bläck kan läcka ut senare. Epson rekommenderar att dynorna byts.",
    start: "Anslut skrivaren med USB-kabeln och slå på den. Tryck sedan på Läs skrivarens status.",
    read: "Läs skrivarens status",
    check: "Kontrollera igen",
    reset: "Återställ E-11",
    logs: "Kopiera logg",
    reading: "Läser skrivaren... Vänta.",
    resetting: "Återställer... Dra inte ur USB-kabeln och stäng inte av skrivaren.",
    no_printer: "Ingen Epson-skrivare hittades. Anslut den med USB-kabeln, slå på den, vänta 30 sekunder och tryck på Läs skrivarens status igen.",
    confirm_title: "Återställ E-11?",
    confirm: "Räknaren för spillbläck nollställs så att skrivaren kan skriva ut igen.\n\nBläckdynorna inuti är fortfarande fulla och bläck kan läcka ut senare. Epson rekommenderar att dynorna byts.\n\nDra inte ur USB-kabeln under återställningen. Fortsätta?",
    done: "Klart. Räknarna är nollställda och kontrollerade.\n\nStäng nu av skrivaren, vänta 10 sekunder, slå på den igen och tryck sedan på Kontrollera igen.",
    nothing_to_change: "Räknarna var redan nollställda. Ingenting ändrades. Om skrivaren fortfarande visar E-11, stäng av och slå på den.",
    changed_since_read: "Skrivarens värden har ändrats sedan de lästes. Ingenting skrevs. Tryck på Läs skrivarens status igen.",
    all_good: "Allt klart: räknarna är nollställda och skrivaren rapporterar inget fel på bläckdynorna. Skrivaren kan användas igen.",
    still_error: "Räknarna är nollställda, men skrivaren rapporterar fortfarande felet. Stäng av och slå på skrivaren en gång till och tryck på Kontrollera igen. Om felet finns kvar, använd WIC Reset Utility med den köpta nyckeln.",
    pads_full: "Skrivaren rapporterar att bläckdynorna är fulla (E-11).",
    pads_ok: "Skrivaren rapporterar inget fel på bläckdynorna.",
    status_unknown: "Skrivarens felstatus kunde inte läsas helt.",
    press_reset: "Tryck på Återställ E-11 för att nollställa räknaren.",
    press_read: "Tryck på Läs skrivarens status.",
    other_error: "Skrivaren rapporterar ett annat fel (kod {code}). Åtgärda det först (papper, lock, bläck) och läs sedan igen. Ingenting ändrades.",
    found: "Skrivare hittad: {model}.",
    not_supported: "Skrivaren rapporterar sig som \"{model}\". Det här programmet stöder bara Epson ET-2870-serien (ET-2871). Ingenting ändrades.",
    no_answer: "Skrivaren svarade inte. Stäng av den och slå på den igen, vänta tills den är klar och försök igen. Ingenting ändrades.",
    read_failed: "Räknarna kunde inte läsas säkert. Ingenting ändrades. Använd WIC Reset Utility med den köpta nyckeln.",
    first_write_failed: "Den första ändringen kunde inte bekräftas, så inga fler ändringar gjordes. Använd WIC Reset Utility med den köpta nyckeln.",
    partial: "Återställningen stannade halvvägs. Tryck på Läs skrivarens status och försök en gång till. Om det misslyckas igen, använd WIC Reset Utility med den köpta nyckeln.",
    details: "Detaljer",
    backup_failed: "Säkerhetskopian av skrivarens värden kunde inte sparas, så återställningen är avstängd.",
    ready_print: "Windows använder sin egen standarddrivrutin för skrivaren (Microsoft IPP Class Driver). Med den svarar skrivaren ibland inte.",
    press_driver: "Tryck på Installera Epsons drivrutin nedan. Programmet hämtar den direkt från Epson.",
    driver_button: "Installera Epsons drivrutin",
    checker_button: "Kör Epsons anslutningskontroll",
    confirm_driver: "Programmet laddar ner Epsons drivrutin för ET-2870 (cirka 70 MB) direkt från Epson, kontrollerar att filen är äkta och startar Epsons installationsprogram.\n\nOm Windows frågar om filen får köras eller göra ändringar, tryck Kör eller Ja. Följ sedan stegen i Epsons installationsprogram.\n\nFortsätta?",
    confirm_checker: "Programmet laddar ner Epsons anslutningskontroll (cirka 8 MB) direkt från Epson, kontrollerar att filen är äkta och startar den.\n\nOm Windows frågar om filen får köras eller göra ändringar, tryck Kör eller Ja.\n\nFortsätta?",
    downloading: "Laddar ner från Epson... {pct} %\n\nStäng inte programmet.",
    verifying: "Kontrollerar att filen kommer från Epson...",
    driver_started: "Epsons installationsprogram har startats. Följ stegen i det.\n\nNär det är klart: starta om datorn, öppna EpsonReset igen och tryck på Läs skrivarens status.",
    checker_started: "Epsons anslutningskontroll har startats. Följ stegen i den. Tryck sedan på Läs skrivarens status igen.",
    fetch_failed: "Det gick inte att ladda ner från Epson. Kontrollera att datorn har internet och försök igen.",
    wrong_file: "Filen från Epson var inte den väntade, så den startades inte. Epson kan ha bytt ut den. Tryck på Kopiera logg och skicka loggen.",
    declined: "Programmet startades inte eftersom Windows-frågan besvarades med Nej. Tryck på knappen igen och svara Ja.",
    copied: "Loggen är kopierad. Öppna chatten, klicka i textrutan och tryck Ctrl+V för att klistra in den.",
    copy_failed: "Loggen kunde inte kopieras. Försök igen.",
    jobs_waiting: "Det finns utskrifter som väntar i skrivarkön. Avbryt dem under Inställningar > Bluetooth och enheter > Skrivare och skannrar och försök igen.",
};

const EN: Text = Text {
    title: "Epson E-11 reset",
    heading: "Reset E-11 on an Epson ET-2871",
    warning: "Note: this only resets the counter. The ink pads inside the printer are still full, and ink can leak out later. Epson recommends replacing the pads.",
    start: "Connect the printer with the USB cable and turn it on. Then press Read printer status.",
    read: "Read printer status",
    check: "Check again",
    reset: "Reset E-11 error",
    logs: "Copy log",
    reading: "Reading the printer... Please wait.",
    resetting: "Resetting... Do not unplug the USB cable or turn off the printer.",
    no_printer: "No Epson printer found. Connect it with the USB cable, turn it on, wait 30 seconds, then press Read printer status again.",
    confirm_title: "Reset E-11?",
    confirm: "This resets the waste ink counter so the printer can print again.\n\nThe ink pads inside are still full and ink can leak out later. Epson recommends replacing the pads.\n\nDo not unplug the USB cable during the reset. Continue?",
    done: "Done. The counters were reset and checked.\n\nNow turn the printer off, wait 10 seconds, turn it on again, then press Check again.",
    nothing_to_change: "The counters were already reset. Nothing was changed. If the printer still shows E-11, turn it off and on.",
    changed_since_read: "The printer's values changed since they were read. Nothing was written. Press Read printer status again.",
    all_good: "All good: the counters are reset and the printer reports no ink pad error. The printer can be used again.",
    still_error: "The counters are reset, but the printer still reports the error. Turn it off and on once more, then press Check again. If the error stays, use the WIC Reset Utility with the purchased key.",
    pads_full: "The printer reports that the ink pads are full (E-11).",
    pads_ok: "The printer reports no ink pad error.",
    status_unknown: "The printer's error status could not be read completely.",
    press_reset: "Press Reset E-11 error to reset the counter.",
    press_read: "Press Read printer status.",
    other_error: "The printer reports a different error (code {code}). Fix that first (paper, cover, ink), then read again. Nothing was changed.",
    found: "Printer found: {model}.",
    not_supported: "The printer reports itself as \"{model}\". This tool only supports the Epson ET-2870 series (ET-2871). Nothing was changed.",
    no_answer: "The printer did not answer. Turn it off and on, wait until it is ready, and try again. Nothing was changed.",
    read_failed: "The counters could not be read reliably. Nothing was changed. Use the WIC Reset Utility with the purchased key.",
    first_write_failed: "The first change could not be confirmed, so no further changes were attempted. Use the WIC Reset Utility with the purchased key.",
    partial: "The reset stopped partway. Press Read printer status and try once more. If it fails again, use the WIC Reset Utility with the purchased key.",
    details: "Details",
    backup_failed: "The backup of the printer's values could not be saved, so the reset is disabled.",
    ready_print: "Windows drives this printer with its own standard driver (Microsoft IPP Class Driver). With it the printer sometimes does not answer.",
    press_driver: "Press Install Epson's driver below. The program gets it straight from Epson.",
    driver_button: "Install Epson's driver",
    checker_button: "Run Epson's connection check",
    confirm_driver: "The program downloads Epson's ET-2870 driver (about 70 MB) straight from Epson, checks that the file is genuine and starts Epson's installer.\n\nIf Windows asks whether the file may run or make changes, press Run or Yes. Then follow the steps in Epson's installer.\n\nContinue?",
    confirm_checker: "The program downloads Epson's connection check (about 8 MB) straight from Epson, checks that the file is genuine and starts it.\n\nIf Windows asks whether the file may run or make changes, press Run or Yes.\n\nContinue?",
    downloading: "Downloading from Epson... {pct} %\n\nDo not close the program.",
    verifying: "Checking that the file comes from Epson...",
    driver_started: "Epson's installer has started. Follow its steps.\n\nWhen it is done: restart the PC, open EpsonReset again and press Read printer status.",
    checker_started: "Epson's connection check has started. Follow its steps, then press Read printer status again.",
    fetch_failed: "The download from Epson failed. Check that the PC is online and try again.",
    wrong_file: "The file from Epson was not the expected one, so it was not started. Epson may have replaced it. Press Copy log and send the log.",
    declined: "The program was not started because the Windows question was answered No. Press the button again and answer Yes.",
    copied: "The log is copied. Open the chat, click in the text box and press Ctrl+V to paste it.",
    copy_failed: "The log could not be copied. Try again.",
    jobs_waiting: "Print jobs are waiting in the printer queue. Cancel them in Settings > Bluetooth & devices > Printers & scanners and try again.",
};

#[must_use]
pub fn text(lang: Lang) -> &'static Text {
    match lang {
        Lang::Sv => &SV,
        Lang::En => &EN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swedish_langid() {
        assert_eq!(Lang::from_langid(0x041D), Lang::Sv);
        assert_eq!(Lang::from_langid(0x081D), Lang::Sv);
        assert_eq!(Lang::from_langid(0x0409), Lang::En);
    }

    #[test]
    fn placeholders_present_in_both() {
        for t in [text(Lang::Sv), text(Lang::En)] {
            assert!(t.found.contains("{model}"));
            assert!(t.not_supported.contains("{model}"));
            assert!(t.other_error.contains("{code}"));
        }
    }
}
