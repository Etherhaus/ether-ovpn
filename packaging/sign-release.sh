#!/bin/sh
# Signs the release files with minisign, so Ether can check an update before
# installing it.
#
#   ./packaging/sign-release.sh dist/*
#
# Needs minisign (sudo apt install minisign) and the private key, by default
# ~/.minisign/minisign.key. Point MINISIGN_KEY elsewhere to use another one.
#
# The public key goes into PUBLIC_KEY in src/updater.rs, as the base64 line of
# the .pub file (the one after "untrusted comment:"). While PUBLIC_KEY is empty
# the client skips the check, so sign and fill it in together.
set -eu

if [ "$#" -eq 0 ]; then
    echo "usage: $0 <file>..." >&2
    exit 2
fi

if ! command -v minisign >/dev/null 2>&1; then
    echo "minisign is not installed (sudo apt install minisign)" >&2
    exit 1
fi

key=${MINISIGN_KEY:-$HOME/.minisign/minisign.key}
if [ ! -f "$key" ]; then
    echo "no signing key at $key (create one with: minisign -G)" >&2
    exit 1
fi

for file in "$@"; do
    case $file in
        *.minisig | SHA256SUMS) continue ;;
    esac
    minisign -S -s "$key" -m "$file"
    echo "signed $file"
done
