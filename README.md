# Whistle

One app store for Linux and Windows.

You search for an app and hit install. Whistle works out the best way to get it
on your machine: your distro's own package if there is one, then Flatpak, then
Wine or WSL, and a VM only if nothing else works.

It doesn't host any software. It talks to the package managers you already have
(pacman, apt, dnf, Flatpak, winget) and only downloads installers from the
vendor's own site.

## Status

Very early. Nothing to install yet.

The first release will be a CLI for Arch, Ubuntu, Fedora and Windows. The
desktop app comes after that.

## Try it

You need Rust 1.88 or newer.

```sh
cargo build
target/debug/whistle doctor                      # what Whistle sees on this machine
target/debug/whistle search office
target/debug/whistle info firefox                # every way to install it here, best first
target/debug/whistle install firefox --dry-run   # print the commands, change nothing
target/debug/whistle install gimp --via flatpak
target/debug/whistle list
target/debug/whistle remove gimp
```

Until whistle-server publishes an index, Whistle uses a small built-in catalog
(`catalog/dev-index.json`). Use `--index <file>` or `WHISTLE_INDEX` to point it at
another one. Set `WHISTLE_ROOT=<dir>` to keep all of Whistle's files in one folder
while testing.

## What's here

This repo is the client: the core library, the CLI and the desktop app (Rust and
Tauri).

Related repos:

- `whistle-server` is the catalog backend (Django).
- `whistle-recipes` holds the install recipes for running apps on the "other" OS.

## License

Apache-2.0 or MIT, your choice.
