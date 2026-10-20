# EpsonReset

A small Windows program that reads and resets the waste ink counters of an Epson ET-2870 series printer (ET-2870, ET-2871, ET-2873, ET-2875) over USB, to clear error E-11.

Resetting the counter does not empty the ink pads. E-11 means the pads inside the printer are full. After a reset the printer prints again, but ink can eventually leak out. Epson recommends replacing the pads.

## Use

1. Download `EpsonReset.zip` from the latest release and unpack it.
2. Connect the printer with a USB cable and turn it on.
3. Start `EpsonReset.exe`. Windows may warn about an unknown publisher; choose "More info", then "Run anyway".
4. Press **Read printer status**.
5. Press **Reset E-11 error** and confirm.
6. Turn the printer off, wait 10 seconds, turn it on, then press **Check again**.

The window uses Swedish when Windows does, English otherwise.

### Svenska

1. Ladda ner `EpsonReset.zip` och packa upp den.
2. Anslut skrivaren med USB-kabeln och slå på den.
3. Starta `EpsonReset.exe`. Om Windows varnar, klicka på "Mer information" och sedan "Kör ändå".
4. Tryck på **Läs skrivarens status**.
5. Tryck på **Återställ E-11** och bekräfta.
6. Stäng av skrivaren, vänta 10 sekunder, slå på den och tryck på **Kontrollera igen**.

## What it does

The program talks to the printer through `usbprint.sys`, the USB printer driver Windows already uses. It installs nothing itself and changes no drivers.

1. It lists the USB printer interfaces with Epson's vendor id and asks the driver for each printer's IEEE 1284 device id. Only a printer that names itself as the ET-2870 series goes further. Nothing is sent to any other printer.
2. It opens an IEEE 1284.4 (D4) session to the printer's `EPSON-CTRL` service and asks for the device id again. Both answers must name the ET-2870 series.
3. It reads the printer's status report, then the 15 EEPROM bytes it may change, twice. Both EEPROM readings must agree. The values are saved to a backup file before a reset can be offered. If the status names an error other than the full ink pads, no reset is offered. A status that cannot be read does not block the reset, because every write is read back.
4. After you confirm, it reads the status again and stops if a new error has appeared. Then it writes the reset values to the 14 counter bytes, one at a time, and reads each one back. If the first write does not read back, it stops at once.
5. If bit 0 of the state byte at 0x100 is still set, it clears that bit only, leaving the other bits as they are.
6. It reads all bytes again. It only reports success when every byte has its reset value.

A write that gets no clear answer is followed by a fresh session and a read, never by a second write. The program cannot be closed while a reset is writing.

### Values

| | Value |
| --- | --- |
| Read key | `0x364A` |
| Write key | `Nbsjcbzb` ("Maribaya" shifted by one) |
| Counter bytes reset to 0x00 | 0x1C, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0xFC, 0xFD, 0xFE |
| Counter bytes reset to 0x5E | 0x36, 0x37, 0xFF |
| State byte | 0x100, bit 0 cleared |

The keys and the counter bytes are the same in reinkpy, Epson-Waste-Reset and epson_print_conf. The state byte step comes from Epson-Waste-Reset (ET-2870) and epson_print_conf, where it was verified on an ET-2814. None of these projects reports a test on an ET-2871, so the first run on a real printer is the confirmation.

## When the printer does not answer

If Windows drives the printer with its own generic driver (Microsoft IPP Class Driver), the maintenance channel may be out of reach and the printer does not answer. The window then offers two buttons:

- **Install Epson's driver** downloads Epson's ET-2870 driver package from `download.epson-europe.com` and starts Epson's installer.
- **Run Epson's connection check** does the same with Epson Connection Checker.

These are Epson's own programs. They are not part of this project and are never bundled; they are fetched from Epson only when you press the button. The download uses HTTPS to that one host with redirects turned off and the size fixed. A file is started only when its SHA-256 matches the one recorded in `src/packages.rs` and it carries a valid Authenticode signature from SEIKO EPSON CORPORATION. If Epson replaces a file, the program refuses to start the new one. Windows' own prompts (running a downloaded file, administrator rights) still appear.

## Files

Logs and backups go to `%LOCALAPPDATA%\EpsonReset`. **Copy log** puts this run's log on the clipboard, ready to paste into a chat. Logs contain every byte sent and received, with serial numbers blanked.

## Testing on the printer

There is no simulated printer data in the program; the tests use a simulated printer only to exercise the code. To test on a real ET-2871:

1. Close other printer software, such as Epson status monitors, and make sure no print job is waiting.
2. Press **Read printer status**. Expected: "Printer found: ET-2870 Series" and a line about the ink pads.
3. Press **Copy log** and keep the text. It holds the exact replies, which shows what happened if a step fails.
4. Press **Reset E-11 error** and confirm. Expected: "Done".
5. Power cycle the printer and press **Check again**. Expected: "All good".

If the reset is refused or stops partway, the message says so. Use the WIC Reset Utility with a purchased key in that case, and keep the log.

## Build

```sh
cargo build --release
```

Needs Rust with the MSVC toolchain. The release build is a single `EpsonReset.exe` with the C runtime linked in.

## Credits

The protocol and printer values come from these projects:

- [reinkpy](https://codeberg.org/atufi/reinkpy) (AGPL-3.0-or-later): D4 and EPSON-CTRL commands, the ET-2870 family values.
- [Epson-Waste-Reset](https://github.com/RxNaison/Epson-Waste-Reset) (Apache-2.0): Windows USB access, D4 session handling, reply formats, the state byte step.
- [epson_print_conf](https://github.com/Ircama/epson_print_conf) (EUPL-1.2): keys and reset values for the ET-2800 family, the state byte step.

See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). To report a security problem, see [SECURITY.md](SECURITY.md).

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE).

This project is not affiliated with Epson. Use it at your own risk.
