# Design system

The visual and interaction rules for the Product Mission Control desktop app. The design direction is called Executive Calm: an interface that helps a product leader judge evidence and exceptions without looking like an alarm wall.

The shipped tokens are in `apps/desktop/src/design-system/tokens.css`. Where this document and that file disagree, the file is right. Components consume tokens only: no component stylesheet outside `tokens.css` declares a raw colour value.

## Principles

- Calm and authoritative. Hierarchy and copy point to the next action, not animation or saturation.
- Plain language. Short, direct sentences; the conclusion comes before the supporting detail.
- Restraint. Typography, spacing and finish support the content instead of competing with it.
- Evidence-aware. Status, freshness, ownership and source are visible wherever they affect trust.
- Governed by default. Sensitive, destructive and approval actions always show their scope and consequence.

The app is Windows-first. It uses the platform's own window controls and keyboard conventions and does not imitate another operating system.

## Information architecture

Seven primary destinations, in this order: Executive Cockpit, Portfolio, Work Queue, Reviews & Reports, Product Vault, People, Settings. System Health is a separate destination below them; today it is a placeholder screen. The registry is `apps/desktop/src/shell/routes.ts`. Every route sets the window title to `<route name> – Product Mission Control`.

The navigation rail is 82 px wide with icons and expands to 232 px, showing labels, on hover and on keyboard focus. The selected destination is marked by more than colour. Detail opens in the main region or in an inspector panel, not in stacked modals.

- Executive Cockpit is exception-oriented. It opens on the Portfolio Lens, which places each Product by milestone timing and outcome observability. It does not rank Products or estimate confidence. The Lens has a table with the same data.
- Portfolio lists Products by name and says that the order is not a priority.
- Work Queue shows owner, commitment, due date, attention reason and the next step the lifecycle allows. Action Requests never appear as accepted Actions, and Decision Requests never appear as resolved Decisions.
- Reviews & Reports shows the flagged work in Cockpit order. Fact Packs and approved reports are not built.

Red, yellow and green are not shown unless each state has an explicit threshold, a text label and an Evidence basis.

## Layout and density

- The default window is 1280 x 800 and the minimum is 1024 x 680 (`apps/desktop/src-tauri/tauri.conf.json`). The design target is 1920 x 1080, and 1366 x 768 must work without hidden primary actions or horizontal page scrolling.
- Monitoring views use medium density; reading views use generous line length and spacing.
- Avoid walls of equal-weight cards. Use section rhythm, aligned lists and one dominant surface.
- Use one-pixel borders for structure before adding elevation. Only floating menus, dialogs and inspectors need obvious elevation.

## Colour

A token name describes a role, not a pigment. Text links, filled actions, decorative separators and control boundaries each have their own token.

Contrast is checked against every background a token may appear on: normal text at least 4.5:1, large text at least 3:1, meaningful non-text controls and focus indicators at least 3:1. `border-subtle` may fall below 3:1 only where it does not identify a control, state or boundary.

### Surface, text and action

| Token                          | Light                 | Dark                 | Role                                                 |
| ------------------------------ | --------------------- | -------------------- | ---------------------------------------------------- |
| `--pmc-color-bg`               | `#f5f5f7`             | `#0e0e10`            | Application background                               |
| `--pmc-color-surface`          | `#ffffff`             | `#1c1c1e`            | Primary working surface                              |
| `--pmc-color-surface-muted`    | `#f0f0f2`             | `#28282b`            | Secondary grouping                                   |
| `--pmc-color-surface-selected` | `#e8f2fc`             | `#173a5e`            | Selected row or destination, always with a text cue  |
| `--pmc-color-overlay`          | `rgba(0, 0, 0, 0.36)` | `rgba(0, 0, 0, 0.6)` | Dialog scrim                                         |
| `--pmc-color-text`             | `#1d1d1f`             | `#f5f5f7`            | Primary text                                         |
| `--pmc-color-text-muted`       | `#6e6e73`             | `#aeaeb2`            | Secondary text                                       |
| `--pmc-color-text-disabled`    | `#8e8e93`             | `#8e8e93`            | Disabled text only, never required information       |
| `--pmc-color-link`             | `#0066cc`             | `#64b5ff`            | Text links                                           |
| `--pmc-color-primary-fill`     | `#0066cc`             | `#0066cc`            | Primary filled action                                |
| `--pmc-color-on-primary`       | `#ffffff`             | `#ffffff`            | Text and icons on the primary fill                   |
| `--pmc-color-primary-boundary` | `transparent`         | `#f5f5f7`            | Outline around a primary fill (see placement below)  |
| `--pmc-color-primary-hover`    | `#005cb8`             | `#1073c7`            | Hovered primary action                               |
| `--pmc-color-primary-pressed`  | `#004f9e`             | `#005cb8`            | Pressed primary action                               |
| `--pmc-color-focus`            | `#005fcc`             | `#64b5ff`            | Keyboard focus ring                                  |
| `--pmc-color-border-subtle`    | `#d2d2d7`             | `#3a3a3c`            | Decorative dividers                                  |
| `--pmc-color-border-control`   | `#86868b`             | `#848489`            | Control boundaries, at least 3:1 on allowed surfaces |

Primary action placement: in the light theme the primary fill may sit on any declared surface. In the dark theme it may sit directly only on `bg` or `surface`; on `surface-muted` or `surface-selected` the component draws the one-pixel `primary-boundary` around the fill in every state. Lightening the blue instead is not allowed, because it would drop white text below 4.5:1. The boundary does not replace the focus ring.

### Status and classification

Status tokens come in `fg`, `bg` and `border` triplets for `success`, `warning`, `danger` and `info`: `--pmc-color-status-{status}-{fg|bg|border}`. Data classification has its own triplets, `--pmc-color-classification-{level}-{fg|bg|border}`, for `public`, `internal`, `confidential`, `restricted` and `unclassified`. A classification is never inferred from a status colour.

| Name                      | Light fg / bg / border        | Dark fg / bg / border         |
| ------------------------- | ----------------------------- | ----------------------------- |
| `success`, `public`       | `#176b3a / #e8f5ec / #176b3a` | `#7ddda6 / #173a29 / #7ddda6` |
| `warning`, `confidential` | `#6f4b00 / #fff3d6 / #6f4b00` | `#ffd27a / #493500 / #ffd27a` |
| `danger`, `restricted`    | `#a9231d / #fdecea / #a9231d` | `#ff9b95 / #4a211f / #ff9b95` |
| `info`, `internal`        | `#245b88 / #eaf3fa / #245b88` | `#9dccf2 / #19364d / #9dccf2` |
| `unclassified`            | `#4a4a4f / #efeff1 / #4a4a4f` | `#d1d1d6 / #323235 / #d1d1d6` |

Every badge spells out its state or classification in words.

### Charts

| Token                         | Light     | Dark      |
| ----------------------------- | --------- | --------- |
| `--pmc-color-chart-1`         | `#0066cc` | `#64b5ff` |
| `--pmc-color-chart-2`         | `#8a4f9e` | `#d5a6e6` |
| `--pmc-color-chart-3`         | `#247a4b` | `#7ddda6` |
| `--pmc-color-chart-4`         | `#9a5a00` | `#ffd27a` |
| `--pmc-color-chart-5`         | `#b23a48` | `#ff9b95` |
| `--pmc-color-chart-6`         | `#2a7f8e` | `#72d7e2` |
| `--pmc-color-chart-7`         | `#6a5cbb` | `#bdb2ff` |
| `--pmc-color-chart-8`         | `#7a6a24` | `#d7cc70` |
| `--pmc-color-chart-grid`      | `#d2d2d7` | `#3a3a3c` |
| `--pmc-color-chart-axis`      | `#6e6e73` | `#aeaeb2` |
| `--pmc-color-chart-reference` | `#1d1d1f` | `#f5f5f7` |

Colour is only a supplemental series cue. Multi-series charts also use a stable dash style and marker shape, label series directly where space allows, and provide the values as a table or list. Grayscale and high-contrast output must stay readable. Success and danger colours keep their semantic meaning and are not used for ordinary categories.

Do not give every Product or status its own brand colour. Use labels, icons, position and text as well.

### Themes

`:root` holds the light theme. The dark theme applies under `@media (prefers-color-scheme: dark)` unless the root carries `data-theme="light"`, and always under `:root[data-theme="dark"]`. The theme button in the top bar records an explicit choice in local storage; with no stored choice the app follows Windows.

## Typography

- `--pmc-font-family-ui`: `"Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`. Inter is not bundled with the app, so on a machine without it the Segoe UI fallback is used.
- `--pmc-font-variant-tabular`: `tabular-nums`, for KPIs, dates, durations and comparisons.
- Type roles are split into `-size`, `-line` and `-weight` tokens:

| Role          | Token prefix               | Size | Line height | Weight |
| ------------- | -------------------------- | ---- | ----------- | ------ |
| Display       | `--pmc-type-display`       | 28px | 34px        | 650    |
| Page title    | `--pmc-type-page-title`    | 22px | 28px        | 650    |
| Section title | `--pmc-type-section-title` | 17px | 24px        | 600    |
| Body          | `--pmc-type-body`          | 14px | 21px        | 400    |
| Supporting    | `--pmc-type-supporting`    | 12px | 18px        | 400    |
| Table         | `--pmc-type-table`         | 13px | 18px        | 400    |

- Dense text never falls below 12 px at 100% scaling.
- `--pmc-app-scale` multiplies text sizes. Settings offers five steps (90, 100, 110, 120 and 130%), independent of Windows display scaling; the choice is stored locally and never reaches the Ledger.
- No all-caps sentences. Small uppercase is only for short data labels.

## Spacing, shape and layering

| Token                                               | Value                             | Use                                              |
| --------------------------------------------------- | --------------------------------- | ------------------------------------------------ |
| `--pmc-space-1` to `-7`                             | 4, 8, 12, 16, 24, 32, 48 px       | Fine spacing to page separation                  |
| `--pmc-radius-sm` / `-md` / `-lg`                   | 6 / 10 / 14 px                    | Fields and badges / buttons and panels / dialogs |
| `--pmc-control-height-compact`                      | 32px                              | Dense control minimum                            |
| `--pmc-control-height-standard`                     | 40px                              | Standard and primary controls                    |
| `--pmc-icon-size-sm` / `-md`                        | 16 / 20 px                        | Compact / standard icons                         |
| `--pmc-focus-width`                                 | 2px                               | Focus ring width                                 |
| `--pmc-focus-offset`                                | 2px                               | Gap between ring and component                   |
| `--pmc-shadow-floating`                             | `0 8px 24px rgba(0, 0, 0, 0.14)`  | Menus and inspectors                             |
| `--pmc-shadow-dialog`                               | `0 18px 48px rgba(0, 0, 0, 0.22)` | Modal dialogs                                    |
| `--pmc-z-base` / `-sticky` / `-popover` / `-dialog` | 0 / 100 / 300 / 500               | Layering                                         |

Avoid glassmorphism over data, heavy translucency, pill shapes on every label, and large rounded cards that reduce density.

## Motion

| Token                   | Value                        | Use                           |
| ----------------------- | ---------------------------- | ----------------------------- |
| `--pmc-motion-fast`     | 120ms                        | Hover and focus feedback      |
| `--pmc-motion-standard` | 180ms                        | Panel and control changes     |
| `--pmc-motion-slow`     | 240ms                        | Dialogs and major transitions |
| `--pmc-ease-standard`   | `cubic-bezier(0.2, 0, 0, 1)` | Deceleration                  |

Motion explains hierarchy or confirms a change; it never celebrates ordinary data entry. Under `prefers-reduced-motion: reduce` all three durations become 0ms.

## Components

### Buttons and actions

- One primary button per decision region.
- Destructive buttons use danger styling and an explicit verb. Never use "Yes" as a destructive label.
- Icon-only buttons have an accessible name and a tooltip.
- Labels use domain verbs, such as prepare accept, approve and complete, or confirm decline.

### Forms

- Labels are always visible; placeholders show examples, not labels.
- Validation appears next to the field.
- A form offers only the transitions the lifecycle allows, and the host validates them again.

### Tables and lists

- Keep the record's identity visible.
- Empty values say what is missing, for example "no owner" or "no deadline recorded", instead of a silent dash.
- Filters narrow a list without reordering it.

### Evidence

Evidence shows its verification state, whether a fingerprint is pinned, its classification and its version. File paths are not shown. Broken or unverifiable Evidence stays visible with an explanation.

### Attention items

Each item says what happened, why it matters, who owns it, when it is due, the next step and why it sits where it does. Order follows the documented [attention ranking policy](policies/attention-ranking.md), not a score.

### Review and approval

- A dialog is only for a focused decision that must interrupt the current task.
- A governed change is prepared first. The review sheet shows the records and versions it changes, the effects of approval, where the classification comes from, and the full payload digest. The preview expires after five minutes.
- Approving sends back the digest the user saw. A changed record, payload or policy invalidates the preview, and each approval receipt is used once.
- Rejecting a preview is recorded. Closing the sheet with Escape, a click outside or "decide later" records nothing, and the row offers a way back to the same preview.
- Destructive operations (a higher tier) additionally require a named confirmation and verified recovery evidence. No such operation is reachable from the desktop today.

### Feedback states

- Loading: skeletons for stable shapes, progress for measurable work.
- Empty: say why the area is empty and what can be done next.
- Error: safe, actionable text and a copyable Correlation ID; technical detail stays in local diagnostics.
- Degraded: keep what still works and name what is unavailable, for example Vault actions when no Vault is attached.
- Partial success: list what succeeded and what failed, and how to recover.

Toasts never carry the only explanation of an error, an approval result or a privacy decision.

## Content and language

- The interface is available in six languages (English, Traditional Chinese, Simplified Chinese, Japanese, Korean and Spanish), each a complete typed message catalog. Canonical domain names stay in English where they are identifiers: Product Mission Control, Product Vault, Product Ledger, Executive Cockpit, and the navigation labels.
- Strings are not built by concatenating fragments, and layouts allow for text expansion.
- Timestamps are stored in UTC and shown in the computer's local time zone, which the top bar names. Dates use `YYYY-MM-DD`.
- Numbers show unit, period and comparison basis when needed.
- Distinguish system fact, source fact and user judgment.

## Platform

- Windows 11 x64 is the only supported platform. The app must work at 100%, 125% and 150% display scaling.
- macOS is not supported. The core avoids Windows-only paths and APIs so a macOS build remains possible.
- Paths are never hard-coded in interface logic, and the interface does not display Vault file paths.

## Accessibility baseline

- Target WCAG 2.2 AA for contrast, semantics, input, focus and error handling, adapted to a desktop app.
- Every action is reachable by keyboard in a logical order.
- Focus is always visible: `:focus-visible` draws a `--pmc-focus-width` outline in `--pmc-color-focus`, offset by `--pmc-focus-offset`.
- Controls have programmatic names, roles and states. Headings, regions, tables, lists, dialogs and live feedback use semantic elements.
- Dialogs trap focus and close on Escape.
- Colour is never the only signal for status, classification, freshness, selection or error.
- Text and meaningful controls meet the contrast targets above. The measured contrast pairs are recorded in `docs/evidence/contrast-results.json`; `apps/desktop/src/design-system/prototypeEvidence.test.ts` checks that every recorded pair meets its minimum, that the shipped tokens contain every measured colour and the full chart palette, and that both themes are defined for the default and the explicit choice.
- Pointer targets are at least 32 x 32 px in dense areas and preferably 40 x 40 px for primary actions.
- Zoom and Windows scaling do not hide content or require two-dimensional scrolling in primary workflows.
- Reduced motion is honoured.
- Charts have an accessible summary and a table or list with the same values.
- JSX accessibility rules (`eslint-plugin-jsx-a11y`) run as part of linting.

## Privacy

- Screenshots, fixtures and demo content use synthetic data only.
- Restricted content does not appear in ordinary exports, notifications or screenshots.
- There is no analytics, telemetry, screen recording or crash upload.
