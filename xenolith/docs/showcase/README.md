# Xenolith demo

The [video](xenolith-demo.mp4) is 32 seconds, 1080 × 1350, H.264 with stereo AAC audio. It works muted: the story is written on screen. The [SVG card](xenolith.svg) is 1200 × 630; [poster.png](poster.png) is the portrait cover.

The page shown in the video is a real app capture from an isolated instance using [fictional notes](weekend-studio.md). The note cards, connections and review sequence are illustrations, not a screen recording or a claim about processing speed. Manual approval must be selected to review changes before they apply. The soundtrack is original synthesized audio with no external samples. There are no live account details or private notes in the assets.

Suggested caption:

> A little less starting over. Xenolith gives your AI apps a shared place for notes, decisions, and context. You can search it, review changes, and keep the files as Markdown.
>
> github.com/willctl/xenolith

To render the video again, install Python packages `Pillow`, `numpy` and `imageio-ffmpeg`, then run `python scripts/render-showcase.py` from the repository. The default fonts are Segoe UI on Windows; set `SHOWCASE_FONT` and `SHOWCASE_FONT_BOLD` to equivalent installed fonts on another platform. The renderer uses the included capture and does not connect to a wiki or model provider. It also writes five inspection frames; those are not needed for sharing.
