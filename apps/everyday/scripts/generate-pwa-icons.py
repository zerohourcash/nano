"""Генерирует PNG-иконки Everyday из геометрии существующего SVG-знака."""

from pathlib import Path
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "public" / "icons"
SCALE = 4

BG = "#EDEDF7"
BRAND = "#5E629B"
TEAL = "#66C6BE"
RESAMPLE = getattr(Image, "Resampling", Image).LANCZOS


def render(size: int, maskable: bool = False) -> Image.Image:
    canvas = size * SCALE
    image = Image.new("RGB", (canvas, canvas), BG)
    draw = ImageDraw.Draw(image)

    # Maskable-иконка держит весь значимый знак внутри безопасных центральных 60%.
    extent = 0.58 if maskable else 0.76
    unit = canvas * extent / 64
    offset = (canvas - 64 * unit) / 2

    def point(x: float, y: float) -> tuple[int, int]:
        return (round(offset + x * unit), round(offset + y * unit))

    nodes = [(32, 10), (51, 21), (51, 43), (32, 54), (13, 43), (13, 21)]
    center = point(32, 32)
    width = max(2, round(1.7 * unit))
    inner_width = max(2, round(1.25 * unit))

    for index, current in enumerate(nodes):
        following = nodes[(index + 1) % len(nodes)]
        color = BRAND if index % 2 == 0 else TEAL
        draw.line([point(*current), point(*following)], fill=color, width=width)
        draw.line([point(*current), center], fill=color, width=inner_width)

    radius = max(3, round(3.2 * unit))
    for index, node in enumerate(nodes):
        x, y = point(*node)
        color = BRAND if index % 2 == 0 else TEAL
        draw.ellipse((x - radius, y - radius, x + radius, y + radius), fill=color)

    # Центральный ключ: округлая рукоять, отверстие и раскрытая головка.
    handle_width = max(5, round(5.2 * unit))
    draw.line([point(22, 40), point(32, 30)], fill=BRAND, width=handle_width)
    hx, hy = point(22, 40)
    hole = max(2, round(1.25 * unit))
    draw.ellipse((hx - hole, hy - hole, hx + hole, hy + hole), fill=BG)
    x1, y1 = point(26, 22)
    x2, y2 = point(40, 36)
    draw.ellipse((x1, y1, x2, y2), fill=BRAND)
    cx1, cy1 = point(32, 21)
    cx2, cy2 = point(41, 30)
    draw.ellipse((cx1, cy1, cx2, cy2), fill=BG)

    return image.resize((size, size), RESAMPLE)


def save(name: str, size: int, maskable: bool = False) -> None:
    render(size, maskable).save(OUT / name, format="PNG", optimize=True)


OUT.mkdir(parents=True, exist_ok=True)
save("everyday-192.png", 192)
save("everyday-512.png", 512)
save("everyday-maskable-192.png", 192, True)
save("everyday-maskable-512.png", 512, True)
save("apple-touch-icon.png", 180, True)
print(f"PWA icons generated in {OUT}")
