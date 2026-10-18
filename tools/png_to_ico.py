#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "pillow>=12",
#     "resvg-py>=0.5,<0.6",
# ]
# ///

from __future__ import annotations

import argparse
import struct
from io import BytesIO
from pathlib import Path

import resvg_py
from PIL import Image

DEFAULT_SIZES = (256, 64, 48, 32, 24, 20, 16)
DEFAULT_LEGACY_BMP_MAX_SIZE = 64


def parse_sizes(raw_sizes: list[str]) -> list[tuple[int, int]]:
    parsed: list[tuple[int, int]] = []
    seen: set[int] = set()

    for raw_size in raw_sizes:
        try:
            size = int(raw_size)
        except ValueError as exc:
            raise argparse.ArgumentTypeError(f'Invalid size: {raw_size}') from exc
        if not 1 <= size <= 256:
            raise argparse.ArgumentTypeError(f'Invalid size: {raw_size}')
        if size in seen:
            continue
        seen.add(size)
        parsed.append((size, size))

    if not parsed:
        raise argparse.ArgumentTypeError('At least one icon size is required')

    return parsed


def parse_crop(raw_crop: str) -> int:
    try:
        crop = int(raw_crop)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(
            'Crop must be an integer number of pixels',
        ) from exc

    if crop < 0:
        raise argparse.ArgumentTypeError(
            'Crop must be zero or greater',
        )

    return crop


def load_image(input_image: Path, render_size: int) -> Image.Image:
    """Rasterize SVGs at icon resolution, then load either source as RGBA."""
    if input_image.suffix.lower() == '.svg':
        png_bytes = resvg_py.svg_to_bytes(
            svg_path=str(input_image),
            width=render_size,
            resources_dir=str(input_image.parent),
        )
        with Image.open(BytesIO(png_bytes)) as image:
            return image.convert('RGBA')

    with Image.open(input_image) as image:
        return image.convert('RGBA')


def _parse_ico_entries(ico_bytes: bytes) -> list[tuple[int, int, int, int, int, int, bytes]]:
    """Parse an in-memory ICO produced by Pillow into its raw frame entries.

    Returns a list of tuples:
    (width, height, color_count, planes, bitcount, size, image_bytes)
    Width/height of 0 in the file means 256 and is normalised back to 256.
    """
    _reserved, _itype, count = struct.unpack_from('<HHH', ico_bytes, 0)
    entries = []
    offset = 6
    for _ in range(count):
        w, h, colors, _reserved2, planes, bitcount, size, data_offset = (
            struct.unpack_from('<BBBBHHII', ico_bytes, offset)
        )
        w = w or 256
        h = h or 256
        image_bytes = ico_bytes[data_offset : data_offset + size]
        entries.append((w, h, colors, planes, bitcount, size, image_bytes))
        offset += 16
    return entries


def _write_ico(
    entries: list[tuple[int, int, int, int, int, int, bytes]],
    output_ico: Path,
) -> None:
    """Write a set of pre-encoded frame entries out as a single ICO file."""
    header = struct.pack('<HHH', 0, 1, len(entries))
    dir_entries = b''
    image_blobs = b''
    offset = 6 + 16 * len(entries)

    for w, h, colors, planes, bitcount, size, image_bytes in entries:
        dir_entries += struct.pack(
            '<BBBBHHII',
            w if w < 256 else 0,
            h if h < 256 else 0,
            colors,
            0,
            planes,
            bitcount,
            size,
            offset,
        )
        image_blobs += image_bytes
        offset += size

    output_ico.write_bytes(header + dir_entries + image_blobs)


def save_ico_with_legacy_bmp_fallback(
    image: Image.Image,
    output_ico: Path,
    sizes: list[tuple[int, int]],
    legacy_bmp_max_size: int,
) -> None:
    """Save small sizes as BMP frames and larger sizes as PNG frames.

    The BMP frames retain the source image's 32-bit RGBA data.
    """
    small_sizes = [s for s in sizes if s[0] <= legacy_bmp_max_size]
    large_sizes = [s for s in sizes if s[0] > legacy_bmp_max_size]

    entries: list[tuple[int, int, int, int, int, int, bytes]] = []

    if small_sizes:
        buf = BytesIO()
        image.save(buf, format='ICO', sizes=small_sizes, bitmap_format='bmp')
        entries += _parse_ico_entries(buf.getvalue())

    if large_sizes:
        buf = BytesIO()
        image.save(buf, format='ICO', sizes=large_sizes)
        entries += _parse_ico_entries(buf.getvalue())

    # Largest first is the conventional ordering used by most ICO files.
    entries.sort(key=lambda entry: entry[0], reverse=True)
    _write_ico(entries, output_ico)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description='Convert a transparent PNG or SVG into a multi-size ICO file.',
    )
    parser.add_argument('input_image', type=Path, help='Source PNG or SVG file')
    parser.add_argument(
        'output_ico',
        nargs='?',
        type=Path,
        help='Destination ICO file (defaults to the input name with .ico suffix)',
    )
    parser.add_argument(
        '-s',
        '--sizes',
        nargs='+',
        default=[str(size) for size in DEFAULT_SIZES],
        metavar='PX',
        help='Icon sizes to include, expressed as square pixel values',
    )
    parser.add_argument(
        '-c',
        '--crop',
        type=parse_crop,
        metavar='PX',
        help=(
            'Crop the source image by PX pixels on each side before resizing. '
            'SVGs are rendered at the largest requested width plus 2*PX '
            'before cropping'
        ),
    )
    parser.add_argument(
        '--legacy-bmp',
        action='store_true',
        help=(
            'Encode small icon sizes as BMP frames for compatibility with '
            'older icon renderers. Larger sizes still use PNG. '
            'Produces a bigger file.'
        ),
    )
    parser.add_argument(
        '--legacy-bmp-max-size',
        type=int,
        default=DEFAULT_LEGACY_BMP_MAX_SIZE,
        metavar='PX',
        help=(
            'Sizes at or below this value use the legacy BMP frame when '
            f'--legacy-bmp is set (default: {DEFAULT_LEGACY_BMP_MAX_SIZE})'
        ),
    )
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()

    input_image = args.input_image.expanduser().resolve()
    output_ico = (args.output_ico or input_image.with_suffix('.ico')).expanduser().resolve()
    try:
        sizes = parse_sizes(args.sizes)
    except argparse.ArgumentTypeError as exc:
        parser.error(str(exc))

    if not 1 <= args.legacy_bmp_max_size <= 256:
        parser.error('--legacy-bmp-max-size must be between 1 and 256')

    if not input_image.is_file():
        parser.error(f'Input file not found: {input_image}')

    if input_image.suffix.lower() not in {'.png', '.svg'}:
        parser.error(f'Input file must be a PNG or SVG: {input_image}')

    render_size = max(width for width, _ in sizes) + 2 * (args.crop or 0)
    try:
        image = load_image(input_image, render_size)
    except (OSError, ValueError) as exc:
        parser.error(f'Cannot read input image {input_image}: {exc}')

    output_ico.parent.mkdir(parents=True, exist_ok=True)

    with image:
        if args.crop is not None:
            image_width, image_height = image.size
            if image_width != image_height:
                parser.error(
                    'Crop requires a square input image: '
                    f'got {image_width}x{image_height}',
                )
            if args.crop * 2 >= image_width:
                parser.error(
                    'Crop is too large for the input image: '
                    f'{args.crop}px per side for {image_width}x{image_height}',
                )
            image = image.crop(
                (
                    args.crop,
                    args.crop,
                    image_width - args.crop,
                    image_height - args.crop,
                )
            )
        if args.legacy_bmp:
            save_ico_with_legacy_bmp_fallback(
                image, output_ico, sizes, args.legacy_bmp_max_size
            )
        else:
            image.save(output_ico, format='ICO', sizes=sizes)

    print(f"Created {output_ico} with sizes: {', '.join(f'{w}x{h}' for w, h in sizes)}")
    if args.legacy_bmp:
        print(
            f'  (sizes <= {args.legacy_bmp_max_size}px use legacy raw-BMP '
            'frames; larger sizes use PNG)'
        )
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
