# Feedback Memo

A local-first desktop app for capturing short notes about teammates and reviewing those moments when it is time to write quarterly feedback.

## Current features

- Tauri 2 desktop app for macOS and Linux
- Compact quick-note panel that opens at the bottom-right by default
- Draggable header with window position restored from SQLite
- A short note field designed for two or three lines
- Keyboard flow: `Person → Date → Note → Save`
- Global shortcut: `Cmd + Shift + M` on macOS and `Ctrl + Shift + M` on Linux
- System tray actions for New note, History, and Quit
- Local SQLite storage for teammates, notes, and window position
- Draft recovery when the panel closes before saving
- History filtering by teammate and keyword
- Teammate setup
- English-only application interface and error messages

The SQLite database is stored as `feedback-memo.sqlite3` in the operating system’s standard application-data directory.

## Development requirements

- Node.js 20 or later
- pnpm 10
- Stable Rust toolchain
- Tauri 2 system dependencies for Linux or macOS

Install dependencies and start the desktop app:

```bash
pnpm install
pnpm tauri dev
```

To preview only the web interface:

```bash
pnpm dev
```

The browser preview uses temporary sample data instead of SQLite.

## Desktop release build

Create a standalone optimized binary and the installers supported by the current operating system:

```bash
pnpm tauri build
```

Linux output is written under `src-tauri/target/release/`:

- `feedback-memo` — standalone executable for the current Linux architecture
- `bundle/appimage/*.AppImage` — portable Linux application
- `bundle/deb/*.deb` — Debian and Ubuntu installer
- `bundle/rpm/*.rpm` — Fedora and RPM-based installer

Run the AppImage without Node.js, pnpm, or a source checkout:

```bash
chmod +x "Feedback Memo_0.1.0_amd64.AppImage"
./"Feedback Memo_0.1.0_amd64.AppImage"
```

Build the macOS `.app` and `.dmg` on a macOS machine with the same `pnpm tauri build` command. Public distribution additionally requires Apple signing and notarization.

## Verification

```bash
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

## Planned improvements

- Deactivate and reorder teammates
- Edit notes and undo deletion
- Quarter-based filtering
- Markdown and CSV export
- Optional top-right, bottom-right, or last-position placement
- AppImage, deb, and dmg release builds

See [PRODUCT_PLAN.md](./PRODUCT_PLAN.md) for the product plan.
