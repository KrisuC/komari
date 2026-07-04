# Trimming Lie Detector Videos

The `scripts/trim_lie_detector.py` script crops full gameplay recordings down to just the lie detector shape region, so they can be used with the "Test transparent shape..." debug feature.

## How it works

1. Crops a **758×505** centered region from the source video (the lie detector detection area)
2. Downsamples from 60 FPS to **30 FPS** (matching the solver's tick rate)
3. Trims the first **10 seconds** (the lie detector typically appears after ~10s)

## Usage

```bash
python scripts/trim_lie_detector.py <input.mp4> [-o output.mp4] [-t seconds] [--crop-x X] [--crop-y Y] [--crop-w W] [--crop-h H]
```

### Options

| Option | Default | Description |
|---|---|---|
| `-o, --output` | `<input>_processed.mp4` | Output file path |
| `-t, --trim` | `10` | Seconds to skip from the start |
| `--crop-x` | centered (309) | Left edge of crop region |
| `--crop-y` | centered (113) | Top edge of crop region |
| `--crop-w` | `758` | Crop width in pixels |
| `--crop-h` | `505` | Crop height in pixels |

### Examples

```bash
# Single video with defaults
python scripts/trim_lie_detector.py "C:\Users\Krisu\Videos\LieDetector_trimmed\video.mp4"

# Custom output path
python scripts/trim_lie_detector.py video.mp4 -o processed.mp4

# Longer trim (lie detector appears later)
python scripts/trim_lie_detector.py video.mp4 -t 12

# Fine-tune crop position
python scripts/trim_lie_detector.py video.mp4 --crop-x 310 --crop-y 115

# Batch process all videos in a folder
for f in "C:\Users\Krisu\Videos\LieDetector_trimmed"\*.mp4; do
    python scripts/trim_lie_detector.py "$f" -o "target\$(basename "$f" .mp4)_processed.mp4"
done
```

## Testing the output

1. Build the debug version: `dx build --package ui`
2. Launch with `ui_debug.bat`
3. Go to the **Debug** tab
4. Click **"Test transparent shape..."**
5. Select a `_processed.mp4` file

A HighGUI window will open showing the solver tracking shapes in the video.

## Default crop parameters

For 1366×768 gameplay recordings (standard MapleStory resolution), the lie detector detection region is centered at:

| Parameter | Value |
|---|---|
| Crop X | 309 |
| Crop Y | 113 |
| Crop Width | 758 |
| Crop Height | 505 |

These were determined by aligning with the detection region defined in `backend/src/player/solve_shape.rs`. If your recordings use a different resolution, you'll need to adjust `--crop-x` and `--crop-y`.
