"""
Trim and crop a full gameplay video to just the lie detector shape region.

The lie detector detection region (from solve_shape.rs) is 755x505 pixels,
starting 20px below the detected title bar. This script crops a centered
755x505 region and trims the first N seconds (default 10) before the
lie detector appears.
"""

import cv2
import argparse
import os
import sys
from pathlib import Path


def process_video(
    input_path: str,
    output_path: str,
    crop_x: int | None = None,
    crop_y: int | None = None,
    crop_w: int = 755,
    crop_h: int = 505,
    trim_seconds: float = 10.0,
):
    """Crop and trim a video for lie detector testing.

    Args:
        input_path: Path to input video.
        output_path: Path to output video.
        crop_x: X offset of crop region (default: centered).
        crop_y: Y offset of crop region (default: centered).
        crop_w: Width of crop region (default: 755).
        crop_h: Height of crop region (default: 505).
        trim_seconds: Seconds to skip from the start.
    """
    cap = cv2.VideoCapture(input_path)
    if not cap.isOpened():
        print(f"ERROR: Cannot open {input_path}")
        sys.exit(1)

    src_w = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
    src_h = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
    fps = cap.get(cv2.CAP_PROP_FPS)
    total_frames = int(cap.get(cv2.CAP_PROP_FRAME_COUNT))

    # Default crop: center the 755x505 region
    if crop_x is None:
        crop_x = (src_w - crop_w) // 2
    if crop_y is None:
        # Estimate: lie detector popup is centered vertically
        # Title bar ~20px, then 505px detection area = ~525px total
        full_popup_h = crop_h + 20
        title_y = (src_h - full_popup_h) // 2
        crop_y = title_y + 20

    # Calculate skip frames
    skip_frames = int(trim_seconds * fps)

    print(f"Input:  {src_w}x{src_h} @ {fps:.2f} FPS, {total_frames} frames")
    print(f"Crop:   x={crop_x}, y={crop_y}, w={crop_w}, h={crop_h}")
    print(f"Trim:   {trim_seconds}s ({skip_frames} frames)")
    print(f"Output: {crop_w}x{crop_h}")

    # Validate crop region
    if crop_x + crop_w > src_w or crop_y + crop_h > src_h:
        print(
            f"ERROR: Crop region ({crop_x},{crop_y},{crop_w},{crop_h}) "
            f"exceeds video size ({src_w}x{src_h})"
        )
        cap.release()
        sys.exit(1)

    if skip_frames >= total_frames:
        print(
            f"ERROR: Trim ({skip_frames} frames) exceeds "
            f"video length ({total_frames} frames)"
        )
        cap.release()
        sys.exit(1)

    # Use a standard FPS matching the original test videos (30 FPS).
    # The solver runs at a fixed tick rate regardless of video FPS,
    # but matching 30 FPS gives correct playback speed.
    target_fps = 30.0
    frame_step = max(1, round(fps / target_fps)) if fps > target_fps else 1
    effective_fps = fps / frame_step

    # Ensure even dimensions for codec compatibility
    out_w = crop_w if crop_w % 2 == 0 else crop_w - 1
    out_h = crop_h if crop_h % 2 == 0 else crop_h - 1

    fourcc = cv2.VideoWriter_fourcc(*"mp4v")
    out = cv2.VideoWriter(output_path, fourcc, effective_fps, (out_w, out_h))

    # Skip to start frame
    cap.set(cv2.CAP_PROP_POS_FRAMES, skip_frames)

    frame_count = 0
    frame_idx = 0
    while True:
        ret, frame = cap.read()
        if not ret:
            break

        if frame_idx % frame_step == 0:
            cropped = frame[crop_y : crop_y + crop_h, crop_x : crop_x + out_w]
            out.write(cropped)
            frame_count += 1
        frame_idx += 1

    cap.release()
    out.release()

    output_size = os.path.getsize(output_path) / (1024 * 1024)
    print(f"Done: {frame_count} frames @ {effective_fps:.0f} FPS -> {output_path} ({output_size:.1f} MB)")


def main():
    parser = argparse.ArgumentParser(
        description="Trim and crop gameplay video to lie detector region"
    )
    parser.add_argument("input", help="Path to input video")
    parser.add_argument(
        "-o", "--output", default=None, help="Output path (default: input_processed.mp4)"
    )
    parser.add_argument("--crop-x", type=int, default=None, help="Crop X offset")
    parser.add_argument("--crop-y", type=int, default=None, help="Crop Y offset")
    parser.add_argument(
        "--crop-w", type=int, default=755, help="Crop width (default: 755)"
    )
    parser.add_argument(
        "--crop-h", type=int, default=505, help="Crop height (default: 505)"
    )
    parser.add_argument(
        "-t",
        "--trim",
        type=float,
        default=10.0,
        help="Seconds to trim from start (default: 10)",
    )

    args = parser.parse_args()

    if args.output is None:
        p = Path(args.input)
        args.output = str(p.parent / f"{p.stem}_processed{p.suffix}")

    process_video(
        args.input,
        args.output,
        crop_x=args.crop_x,
        crop_y=args.crop_y,
        crop_w=args.crop_w,
        crop_h=args.crop_h,
        trim_seconds=args.trim,
    )


if __name__ == "__main__":
    main()
