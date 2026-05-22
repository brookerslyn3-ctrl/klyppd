# AUR Submission Guide

This directory contains three PKGBUILDs:

| Package        | What it does                                              | Build time   |
| -------------- | --------------------------------------------------------- | ------------ |
| **`klyppd-bin`** | Downloads a prebuilt `.deb` from GitHub Releases.       | ~10 seconds  |
| **`klyppd`**     | Builds from a tagged GitHub release.                    | 5-15 min     |
| **`klyppd-git`** | Builds from latest `main`.                              | 5-15 min     |

> **Recommend `klyppd-bin` to most users** — it's nearly instant. The other two are for people who want to compile themselves or track latest commits.

## Prerequisite: cut a GitHub release

The `-bin` package needs prebuilt artifacts. Tag a release in the main repo:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The `.github/workflows/release.yml` workflow runs on tag push, builds the `.deb` and `.AppImage`, and attaches them to the GitHub release. Wait for it to finish (~5 min on GitHub Actions) before publishing the AUR package.

The expected artifact name is `klyppd_0.1.0_amd64.deb` (Tauri's default).

## One-time AUR setup

1. Create an account at <https://aur.archlinux.org/register>.
2. Add your SSH public key in **My Account → Edit profile**.
3. Test:
   ```bash
   ssh aur@aur.archlinux.org help
   ```

## Test locally before pushing

```bash
cd packaging/aur/klyppd-bin
updpkgsums                # fills in the real sha256sum
makepkg -si               # builds + installs
namcap PKGBUILD           # lints
namcap *.pkg.tar.zst      # lints the built package
```

## Generate `.SRCINFO`

The AUR requires `.SRCINFO` next to every `PKGBUILD`:

```bash
makepkg --printsrcinfo > .SRCINFO
```

Re-run this every time you change `PKGBUILD`.

## Push to AUR

Each package is its own git repo on `aur.archlinux.org`:

```bash
# klyppd-bin
git clone ssh://aur@aur.archlinux.org/klyppd-bin.git
cp packaging/aur/klyppd-bin/{PKGBUILD,.SRCINFO} klyppd-bin/
cd klyppd-bin
git add PKGBUILD .SRCINFO
git commit -m "Initial release v0.1.0"
git push
```

Repeat the same flow for `klyppd` and `klyppd-git` once you're ready.

## Updating after a new release

1. Bump `pkgver` to the new version, set `pkgrel=1` in `klyppd-bin/PKGBUILD`.
2. `updpkgsums` to refresh the checksum.
3. `makepkg --printsrcinfo > .SRCINFO`.
4. Test: `makepkg -si`.
5. Commit and push to the AUR repo.

For `klyppd-git`, only bump `pkgrel` if the PKGBUILD itself changes — `pkgver` regenerates from git.

## Tools

```bash
sudo pacman -S namcap pacman-contrib
```

- **`namcap`** — lints PKGBUILD and built packages
- **`updpkgsums`** — refreshes source checksums
- **`pacman-contrib`** — provides `checkupdates`, `paccache`, etc.
