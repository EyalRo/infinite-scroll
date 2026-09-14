# Infinite Scroll — Post Composer Web App

Status: approved, not yet implemented
Date: 2026-09-13
Related: Ops-in-Net Knowledge page "Infinite Scroll" (https://ops.in.net/account/knowledge/infinite-scroll)

## Purpose

A manual authoring tool for the Infinite Scroll installation. Lets the artist
(or Eyal) fill in the details of a fictional/satirical "professional network"
post — name, title, body text, avatar initials, reaction/comment/repost
counts — and print it directly to the installed Arkscan 2054A over the
existing validated raw-USB ZPL pipeline. A second "Design" view exposes the
CSS used to render posts, with a live preview, so the visual style can be
iterated without editing code or reprinting paper to check every change.

This is a standalone authoring tool, decoupled from the not-yet-built
automatic/dynamic printing system described on the Knowledge page. Posts
composed here are printed once and are not added to any shared pool.

## Non-goals

- No automatic/scheduled printing (that's the separate, not-yet-built
  `infinite-scroll.service`).
- No persistence of composed posts beyond the print-ready record files
  already used for logging.
- No multi-user accounts or auth — LAN-only, single trusted user, matching
  how the Pi is otherwise administered (SSH key-only, no public exposure).
- No editing of the HTML structure through the UI — only CSS is editable.

## Architecture

Single Flask app, Python, running as a new systemd service directly on the
installation Pi (same host the Arkscan is wired to over USB) — printing
requires direct access to `/dev/usb/lp0`, so the app must run there rather
than on a separate machine.

```
Browser (LAN)
   |
   v
Flask app on infinite-scroll.local:8080
   |
   +-- GET  /            compose form
   +-- POST /print       render + print one post
   +-- GET  /design       CSS editor + live preview
   +-- POST /design/preview   render a sample post with in-browser (unsaved) CSS -> PNG
   +-- POST /design       save CSS to template.css
```

### Render pipeline

```
Jinja2 HTML skeleton (fixed structure, bundled with app)
  + template.css (editable, persisted on disk)
  -> WeasyPrint: HTML/CSS -> PDF (fixed width, generous max height so content never paginates)
  -> pdftoppm (poppler-utils): PDF -> PNG at a DPI tuned to land on 650px width
  -> PIL: crop trailing whitespace -> content-driven height PNG
  -> PIL: convert('1') (Floyd-Steinberg dither, matches existing contract)
  -> pack rows -> ^GFA hex -> self-contained ZPL job
  -> write to /dev/usb/lp0
```

This reuses the conversion contract already validated and documented on the
Knowledge page (650-dot width, Floyd-Steinberg, `^GFA`, `/dev/usb/lp0`)
end-to-end; only the *rasterization of the source image* changes, from
ad-hoc PIL layout code to HTML+CSS via WeasyPrint.

Chosen over a headless-Chromium approach (e.g. Playwright) for footprint and
render speed on Pi 4 hardware — WeasyPrint is pure Python + Cairo/Pango, no
browser binary to install or keep updated.

### Template & CSS model

- **Structure is fixed**, **style is editable**. The Jinja2 template defines
  which fields render where: avatar circle (initials), name, title, "2h ·
  🌐"-style meta line, body paragraphs, divider, reaction-count line,
  divider, a LinkedIn-style button row (Like / Comment / Repost / Send, each
  with an icon), and the existing `FICTIONAL TEST POST · NOT LINKEDIN DATA` /
  `FICTIONAL SATIRE` disclaimer footer (kept — non-negotiable, always
  rendered regardless of CSS).
- Icons are inlined SVGs from [Lucide](https://lucide.dev) (ISC license):
  `thumbs-up`, `message-circle`, `repeat-2`, `send`. Bundled with the app,
  not user-replaceable through this UI.
- CSS lives at `/var/lib/infinite-scroll/webapp/template.css` on the Pi,
  seeded at first deploy with a **LinkedIn-facsimile default**: circular
  avatar, bold name + gray subtitle, meta line, body text, reaction count
  line, and an icon button row — mimicking real LinkedIn post layout and
  typography for the satire to land, without reproducing LinkedIn's actual
  logo mark or brand color (moot anyway on a monochrome thermal print). No
  backup/versioning of previous CSS is built in; the file can be
  git-tracked manually in this repo if history is wanted later.
- `/design` loads the current CSS into a textarea. Typing triggers a
  debounced call to `/design/preview` (posts the in-progress CSS text plus a
  fixed sample post's field values, gets back a rendered PNG, shown inline)
  — so you can iterate on the stylesheet without saving or printing.
  "Save" persists the textarea content to `template.css`; that's the only
  action that affects future real prints.

## Print safety & concurrency

Today's incident (a hung write to `/dev/usb/lp0` blocking the whole SSH
session until manually killed) drives two requirements:

- Each print write runs in a **background thread with a bounded timeout**
  (~15s). The HTTP request doesn't block indefinitely; the UI shows
  *printing…* then either *printed ✓* or *failed — check paper, cover, and
  power on the Arkscan* (a stuck/timed-out write is treated as failure, not
  silently retried).
- An **in-process lock** serializes all print attempts, so two browser tabs
  (or a stray retry) can never write to `/dev/usb/lp0` concurrently — that
  interleaving risk is exactly what produced the orphaned `cat` processes
  during manual printing earlier today.
- Before every write: verify the job file is non-empty and `/dev/usb/lp0`
  exists as a character device (same preflight as the documented manual
  procedure). If either check fails, report the error without attempting
  the write.

## Record-keeping

Every successful print writes `<slug>-<timestamp>.png` and
`<slug>-<timestamp>.zpl` to `/var/lib/infinite-scroll/print-ready/`, with
sha256 of each logged (server log line), consistent with the existing
manual-batch convention on the Knowledge page. `<slug>` is derived from the
submitted name (slugified). No database — the directory listing is the
record.

## Deployment

- Python venv on the Pi with `flask`, `weasyprint`, `Pillow`; `poppler-utils`
  installed via apt for `pdftoppm` (Raspberry Pi OS is Debian-based).
- New systemd unit `infinite-scroll-webapp.service`, `Restart=on-failure`,
  binds `0.0.0.0:8080`. Separate from (and does not depend on) the
  not-yet-built automatic `infinite-scroll.service`.
- Reachable at `http://infinite-scroll.local:8080` on the LAN. No auth.

## Testing

- A unit test locks in the render → dither → pack pipeline against one fixed
  sample post (name/title/body/counts + a fixed CSS fixture), asserting a
  stable output sha256 — a regression guard against template or CSS-loading
  changes silently altering output.
- Preflight-check unit tests: missing `/dev/usb/lp0`, empty job file, and a
  simulated write timeout each produce the correct *failed* UI state without
  hanging the test suite.
- Manual acceptance pass before calling this done: compose and print one
  real post end-to-end; edit the CSS, confirm the live preview updates;
  save; print again and confirm the style change is reflected on paper.

## Open questions

None outstanding — all resolved during brainstorming. (If real-world testing
surfaces that WeasyPrint's CSS support is too limited for a desired style,
the render-stack swap is isolated to one component per the architecture
above and can be revisited without touching the print pipeline, template
model, or Flask routes.)
