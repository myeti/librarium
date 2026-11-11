# Librarium design

Librarium is a home for books, so it is dressed like one. **Each theme is a
book cover**: its colours, its lettering, its ornaments, the covers it
draws for books that have none, and what moves behind the page. The
interface itself stays quiet and legible (Inter, WCAG AA everywhere); the
character lives in the type, the metal and the ornaments.

This page explains the design and where each part lives. The token tables
are in [SPECS.md › Design System](../SPECS.md#design-system).

## The four themes

Every theme is dark, like the covers it comes from: there is no light
mode.

| Theme | Inspired by | Colours | Lettering (display / book titles) | Ornaments | Background effect |
|---|---|---|---|---|---|
| **Nocturne** (default) | *The Loneliest Girl in the Universe*: a round window on the stars | Midnight blue, coral | Poiret One / Cormorant Garamond | A ring with a coral crescent; a window on the stars (also the app icon, `src/assets/icons/library.svg`) | A twinkling starfield with shooting stars |
| **Saphir** | *The House Saphir*: cobalt filigree, gold keys | Sapphire, gold leaf, ivory | Cinzel / Cormorant Garamond | Baroque scrolls and a diamond; an ornate key | Gold dust and sparkles |
| **Folklore** | *The Beholders*, *Starling House*: folk-art florals | Ink black, vermilion, gold | Fraunces / Fraunces | Vines, leaves and blossoms; an arched window | Drifting embers |
| **Cerulean** | *The Cerulean*: a city above the clouds | Moonlit silver, mist | Italiana / Cormorant Garamond | A moon with a halo; a moon over clouds and a spire | Silver moon dust, drifting, rare shooting stars |

Older theme names are read as their nearest successor (autumn → Folklore,
winter → Cerulean, spring → Nocturne, summer and "seasonal" → Saphir): see
`themeNameFrom` in `src/stores/theme.ts`.

## Where things live

| What | Where |
|---|---|
| Every theme's tokens (colours, fonts, ornaments, cover cloths) | `src/assets/themes.css` |
| Theme-independent tokens, the page layers, utilities (`.display-title`, `.heading-title`, `.foil-text`, `.ornament`) | `src/assets/main.css` |
| The contrast test (fails on any unreadable pair) | `src/assets/themes.test.ts` (parser: `src/test/themeTokens.ts`) |
| Theme choice, migration of old names | `src/stores/theme.ts` |
| Fonts (all bundled, no network) | `src/main.ts` (`@fontsource` packages) |
| Background effects | `src/components/effects/`, `src/utils/effects/` (`starfield.ts` is the night sky) |
| Generated covers | `src/components/library/BookCover.vue`, `src/utils/coverArt.ts` |
| The theme picker (a title page per theme, in its own colours) | `src/components/settings/ThemePicker.vue` |
| The ornament masks' sources | generated once into `themes.css`; to redraw one, edit the SVG in the token (it is a plain data URI) |

## The building blocks

### Type

- **Display** (`--font-display`, `.display-title`): the wordmark, view
  titles, the user picker. Set in capitals, tracked wide
  (`--display-tracking`). Some faces are hairline-thin (Poiret One,
  Italiana): only at 22px and up.
- **Heading** (`--font-heading`, `.heading-title`): dialog titles, Settings
  sections, the user picker's question. A sturdier face than the display
  one where the display face is thin.
- **Serif** (`--font-serif`): book titles, the book details' author and
  description. Cormorant has a small x-height, so titles are multiplied by
  `--serif-scale` (1.14; 1 for Fraunces).
- **Sans** (`--font-sans`, Inter): everything else. The interface never
  depends on a decorative face to be understood.

### Metal

`--foil` is the theme's metal, a gradient: gold leaf (Saphir, Folklore),
coral copper (Nocturne), silver (Cerulean). It paints the wordmark
(`.foil-text`, whose highlight drifts slowly), the ornaments and the
progress bars. It is decorative only: text
in the accent colour uses `--accent-text`, which is contrast-tested.

`--hairline` is the gilt 1px line around panels, dialogs and menus;
`--border` the quieter line between rows.

### Ornaments

Three shapes per theme, drawn as SVG **masks** in `themes.css`:

- `--ornament-rule`: a divider (10:1), under the wordmark and titles;
- `--ornament-corner`: the top-left corner of a frame (1:1), mirrored for
  the other corners (dialogs, generated covers, the theme picker);
- `--cover-art`: the theme's emblem (3:4), on generated covers and behind
  empty states.

Use them with `<span class="ornament -rule" aria-hidden="true">`: the
shape comes from the theme, the paint from `--ornament-paint` (the foil by
default). Size one dimension; the aspect ratio gives the other. They are
always `aria-hidden`.

### The user picker

The first screen is a title page in the theme's dress: its cover emblem
(the same `--cover-art` as on generated covers) glowing in its metal, the
wordmark between two dividers, a tagline; under it, "Who's reading?" and
the users on one row, "Add user" last. Each user is a medallion in one of
the theme's cover cloths (`avatarColor()`), lettered in the
cover ink and the heading face, in a gilt ring. The cloths are dealt at
random at each launch from a shuffled deck, reshuffled when it runs out:
any six users get six different colours.

### The page

Back to front: the theme's `--sky` (a vertical gradient), two coloured
glows (`--body-overlay-1/2`), the `--motif` (two sparse tiles of 701 and
437 px, so the repeat never lines up into a grid) and a paper grain
(`body::before`, `--grain-opacity`). Panels are glass over it (`--glass`,
`backdrop-filter`).

### Books

A book in the grid is an object, not a tile: no card box, a square spine
with the hinge's shading, a slightly rounded fore-edge, a layered shadow.
Hovering lifts it with a little overshoot (`--ease-lift`) and runs a glint
across it once (`--duration-sheen`). A selected book gets the accent tint
behind it; the current one (keyboard focus, last shown in the details) an accent
ring.

A book without a cover gets a **generated cover**: one of the theme's six
cloths (`--cover-1` … `--cover-6`, picked by `coverColor()`, the same for
every book of a series), a gilt frame and corners, the series and the title
in the theme's lettering, its divider and emblem, the author. The lettering
is sized in container units, so a wide card and a narrow one look like the
same book. It is a picture (`aria-hidden`): the card writes the same text
below it at full size. A file only on the e-reader gets one too.

Two marks sit on a cover, only when the book is on the e-reader: where it
is, a small glass pill in the bottom-left corner (a book and a tablet: in
the library and on the device; a tablet alone: only on the device), and
how far it has been read, a "42%" plate in the theme's accent in the
bottom-right corner. A book only in the library carries no mark: it is the
usual case. A favorite carries its star in the top-right corner: the star
alone, no plate, filled in the accent, with a soft shadow that keeps it
readable on a pale cover (on any other book it shows on hover, outlined
in white).

## Accessibility and motion

- Contrast is measured, not eyeballed: `themes.test.ts` checks every text
  token on every surface it is drawn on, including the sky and the glows,
  the cover inks on every cloth, the primary button's label at rest and on
  hover, badges and the focus ring. A palette edit that breaks a pair fails
  the test.
- `prefers-reduced-motion` turns off the lifts, glints, the foil's drift
  and the hero's entrance; the durations drop to 1 ms. Background effects
  are Off by default then (Settings › Appearance).
- Effects pause while the window is hidden and draw at 30 fps at the
  Subtle level.

## Adding or changing a theme

1. Add its two blocks in `themes.css` (type, ornaments and covers; then
   the palette): copy a theme's, every token must be defined, the test
   lists them.
2. Add the name to `THEME_NAMES` (and `RETIRED_THEMES` if it replaces
   one), its label and tagline in both translation files, and its effect
   in `BackgroundEffects.vue`.
3. Add its `data-theme` value to `THEMES` in `src/test/themeTokens.ts`.
4. Run `npx vitest run src/assets/themes.test.ts`: it tells you which pair
   is unreadable and by how much.
