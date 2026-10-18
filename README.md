<div align="center">

<table>
  <tr>
    <td valign="middle">
      <a href="https://github.com/OWNER/environ">
        <picture>
          <source media="(prefers-color-scheme: dark)" srcset="docs/brand/ns-environ-logo-dark.png">
          <img alt="Environ logo" src="docs/brand/ns-environ-logo.png" width="120">
        </picture>
      </a>
    </td>
    <td valign="middle">
      <h1>Environ</h1>
    </td>
  </tr>
</table>

**A focused, Rust-native desktop code editor with a familiar workbench.**

[![CI](https://github.com/OWNER/environ/actions/workflows/ci.yml/badge.svg)](https://github.com/OWNER/environ/actions)
[![Release](https://img.shields.io/github/v/release/OWNER/environ)](https://github.com/OWNER/environ/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](#license)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/)
![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)

[Features](#features) · [Download](#download) · [Build from source](#build-from-source) · [Shortcuts](#keyboard-shortcuts) · [Security](#security) · [License](#license)

</div>

---

Environ is inspired by the familiar workbench pattern of VS Code. It is an independent implementation with its own UI, logo, and workspace services; it does not copy VS Code source or branding.

<p align="center">
  <img src="docs/screenshots/environ-desktop.png" alt="Environ desktop editor" width="900">
</p>

## Features

**Workbench**

- Native desktop runtime built with `eframe`/`egui`.
- Dark, VS Code-style layout: activity rail, Explorer / Search / Extensions views, editor breadcrumbs, tabs, command search bar, terminal panel, and status bar.
- Environ logo in the window, workbench, welcome screen, and about dialog.

**Editing**

- Workspace explorer with lazy directory loading, folders-first sorting, and hidden-file support.
- Multiple editor tabs with dirty-state indicators, plus Save and Save All.
- Find and replace across the active document.
- Command palette with keyboard-driven actions.

**Extensions and runners**

- Built-in runnable extensions for Rust workspaces, the active Python file, and the active Node.js file. Commands run in the selected workspace on a background thread, so the editor stays responsive.
- Optional project-local runners defined in `environ.extensions.json`. They are listed for review and run only after an explicit click.

**Safety**

- Workspace-scoped file access with canonical-path containment checks, symlink escape rejection, UTF-8 validation, binary detection, and a 2 MB text-file limit.
- Optimistic save protection: a file changed by another process is never silently overwritten.
- Saves go through a temporary file with flush-before-replace.

**Delivery**

- GitHub Actions validate formatting, tests, and Clippy, and build tagged Windows, Linux, and macOS releases.

## Download

Prebuilt binaries are published on the [Releases](https://github.com/OWNER/environ/releases) page for each tagged version.

| Platform | Asset |
| --- | --- |
| Windows x64 | `Environ-vX.Y.Z-windows-x64.exe` or `Environ-vX.Y.Z-windows-x64.zip` |
| Linux x64 | Archive attached to the release |
| macOS arm64 | Archive attached to the release |
| Checksums | `SHA256SUMS` |

## Build from source

**Requirements**

- Rust stable toolchain.
- A desktop supported by `eframe` (Windows, macOS, or Linux).
- On Windows, local builds need the MSVC linker and the Windows SDK. The GitHub release workflow provides hosted Windows builds if the local linker is unavailable.

**Run in development**

```sh
cargo run
```

**Build a release binary**

```sh
cargo build --release
```

The Windows executable is written to `target/release/environ.exe`.

**Validate before committing**

```sh
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl/Cmd+O` | Open a project folder |
| `Ctrl/Cmd+S` | Save the active file |
| `Ctrl/Cmd+Shift+S` | Save all files |
| `Ctrl/Cmd+F` | Find and replace |
| `Ctrl/Cmd+P` | Open the command palette |
| `Ctrl/Cmd+B` | Toggle the sidebar |
| ``Ctrl/Cmd+` `` | Toggle the integrated terminal |
| `Escape` | Close transient dialogs |

## Project runners

Copy `environ.extensions.example.json` to `environ.extensions.json` in a workspace and adapt the entries. `{file}` is replaced with the active file's workspace-relative path.

Project runners must use executable names (not absolute or path-based programs), are shown in the Extensions view, and execute only when you press **Run**.

## Security

- File operations are restricted to the selected workspace. Every directory listing, open, and save re-canonicalizes the path and rejects paths outside the workspace, including symlinked paths.
- Saves compare the on-disk file with the version originally opened, preventing accidental overwrites after an external edit.
- Runner commands use the selected workspace as their working directory. Built-in runners use fixed executable names and arguments.
- The integrated terminal accepts commands typed by the local user, as expected for a code editor.

## Project layout

```text
.
├── assets/environ.svg                # Environ brand mark
├── docs/brand/ns-environ-logo.png    # Logo used in this README (light mode)
├── docs/brand/ns-environ-logo-dark.png # Logo used in this README (dark mode)
├── docs/screenshots/                 # README screenshots
├── src/main.rs                       # Native application and workspace service
├── Cargo.toml                        # Rust package metadata and dependencies
├── Cargo.lock                        # Reproducible dependency resolution
├── environ.extensions.example.json   # Optional project runner schema
└── .github/workflows/                # CI and release automation
```

## Releasing

Update the version in `Cargo.toml`, commit, and push a matching tag:

```sh
git tag v0.3.0
git push origin main --tags
```

The tagged workflow builds and publishes the Windows `.exe` and `.zip`, Linux x64 and macOS arm64 archives, and `SHA256SUMS`. The release job fails if the tag and the Cargo package version differ.

## License

Distributed under the MIT License. See the repository history and release metadata for attribution details.
