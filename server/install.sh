#!/bin/sh
# Run as the VM's SSH user, from an extracted Rectangle source archive.
set -eu
umask 077
[ "$(id -u)" -ne 0 ] || { echo 'Run as your Rectangle SSH user, not root.' >&2; exit 1; }
case ${1:-} in
    --direct-https|--vpn-only) ;;
    *) echo 'Usage: sh server/install.sh --direct-https|--vpn-only' >&2; exit 2 ;;
esac
mode=$1
cd -- "$(dirname -- "$0")"
for tool in docker python3 timeout flock bash; do
    command -v "$tool" >/dev/null || { echo "Install $tool before running this installer." >&2; exit 1; }
done
docker info >/dev/null 2>&1 || {
    echo 'Docker is unavailable. Start Docker and verify this SSH account can run docker info.' >&2
    exit 1
}
docker build --pull -t rectangle-browser:demo .
docker build --pull -f Dockerfile.agent -t rectangle-agent:demo .
docker build --pull -f Dockerfile.egress -t rectangle-egress:demo .
docker build --pull -f Dockerfile.vpn -t rectangle-vpn:demo .
docker build --pull -f Dockerfile.proton -t rectangle-proton:demo .
install -d -m 700 "$HOME/.local/bin" "$HOME/.config/rectangle"
install -d -m 700 "$HOME/.config/rectangle/agent-context"
install -m 600 agent-entrypoint clipboard-stream.py memory-stream.py "$HOME/.config/rectangle/agent-context/"
install -m 600 session-seccomp.json "$HOME/.config/rectangle/session-seccomp.json"
install -m 700 rectangle-proton "$HOME/.local/bin/rectangle-proton.next"
mv -f "$HOME/.local/bin/rectangle-proton.next" "$HOME/.local/bin/rectangle-proton"
install -m 700 rectangle-session "$HOME/.local/bin/rectangle-session.next"
mv -f "$HOME/.local/bin/rectangle-session.next" "$HOME/.local/bin/rectangle-session"
if [ "$mode" = --direct-https ]; then
    touch "$HOME/.config/rectangle/cloud-egress"
else
    rm -f -- "$HOME/.config/rectangle/cloud-egress"
fi
echo 'Rectangle host is ready.'
