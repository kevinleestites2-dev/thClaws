---
name: gamedev
description: Universal conventions and recipe catalog for /gamedev sessions. Read before writing any game code.
---

# /gamedev — p5.js game development

You are working in a `/gamedev` session. The stack is **p5.js + vanilla JavaScript** loaded straight in the browser — no bundler, no TypeScript, no React. Games render to a single `<canvas>` mounted by p5 in `setup()`.

If `./SKILL.md` exists in the project root, read it first. Project conventions override this skill.

## Conventions (non-negotiable)

These exist so generated code stays consistent across one project. Do not deviate unless the user explicitly asks.

### Entity model
- Entities are plain objects produced by factory functions, not ES6 classes:
  `function makeBullet(x, y, vx, vy) { return { kind: 'bullet', x, y, vx, vy, alive: true }; }`.
- Behavior lives in free functions that switch on `kind`: `updateEntity(e, dt)`, `drawEntity(e)`. Not methods on the object.
- All entities live in one array: `state.entities`. Iterate to update and draw. Reap dead entities (`alive === false`) once per frame at the end of `update`, never mid-iteration.

### State location
- One module-scope `const state = { ... }` holds every game value: entities, score, scene, timers, input intents.
- Top-level `let` is reserved for genuinely re-bound references (e.g. `let player = null` assigned in `setup()`).
- No globals on `window` except what p5 itself defines.

### Coordinate system
- Origin top-left, +x right, +y down (canvas default).
- Use the p5 globals `width` / `height` for canvas size. Never hardcode pixel dimensions in game logic.

### Time and motion
- All movement is delta-based: `e.x += e.vx * dt`. `dt` is `deltaTime / 1000` (seconds).
- Frame-based counters are allowed only for cosmetic animation cycles (sprite frame index). Never for gameplay timing.

### Input
- Continuous input (move, hold-to-charge) → poll with `keyIsDown(LEFT_ARROW)` inside `update`.
- Discrete actions (jump, fire, pause) → set an intent flag from the `keyPressed()` event handler, consume it in `update`, clear it before returning.
- Never read input from `draw()`.

### File layout
- Start single-file: `index.html` loads p5 from CDN and `game.js`. `game.js` is the whole game.
- Split into `entities.js`, `scenes.js`, `input.js` only when `game.js` passes ~400 lines or the user asks.
- No build step. Files are loaded via `<script>` tags.

## Recipes

Recipes cover things that are easy to get wrong. Ask the bundled `gamedev` MCP server for the recipe text when you hit one of these:

- `juice` — screen shake, hit pause, particles, easing. Game-feel polish.
- `collision` — AABB, circle, swept collision, spatial partitioning.
- `state-machine` — scene transitions, entity state, pause/resume layering.
- `audio` — playback without latency, music vs sfx, mobile autoplay rules.
- `save-load` — `localStorage`, schema versioning, migration.

For everything not in this list (score display, basic movement, level data, UI text), follow the conventions above and write directly. Do not search for a recipe that does not exist.

## Workflow

1. Read `./SKILL.md` if it exists. It is the source of truth for project-specific conventions.
2. Scaffold from a template via the bundled `gamedev` MCP server (when present). Otherwise create `index.html` + `game.js` by hand following the file layout above.
3. Iterate. After any visible change, take a canvas snap (when the snap tool is available) and verify it looks right before continuing. Code that parses is not code that runs.
4. Keep changes small and shippable. A working game with three rough features beats a half-built game with ten.

## Anti-patterns

Do not, unless the user explicitly asks:

- Introduce TypeScript, JSX, Webpack/Vite/Parcel, npm packages, or test frameworks. p5.js is browser-loadable; keep it that way.
- Refactor factory-function entities into ES6 classes "for clarity". Project convention is plain objects.
- Call `requestAnimationFrame` yourself. p5 owns the loop.
- Add ECS, scene graphs, or other Unity-style abstractions to a small game. They cost more than they save below ~2000 lines.
