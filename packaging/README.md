# Packaging

`sh packaging/build-packages.sh` builds three Linux packages into `dist/`:

| File | For | Install |
|---|---|---|
| `ironswift_<version>_amd64.deb` | Debian, Ubuntu, Mint, Pop!_OS | `sudo apt install ./ironswift_*.deb` (registers it as an `x-www-browser` choice) |
| `Ironswift-<version>-x86_64.AppImage` | any distribution | `chmod +x Ironswift-*.AppImage && ./Ironswift-*.AppImage` (carries its own DejaVu fonts) |
| `ironswift-<version>-linux-x86_64.tar.gz` | any distribution, no root | unpack, then `sh install.sh` (into `~/.local`) |
| `Ironswift-<version>.flatpak` | any distribution with Flatpak | `flatpak install --user Ironswift-*.flatpak` |

`sh packaging/build-packages.sh deb` (or `appimage`, `tar`) builds just one. `SKIP_BUILD=1`
reuses an existing `target/release/ironswift`. The AppImage step downloads `appimagetool`
the first time unless `APPIMAGETOOL` points at one.

Packages run on systems with the same or a newer glibc than the machine that built them.
Release builds therefore come from `.github/workflows/release.yml` on Ubuntu 22.04 (glibc
2.35), triggered by pushing a version tag:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

That workflow attaches the packages, the bare program (`ironswift-linux-x86_64` and its
`.sha256`, which installed copies use to update themselves) and a `SHA256SUMS` to the GitHub
release. The version in `Cargo.toml` must match the tag. Running the workflow by hand from the
Actions tab builds everything without publishing.

## Flatpak

`flatpak/io.github.hirejohndougan_art.Ironswift.yml` packages the program built above (it
doesn't compile Rust inside Flatpak). To build one locally:

```sh
cargo build --release && mkdir -p dist && cp target/release/ironswift dist/ironswift-linux-x86_64
flatpak-builder --user --install --force-clean build-flatpak packaging/flatpak/io.github.hirejohndougan_art.Ironswift.yml
flatpak run io.github.hirejohndougan_art.Ironswift
```

Flatpak updates the app itself, so Ironswift's own updater stays off inside it. Publishing on
Flathub would need a from-source build (Rust crates vendored with `flatpak-cargo-generator`).

`linux/install.sh` is still the quickest way to install a build straight from a checkout.
