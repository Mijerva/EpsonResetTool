# Security

Report a vulnerability privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability**. Please do not open a public issue for it.

Include the version, what you did, and what happened. A log from **Copy log** helps; serial numbers are already blanked in it.

Only the latest release gets fixes.

## Scope

- Anything that makes the program write to the printer other than the documented reset values.
- Anything that makes the program start a file that is not the recorded Epson package.
- Data leaving the PC. The program's only network access is the optional download from `download.epson-europe.com`. While checking the file's signature, Windows itself may contact certificate and revocation servers.
