# Whistle

One app store for Linux and Windows.

You search for an app and hit install. Whistle works out the best way to get it
on your machine: your distro's own package if there is one, then Flatpak, then
Wine or WSL, and a VM only if nothing else works.

It doesn't host any software. It talks to the package managers you already have
(pacman, apt, dnf, zypper, Flatpak, winget) and only downloads installers from
the vendor's own site, checked against a pinned SHA-256.

## Status

Early, but the CLI works:

- Native packages (pacman, apt, dnf, zypper), Flatpak (per-user) and winget.
- Windows apps on Linux through Wine recipes, with menu entries for each app.
- About 1000 apps in the catalog: the most popular Flathub apps plus
  hand-maintained entries.

Proton, WSL and VM recipes are listed but not supported yet. The desktop app
comes after the CLI.

## Try it

You need Rust 1.88 or newer.

```sh
cargo build
target/debug/whistle update                      # download the latest catalog
target/debug/whistle doctor                      # what Whistle sees on this machine
target/debug/whistle search office
target/debug/whistle info firefox                # every way to install it here, best first
target/debug/whistle install firefox --dry-run   # print the commands, change nothing
target/debug/whistle install gimp --via flatpak
target/debug/whistle install notepad++           # Windows app, through Wine
target/debug/whistle list
target/debug/whistle remove gimp
```

Whistle asks before changing anything; `-y` skips the question. `--via <method>`
forces a method (`native`, `apt`, `flatpak`, `wine`, ...), and `--allow-draft`
lets you try recipes that nobody has reviewed yet.

## How it works

`whistle update` downloads the catalog index that whistle-server publishes at
`https://github.com/thisisroot/whistle-server/releases/download/index/index.json`.
For each app, the index lists the sources (package names per distro, Flatpak
refs, winget ids) and any recipes for running it on the other OS.

`whistle install` checks which of those fit this machine and picks the best one.
Every command runs as a plain argument list, never through a shell. A Wine
recipe gets its own prefix in `~/.local/share/whistle/prefixes/<recipe>` and a
menu entry per launcher. Installed apps are recorded in
`~/.local/share/whistle/installed.json`.

Environment variables:

| Variable            | Effect                                                        |
| ------------------- | ------------------------------------------------------------- |
| `WHISTLE_INDEX_URL` | Where `whistle update` downloads from.                        |
| `WHISTLE_INDEX`     | Use this index file instead of the downloaded one.            |
| `WHISTLE_ROOT`      | Keep all of Whistle's files under this folder (for testing).  |

Before the first update, Whistle uses a small built-in catalog
(`catalog/dev-index.json`).

## What's here

- `crates/whistle-core`: catalog, host detection, resolver, installers. The CLI
  and the future desktop app both use it.
- `crates/whistle-cli`: the `whistle` command.
- `catalog/dev-index.json`: the built-in catalog.

Run the tests with `cargo test`.

Related repos:

- [whistle-server](https://github.com/thisisroot/whistle-server) builds and publishes the catalog (Django).
- [whistle-recipes](https://github.com/thisisroot/whistle-recipes) holds the recipes for running apps on the "other" OS.

## License

Apache-2.0 or MIT, your choice.
