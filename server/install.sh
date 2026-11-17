#!/bin/sh
# Run as the VM's SSH user, from an extracted Rectangle source archive.
set -eu
umask 077
[ "$(id -u)" -ne 0 ] || { echo 'Run as your Rectangle SSH user, not root.' >&2; exit 1; }
case ${1:-} in
    --direct-https|--vpn-only|--keep-egress) ;;
    *) echo 'Usage: sh server/install.sh --direct-https|--vpn-only|--keep-egress [--enable-server-updates]' >&2; exit 2 ;;
esac
[ "$#" -le 2 ] && { [ "$#" -eq 1 ] || [ "$2" = --enable-server-updates ]; } || exit 2
for tool in docker python3 timeout flock bash; do
    command -v "$tool" >/dev/null || { echo "Install $tool before running this installer." >&2; exit 1; }
done
if [ "${2:-}" = --enable-server-updates ]; then
    command -v git >/dev/null || { echo 'Install Git before enabling server updates.' >&2; exit 1; }
fi
docker info >/dev/null 2>&1 || { echo 'Docker is unavailable for this SSH account.' >&2; exit 1; }
exec python3 -I "$(dirname -- "$0")/release.py" "$@"
