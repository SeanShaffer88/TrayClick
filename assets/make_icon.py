#!/usr/bin/env python3
"""Generate assets/icon.ico for TrayClick.

Design: the SEAN family look — black rounded square, a red click-target
ring, and a white mouse-pointer arrow over it. Run: python3 assets/make_icon.py
"""
from PIL import Image, ImageDraw

S = 256
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# Black rounded-square background with a dark red edge.
d.rounded_rectangle([8, 8, S - 8, S - 8], radius=48, fill=(16, 16, 16, 255),
                    outline=(120, 16, 20, 255), width=6)

# Red click-target: outer ring + centre dot.
cx = cy = S // 2
d.ellipse([cx - 74, cy - 74, cx + 74, cy + 74], outline=(229, 25, 32, 255), width=16)
d.ellipse([cx - 18, cy - 18, cx + 18, cy + 18], fill=(229, 25, 32, 255))

# White pointer arrow sweeping in from the lower right.
arrow = [(150, 128), (226, 204), (186, 208), (200, 246), (178, 254),
         (162, 216), (138, 240)]
d.polygon(arrow, fill=(245, 245, 245, 255), outline=(16, 16, 16, 255))

img.save("assets/icon.ico", sizes=[(16, 16), (32, 32), (48, 48), (256, 256)])
print("wrote assets/icon.ico")
