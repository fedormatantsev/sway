#!/usr/bin/env python3
"""Generates the "pulse" demo sprite content for add-sprite-material task 3.1.

Produces two equal-length, zero-padded frame folders next to this script:

  pulse_color/000.png .. 029.png   RGBA8,  256x256, sRGB   (colour run)
  pulse_depth/000.png .. 029.png   L8,      64x64,  linear (depth run)

Both runs are evaluated from the *same* closed-form field (a rotating,
breathing 5-lobed blob with two outward-travelling ripple rings), sampled at
each run's own resolution — per design D8/D7 the two runs need not agree in
resolution, only in frame count and ordering. Depth is written as a plain
8-bit greyscale PNG: Bevy's PNG loader (`Image::from_dynamic`,
`DynamicImage::ImageLuma8` branch) expands a greyscale PNG to `Rgba8Unorm`
(or `Rgba8UnormSrgb` if `is_srgb` is left on) with R=G=B=the stored value, so
this is the plain, ordinary-PNG route D8 asks for — no manual RGBA padding
needed, and `FrameSequence.color_space` should be set to a *non*-sRGB /
linear value for the depth folder so the stored byte maps linearly to
displacement.

Sign convention (matches `sprite_depth_spike.rs`::`SPIKE_DEPTH_PIVOT` /
the_depth_range_pushes_each_half_clear_of_the_cube): stored value 0.0 is
nearest the camera, 1.0 is farthest, 0.5 is exactly on the quad's plane.
`offset = (value - pivot) * depth_range`.

Depth stays continuous everywhere alpha is continuous (design Risks: skirt
triangles at depth discontinuities coincide with the alpha edge and are
discarded, so authoring means keeping the *interior* smooth): the ripple
field is a sum of sinusoids with no clamps or steps, and an `edge_env`
term additionally eases the displacement back toward the pivot (0.5,
"on the plane") as the radius approaches the silhouette boundary, so the
handful of anti-aliased edge pixels straddling opaque/transparent carry
only a small residual offset rather than the field's full swing.

Re-run with:  python3 generate_pulse_sprites.py
Requires: pillow, numpy (pip install pillow numpy).
"""

from __future__ import annotations

import math
from pathlib import Path

import numpy as np
from PIL import Image

FRAMES = 30
COLOR_SIZE = 256
DEPTH_SIZE = 64

LOBES = 5
ROT_TURNS = 1.0  # petals complete exactly one rotation over the loop
PULSE_CYCLES = 2.0  # breathing cycles over the loop
RIPPLE_RINGS = 3.0
RIPPLE_TRAVEL = 2.0  # ripple phase cycles over the loop (outward motion)

DEPTH_PIVOT = 0.5
DEPTH_AMPLITUDE = 0.46  # keeps the field inside [~0.02, ~0.98], see report

SCRIPT_DIR = Path(__file__).resolve().parent
COLOR_DIR = SCRIPT_DIR / "pulse_color"
DEPTH_DIR = SCRIPT_DIR / "pulse_depth"


def polar_grid(size: int) -> tuple[np.ndarray, np.ndarray]:
    """uv in [-1, 1] (pixel-centre sampled), returned as (r, theta)."""
    coords = (np.arange(size, dtype=np.float64) + 0.5) / size * 2.0 - 1.0
    u, v = np.meshgrid(coords, coords)
    r = np.sqrt(u * u + v * v)
    theta = np.arctan2(v, u)
    return r, theta


def shape_radius(theta: np.ndarray, t: float) -> np.ndarray:
    """Silhouette radius: 5 rotating, breathing lobes."""
    pulse = 0.06 * math.sin(2.0 * math.pi * PULSE_CYCLES * t)
    return 0.55 + 0.18 * np.cos(LOBES * theta - 2.0 * math.pi * ROT_TURNS * t) + pulse


def depth_field(r: np.ndarray, theta: np.ndarray, t: float, edge: np.ndarray) -> np.ndarray:
    """Normalized depth in [0, 1] before clipping; 0.5 is the quad's plane."""
    ripple = np.sin(2.0 * math.pi * (RIPPLE_RINGS * r - RIPPLE_TRAVEL * t))
    petal = r * np.cos(LOBES * theta - 2.0 * math.pi * ROT_TURNS * t)
    field = 0.6 * ripple + 0.4 * petal
    return DEPTH_PIVOT + DEPTH_AMPLITUDE * field * edge


def edge_envelope(r: np.ndarray, edge_r: np.ndarray) -> np.ndarray:
    """1.0 well inside the silhouette, eases to 0.0 by ~0.12 past the edge,
    so displacement relaxes toward the pivot before the alpha cut discards
    the triangle — keeps any residual skirt at the boundary small."""
    band = 0.12
    dist_inside = edge_r - r  # positive well inside, negative outside
    return np.clip(0.5 + dist_inside / band, 0.0, 1.0)


def hsv_to_rgb(h: np.ndarray, s: np.ndarray, v: np.ndarray) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Vectorized HSV -> RGB, all arrays in [0, 1]."""
    i = np.floor(h * 6.0)
    f = h * 6.0 - i
    p = v * (1.0 - s)
    q = v * (1.0 - f * s)
    tt = v * (1.0 - (1.0 - f) * s)
    i_mod = (i.astype(np.int64)) % 6

    r = np.select(
        [i_mod == 0, i_mod == 1, i_mod == 2, i_mod == 3, i_mod == 4, i_mod == 5],
        [v, q, p, p, tt, v],
    )
    g = np.select(
        [i_mod == 0, i_mod == 1, i_mod == 2, i_mod == 3, i_mod == 4, i_mod == 5],
        [tt, v, v, q, p, p],
    )
    b = np.select(
        [i_mod == 0, i_mod == 1, i_mod == 2, i_mod == 3, i_mod == 4, i_mod == 5],
        [p, p, tt, v, v, q],
    )
    return r, g, b


def make_color_frame(idx: int, t: float) -> Image.Image:
    r, theta = polar_grid(COLOR_SIZE)
    edge_r = shape_radius(theta, t)
    edge = edge_envelope(r, edge_r)
    depth = np.clip(depth_field(r, theta, t, edge), 0.0, 1.0)

    # Soft alpha cut, ~2 texels of antialiasing at this resolution.
    edge_width = 2.0 / COLOR_SIZE
    alpha = np.clip((edge_r - r) / edge_width + 0.5, 0.0, 1.0)

    hue = (theta / (2.0 * math.pi) + 0.5 * t) % 1.0
    saturation = np.full_like(hue, 0.62)
    # Shading tied to depth: nearer (depth < 0.5) reads brighter.
    shade = 0.55 + 0.9 * (DEPTH_PIVOT - depth)
    value = np.clip(shade, 0.15, 1.0)

    rr, gg, bb = hsv_to_rgb(hue, saturation, value)
    rgba = np.stack(
        [
            np.clip(rr * 255.0, 0, 255),
            np.clip(gg * 255.0, 0, 255),
            np.clip(bb * 255.0, 0, 255),
            np.clip(alpha * 255.0, 0, 255),
        ],
        axis=-1,
    ).astype(np.uint8)
    return Image.fromarray(rgba)


def make_depth_frame(idx: int, t: float) -> Image.Image:
    r, theta = polar_grid(DEPTH_SIZE)
    edge_r = shape_radius(theta, t)
    edge = edge_envelope(r, edge_r)
    depth = np.clip(depth_field(r, theta, t, edge), 0.0, 1.0)
    values = np.clip(depth * 255.0, 0, 255).astype(np.uint8)
    return Image.fromarray(values)


def main() -> None:
    COLOR_DIR.mkdir(parents=True, exist_ok=True)
    DEPTH_DIR.mkdir(parents=True, exist_ok=True)

    depth_min = 1.0
    depth_max = 0.0

    for idx in range(FRAMES):
        t = idx / FRAMES
        name = f"{idx:03d}.png"

        color_img = make_color_frame(idx, t)
        color_img.save(COLOR_DIR / name, optimize=True)

        r, theta = polar_grid(DEPTH_SIZE)
        edge_r = shape_radius(theta, t)
        edge = edge_envelope(r, edge_r)
        raw = np.clip(depth_field(r, theta, t, edge), 0.0, 1.0)
        depth_min = min(depth_min, float(raw.min()))
        depth_max = max(depth_max, float(raw.max()))

        depth_img = make_depth_frame(idx, t)
        depth_img.save(DEPTH_DIR / name, optimize=True)

    print(f"Wrote {FRAMES} frames to {COLOR_DIR} and {DEPTH_DIR}")
    print(f"Normalized depth range across all frames: [{depth_min:.4f}, {depth_max:.4f}]")
    print(f"(pivot = {DEPTH_PIVOT}; offset = (value - pivot) * depth_range)")


if __name__ == "__main__":
    main()
