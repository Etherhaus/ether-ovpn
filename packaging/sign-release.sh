#!/bin/sh
# Signs the release files with minisign, so Ether can check an update before
# installing it.
#
#   ./packaging/sign-release.sh dist/*
#
# Needs minisign (sudo apt install minisign) and the private key, by default
# ~/.minisign/minisign.key. Point MINISIGN_KEY elsewhere to use another one,
# and MINISIGN_PUB at its public half if that isn't next to it.
#
# The public key goes into PUBLIC_KEY in src/updater.rs, as the base64 line of
# the .pub file (the one after "untrusted comment:"). While PUBLIC_KEY is empty
# the client skips the check, so sign and fill it in together. This script
# refuses to sign with a key that doesn't match PUBLIC_KEY, and checks every
# signature it writes.
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

pub=${MINISIGN_PUB:-${key%.key}.pub}
if [ ! -f "$pub" ]; then
    echo "no public key at $pub (set MINISIGN_PUB to its path)" >&2
    exit 1
fi

root=$(dirname "$0")/..
configured=$(sed -n 's/^const PUBLIC_KEY: &str = "\(.*\)";$/\1/p' "$root/src/updater.rs")
if [ -z "$configured" ]; then
    echo "warning: PUBLIC_KEY in src/updater.rs is empty, so clients skip the check" >&2
elif [ "$configured" != "$(sed -n '2p' "$pub")" ]; then
    echo "$pub is not the key in PUBLIC_KEY in src/updater.rs" >&2
    exit 1
fi

for file in "$@"; do
    case ${file##*/} in
        *.minisig | SHA256SUMS) continue ;;
    esac
    minisign -S -s "$key" -m "$file"
    minisign -V -q -p "$pub" -m "$file"
    echo "signed $file"
done
