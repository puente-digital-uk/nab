# Nab privacy policy

**Short version: Nab doesn't collect, store or send anything about you.**

## What Nab does with your data

- **Screen contents.** When you use a shortcut, Nab takes a picture of your screen, finds the text in it with the OCR engine built into Windows (`Windows.Media.Ocr`), and puts the text you chose on your clipboard. The picture and the text exist only in memory on your PC and are discarded as soon as the copy is done. Nothing is written to disk and nothing leaves your computer.
- **Clipboard.** Nab writes to the clipboard when you copy something. It never reads the clipboard. Whatever you copy is then subject to your own Windows settings: Clipboard History and cloud clipboard sync apply to it like to any other copy.
- **Notifications.** The "Copied …" notification shows the first 180 characters of what you copied. Windows keeps notifications in Notification Centre until you dismiss them and may show them on the lock screen, so treat that preview like the clipboard itself: don't nab a password or a one-time code on a PC you share, or clear the notification afterwards.
- **Settings.** Your shortcuts, the Start with Windows choice and the time-saved counter (a number of seconds, nabs and steps) are stored in the Windows registry under `HKEY_CURRENT_USER\Software\PuenteDigital\Nab`. Deleting that key resets Nab. The Microsoft Store version keeps them in the app's own storage, which Windows removes when you uninstall.
- **Network.** Nab makes no network connections. It has no account, no update check, no analytics, no crash reporting and no telemetry. The only time anything opens a browser is when you click a link in Nab's own window (this project's page, Puente Digital's site or Buy me a coffee), and that just opens the link in your default browser.

## What we collect

Nothing. We have no servers that Nab talks to, so we can't see who uses it or what they copy.

## Permissions

Nab is an ordinary desktop app. It registers a global keyboard shortcut and a low-level mouse hook so your shortcuts work in any app, and it reads the screen to find text. It doesn't need or ask for anything else.

## Third parties

The Microsoft Store and GitHub may collect their own download statistics under their own policies. Nab itself doesn't take part in that.

## Changes

If any of this ever changes, this file and the release notes will say so before the change ships.

## Contact

Pablo Puente, Puente Digital — hello@puentedigital.co.uk — https://www.puentedigital.co.uk/
