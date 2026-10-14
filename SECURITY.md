# Security

Nab reads your screen and writes to your clipboard, so we take reports seriously.

**Please don't open a public issue for a security problem.** Email **hello@puentedigital.co.uk** with the details and, if you can, steps to reproduce. You'll get a reply within 5 working days, and a fix in a release as soon as it's ready. We'll credit you in the release notes unless you'd rather we didn't.

## What's in scope

- Anything that lets another process on the same PC crash Nab, read what it captured, or make it copy something the user didn't choose.
- Anything that makes Nab send data off the machine (it never should).
- Problems in the release pipeline: a published binary that doesn't match the tagged source.

## What's checked, automatically

- **Every push and every week:** [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) checks the dependency tree against the RustSec advisory database, allowed licences and allowed sources (crates.io only). Nab has one runtime dependency, Microsoft's `windows` crate, pinned with checksums in `Cargo.lock`; CI builds with `--locked`.
- **Every push:** GitHub CodeQL analyses the Rust code; findings appear under the repo's Security tab.
- **Weekly:** [OpenSSF Scorecard](https://securityscorecards.dev/) rates the repository's practices (pinned dependencies, branch protection, token permissions …); the badge is in the README.
- **Dependabot** opens pull requests for crate and GitHub Actions updates.
- **Every release:** the exact `nab.exe` is scanned by [VirusTotal](https://www.virustotal.com/) (≈70 antivirus engines) and the result link is written into the release notes, so you can see the verdict for the file you're downloading. (Maintainers: this needs a free VirusTotal API key in the repository secret `VT_API_KEY`; the step is skipped without it.)
- **Clippy with every warning treated as an error**, on a pinned Rust version, and release builds with `panic = "abort"`, Control Flow Guard and a statically linked C runtime.
- **Reviews:** the code has had an adversarial review against a written threat model (hostile local processes, hostile screen content, multi-monitor and packaged-app environments). The findings and fixes are in the commit history.

## Verifying a download

Every GitHub Release lists the SHA-256 of `nab.exe`; check it with `Get-FileHash nab.exe` in PowerShell. Releases are built by GitHub Actions from the tag, and the run is linked from the release notes. The Microsoft Store version is signed by Microsoft.
