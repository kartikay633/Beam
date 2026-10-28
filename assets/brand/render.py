"""Renders the Beam logo (direction A, "Link") to the PNGs the apps need.

The mark is two points joined by a beam, drawn in a 100x100 box:
circles of radius 13 at (24, 50) and (76, 50), and a 52x10 bar between them.
`logo.svg` and `mark.svg` beside this file are the same drawing.

    python assets/brand/render.py

writes `assets/brand/icon-1024.png` (the app icon: white mark on a rounded
ink square), which `npx tauri icon` turns into every desktop icon size.
"""

from pathlib import Path

from PIL import Image, ImageDraw

INK = (20, 23, 28, 255)  # #14171C
WHITE = (255, 255, 255, 255)
HERE = Path(__file__).resolve().parent


def mark(draw: ImageDraw.ImageDraw, x0: float, y0: float, size: float, color) -> None:
    """Draws the mark scaled so its 100-unit box is `size` pixels at (x0, y0)."""
    s = size / 100
    for cx in (24, 76):
        draw.ellipse(
            [x0 + (cx - 13) * s, y0 + (50 - 13) * s, x0 + (cx + 13) * s, y0 + (50 + 13) * s],
            fill=color,
        )
    draw.rectangle([x0 + 24 * s, y0 + 45 * s, x0 + 76 * s, y0 + 55 * s], fill=color)


def app_icon(size: int) -> Image.Image:
    # Drawn 4x larger and scaled down, for smooth edges.
    big = size * 4
    image = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle([0, 0, big - 1, big - 1], radius=big * 0.225, fill=INK)
    inset = big * 0.15
    mark(draw, inset, inset, big - 2 * inset, WHITE)
    return image.resize((size, size), Image.LANCZOS)


if __name__ == "__main__":
    app_icon(1024).save(HERE / "icon-1024.png")
    print("wrote", HERE / "icon-1024.png")
