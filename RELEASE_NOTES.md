# StickyFlow v0.1.0

Initial Linux desktop release.

## Highlights

- encrypted local-first note storage
- optional application password and locked startup
- sticky windows with expanded/chip modes and quick edit
- snippet copy workflow
- optional desktop autostart
- encrypted backup/restore with a separate backup password
- plaintext JSON export/import for portability
- Ubuntu 22.04 amd64 Debian package

## Linux packaging notes

- Debian package name: `sticky-flow`
- installed executable: `/usr/bin/stickyflow`
- desktop entry: `/usr/share/applications/StickyFlow.desktop`
- user data: `~/.local/share/dev.nbatamturk.stickyflow`
- autostart entry: `~/.config/autostart/StickyFlow.desktop`

StickyFlow defaults to GTK X11/XWayland on Linux so sticky always-on-top behavior remains reliable under GNOME sessions where the native Wayland backend does not honor it consistently.

## Security and portability

Encrypted backups are protected by a separate backup password and include the data/security material needed for restore. Plaintext JSON export is intentionally decrypted and should be handled as sensitive data.

## Release validation

The Debian package and AppImage were manually validated on Ubuntu 22.04 amd64. Debian install/reinstall/uninstall behavior, retained user data, note/snippet/sticky flows, encrypted backup/restore, plaintext export/import and sticky always-on-top behavior passed. A real reboot/login autostart test also passed: StickyFlow started automatically in the locked state, exposed no sticky content before unlock, and restored sticky windows after successful password entry.

## Out of scope for v0.1.0

Cloud sync, search/tags, tray controls and global shortcuts are deferred to later versions.
