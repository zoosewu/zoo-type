"""Wrap the generated PNG representations in a standard ICNS container.

PNG resource types are documented by Pillow's IcnsImagePlugin:
https://github.com/python-pillow/Pillow/blob/main/src/PIL/IcnsImagePlugin.py
No image conversion or desktop icon service is needed here.
"""
import struct
import sys
from pathlib import Path

source, destination = map(Path, sys.argv[1:])
representations = [
    (b"icp4", 16, 1), (b"ic11", 16, 2),
    (b"icp5", 32, 1), (b"ic12", 32, 2),
    (b"ic07", 128, 1), (b"ic13", 128, 2),
    (b"ic08", 256, 1), (b"ic14", 256, 2),
    (b"ic09", 512, 1), (b"ic10", 512, 2),
]
entries = []
for kind, size, scale in representations:
    suffix = "@2x" if scale == 2 else ""
    png = (source / f"icon_{size}x{size}{suffix}.png").read_bytes()
    if png[:8] != b"\x89PNG\r\n\x1a\n" or struct.unpack(">II", png[16:24]) != (size * scale, size * scale):
        raise SystemExit(f"Invalid PNG dimensions for {kind!r}")
    header = struct.pack(">4sI", kind, 8 + len(png))
    entries.append((header, png))
toc = b"".join(header for header, _ in entries)
payload = struct.pack(">4sI", b"TOC ", 8 + len(toc)) + toc
payload += b"".join(header + png for header, png in entries)
destination.write_bytes(struct.pack(">4sI", b"icns", 8 + len(payload)) + payload)
