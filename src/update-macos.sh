#!/bin/sh
set -eu
repo=https://github.com/mdsmithson/rectangle.git
read_version() {
    sed -nE 's/^version[[:space:]]*=[[:space:]]*"([0-9]+\.[0-9]+\.[0-9]+)"[[:space:]]*$/\1/p' "$1" | head -1
}
case "$RECTANGLE_UPDATE_MODE" in
check)
    sha=$(git -c core.hooksPath=/dev/null ls-remote "$repo" refs/heads/main | awk 'NR == 1 {print $1}')
    case "$sha" in *[!0-9a-f]*|'') echo 'Invalid update revision' >&2; exit 1;; esac
    test "${#sha}" -eq 40
    manifest=$(mktemp)
    trap 'rm -f "$manifest"' EXIT
    curl --fail --silent --show-error --connect-timeout 10 --max-time 30 "https://raw.githubusercontent.com/mdsmithson/rectangle/$sha/Cargo.toml" -o "$manifest"
    version=$(read_version "$manifest")
    test -n "$version"
    printf '%s %s\n' "$sha" "$version"
    ;;
build)
    sha=$RECTANGLE_UPDATE_SHA
    case "$sha" in *[!0-9a-f]*|'') exit 1;; esac
    test "${#sha}" -eq 40
    root=$RECTANGLE_UPDATE_DIR
    export GIT_TERMINAL_PROMPT=0
    git -c core.hooksPath=/dev/null init --quiet "$root/source"
    git -c core.hooksPath=/dev/null -C "$root/source" fetch --quiet --depth=1 "$repo" "$sha"
    git -c core.hooksPath=/dev/null -C "$root/source" checkout --quiet --detach "$sha"
    test "$(read_version "$root/source/Cargo.toml")" = "$RECTANGLE_UPDATE_VERSION"
    cd "$root/source"
    # Ignore a caller's cross-compilation settings: updates must run on this Mac.
    unset CARGO_BUILD_TARGET
    export CARGO_TARGET_DIR="$root/target"
    cargo build --release --locked
    cp "$root/target/release/rectangle" "$root/rectangle"
    chmod 755 "$root/rectangle"
    printf '%s\n' "$RECTANGLE_UPDATE_VERSION" > "$root/version.txt"
    ;;
install)
    target=$RECTANGLE_UPDATE_TARGET
    candidate=$RECTANGLE_UPDATE_CANDIDATE
    pid=$RECTANGLE_UPDATE_PID
    log=$(dirname "$candidate")/install-error.txt
    exec 2>"$log"
    count=0
    while kill -0 "$pid" 2>/dev/null; do
        count=$((count + 1))
        if [ "$count" -gt 60 ]; then echo 'Rectangle did not exit in time' >&2; exit 1; fi
        sleep 1
    done
    staged=$(mktemp "$target.XXXXXX")
    backup=$(mktemp "$target.backup.XXXXXX")
    trap 'rm -f "$staged"' EXIT
    cp -p "$target" "$backup"
    if cp "$candidate" "$staged" && chmod 755 "$staged" && /usr/bin/codesign --verify "$staged" && mv -f "$staged" "$target"; then
        # Refresh bundle metadata for source-built .app installations.
        plist=$(dirname "$(dirname "$target")")/Info.plist
        version_file=$(dirname "$candidate")/version.txt
        if [ -f "$plist" ] && [ -f "$version_file" ]; then
            version=$(cat "$version_file")
            /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$plist" || true
            /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$plist" || true
        fi
        cd "$RECTANGLE_UPDATE_CWD"
        "$target" "$@" </dev/null >/dev/null 2>>"$log" &
        rm -f "$backup" "$candidate"
    else
        mv -f "$backup" "$target"
        echo 'Update installation failed; previous executable preserved.' >&2
        "$target" "$@" </dev/null >/dev/null 2>>"$log" &
        exit 1
    fi
    ;;
*) echo 'Unknown update operation' >&2; exit 1;;
esac
