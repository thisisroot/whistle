#!/bin/sh
# Install the latest Whistle release:
#
#   curl -fsSL https://raw.githubusercontent.com/thisisroot/whistle/main/install.sh | sh
#
# Puts the `whistle` binary in ~/.local/bin. No root needed. Run it again to
# upgrade. Settings, all optional:
#
#   WHISTLE_VERSION       a release tag such as v0.2.0 (default: the latest)
#   WHISTLE_INSTALL_DIR   where to put the binary (default: ~/.local/bin)
#   WHISTLE_NO_COMPLETION set to 1 to skip setting up tab completion
set -eu

REPO=thisisroot/whistle
dir=${WHISTLE_INSTALL_DIR:-$HOME/.local/bin}
version=${WHISTLE_VERSION:-latest}

say() { printf 'whistle: %s\n' "$*"; }
die() { printf 'whistle: error: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = Linux ] || die "only Linux is supported for now (this is $(uname -s))"
case $(uname -m) in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) die "no build for $(uname -m) yet" ;;
esac

if [ "$version" = latest ]; then
    base=${WHISTLE_BASE_URL:-https://github.com/$REPO/releases/latest/download}
else
    base=${WHISTLE_BASE_URL:-https://github.com/$REPO/releases/download/$version}
fi
asset=whistle-$arch-linux.tar.gz

if command -v curl >/dev/null 2>&1; then
    fetch() { curl --proto '=https,file' --tlsv1.2 -fsSL -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget --https-only -q -O "$2" "$1"; }
else
    die "needs curl or wget"
fi

if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
    die "needs sha256sum or shasum"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading $asset ($version)"
fetch "$base/$asset" "$tmp/$asset" || die "download failed: $base/$asset"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || die "download failed: $base/SHA256SUMS"

expected=$(grep " $asset\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
[ -n "$expected" ] || die "$asset is not listed in SHA256SUMS"
actual=$(sha256 "$tmp/$asset")
[ "$actual" = "$expected" ] || die "checksum mismatch for $asset: expected $expected, got $actual"

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$dir"
# Replace the binary atomically, so a running whistle is never half-written.
cp "$tmp/whistle/whistle" "$dir/.whistle.new"
chmod 755 "$dir/.whistle.new"
mv -f "$dir/.whistle.new" "$dir/whistle"

say "installed $("$dir/whistle" --version) to $dir/whistle"

# Tab completion, in the folders bash (with bash-completion) and fish load
# from by themselves. zsh has no such folder, so it gets a line to add.
if [ "${WHISTLE_NO_COMPLETION:-}" != 1 ]; then
    completions=${XDG_DATA_HOME:-$HOME/.local/share}/bash-completion/completions
    if mkdir -p "$completions" && COMPLETE=bash "$dir/whistle" >"$completions/whistle"; then
        say "tab completion is set up for new bash shells"
    fi
    if command -v fish >/dev/null 2>&1; then
        completions=${XDG_CONFIG_HOME:-$HOME/.config}/fish/completions
        mkdir -p "$completions" && COMPLETE=fish "$dir/whistle" >"$completions/whistle.fish" || true
    fi
    case ${SHELL:-} in
        */zsh) say "for tab completion in zsh, add this line to ~/.zshrc:  source <(COMPLETE=zsh whistle)" ;;
    esac
fi
case ":$PATH:" in
    *":$dir:"*) say "run 'whistle update' to download the catalog" ;;
    *)
        say "$dir is not on your PATH. Add this line to your shell's startup file (~/.bashrc, ~/.zshrc):"
        # shellcheck disable=SC2016 # $PATH is printed for the user to copy, not expanded
        printf '\n    export PATH="%s:$PATH"\n\n' "$dir"
        ;;
esac
