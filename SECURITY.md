# Security

Nab reads your screen and writes to your clipboard, so we take reports seriously.

**Please don't open a public issue for a security problem.** Email **hello@puentedigital.co.uk** with the details and, if you can, steps to reproduce. You'll get a reply within 5 working days, and a fix in a release as soon as it's ready. We'll credit you in the release notes unless you'd rather we didn't.

## What's in scope

- Anything that lets another process on the same PC crash Nab, read what it captured, or make it copy something the user didn't choose.
- Anything that makes Nab send data off the machine (it never should).
- Problems in the release pipeline: a published binary that doesn't match the tagged source.

## Verifying a download

Every GitHub Release lists the SHA-256 of `nab.exe`; check it with `Get-FileHash nab.exe` in PowerShell. Releases are built by GitHub Actions from the tag, and the run is linked from the release notes. The Microsoft Store version is signed by Microsoft.
