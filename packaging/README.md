# Packaging

`sh packaging/build-packages.sh` builds three Linux packages into `dist/`:

| File | For | Install |
|---|---|---|
| `ironswift_<version>_amd64.deb` | Debian, Ubuntu, Mint, Pop!_OS | `sudo apt install ./ironswift_*.deb` (registers it as an `x-www-browser` choice) |
| `Ironswift-<version>-x86_64.AppImage` | any distribution | `chmod +x Ironswift-*.AppImage && ./Ironswift-*.AppImage` (carries its own DejaVu fonts) |
| `ironswift-<version>-linux-x86_64.tar.gz` | any distribution, no root | unpack, then `sh install.sh` (into `~/.local`) |

`sh packaging/build-packages.sh deb` (or `appimage`, `tar`) builds just one. `SKIP_BUILD=1`
reuses an existing `target/release/ironswift`. The AppImage step downloads `appimagetool`
the first time unless `APPIMAGETOOL` points at one.

Packages run on systems with the same or a newer glibc than the machine that built them.
Release builds therefore come from `.github/workflows/release.yml` on Ubuntu 22.04 (glibc
2.35), triggered by pushing a version tag:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

That workflow attaches the three files and a `SHA256SUMS` to the GitHub release.

`linux/install.sh` is still the quickest way to install a build straight from a checkout.
