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
[Releases](https://github.com/ESO-Toolkit/kalpa/releases/latest) page, you have
two layers of assurance:

1. **Transport integrity (always).** Downloads come over **HTTPS from GitHub's
   release storage**, so the transfer is protected against tampering in transit.
   And once installed, the auto-updater re-verifies every future update against
   the signing key compiled into the app before applying it.
2. **Checksum (when published).** Some releases include a **SHA-256 checksum** in
   their notes. If one is present, you can confirm the file matches it. Compute
   the installer's hash in PowerShell:

   ```powershell
   Get-FileHash .\Kalpa_<version>_x64-setup.exe -Algorithm SHA256
   ```

   Compare the output to the checksum in the release notes. If the values match,
   the file is the one we built; if they differ, delete it and download again.
   (If a release doesn't list a checksum, rely on layer 1 above.)

## Reporting a problem

If a checksum doesn't match or the updater reports a signature error, please
[open an issue](https://github.com/ESO-Toolkit/kalpa/issues) and do not run the
installer.
