# Contributing

Bug reports and pull requests are welcome.

## Reports

Say which printer model you have, what you pressed, and what the window showed. Paste the text from **Copy log**.

## Code

- Run `cargo fmt --all`, `cargo clippy --all-targets --release -- -D warnings` and `cargo test --release` before opening a pull request.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/).
- Printer commands and values must come from a verified source, such as a working implementation or a capture from a real printer. Name the source in the pull request. Guessed commands are not accepted, because a wrong write can damage a printer.
- Support for another model needs its reset values from such a source and a test log from that model.
- No code that bypasses licence checks of other tools, and no changes to printer drivers.

By contributing you agree that your contribution is licensed under AGPL-3.0-or-later.
