# Recording the product video

The README video runs through XenoNet's console control channel in the isolated
`showcase` workspace.
The title cards and captions are drawn in code with XenoNet's SVG mark.

Install dependencies, put [FFmpeg](https://ffmpeg.org/download.html) on your
PATH, then run:

```sh
node cli/xenonet.js script run scripts/showcase.mjs --instance showcase --hidden
```

The `showcase` instance uses port 47845 and its own app profile. It keeps
running after capture so you can inspect it. Quit only that instance with:

```sh
node cli/xenonet.js console quit --instance showcase
```

The script writes `docs/showcase.mp4`, `docs/showcase-poster.png` and a short
`docs/showcase-preview.gif` for the README. GitHub displays the moving preview
and links to the full video. Raw
clips, chapter frames and capture metadata stay under `.xenonet/showcase`,
which Git ignores. FFmpeg needs the `libx264` encoder. The final video is
1920 by 1080, 30 frames per second, H.264 with a streaming header and
AAC stereo audio captured from XenoNet's own music and sound effects. Captions
also carry the explanation when the viewer has sound off.

The video shows no clock time or date. The script sets the page's time zone
to UTC and checks it. For the recording, the console's clock holds at
`--:--:--`, and dates and times in the panels read as words such as "today",
or a span in minutes. Only what the page shows changes, not XenoNet's data.
The script watches the page's text all through the recording, and fails,
naming what it saw, if a clock or a date showed. The video, poster and
preview carry no metadata: no tags, encoder names or encoder settings.

Start with a fresh showcase instance: quit it using the command above before
recording again. The script turns on its sound and captures the app's final
Web Audio mix after its compressor. It does not capture a microphone or other
applications. Each chapter's audio is aligned to the frame recorder's start
time; only brief fades at the edits are added. Theme changes use the actual
theme music. Title cards carry the app's ambient audio recorded at that point.

The question and edit comparison use closer crops so their text is readable.
The appearance chapters preview Phosphor, Helios and Rime, then change the
environment through Vault, Aurora and Nebula with the same theme. The README
preview includes the theme changes. The end card points to the repository.

Review the question, edit comparison, actor panel and appearance changes at full size before
publishing. All product interactions go through the console driver or its
input actions. Fixed waits give viewers time to read after each action has
settled. Real capture time determines the chapter lengths.
