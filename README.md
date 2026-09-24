# Feedback Memo

A local-first macOS desktop app for capturing short notes about teammates and reviewing those moments when it is time to write quarterly feedback.

Everything stays on your Mac. There is no account, no server, and no network access.

## Install

Feedback Memo ships as a normal macOS application. **You do not need Rust, Node.js, or pnpm to run it** — those are build-time tools only. The bundled app is a self-contained native binary that links against nothing but the system frameworks, and it draws its interface with the WebKit engine already present in macOS.

### Requirements

- macOS 12 Monterey or later
- Apple Silicon or Intel. The released disk image is a universal binary that runs natively on both.

### Install from the disk image

1. Open `Feedback Memo_0.1.0_universal.dmg`.
2. Drag **Feedback Memo** into your `Applications` folder.
3. Launch it from Launchpad or Spotlight.

The app is not code-signed or notarized, so the first launch needs one extra step. Right-click the app and choose **Open**, then confirm — macOS remembers the choice. If macOS refuses to open it at all, clear the quarantine flag and try again:

```bash
xattr -dr com.apple.quarantine "/Applications/Feedback Memo.app"
```

### First run

Feedback Memo lives in the menu bar and deliberately has no Dock icon. Open the quick-note panel in any of these ways:

- Press `Cmd + Shift + M` from any application
- Click the menu bar icon
- Right-click the menu bar icon and choose **New note** or **History**

Add a teammate from the **+ Add teammate** entry in the person dropdown, then start capturing notes. Quit from the menu bar icon — closing the panel only hides it.

To launch the app automatically, add it under **System Settings → General → Login Items**.

### Where your data lives

```
~/Library/Application Support/jp.feedback-memo.desktop/feedback-memo.sqlite3
```

A plain SQLite database. Back it up by copying that file while the app is not running; because write-ahead logging is enabled, copy the `-wal` and `-shm` siblings too if they exist. You can also export notes as JSON from the History screen at any time.

### Uninstall

Delete `/Applications/Feedback Memo.app`. To remove your notes as well, delete the application-support directory shown above.

## Features

- Compact quick-note panel that opens at the bottom-right and remembers where you drag it
- Keyboard flow: `Person → Date → Note → Save`, with `Cmd + Enter` to save and `Esc` to dismiss
- Global shortcut `Cmd + Shift + M`. If another application already owns it, Feedback Memo still starts and writes a warning to stderr; use the menu bar icon instead
- Menu bar actions for New note, History, and Quit
- Add teammates from a modal in the panel, without leaving the capture flow
- History with person, period, and keyword filters, paged 20 notes at a time
- Period presets for the current quarter, the previous quarter, the last three months, or a custom date range (calendar quarters starting in January)
- Export the filtered notes as JSON, either to a file or to the clipboard; the export always covers every match, not just the visible page
- Draft recovery when the panel closes before saving
- Local SQLite storage with a schema versioned through `PRAGMA user_version` migrations
- Content Security Policy enabled for the webview, with a separate development policy for the Vite dev server
- English-only interface and error messages

## Development

Requirements: Node.js 20 or later, pnpm 10, a stable Rust toolchain, and the Xcode Command Line Tools.

```bash
pnpm install
pnpm tauri dev
```

To preview only the web interface, with temporary sample data instead of SQLite:

```bash
pnpm dev
```

### Verification

```bash
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

### Release build

Release builds are universal, so a single disk image covers Apple Silicon and Intel. Add both targets once:

```bash
rustup target add aarch64-apple-darwin
rustup target add x86_64-apple-darwin
```

Then build:

```bash
pnpm tauri build --target universal-apple-darwin
```

Output lands under `src-tauri/target/universal-apple-darwin/release/bundle/`:

- `macos/Feedback Memo.app` — the standalone application
- `dmg/Feedback Memo_0.1.0_universal.dmg` — disk image for distribution

Plain `pnpm tauri build` also works and is faster, but it produces a single-architecture bundle for whichever Mac runs it, written to `src-tauri/target/release/bundle/`. Use it while developing, and the universal target for anything you hand to someone else.

### Signing and notarization

Public distribution requires an Apple Developer ID certificate and notarization. Without them, every recipient has to clear the quarantine flag by hand. See the Tauri documentation for `APPLE_SIGNING_IDENTITY` and the notarization options of `tauri build`.

## Planned improvements

- Deactivate and reorder teammates
- Edit notes and undo deletion
- A configurable first month for the quarter cycle
- Markdown and CSV export
- Optional top-right, bottom-right, or last-position placement
- Signed and notarized release builds

See [PRODUCT_PLAN.md](./PRODUCT_PLAN.md) for the product plan.
