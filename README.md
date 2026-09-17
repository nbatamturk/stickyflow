# StickyFlow

StickyFlow is a lightweight, local-first desktop notes app built with Tauri, React and Rust. It combines encrypted note storage with sticky windows, snippets, an optional application password, desktop autostart, encrypted backups and portable JSON export/import.

## v0.1.0 scope

- encrypted notes stored locally in SQLite
- optional application password
- sticky windows with expanded/chip modes and quick edit
- snippet copy workflow
- configurable sticky appearance/opacity
- optional desktop autostart
- encrypted backup and restore using a separate backup password
- plaintext JSON export/import for portability

Cloud sync, search/tags, tray controls and global shortcuts are intentionally outside the v0.1.0 scope.

## Linux support

The v0.1.0 release is packaged and manually validated on Ubuntu 22.04 amd64.

StickyFlow defaults to the GTK X11/XWayland backend on Linux because GNOME Wayland does not reliably honor always-on-top behavior for sticky windows. An explicitly supplied `GDK_BACKEND` still overrides that default.

## Install from the Debian package

```bash
sudo apt install ./StickyFlow_0.1.0_amd64.deb
```

The installed executable is:

```text
/usr/bin/stickyflow
```

The desktop entry is installed under `/usr/share/applications/StickyFlow.desktop`.

## AppImage

If using the AppImage build:

```bash
chmod +x StickyFlow_0.1.0_amd64.AppImage
./StickyFlow_0.1.0_amd64.AppImage
```

AppImage support depends on the local Linux/Tauri bundling toolchain and is validated separately from the Debian package.

## First run and security

On first launch StickyFlow asks whether to enable an application password.

With a password enabled, the master encryption key is wrapped with a password-derived key and note data is not available until the password is verified. Without an application password, notes are still encrypted at rest, but the local encryption key is protected by the current OS user account and restrictive file permissions.

StickyFlow keeps decrypted note content out of persistent logs and does not expose notes before unlock in password-protected mode.

## Backup, restore and portability

Settings → **Data & Portability** provides four operations:

- **Create encrypted backup**: creates a `.stickyflow-backup` file protected by a separate backup password.
- **Restore encrypted backup**: replaces the current StickyFlow dataset after validation and confirmation.
- **Export plaintext JSON**: exports readable note titles/content for portability. This file is intentionally **not encrypted**.
- **Import plaintext JSON**: merges exported notes into the current encrypted store with new IDs.

Encrypted backups include the data and security metadata required for disaster recovery. OS-level autostart registration is intentionally not included in a backup.

## Autostart

Autostart can be enabled or disabled in Settings. On Linux, StickyFlow writes a desktop autostart entry under:

```text
~/.config/autostart/StickyFlow.desktop
```

For the installed Debian package the entry should launch `/usr/bin/stickyflow`.

## Local data

StickyFlow application data is stored under:

```text
~/.local/share/dev.nbatamturk.stickyflow
```

This contains the SQLite database and local security material required by the application. Treat it as private data and do not edit its files while StickyFlow is running.

## Uninstall

Remove the Debian package with:

```bash
sudo apt remove sticky-flow
```

Removing the package removes the installed application files but does not intentionally delete the user's StickyFlow data directory under `~/.local/share/dev.nbatamturk.stickyflow`. This allows reinstall/upgrade scenarios to retain local notes. If you want to remove all local StickyFlow data, first create any backup you need, then remove the data directory manually after uninstalling.

Autostart is user-level state. If an autostart entry remains after removing the package, remove it with:

```bash
rm -f ~/.config/autostart/StickyFlow.desktop
```

## Development

Requirements include Node.js, Rust and the Linux GTK/WebKit dependencies required by Tauri 2.

```bash
npm install
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

Release bundles can be produced with:

```bash
npm run tauri build -- --bundles deb,appimage
```

## Project

Repository: https://github.com/nbatamturk/stickyflow
