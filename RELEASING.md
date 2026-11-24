# Releasing Setu

1. Set the version in `Cargo.toml` (`[workspace.package]`), `app/src-tauri/tauri.conf.json`
   and `app/package.json`. The release workflow refuses a tag that does not match.
2. Commit, then tag and push: `git tag v0.1.0-alpha.N && git push origin main v0.1.0-alpha.N`.
3. The release workflow builds the Windows setup program and the Linux `.tar.gz` and publishes
   a pre-release.

## Signing (in-app updates need it)

Setu installs an update only when the download has a `.sig` made with the private key that
matches `app/src-tauri/update-key.pub`. A release without `.sig` files is never offered.

- **In CI:** set the repository secrets `TAURI_SIGNING_PRIVATE_KEY` (the key file's contents)
  and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The workflow then signs and uploads the `.sig`
  files itself.
- **By hand** (the key never leaves your computer), after the workflow has published:

  ```sh
  gh release download vX -D rel -p '*-setup.exe' -p '*.tar.gz'
  cd app
  for f in ../rel/*; do npx tauri signer sign -f <key file> -p <password> "$f"; done
  gh release upload vX ../rel/*.sig
  ```

Keep the private key and its password backed up. Losing them means installed copies of Setu
can no longer update themselves: people would have to install the next version by hand.
