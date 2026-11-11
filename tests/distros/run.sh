#!/usr/bin/env bash
# Check Whistle against real distros, in containers:
#   1. every native package the catalog lists for the distro exists in its repos;
#   2. one app installs and removes for real (as root, so without sudo).
#
# Usage: tests/distros/run.sh <index.json> [arch debian ubuntu fedora opensuse]
# Needs Docker and a built whistle binary (target/debug/whistle, or $WHISTLE_BIN).
# $TEST_APP picks the app to install (default: Gcolor3, which is small).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
index=$(realpath "${1:?usage: run.sh <index.json> [distro...]}")
shift
[ $# -gt 0 ] || set -- arch debian ubuntu fedora opensuse
bin=$(realpath "${WHISTLE_BIN:-$here/../../target/debug/whistle}")
app=${TEST_APP:-nl.hjdskes.gcolor3}
out=$(mktemp -d)
echo "logs: $out"

config() {
  case $1 in
    arch)
      image=archlinux:latest tag=linux:arch kind=pacman
      refresh='pacman -Sy --noconfirm >/dev/null' list='pacman -Slq' query='pacman -Q' ;;
    debian | ubuntu)
      [ "$1" = debian ] && image=debian:stable || image=ubuntu:latest
      tag=linux:$1 kind=apt
      refresh='apt-get update -qq' list='apt-cache pkgnames' query='dpkg -s' ;;
    fedora)
      image=fedora:latest tag=linux:fedora kind=dnf
      # Cisco's openh264 server answers 403 in some regions; Fedora's noopenh264
      # stub satisfies the dependency without it.
      refresh='sed -i s/^enabled=1/enabled=0/ /etc/yum.repos.d/fedora-cisco-openh264.repo && dnf -q makecache' list="dnf -q repoquery --available --queryformat '%{name}\n'" query='rpm -q' ;;
    opensuse)
      image=opensuse/tumbleweed tag=linux:opensuse kind=zypper
      refresh='zypper -q --non-interactive refresh'
      list="zypper -q --non-interactive --xmlout search -t package | sed -n 's/.* name=\"\\([^\"]*\\)\".*/\\1/p'"
      query='rpm -q' ;;
    *) echo "unknown distro: $1" >&2; return 1 ;;
  esac
}

# "<package> <app id>" for every native source the catalog lists for this distro.
expected() {
  python3 -I -c '
import json, sys
index, kind, tag = json.load(open(sys.argv[1])), sys.argv[2], sys.argv[3]
for app in index["apps"]:
    for s in app["sources"]:
        if s["kind"] == kind and tag in s["platforms"]:
            print(s["ref"], app["id"])
' "$index" "$kind" "$tag" | sort -u
}

failed=()
for distro in "$@"; do
  config "$distro"
  dir=$out/$distro
  mkdir -p "$dir"
  expected >"$dir/expected.txt"
  pkg=$(awk -v app="$app" '$2 == app { print $1; exit }' "$dir/expected.txt")
  echo "== $distro ($image): $(wc -l <"$dir/expected.txt") packages in the catalog, test app package: ${pkg:-none}"

  script="set -euo pipefail
$refresh
$list | sort -u > /out/available.txt
whistle update --from /index.json
whistle doctor > /out/doctor.txt && head -4 /out/doctor.txt
if [ -n \"\$PKG\" ]; then
  whistle install \"\$APP\" --via native -y
  whistle list | grep -q \"\$APP\"
  $query \"\$PKG\" >/dev/null
  whistle remove \"\$APP\" -y
  if $query \"\$PKG\" >/dev/null 2>&1; then echo 'package still installed after remove'; exit 1; fi
  echo 'install and remove: ok'
fi"
  if ! docker run --rm -e APP="$app" -e PKG="$pkg" -e DEBIAN_FRONTEND=noninteractive \
      -v "$bin:/usr/local/bin/whistle:ro" -v "$index:/index.json:ro" -v "$dir:/out" \
      "$image" bash -c "$script" >"$dir/log.txt" 2>&1; then
    echo "   FAILED, see $dir/log.txt"; tail -5 "$dir/log.txt" | sed 's/^/   | /'
    failed+=("$distro")
    continue
  fi
  grep 'install and remove' "$dir/log.txt" | sed 's/^/   /' || true
  awk '{ print $1 }' "$dir/expected.txt" | sort -u | comm -23 - "$dir/available.txt" >"$dir/missing.txt"
  if [ -s "$dir/missing.txt" ]; then
    echo "   $(wc -l <"$dir/missing.txt") catalog packages don't exist in $distro:"
    grep -wFf "$dir/missing.txt" "$dir/expected.txt" | head -20 | sed 's/^/     /'
    failed+=("$distro")
  else
    echo "   all catalog packages exist"
  fi
done

[ ${#failed[@]} -eq 0 ] && echo "PASS" || { echo "FAILED: ${failed[*]}"; exit 1; }
