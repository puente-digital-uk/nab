# Changelog

All notable changes to Nab. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [SemVer](https://semver.org/).

## [0.4.0] – 2026-10-14

First public release.

### Added
- Ctrl+Shift+click copies the block of text under the pointer; Ctrl+Shift+drag copies a box. Both shortcuts can be changed.
- Alt+Space freezes the screen with hover highlighting for keyboard use.
- Dragged tables paste into Excel, Word and Outlook as real rows and columns (TSV + HTML Format on the clipboard).
- Wrapped lines are joined back into paragraphs; lists, addresses and code keep their line breaks.
- A time-saved counter: each copy is counted against the screenshot → OCR/AI tool → copy-back route it replaces.
- Start with Windows, including in the Microsoft Store (MSIX) build via a startup task.
- MSIX packaging (`msix/`) for the Microsoft Store; GitHub Releases built by CI from version tags, with SHA-256 checksums.

### Security
- Results from worker threads are only accepted if the app itself sent them, so a stray message from another process can't crash it.
- Shortcuts that would hijack Alt+F4, Alt+Tab, Ctrl+Tab, Win+L or plain Ctrl+A/C/V/X/Z are refused.
