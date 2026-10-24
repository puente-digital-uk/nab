# Contributing

Thanks for looking. Nab is small on purpose, so the bar for new features is "does most users' day get better?" Bug reports and fixes are always welcome.

## Before you start

- Open an issue first for anything bigger than a bug fix, so we can agree on the shape before you spend time on it.
- Out of scope, to save you the effort: cloud or AI features, translation, clipboard history, anything that needs the network.
- Nab is open core: this repository is the whole free tool (MIT). A separate, closed add-on (Nab Pro) plugs in through `src/ext.rs` and is not part of this repository or open to contributions. Features that exist in the free tool today will stay free; if you are unsure whether an idea belongs in the free tool, ask in an issue first.

## Ground rules for code

- **One dependency.** Only Microsoft's `windows` crate (plus `embed-resource` at build time). Please don't add crates.
- **It must not crash.** Release builds use `panic = "abort"`. No `unwrap`, `expect` or indexing that can panic in app code.
- **Keep the mouse hook trivial.** `shortcuts::hook_proc` runs for every mouse event on the desktop: atomics and a `PostMessage`, nothing else.
- **Never hold a `RefCell` borrow across a call that re-enters the window procedure** (`DestroyWindow`, `ShowWindow`, `SetForegroundWindow`, `SetCapture`, `TrackPopupMenu` …).
- **Pure logic goes in pure modules** (`blocks.rs`, `trigger.rs`, `logic.rs`) with unit tests that run on any OS.
- **Tests never touch the real settings**: they use the `…\Nab-tests` registry key and the `Nab-tests` Run value.
- **`src/ext.rs` is the boundary, so changes to it are API changes.** Every hook must default to doing nothing, the free tool must never wait for the closed module, and nothing in the free paths may depend on whether a module is installed. Nothing in this repository may enable the Windows Store API (CI checks).
- British English in anything the user sees.

## Toolchain

`rust-toolchain.toml` pins the Rust version; rustup installs it the first time you build. CI uses the same one, so a lint that is new in a newer Rust can't break the build unannounced. Bump it deliberately, in its own commit, together with whatever the newer clippy asks for.

## Checks

```powershell
.\build.ps1    # clippy -D warnings, every test (some drive real windows on screen), release build
```

Everything runs with `--locked`: `Cargo.lock` is the record of exactly what a release was built from, so a build never rewrites it. If you need a newer dependency, change it on purpose (`cargo update -p <crate>`) and commit the lockfile with the change.

CI runs the same on a clean Windows runner for every pull request. Please keep it green.

## Commits and pull requests

- Small, focused PRs. Say what changed and why; link the issue.
- Bump the version in `Cargo.toml`, `assets/app.rc` and `assets/app.manifest` together, and add a line to `CHANGELOG.md`.
