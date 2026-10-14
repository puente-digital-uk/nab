# Contributing

Thanks for looking. Nab is small on purpose, so the bar for new features is "does most users' day get better?" Bug reports and fixes are always welcome.

## Before you start

- Open an issue first for anything bigger than a bug fix, so we can agree on the shape before you spend time on it.
- Out of scope, to save you the effort: cloud or AI features, translation, clipboard history, anything that needs the network.

## Ground rules for code

- **One dependency.** Only Microsoft's `windows` crate (plus `embed-resource` at build time). Please don't add crates.
- **It must not crash.** Release builds use `panic = "abort"`. No `unwrap`, `expect` or indexing that can panic in app code.
- **Keep the mouse hook trivial.** `shortcuts::hook_proc` runs for every mouse event on the desktop: atomics and a `PostMessage`, nothing else.
- **Never hold a `RefCell` borrow across a call that re-enters the window procedure** (`DestroyWindow`, `ShowWindow`, `SetForegroundWindow`, `SetCapture`, `TrackPopupMenu` …).
- **Pure logic goes in pure modules** (`blocks.rs`, `trigger.rs`, `logic.rs`) with unit tests that run on any OS.
- **Tests never touch the real settings**: they use the `…\Nab-tests` registry key and the `Nab-tests` Run value.
- British English in anything the user sees.

## Checks

```powershell
.\build.ps1    # clippy -D warnings, every test (some drive real windows on screen), release build
```

CI runs the same on a clean Windows runner for every pull request. Please keep it green.

## Commits and pull requests

- Small, focused PRs. Say what changed and why; link the issue.
- Bump the version in `Cargo.toml`, `assets/app.rc` and `assets/app.manifest` together, and add a line to `CHANGELOG.md`.
