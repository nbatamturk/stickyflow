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

## Known / deferred validation

The packaged autostart entry has been confirmed to point to `/usr/bin/stickyflow`. A full real logout/login autostart cycle with password-protected locked startup remains a manual validation item before the v0.1.0 release is closed.

## Out of scope for v0.1.0

Cloud sync, search/tags, tray controls and global shortcuts are deferred to later versions.
