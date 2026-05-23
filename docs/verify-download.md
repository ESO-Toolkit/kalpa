# Verify your download

Each Kalpa release publishes three files:

| File | What it is |
|---|---|
| `Kalpa_<version>_x64-setup.exe` | The Windows installer you run |
| `Kalpa_<version>_x64-setup.exe.sig` | The **auto-updater signature** for that installer |
| `latest.json` | Update manifest the app reads to find new versions |

## About the `.sig` file

The `.sig` is **not** a GPG/PGP signature — it is a
[minisign](https://jedisct1.github.io/minisign/) signature produced by Tauri's
updater. Kalpa's auto-updater uses it automatically: when a new version is
available, the app downloads the update and its `.sig`, then verifies the
signature against a public key **compiled into the app** before installing
anything. An update with a missing or invalid signature is refused.

Because the public key is baked into the app and the signature is checked for
you, there is normally nothing to do by hand — updates are verified
automatically. `gpg --verify` does not apply to this file.

## Verifying a fresh download manually

If you download the installer directly from the
[Releases](https://github.com/ESO-Toolkit/kalpa/releases/latest) page and want
to confirm it arrived intact:

1. Downloads come over **HTTPS from GitHub's release storage**, so the transfer
   is already protected against tampering in transit.
2. To check the file itself, compute its SHA-256 hash in PowerShell:

   ```powershell
   Get-FileHash .\Kalpa_<version>_x64-setup.exe -Algorithm SHA256
   ```

3. Compare the output against the SHA-256 checksum published in that release's
   notes. If the values match, the file is the one we built; if they differ,
   delete it and download again.

## Reporting a problem

If a checksum doesn't match or the updater reports a signature error, please
[open an issue](https://github.com/ESO-Toolkit/kalpa/issues) and do not run the
installer.
