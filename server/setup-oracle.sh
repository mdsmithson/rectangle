#!/bin/sh
# Compatibility wrapper for existing installations.
set -eu
case ${1:-} in
    --cloud-egress) shift; set -- --direct-https "$@" ;;
esac
exec sh "$(dirname -- "$0")/install.sh" "$@"
