# Microsoft Store listing — Nab

Paste these into Partner Center → Store listing (English (United Kingdom); add English (United States) with the same text).

## Product name

Nab

## Short title (if asked)

Copy text from anything on screen

## Description

Text stuck in a screenshot, a scanned PDF, a video, an error box or a remote desktop? Nab gets it onto your clipboard in one click.

Hold Ctrl+Shift and click on any text on your screen. Nab works out where that paragraph, heading or label starts and ends, reads it with the OCR engine built into Windows, and copies it. Hold Ctrl+Shift and drag a box instead to copy exactly what you choose.

• Click, don't fiddle: it finds the block of text under your pointer.
• Tables paste into Excel as real cells: drag over a table in a PDF, screenshot or shared screen, then paste into Excel, Word or an email. Rows and columns are kept.
• Clean text: lines that only broke because the text wrapped are joined back into paragraphs; lists and addresses keep their line breaks.
• Works on anything you can see: images, videos, games, remote desktops, scanned PDFs, any app.
• Private and offline: nothing is uploaded, there's no account and no telemetry. It uses the OCR already in Windows 10 and 11, in your Windows display languages.
• Shows what it saves: every copy tells you what the screenshot-and-OCR route would have cost you, and Nab keeps a running total.
• Tiny: one small app that sits quietly in the tray. Start it with Windows and the shortcut is always there.
• Both shortcuts can be changed. Prefer the keyboard? Alt+Space freezes the screen; click a block or drag a box.

Free and open source (MIT). Made by Puente Digital.

## What's new (first submission)

First release.

## Search terms (up to 7)

OCR, copy text, screenshot to text, extract text, image to text, PDF table, clipboard

## Copyright and trademark info

© 2026 Pablo Puente / Puente Digital. MIT licence.

## Additional licence terms

MIT licence: https://github.com/puente-digital-uk/nab/blob/main/LICENSE

## Developer / support

Website: https://www.puentedigital.co.uk/
Support: hello@puentedigital.co.uk
Privacy policy: https://github.com/puente-digital-uk/nab/blob/main/PRIVACY.md

## Notes for certification (Submission options)

Nab is a desktop tray app. It needs runFullTrust for a global hotkey (RegisterHotKey), a low-level mouse hook (so Ctrl+Shift+click works in any app), screen capture (BitBlt) and the clipboard. It makes no network connections.

To test: launch Nab; its window opens and it adds a tray icon. Open any document or web page, hold Ctrl+Shift and left-click on a paragraph: the screen dims for a moment, the paragraph flashes green, a notification says "Copied N characters", and Ctrl+V pastes the text. Hold Ctrl+Shift and drag a box over a table, then paste into Excel: it arrives as rows and columns. Alt+Space opens the same selection screen from the keyboard; Esc closes it. "Start with Windows" in the window toggles the package's startup task.

The screen image is processed in memory with Windows.Media.Ocr and discarded. Settings are stored in the app's registry hive.
