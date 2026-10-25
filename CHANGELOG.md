# Changelog

All notable changes to Nab. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [SemVer](https://semver.org/).

## [0.2.0] – 2026-10-25 (beta)

The first release for the Microsoft Store. Tables from scanned sources come back with their numbers intact, the download is easier to verify, and there is room for an optional, separate add-on.

### Fixed
- Tables: numbers misread at low resolution are repaired using the rest of the column as evidence, never a cell alone: `1,533` read as `1.533`, and `12.4` read as `12 4`.
- Tables: Excel no longer turns a number such as `19.8` into a date (19-Aug). Numeric cells carry their value, so Excel takes the number as given.

### Added
- Each release publishes `nab.zip` beside `nab.exe` (both hashed) for browsers and filters that block a new exe, and the README's *Is this safe?* section shows how to check the hash and the build-provenance attestation.
- `src/ext.rs`: the single, documented place where an optional closed module (**Nab Pro**, sold separately on the Microsoft Store) can plug in. With no module installed, which is every GitHub download, every hook does nothing.
- `nab` is now a library plus a thin `main`, so the Store build can link the free tool and the closed module together. The GitHub build is unchanged: it is this crate with no module.
- `msix/build.ps1 -Version` (the Store package can carry its own version).
- CI checks that the public build never enables the Windows Store API.

### Free forever
- Everything that was free is still free: click to copy, drag a box, tables, paragraph reflow, Alt+Space, configurable shortcuts. Nothing moves behind a payment.

## [0.1.1] – 2026-10-14 (beta)

Hardening from a security review (no vulnerabilities found; all defence-in-depth):

- The exe resolves its DLL imports from System32 only, so a DLL planted next to `nab.exe` can't be loaded in its place.
- A forged hotkey or menu message from another process no longer makes Nab grab or quit; the tray menu returns its choice directly.
- At most two OCR reads run at once (a flood of forged drag events used to start one thread each).
- The single-instance check only hands over to a window that belongs to a real `nab.exe`.
- The build fails rather than ship an exe without its manifest.
- Dev-signing certificate: non-exportable, 3 months, and `msix/uninstall-dev.ps1` removes the trust again.
- GitHub Actions pinned to commit SHAs; PRIVACY.md says what the notification preview implies.

## [0.1.0] – 2026-10-14 (beta)

First public release, as a beta: it works and is used daily, but expect rough edges. Please report them.

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
