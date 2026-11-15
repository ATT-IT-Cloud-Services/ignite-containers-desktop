# Ignite Containers Desktop

Native egui desktop client for Ignite Containers control planes.

- Manage pods, nodes, templates, allocations, backups, schedules, SFTP,
  databases, notifications, API keys, Caddy routes and metrics.
- Live console over authenticated SSE (`/pods/{id}/logs/stream`) with a
  cursor-polling fallback.
- Signed-in operator workflow with pod filters, confirmations, key/value
  editors and a raw API escape hatch.

## Releases

Installers are published from the private source monorepo into this public
repository:

- Linux: `.AppImage`, `.deb`, `.rpm`
- Windows: `.exe` (NSIS) and `.msi` (WiX)

## Building

The desktop crate uses private control-plane crates. Authorized maintainers
build it from the private `ignite-containers-monorepo`; release workflows run
there and publish the resulting installers here. The source in this repository
is a public mirror of the `desktop/` directory.

License: ATT IT AS-IS Software License Agreement. See `LICENSE`.
