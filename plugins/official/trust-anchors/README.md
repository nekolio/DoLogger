# Plugin Trust Anchors

This directory is the **public** trust store for official plugin signing keys.
Only PUBLIC material lives here — private keys (seeds) never enter this
repository. The private seed is stored exclusively in the
`DOLOGGER_PLUGIN_SIGNING_KEY` GitHub Actions secret (AES-256-encrypted at rest
by GitHub's infrastructure).

| File | Content |
| :- | :- |
| `active.pub` | Active Ed25519 public keys — one 64-hex key per line. A plugin whose `.sig` verifies against ANY active, non-revoked key is granted **Blue** trust. |
| `revoked.txt` | Revocation list (CRL) — `<64-hex SHA-256 fingerprint> [reason] [unix-ts]` per line. A key on this list can **never** grant Blue, even if it is still in `active.pub`. |

The loader reads this store via `plugin_trust_store` in `dologger.toml` (or
`dologctl plugin verify --trust-store <dir>`).

## One-time bootstrap

```bash
dologctl plugin keygen signing.key
# → prints the public key (64 hex)

# 1. Set the printed public key as a line in active.pub and commit it.
# 2. Store the seed (content of signing.key) as the GitHub Actions secret
#    DOLOGGER_PLUGIN_SIGNING_KEY (Settings → Secrets and variables → Actions).
# 3. Protect your local copy of signing.key: dologctl plugin wrap-key signing.key signing.key.enc
```

From then on, the release workflow signs official bundles with that seed and
ships a `.sig` sidecar next to each bundle.

## Scheduled rotation

1. `dologctl plugin keygen new-signing.key` — generate a fresh key.
2. Add the new public key to `active.pub` and commit (both keys now active).
3. Replace the `DOLOGGER_PLUGIN_SIGNING_KEY` secret with the new seed.
4. After the grace window, move the **old** key's fingerprint into
   `revoked.txt` with reason `superseded` and commit — old-key signatures now
   fail verification.

## Emergency revocation (compromise)

Loss stays bounded: a leaked key can only vouch for plugins released between
the compromise and the revocation.

1. Append the compromised fingerprint to `revoked.txt` with reason `compromised`
   and commit — the loader rejects its signature **immediately**, even in dev
   mode, and even if the key is still listed in `active.pub`.
2. Rotate the secret: `dologctl plugin keygen new-signing.key` → update
   `DOLOGGER_PLUGIN_SIGNING_KEY` → add the new public key to `active.pub` → commit.
3. Any already-shipped artifact signed by the revoked key will fail
   verification for any loader with this updated store.

## Notes

- The release workflow signs with the **raw secret**; `wrap-key`/`unwrap-key`
  are for your local key files only (SSH-style passphrase protection). Do not
  introduce a CI unwrap path.
- Enable GitHub secret-scanning and push protection in
  Settings → Code security. The default scanner does not detect raw 64-hex
  seeds, so the `.gitignore` rules and the `leak-hygiene` workflow are the
  practical backstop.
