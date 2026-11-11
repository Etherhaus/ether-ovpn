# Minisign fixtures for the updater tests

Throwaway keys and signatures used by the tests in `src/updater.rs`. They have
nothing to do with the key releases are signed with, and the private halves are
deliberately not kept here: a project whose point is signed updates shouldn't
carry a private key in its history.

- `test-key.pub` — public key the payload is signed with.
- `other-key.pub` — a second public key, for the "signed by someone else" case.
- `payload.txt` — the signed file.
- `payload.txt.minisig` — its signature, made the way `minisign -S` does it.
- `payload.txt.legacy.minisig` — the same file signed in the old, non-hashed format.
- `payload-tampered.txt` — the same file with one word changed.
- `garbage.minisig` — not a signature at all.

To regenerate, make throwaway keys, sign the payload with one of them and copy
the public keys and the signatures back:

```bash
minisign -G -p /tmp/test-key.pub -s /tmp/test-key.key -W
minisign -G -p /tmp/other-key.pub -s /tmp/other-key.key -W
minisign -S -s /tmp/test-key.key -m payload.txt
minisign -S -l -s /tmp/test-key.key -m payload.txt -x payload.txt.legacy.minisig
cp /tmp/test-key.pub /tmp/other-key.pub payload.txt.minisig payload.txt.legacy.minisig .
```

Then delete the private keys. The tests read the `.pub` files and the
signatures, so nothing else has to change.
