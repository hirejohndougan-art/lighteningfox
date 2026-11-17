#!/usr/bin/env python3
"""Make Swiftiron's app icons from the artwork in assets/icon/swiftiron-source.jpg.

    python3 tools/make_icon.py

1. Removes the white background: only white that connects to the edges of the picture counts as
   background, so the light steel highlights inside the bird stay solid.
2. Softens the outline (pixels on the edge get partial transparency, with the white blended out
   of their color) so the bird looks clean on dark and light backgrounds.
3. Crops to the bird, centers it on a square with a little margin, and saves every icon size,
   plus a .ico file for Windows and a preview sheet.
"""
import os
import numpy as np
from PIL import Image, ImageFilter
from scipy import ndimage

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
SRC = os.path.join(ROOT, "assets", "icon", "swiftiron-source.jpg")
OUT = os.path.join(ROOT, "assets", "icon")
SIZES = [16, 24, 32, 48, 64, 128, 256, 512, 1024]

# A pixel is "near white" when its darkest channel is above this (JPEG noise keeps pure white
# from being exactly 255).
WHITE = 236
# Edge pixels: how dark a pixel must be to count as fully the bird.
SOLID = 190


def cut_out(rgb):
    lo = rgb.min(axis=2)
    near_white = lo > WHITE
    # Background = near-white regions that touch the picture's border.
    labels, _ = ndimage.label(near_white)
    border = np.unique(np.concatenate([labels[0], labels[-1], labels[:, 0], labels[:, -1]]))
    background = np.isin(labels, border[border > 0])
    # Fill tiny holes the JPEG noise leaves in the background.
    background = ndimage.binary_closing(background, iterations=2)
    alpha = np.where(background, 0.0, 1.0)
    # A soft edge: the bird's outermost 2 pixels fade with how close to white they are.
    edge = ~background & ndimage.binary_dilation(background, iterations=2)
    fade = np.clip((255.0 - lo) / (255.0 - SOLID), 0.0, 1.0)
    alpha[edge] = fade[edge]
    # Drop specks: faint JPEG noise in the white that isn't part of the bird. Keep every piece at
    # least 1% the size of the biggest one (the bird).
    pieces, count = ndimage.label(alpha > 0.03)
    if count > 1:
        sizes = ndimage.sum(np.ones_like(alpha), pieces, range(1, count + 1))
        keep = np.isin(pieces, 1 + np.nonzero(sizes >= sizes.max() * 0.01)[0])
        alpha[~keep] = 0.0
    # Take the white back out of the edge colors ("un-blend" from a white background).
    a = alpha[..., None]
    color = np.where(a > 0.02, (rgb - 255.0 * (1.0 - a)) / np.maximum(a, 0.02), 0.0)
    color = np.clip(color, 0, 255)
    return np.dstack([color, alpha * 255.0]).astype(np.uint8)


def main():
    rgb = np.asarray(Image.open(SRC).convert("RGB")).astype(float)
    rgba = Image.fromarray(cut_out(rgb), "RGBA")
    # Crop to the bird, then center it on a square with a 4% margin.
    box = rgba.getchannel("A").point(lambda v: 255 if v > 8 else 0).getbbox()
    bird = rgba.crop(box)
    side = int(max(bird.size) * 1.08)
    square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    square.paste(bird, ((side - bird.width) // 2, (side - bird.height) // 2), bird)
    os.makedirs(os.path.join(OUT, "png"), exist_ok=True)
    images = {}
    for n in SIZES:
        im = square.resize((n, n), Image.LANCZOS)
        if n <= 48:
            # Small sizes lose detail when shrunk: a light sharpen keeps the shape readable.
            im = im.filter(ImageFilter.UnsharpMask(radius=0.6, percent=60, threshold=1))
        if n < 1024:
            im.save(os.path.join(OUT, "png", f"swiftiron-{n}.png"), optimize=True)
        images[n] = im
    images[1024].save(os.path.join(OUT, "swiftiron.png"), optimize=True)
    images[256].save(os.path.join(OUT, "swiftiron.ico"), sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])

    # Preview: every size on light and dark backgrounds.
    shown = [16, 24, 32, 48, 64, 128, 256]
    width = sum(shown) + 20 * (len(shown) + 1)
    sheet = Image.new("RGBA", (width, 2 * 296), (0, 0, 0, 0))
    for row, bg in enumerate([(245, 245, 245, 255), (32, 33, 36, 255)]):
        band = Image.new("RGBA", (width, 296), bg)
        x = 20
        for n in shown:
            band.paste(images[n], (x, (296 - n) // 2), images[n])
            x += n + 20
        sheet.paste(band, (0, row * 296))
    sheet.convert("RGB").save(os.path.join(OUT, "preview.png"))
    print("crop box", box, "-> square", side, "px; wrote", ", ".join(str(n) for n in SIZES))


if __name__ == "__main__":
    main()
