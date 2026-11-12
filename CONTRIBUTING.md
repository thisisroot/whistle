# Contributing to Whistle

Thanks for helping. Whistle is three repos, and most contributions fit one of them:

| You want to... | Go to |
| --- | --- |
| Fix a bug or add a feature in the `whistle` command | this repo |
| Add an app, or fix a package name | [whistle-server](https://github.com/thisisroot/whistle-server), `seed/apps.json` |
| Make a Windows app work on Linux | [whistle-recipes](https://github.com/thisisroot/whistle-recipes) |
| Report a security problem | [SECURITY.md](SECURITY.md), privately, not in an issue |

## Before you start

For anything bigger than a small fix, open an issue first and describe what you
want to change. It saves you from writing code that goes in a different direction.

## Working on the client

You need Rust 1.88 or newer.

```sh
cargo build
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same checks on the latest stable Rust, so a new clippy lint may
show up there first.

If you change how apps are installed, also run the real-distro tests (needs Docker):

```sh
target/debug/whistle update
tests/distros/run.sh ~/.cache/whistle/index.json
```

Try your change without touching your system:

```sh
WHISTLE_ROOT=/tmp/whistle-test target/debug/whistle install gimp --dry-run
```

## Ground rules for the code

- **No shell.** Run commands as argument lists with `Cmd`, never through `sh -c`.
- **Treat the catalog as untrusted input.** Validate package names, paths and
  registry keys before using them, even though the server checks them too.
- **Ask before changing the system**, and support `--dry-run`.
- **Test it.** New behavior gets a unit test. `DryRunner` records commands
  without running them.

## Pull requests

- Keep each pull request to one change, and explain why in the description.
- Write commit messages in the imperative: "Add zypper support", not "Added".
- **Sign off your commits** (`git commit -s`). This adds a `Signed-off-by` line,
  which certifies the [Developer Certificate of Origin](https://developercertificate.org/):
  that you wrote the change, or have the right to submit it under the project's license.

## License

By contributing, you agree that your contribution is licensed under the
repo's license: GPL-3.0-or-later for whistle, AGPL-3.0-or-later for
whistle-server, and CC0-1.0 for whistle-recipes.
