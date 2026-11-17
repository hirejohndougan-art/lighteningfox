# Ironswift icon

The iron bird. `ironswift-source.jpg` is the original artwork (1024×1024, white background).
Everything else here is made from it by `python3 tools/make_icon.py`:

| File | Use |
|---|---|
| `ironswift.png` | Full-size icon with a transparent background |
| `png/ironswift-N.png` (16–512) | App icons: the window (32, 64, 128 are built into the program), the applications menu (installed by `packaging/linux/install.sh`), the start page (128) |
| `ironswift.ico` | Windows icon, for a future Windows build |
| `preview.png` | Every size on light and dark backgrounds, for checking |

To change the icon, replace `ironswift-source.jpg` (keep a plain white background) and run the
script again.
