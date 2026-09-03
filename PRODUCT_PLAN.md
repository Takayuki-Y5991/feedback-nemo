# Feedback Memo — Product Plan

Last updated: September 2, 2026

## 1. Problem

The development team changes every quarter. When teammates wait three months before writing feedback, specific events and smaller contributions are easily forgotten.

Feedback Memo is a lightweight desktop companion for capturing what happened, when it happened, and who it involved without interrupting the current task. At the end of the quarter, those notes become concrete source material for thoughtful feedback.

## 2. Product principles

1. **Capture in five seconds** — Open the panel with a global shortcut and start immediately.
2. **Only three inputs** — Date, person, and note. Do not require classification or evaluation while capturing.
3. **Reduce recall effort** — Present observations chronologically by person and period.
4. **Private by default** — Store MVP data locally and prevent accidental sharing.
5. **Capture, do not send** — Focus first on recording and retrieving source material.

## 3. Product format

Build a system-tray desktop app for macOS and Linux.

- Open the panel from the tray icon or with `Cmd + Shift + M` on macOS and `Ctrl + Shift + M` on Linux.
- Show the panel at the bottom-right by default and keep it above other windows while it is needed.
- Let the user drag the panel and restore its most recent position.
- Keep the panel open after saving so several notes can be captured in sequence.
- Close it explicitly with `Esc` or the close button.
- Work offline without authentication or network access.

The MVP uses Tauri 2, Rust, React, TypeScript, pnpm, and SQLite. A regular web app or browser extension would add friction whenever the user is working outside the browser.

Linux desktop environments differ in tray and window-position behavior. When Wayland prevents absolute placement, the compositor-controlled or last-known position should be treated as the fallback.

## 4. MVP screens and behavior

### 4.1 Quick-note panel

Display a compact, frameless panel approximately 400px wide. Keep placement logic separate from the interface so top-right, bottom-right, and remembered-position options can be compared later.

1. **Person** — Choose an active teammate. Keep the label and empty-state copy short.
2. **Date** — Default to today. Preserve the current time internally while presenting a compact date picker.
3. **Note** — A multiline input optimized for two or three short lines.
4. **Save** — Use `Cmd/Ctrl + Enter` or the compact Save button.

Supporting behavior:

- Move through the form in the order `Person → Date → Note → Save` with Tab.
- Use Shift+Tab to move backward.
- Use `Esc` to close the panel.
- Preserve an unfinished note locally when the panel closes.
- Keep the panel open after saving, clear the note, and focus the note field again.
- Show actionable inline errors when the person or note is missing.
- Save without waiting for a network request.

### 4.2 History

- Show notes newest first.
- Filter by teammate and keyword.
- Add date-range and quarter filters.
- Show a chronological view for an individual teammate.
- Edit and delete notes, with Undo immediately after deletion.
- Copy a teammate’s notes as Markdown or export them as CSV.

AI-generated prose and sending to Slack or other systems are outside the MVP. The app must not silently rate, rewrite, summarize, or transmit source notes.

### 4.3 Settings

- Add, rename, reorder, and deactivate teammates.
- Continue showing deactivated teammates in historical notes.
- Change the global shortcut.
- Configure the first month of the organization’s quarter cycle.
- Export, back up, and restore local data.
- Choose top-right, bottom-right, or remembered window placement.

## 5. Data model

### Person

| Field | Type | Notes |
| --- | --- | --- |
| id | integer | Internal SQLite identifier |
| name | text | Required; managed in Settings |
| is_active | boolean | Deactivate instead of deleting during team changes |
| sort_order | integer | Ordering in selection controls; planned |
| created_at | datetime | Creation timestamp |

### Memo

| Field | Type | Notes |
| --- | --- | --- |
| id | integer | Internal SQLite identifier |
| occurred_at | datetime | Defaults to the current local date and time; editable |
| person_id | integer | Required reference to Person |
| content | text | Required note content |
| created_at | datetime | Audit timestamp |
| updated_at | datetime | Audit timestamp |

### Setting

| Field | Type | Notes |
| --- | --- | --- |
| key | text | Unique setting name |
| value | text | Serialized setting value |

The quarter is calculated from `occurred_at` and the configured start month rather than stored on each memo. Changing the quarter configuration therefore does not require a data migration.

## 6. Interface direction

- Use a quiet, polished desktop-palette aesthetic inspired by macOS utility windows.
- Favor spacing, typography, and subtle surface depth over decorative elements.
- Use a neutral canvas with a restrained teal accent. Reserve red for errors and destructive actions.
- Keep visible icons small while preserving sufficiently large interaction targets.
- Maintain at least 4.5:1 contrast for normal text.
- Communicate save, error, and focus states with text or icons in addition to color.
- Support the complete capture flow from the keyboard with visible focus rings.
- Limit motion to meaningful open, close, and save feedback around 150–200ms.
- Respect the operating system’s reduced-motion preference.
- Keep all application copy in English.

## 7. Technical direction

| Area | Decision |
| --- | --- |
| Desktop shell | Tauri 2 and Rust |
| Interface | React and TypeScript |
| Package manager | pnpm |
| Persistence | Local SQLite file; no external database server |
| State management | Prefer React primitives until additional state tooling is justified |
| Verification | TypeScript checks, Rust formatting and Clippy, unit tests, and critical-path E2E tests |
| Distribution | macOS `.dmg`; Linux `.AppImage` and `.deb` |

The MVP has no account, cloud sync, administrator console, or server. The database location must be documented, and users must be able to export their data.

### 7.1 Language and desktop framework decision

Tauri was selected for the MVP. Wails can produce an equally flexible web-based interface, but Tauri provides a clearer supported path for the product’s central requirements: a system tray, global shortcuts, a small always-on-top window, and lightweight distribution.

Keep Rust limited to operating-system integration, SQLite access, window behavior, and Tauri commands. Implement visual presentation and form state in React and TypeScript.

The alternatives considered were:

| Candidate | Main languages | Strengths | Tradeoffs |
| --- | --- | --- | --- |
| Tauri | Rust + TypeScript | Lightweight, strong OS integration, flexible web UI | Requires both Rust and frontend knowledge |
| Electron | TypeScript | Mature ecosystem and fast UI development | Larger memory footprint and distribution size |
| Go + Wails | Go + TypeScript | Go application layer with flexible web UI | Tray and shortcut behavior needs additional validation |
| Go + Fyne | Go | Mostly one language and built-in system-tray support | Less visual flexibility and weaker native integration |
| Clojure + cljfx/JavaFX | Clojure | Declarative UI and broad JVM ecosystem | JVM startup, memory, packaging, and global-hook dependencies |
| Nim + GTK 4 | Nim | Native compilation, lightweight, built-in SQLite access | Smaller cross-platform desktop integration ecosystem |
| Python + Qt | Python | Fast prototyping and straightforward SQLite support | Packaging and platform-specific tuning can become costly |

## 8. Delivery phases

### Phase 0 — Specification and interaction prototype

- Confirm target operating systems and distribution constraints.
- Prototype the quick panel, History, and Settings.
- Validate keyboard focus and shortcut behavior.

### Phase 1 — Tauri technical slice and persistent shell

- Validate tray, global shortcut, draggable frameless window, placement, and SQLite persistence.
- Compare startup behavior and Linux Wayland/X11 behavior.
- Record significant findings in an Architecture Decision Record.

### Phase 2 — Quick capture

- Complete teammate setup, memo persistence, and draft recovery.
- Refine the compact English interface.
- Verify repeated capture without closing the panel.

### Phase 3 — Review and data management

- Add person, period, and keyword filtering.
- Add editing, deletion, and Undo.
- Add Markdown copying and CSV export.
- Add configurable quarter boundaries.

### Phase 4 — Quality and distribution

- Verify keyboard access, accessibility, and multi-monitor placement.
- Test SQLite migrations, export, backup, and restore.
- Produce macOS and Linux builds with installation documentation.

## 9. MVP acceptance criteria

- A global shortcut opens the panel while another application is active.
- The panel opens at a predictable position and can be dragged elsewhere.
- The most recent quick-panel position survives an application restart.
- The complete capture flow works without a mouse.
- The initial date uses the device’s current local date and retains the current time internally.
- Only configured active teammates appear in the capture selector.
- A saved note appears in History immediately.
- The panel stays open after saving and is ready for another note.
- Notes and unfinished drafts survive an application restart.
- Deactivating a teammate does not break historical notes.
- Notes can be filtered by teammate and period and exported as Markdown or CSV.
- Major actions have meaningful accessible names and visible focus states.
- All application-facing copy is English.

## 10. Existing-product alternatives

### Fastest habit test

For a macOS-first team, Raycast Notes can test the habit quickly by opening a floating note with a hotkey and keeping one section per teammate. Person selection, quarter filtering, and structured export would remain manual.

A Notion database with Date, Person, and Note fields could be opened from a Raycast Quicklink. This provides structure but adds navigation, network dependency, and more friction at capture time.

If the organization already licenses a performance-management platform with continuous feedback, it may also be viable. Its private-note behavior, visibility rules, and capture speed must be verified before adoption.

### Decision guide

- **Test only whether the habit works:** Run a one-week Raycast Notes trial.
- **Structure the three fields and reuse them every quarter:** Continue building Feedback Memo.
- **Manage organization-wide reviews, permissions, and delivery:** Evaluate an existing performance-management platform.

## 11. Post-MVP opportunities

- Quick capture from Slack or Microsoft Teams
- Encrypted sync between devices
- Project or theme tags rather than Good/Improve ratings
- AI-assisted feedback drafting from a user-selected date range
- A subtle reminder when no recent notes exist for a teammate

If AI drafting is added, source observations and generated prose must remain visibly separate. Nothing is sent externally until the user explicitly reviews and approves it.

## 12. Remaining product decisions

1. Which platform should receive release-level polish first: macOS or Linux?
2. Which Linux distributions, desktop environments, and Wayland/X11 sessions are required?
3. Do company devices require macOS signing and notarization?
4. Must memo content remain completely offline in every future version?
5. Does the organization use calendar quarters or a custom starting month?
