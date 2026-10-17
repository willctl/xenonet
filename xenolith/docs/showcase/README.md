# Xenolith demo

- [xenolith-demo.mp4](xenolith-demo.mp4): the video, 32 seconds, 1080 × 1350 at 24 fps, H.264 with stereo AAC audio. It works muted: the story is written on screen.
- [poster.png](poster.png): the portrait cover, 1080 × 1350, the video's frame at 3 seconds.
- [xenolith.svg](xenolith.svg): the card, 1200 × 630, drawn by hand.
- [demo-page.png](demo-page.png): the window capture used in the video, 449 × 884, dark theme.
- [weekend-studio.md](weekend-studio.md): the fictional page in that capture.

The page shown in the video is a real capture of the Xenolith window from an isolated instance that holds only the [fictional notes](weekend-studio.md). The note cards, connections and review sequence are illustrations, not a screen recording or a claim about processing speed. Manual approval must be selected to review changes before they apply. The soundtrack is original synthesized audio with no external samples. There are no live account details or private notes in the assets.

## Rendering

From the `xenolith` folder:

1. When the window has changed, capture it again. Build the window and the server (`npm run build`, then `cargo build --release` in `rust/`), and run the capture with the Electron from XenoNet's dev dependencies:

   ```sh
   ../node_modules/.bin/electron scripts/capture-window.mjs
   ```

   It starts a throwaway instance on a free port, with `XENOLITH_HOME` and `XENOLITH_DIR` in a temp folder that holds only `weekend-studio.md`, captures the page in a hidden, muted window and writes `demo-page.png`. It never reads your wiki. `--bin`, `--out` and `--tmp` change the server program, the output file and the temp folder.

2. Render the video and the poster. Install the Python packages `Pillow`, `numpy` and `imageio-ffmpeg` (a virtual environment is enough), then run:

   ```sh
   python scripts/render-showcase.py
   ```

   It writes `xenolith-demo.mp4`, `poster.png` and five inspection frames, `frame-*.png`, one per scene. The frames are ignored by git and aren't needed for sharing. The renderer uses the included capture and doesn't connect to a wiki or a model provider.

Fonts: Segoe UI on Windows, Avenir Next (Medium and Bold) on macOS. On other systems, or to use other fonts, set `SHOWCASE_FONT` and `SHOWCASE_FONT_BOLD` to a regular and a bold font file. For a collection (`.ttc`), add the face's style or index after a `#`:

```sh
SHOWCASE_FONT="/System/Library/Fonts/HelveticaNeue.ttc#Regular" \
SHOWCASE_FONT_BOLD="/System/Library/Fonts/HelveticaNeue.ttc#Bold" \
python scripts/render-showcase.py
```

Text widths differ a little between fonts, so look at the inspection frames after a font change.
