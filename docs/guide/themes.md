# Themes

A theme sets everything you see and hear in the console:

- the colors of the panels, nodes, lines and alerts
- the fonts
- the sky through the day
- what the stars, the floor, the sweep, the sonar and the lines between nodes are drawn as
- the dust and the glow
- the pace of things
- the music
- how actors look

The terminal, posters, replays and the desktop window take its colors too.

## Choosing the surroundings

Use the **environment** button, or **Environment…** in the palette, to
choose the backdrop independently of the theme. Nebula keeps the original
star field. Aurora draws flowing curtains of light, Trench draws underwater
caustics and shafts, and Vault draws an arcade of illuminated arches.
The three alternatives replace the star field and outer lattice; the map,
floor, particles, music and theme stay yours.

Click a card or use the arrow keys to preview on the map. **Enter** or
**Keep** saves it across windows and restarts. **Esc** or **Cancel** restores
the environment in use. Changing the theme recolors the surroundings and
preserves the selected environment.

## The themes that come with XenoNet

Eight themes come with XenoNet.

| Theme | Look |
| --- | --- |
| `xenonet` | Xeno green on void black |
| `phosphor` | Amber phosphor on black glass, like an old monochrome terminal, in monospace throughout. Its actors are block cursors in a ring of memory cells. |
| `ochre` | Dunes under a bruised sky, in ochre, rust and the blue of a desert night |
| `gridline` | Light trails on an endless grid, in electric cyan and orange on black |
| `helios` | The sun's surface up close, quick and bright. It has a granulated floor, plasma loops circling it, coronal ripples, and lines that rise like prominences. |
| `rime` | Frost on black glass, very slow and quiet. It has falling snow, a hexagonal frost floor, an aurora on the horizon, and hairlines. |
| `trench` | The deep sea at night, buoyant. It has marine snow drifting down, a ping and its echo, and bioluminescent filaments. |
| `eclipse` | Black and white, with color only where something needs you, in Okabe and Ito's colors, which color-blind eyes tell apart. It has the highest contrast and is the calmest, with no sweep, no twinkle and the slowest pace, for focus and for sharing a screen. |

## Choosing a theme

Open the palette with `Ctrl+K`, or `⌘K` on a Mac, and choose `Theme…`. It lists every theme, yours too, and typing narrows the list.

As you move through the list with the up and down arrow keys, XenoNet draws each theme on the real map. This happens in this window only. Nothing is saved, and no other window changes. `Enter` keeps the theme shown, and every XenoNet window follows. `Esc` puts yours back.

To switch straight to a theme, use any of these:

- a `Theme:` command in the palette, such as `Theme: Phosphor`
- the Theme menu in the tray or the menu bar, with the theme in use checked
- `xenonet theme <name>` in a terminal

On a Mac, the View menu has the Theme menu too.

Either way, XenoNet draws the new theme in place. Nothing reloads, so what you were looking at stays as it was.

## The gallery

`Theme gallery` in the palette opens a card for every theme beside the map. Each card is in its theme's own colors, and shows:

- the sky through the day, with the theme's actors drawn over it
- swatches
- a line of type in the theme's fonts
- the theme's mood

Point at a card, or move to it with the arrow keys, and XenoNet draws that theme on the map in this window. Click the card or press `Enter` to keep it. `Esc` puts yours back.

## Making your own

You can start a theme of your own in any of these ways:

- `Make your own`, the gallery's last card, copies the theme you're looking at.
- `Make a theme of your own` in the palette copies the theme in use.
- `xenonet theme copy <theme> <name>` copies the theme you name.

The copy is `themes/<name>.json` in the [state folder](how-it-works.md#where-xenonet-keeps-files), with every field written out, and XenoNet puts it in use. Edit it, and XenoNet follows each save. The gallery can show the file in Finder or Explorer.

A theme of your own names the theme it `extends`, and needs only the fields it changes. For example:

```json
{
  "name": "Ember",
  "extends": "helios",
  "colors": { "accent": "#ff2200", "codex": "#7affd4" },
  "grid": { "style": "hex", "opacity": 0.6 },
  "sweep": { "shape": "flare" },
  "lines": { "style": "arc", "curvature": 0.3 },
  "glow": { "bloom": 1.4 },
  "sound": { "bpm": 58 }
}
```

## The fields

`core/themes.js` has XenoNet's own value for every field, and each built-in theme's values.

- `colors` sets colors by role, such as `bg`, `surface`, `text`, `muted`, `dim`, `accent`, `claude`, `codex`, `grok`, `attention`, `danger` and `collision`. `actor` and `human` are for actors.
- `categories` holds the colors of tool families, and `pages` the colors of Xenolith's page types.
- `identities` holds five colors for the rings that named identities' projects stand on. If your `colors` bring a ring close to a color with a meaning, such as Codex's or a question's, that ring moves over to a spare color.
- `fonts` has `display`, `body` and `mono`. `display` is for titles and the map's labels. `body` is for the panels' text, and is the display font when left out. Each is a list of font names, quoted or not, such as `"Avenir Next", "Segoe UI", sans-serif`. A list may hold letters, digits, spaces, quotes, commas and `. _ + & -`. A list that holds anything else, such as `; : { } ( ) < > \ /`, keeps the fonts underneath it.
- `sky` has `night`, `dawn`, `day` and `dusk`, with four colors each.
- `stars` has `count`, `tints`, `twinkle`, `shape` and `drift`. `twinkle` runs from 0 to 1. `drift` sets how flakes and motes drift, in units a second, such as `[0, -4, 0]` for snow. `shape` is one of these:
  - `point`
  - `flake`, falling snow
  - `mote`, drifting motes
- `grid` is the floor. Its `style` is `polar`, `hex`, `cells` or `none`. It also has `center` and `lines` colors, and an `opacity`.
- `sweep` is what circles the floor. Its `shape` is `beam`, `aurora`, `flare` or `none`. `opacity` sets how strongly it shows. XenoNet's faint beam is 0.03, and the other shapes draw brighter at the same value. It also has a `speed`.
- `sonar` is the wave the core sends out. Its `style` is `ring`, `ripple` or `ping`. For now, `tick` is drawn as the ring. The core sends a wave `every` so many seconds, never under one. At 0 or less, it sends none.
- `lines` are the lines between nodes. Their `style` is `solid`, `arc`, `filament` or `hairline`. At a `curvature` of 0 they're straight, though an arc always bows a little. A `width` of 1 is XenoNet's own.
- `motion` has `pace` and `breathe`. A `pace` of 1 is XenoNet's own. `breathe` runs from 0 to 1. At 0.1, sessions bob as they always have. Past 0.1, every node's glow swells and sinks.
- `lattice` is the lattice around the map, and `dust` the dust in the air. `glow` sets bloom, exposure and fog.
- `charts` holds the stats panel's colors.
- `sound` has `chords`, `scale`, `bpm`, the `pad` and `lead` voices, and the `drone`.
- `actors` sets how actors look on their layer above the map:
  - `form` is `orb`, `cursor`, `crystal`, `flame`, `swarm` or `jelly`, with its `params`.
  - `palette` holds the colors that the other actors take theirs from. You take `colors.human`.
  - `human` holds your own form.
  - `glow`, `breathe`, `beam` and `react` set how actors glow, breathe, beam into what they act on, and react. A `beam`'s `style` is `pulse`, `stream` or `bolt`.

## When a value is missing or wrong

Every field is optional. What a theme leaves out comes from the theme it extends.

A value that doesn't fit also keeps the value underneath it. That covers a color that isn't a hex color like `#ff2200`, and a style XenoNet doesn't know. So a slip in a file costs only that one value.

A file named for a built-in theme, such as `phosphor.json`, takes that theme's place. It starts from the built-in theme, unless it `extends` another.

## Contrast

The built-in themes keep text readable on the page and on the panels. They hold a WCAG contrast of 4.5:1 for text, labels and muted text, and 3:1 for dim text and alerts. Their chart colors stand out on the stats panel. Tests hold the built-in themes to both rules.
