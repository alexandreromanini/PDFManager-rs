# Contributing to PDFManager-rs

Thank you for your interest in PDFManager! Contributions are welcome, in English or French.

## Ways to help

- **Report a bug**: open an [issue](https://github.com/alexandreromanini/PDFManager-rs/issues/new/choose)
  with the steps to reproduce it. A sample PDF helps a lot (never share a confidential document).
- **Suggest a feature**: describe the problem you want to solve before the solution.
- **Improve the code or the documentation**: pull requests are welcome.

PDFManager has two editions with the same features: [PDFManager](https://github.com/alexandreromanini/PDFManager)
(Python) and [PDFManager-rs](https://github.com/alexandreromanini/PDFManager-rs) (Rust). A bug
found in one of them often exists in the other: please mention it if you can check.

## Development

The setup, the project structure and technical notes are in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).
Before opening a pull request, make sure everything passes:

```bash
cargo test --release
cargo build --release --features capture
python tests/parcours.py      # end-to-end scenarios (pip install pymupdf)
```

## Guidelines

- Keep pull requests focused: one fix or one feature at a time.
- Add or update a test for any change in behaviour.
- The interface is in French. User-facing texts use the short dash « - », never the long dash (U+2014).
- Write commit messages as a short summary, optionally prefixed (`fix:`, `feat:`, `docs:`,
  `test:`, `build:`), in French or English.
- Every change must keep working on Windows: the CI builds and tests the Windows installer.

## License

By contributing, you agree that your contributions are licensed under the
[GNU AGPL v3.0 or later](LICENSE), like the rest of the project.
