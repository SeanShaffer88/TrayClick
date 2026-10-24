#!/usr/bin/env python3
"""Generate the TrayClick tray icons.

- assets/icon.ico — idle: black rounded square, red click-target ring,
  white mouse-pointer arrow (the SEAN family look).
- assets/icon_active.ico — clicking ON: the same target inverted to a
  red field with a black ring. The tray code blinks between the two
  while auto-clicking so the state is visible at a glance.

Run: python3 assets/make_icon.py
"""
from PIL import Image, ImageDraw

S = 256
SIZES = [(16, 16), (32, 32), (48, 48), (256, 256)]
ARROW = [(150, 128), (226, 204), (186, 208), (200, 246), (178, 254),
         (162, 216), (138, 240)]


def draw(bg, edge, ring, dot):
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([8, 8, S - 8, S - 8], radius=48, fill=bg, outline=edge, width=6)
    cx = cy = S // 2
    d.ellipse([cx - 74, cy - 74, cx + 74, cy + 74], outline=ring, width=16)
    d.ellipse([cx - 18, cy - 18, cx + 18, cy + 18], fill=dot)
    d.polygon(ARROW, fill=(245, 245, 245, 255), outline=(16, 16, 16, 255))
    return img


idle = draw((16, 16, 16, 255), (120, 16, 20, 255), (229, 25, 32, 255), (229, 25, 32, 255))
idle.save("assets/icon.ico", sizes=SIZES)
print("wrote assets/icon.ico")

active = draw((229, 25, 32, 255), (255, 120, 124, 255), (16, 16, 16, 255), (16, 16, 16, 255))
active.save("assets/icon_active.ico", sizes=SIZES)
print("wrote assets/icon_active.ico")
