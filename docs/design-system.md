# PreviouslyOn design system

The embedded review UI and the constraints below are the visual source of truth.

## Palette

- Canvas: `#f7f9fc`
- Surface: `#ffffff`
- Strong text: `#171b25`
- Secondary text: `#5d6678`
- Hairline: `#dce2ec`
- Primary: `#0b63f6`
- Primary quiet: `#edf5ff`
- Success: `#159455`
- Success quiet: `#eaf8ef`
- Warning: `#b97800`
- Warning quiet: `#fff6df`
- Danger: `#d92d20`

No gradients, color overlays, or warm/cream substitutions are permitted.

## Typography

- UI/content: `Inter`, `SF Pro Text`, `-apple-system`, `BlinkMacSystemFont`, `Segoe UI`, sans-serif
- Code: `SFMono-Regular`, `Cascadia Code`, `Roboto Mono`, monospace
- Task title: 30px/36px desktop, 24px/30px mobile, weight 650
- Section title: 14px/20px, weight 650
- Body: 14px/21px
- Utility/control: 13px/18px, weight 550
- Caption: 12px/17px

Controls must set their own font metrics; browser defaults are not accepted.

## Layout

- Desktop viewport spec: 1600×1000
- Header: 64px
- Sidebar: 210px
- Evidence inspector: 390px
- Main content gutters: 28px
- Mobile breakpoint: 760px
- Mobile bottom navigation: 72px plus safe area
- Mobile evidence inspector becomes a bottom sheet.

## Geometry

- General radius: 10px
- Compact controls: 8px
- Borders: 1px hairlines
- Shadow: only for elevated inspector/sheet; no decorative card shadows
- Lists and lineage rails are preferred to card grids.

## Signature motif

A thin cobalt lineage rail connects checkpoints. The selected checkpoint continues into the
evidence inspector through a short connector. This is the only decorative structural motif.

## Allowed first-viewport copy

- PreviouslyOn
- Tasks
- Sessions
- Evidence
- Settings
- Refactor authentication boundary
- Review
- Dismiss
- Preview context pack
- Evidence inspector
- Context pack preview

Repository paths, branch names, SHAs, event IDs, dates, and evidence content are data, not fixed
copy.

## Interaction states

- Selected checkpoint: cobalt border and rail node, quiet blue background.
- Fresh/confirmed: semantic green, text label retained.
- Degraded/stale: semantic amber, text label retained.
- Invalid/failing: semantic red, never color-only.
- Focus: 2px cobalt outline with 2px offset.
- Motion: 140–180ms ease-out for row selection and inspector transitions; disabled under
  `prefers-reduced-motion`.
