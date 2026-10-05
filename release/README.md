# Releasing the Truthcoin App

A tag `vX.Y.Z`, which you sign with the release key, makes GitHub build the Linux and macOS packages
(`.github/workflows/release.yml`) and leave them in a **draft** release with `SHA256SUMS`. The draft is published only
after you have signed `SHA256SUMS` with the same key. `release-app.sh` drives every step and checks GitHub's work; you
only sign: the tag, then `SHA256SUMS`.

The packages of release `X.Y.Z`:

| File | What |
|---|---|
| `truthcoin-app_X.Y.Z_amd64.deb` | Ubuntu 22.04+ / Debian 12+. Rebuilds byte for byte from the tag (`build/linux/rebuild.sh`) |
| `Truthcoin-App_X.Y.Z_amd64.AppImage` | Other Linux |
| `Truthcoin-App_X.Y.Z_universal.dmg` | macOS, Apple Silicon and Intel (ad-hoc signed, not notarised) |
| `Truthcoin-App_X.Y.Z_universal.app.tar.gz` | The same Mac app alone, for an updater |
| `SHA256SUMS`, `SHA256SUMS.sig` | Every file's SHA-256, and its signature by the release key |

The names have a hyphen where Tauri puts a space ("Truthcoin App"): a GitHub release would turn the space into a dot,
and `SHA256SUMS` must list the names as published.

## Once

1. **Make the release key** (ed25519, with a passphrase), and keep a copy of the private half offline:

   ```
   ssh-keygen -t ed25519 -f ~/.ssh/truthcoinapp-release -C truthcoinapp-release
   cp ~/.ssh/truthcoinapp-release.pub release/truthcoinapp-release.pub
   git add release/truthcoinapp-release.pub && git commit -m "Release key"
   ```

   Put the same line in the README's "Verify your download" (below) in place of `ssh-ed25519 AAAA…`. Until the `.pub`
   is committed, every step except `scan` stops.

2. **GitHub.** Create `truthcoinapp/truthcoin-app`, add it as the remote `origin`, check that no commit holds private
   details, and push `main`:

   ```
   git remote add origin https://github.com/truthcoinapp/truthcoin-app.git
   release/release-app.sh 0.1.0 scan
   git push origin main
   ```

   The whole history goes public with the repository, so `scan` checks every commit GitHub doesn't have yet, files and
   messages. If it finds anything in an old commit, fixing the file isn't enough: that commit has to be rewritten
   (or the history squashed) before the first push.

3. **Settings on GitHub.**
   - **Never enable Pages on any other repository of the `truthcoinapp` organisation, and keep the organisation for
     this app alone:** every Pages site there shares the phone page's origin, and with it the phone's keys (VERIFY.md).
   - Make the repository public before the first tag: GitHub records build attestations only for public
     repositories, and `check` requires them.
   - Pages: Settings > Pages > Source: **GitHub Actions**. Then Settings > Environments > `github-pages` >
     Deployment branches and tags: add the tag rule `v*`. Without it, the phone page's deploy from a tag is refused.

## Each release

1. Set the version in `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`, write
   `release-notes/vX.Y.Z.md` (it heads the release's notes), and commit.
2. `release/release-app.sh X.Y.Z dry`: pushes the commit to branch `release-vX.Y.Z` and runs the release workflow
   there (it builds everything and publishes nothing), then rebuilds the `.deb` here in Docker (`JOBS=2`) and
   compares it with GitHub's.
3. Sign the tag, on the machine with the key (it asks for the key's passphrase):

   ```
   release/sign-tag.sh vX.Y.Z
   ```

   It tags the commit the dry run built and checked, signed with the release key (git's SSH signing), checks the
   signature against `release/truthcoinapp-release.pub`, and pushes nothing. Anyone can then check that the source they
   review is the release's (VERIFY.md, step 1).
4. `release/release-app.sh X.Y.Z release`: checks that the tag is at the dry run's commit and signed by the release
   key (it never makes a tag itself), pushes it (and `main` if it doesn't hold the commit yet), watches the build,
   then runs `check`. The tag also deploys the phone page (`pages.yml`).
   (`release/release-app.sh X.Y.Z all` runs 2 and then 4; it stops after 2 until the tag is signed.)
5. `check` (step 4 runs it; run it again any time): the draft holds exactly the four packages and `SHA256SUMS`,
   each matches it and has a build attestation from `release.yml` at the tag, the `.deb` equals your rebuild, and
   the Mac app is this version alone. It ends by printing the signing command, with the checked file's sha256.
6. Sign `SHA256SUMS`, on the machine with the key:

   ```
   release/sign-sums.sh vX.Y.Z <sha256 that check printed>
   ```

   It signs only the `SHA256SUMS` that `check` verified, checks the signature against `release/truthcoinapp-release.pub`,
   and uploads `SHA256SUMS.sig` to the draft.
7. `release/release-app.sh X.Y.Z after`: checks the signature again, over the file `check` verified, that GitHub's
   tag is still the signed one, and that the phone page's deploy from the tag is green; then publishes the draft as
   the latest release, and checks that GitHub serves the signed `SHA256SUMS`.

The scripts keep their work files in `~/.cache/truthcoin-app-release/` (`TRUTHCOINAPP_RELEASE_CACHE`), outside the
repository. Other settings: `REF` (the commit to release, default `HEAD`), `TRUTHCOINAPP_REMOTE` (default `origin`),
`TRUTHCOINAPP_RELEASE_KEY` (default `~/.ssh/truthcoinapp-release`). `ATTEST=skip` and `PAGES=skip` exist for
rehearsals on a private repository only.

## Verify your download

*(For the main README. Replace `ssh-ed25519 AAAA…` with the line in `release/truthcoinapp-release.pub`.)*

Every release lists each file's SHA-256 in `SHA256SUMS`, signed with the Truthcoin App release key in
`SHA256SUMS.sig`. Download both into the folder with the file you downloaded, then:

1. Check the signature (OpenSSH 8.2 or later; the key is also in this repository, `release/truthcoinapp-release.pub`):

   ```
   echo "truthcoinapp-release ssh-ed25519 AAAA…" > allowed_signers
   ssh-keygen -Y verify -f allowed_signers -I truthcoinapp-release -n truthcoinapp-sums -s SHA256SUMS.sig < SHA256SUMS
   ```

   It must say `Good "file" signature for truthcoinapp-release`.

2. Check your file against the signed list:

   ```
   sha256sum -c --ignore-missing SHA256SUMS
   ```

   On macOS: `grep ' Truthcoin-App_X.Y.Z_universal.dmg$' SHA256SUMS | shasum -a 256 -c`. It must say `OK`.

3. Optionally, check GitHub's record that the file was built by this repository's release workflow
   (GitHub CLI 2.49 or later):

   ```
   gh attestation verify <file> --repo truthcoinapp/truthcoin-app
   ```
