# Changelog

All notable changes to Nab. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [SemVer](https://semver.org/).

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
