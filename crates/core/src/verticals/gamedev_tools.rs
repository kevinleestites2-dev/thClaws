//! Native tools contributed by the `/gamedev` vertical pack when
//! `THCLAWS_GAMEDEV_LOCAL` is set.
//!
//! Each tool reads files from the directory the env var points at — the
//! user's private game collection. The tools are registered into the
//! ToolRegistry only when the env var is present, so the OSS thClaws
//! binary stays inert without a local pack.
//!
//! When the project graduates from "local-config" to a hosted Pro pack
//! (option A or B in dev-log/gamedev-design), these will be replaced
//! by an MCP server speaking the same tool shapes. The schemas here
//! intentionally match what an MCP server would expose so the swap is
//! a config change, not a rewrite.

use crate::error::{Error, Result};
use crate::tools::{Tool, UiResource};
use async_trait::async_trait;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::oneshot;

pub const ENV_VAR: &str = "THCLAWS_GAMEDEV_LOCAL";

/// Resolve the configured local pack root. Returns `None` if the env
/// var is unset, `Err` if set but pointing at a non-existent path.
pub fn local_root() -> Result<Option<PathBuf>> {
    let raw = match std::env::var(ENV_VAR) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => return Ok(None),
    };
    let path = PathBuf::from(raw);
    if !path.is_dir() {
        return Err(Error::Config(format!(
            "{ENV_VAR} points at {} which is not a directory",
            path.display()
        )));
    }
    Ok(Some(path))
}

/// Heuristic: a directory entry is a "game" if it's a subdirectory
/// (PascalCase by convention, but we don't enforce that) that contains
/// an `index.html` and a `<DirName>.js` of the same name. Excludes
/// underscore-prefixed dirs (`_templates`) and known non-game dirs
/// (`screenshots`, `others`).
fn is_game_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    if name.starts_with('_') || name == "screenshots" || name == "others" {
        return false;
    }
    path.join("index.html").is_file() && path.join(format!("{name}.js")).is_file()
}

// ─────────────────────────────────────────────────────────────────────
// GamedevListExamples
// ─────────────────────────────────────────────────────────────────────

pub struct GamedevListExamplesTool;

#[async_trait]
impl Tool for GamedevListExamplesTool {
    fn name(&self) -> &'static str {
        "GamedevListExamples"
    }

    fn description(&self) -> &'static str {
        "List reference game examples available in the local game collection. \
         Each entry is the name of a sibling game directory (Breakout, SpaceShooter, \
         Sudoku, etc.). Use this to pick a closest-genre reference before writing \
         a new game, then fetch its source with `GamedevExample`."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn call(&self, _input: Value) -> Result<String> {
        let root = local_root()?.ok_or_else(|| {
            Error::Tool(format!(
                "{ENV_VAR} is not set; tool is only available inside a configured /gamedev session"
            ))
        })?;
        let mut names: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&root)
            .map_err(|e| Error::Tool(format!("read {}: {e}", root.display())))?
            .flatten()
        {
            let p = entry.path();
            if is_game_dir(&p) {
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    names.push(name.to_string());
                }
            }
        }
        names.sort();
        if names.is_empty() {
            return Ok(format!("no game examples found under {}", root.display()));
        }
        Ok(format!(
            "{} reference games available:\n{}",
            names.len(),
            names
                .iter()
                .map(|n| format!("  - {n}"))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────
// GamedevExample
// ─────────────────────────────────────────────────────────────────────

pub struct GamedevExampleTool;

#[async_trait]
impl Tool for GamedevExampleTool {
    fn name(&self) -> &'static str {
        "GamedevExample"
    }

    fn description(&self) -> &'static str {
        "Return the full source of one reference game: index.html, style.css, \
         the main <Name>.js file, plus a list of the game's asset filenames. \
         Use after picking a target via `GamedevListExamples`."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Game name as returned by GamedevListExamples (e.g. 'Breakout')."
                }
            },
            "required": ["name"]
        })
    }

    async fn call(&self, input: Value) -> Result<String> {
        let name = input
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool("missing 'name' argument".into()))?;
        let root = local_root()?.ok_or_else(|| Error::Tool(format!("{ENV_VAR} is not set")))?;
        let game_dir = root.join(name);
        if !is_game_dir(&game_dir) {
            return Err(Error::Tool(format!(
                "no game named '{name}' under {} (try GamedevListExamples)",
                root.display()
            )));
        }
        let index_html = std::fs::read_to_string(game_dir.join("index.html"))
            .map_err(|e| Error::Tool(format!("read index.html: {e}")))?;
        let style_css = std::fs::read_to_string(game_dir.join("style.css"))
            .unwrap_or_else(|_| "(no style.css)".to_string());
        let js_path = game_dir.join(format!("{name}.js"));
        let js_src = std::fs::read_to_string(&js_path)
            .map_err(|e| Error::Tool(format!("read {}: {e}", js_path.display())))?;
        let mut assets: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&game_dir)
            .map_err(|e| Error::Tool(format!("list {}: {e}", game_dir.display())))?
            .flatten()
        {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let Some(fname) = p.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            // Skip the three text files we already inlined.
            if fname == "index.html" || fname == "style.css" || fname == format!("{name}.js") {
                continue;
            }
            assets.push(fname.to_string());
        }
        assets.sort();
        let asset_list = if assets.is_empty() {
            "(none)".to_string()
        } else {
            assets
                .iter()
                .map(|a| format!("  - {a}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(format!(
            "=== {name} — example game source ===\n\n\
             ── index.html ──────────────────────────────────────\n{index_html}\n\n\
             ── style.css ───────────────────────────────────────\n{style_css}\n\n\
             ── {name}.js ──────────────────────────────────────\n{js_src}\n\n\
             ── assets in this game dir ────────────────────────\n{asset_list}"
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────
// GamedevLibrary
// ─────────────────────────────────────────────────────────────────────

pub struct GamedevLibraryTool;

#[async_trait]
impl Tool for GamedevLibraryTool {
    fn name(&self) -> &'static str {
        "GamedevLibrary"
    }

    fn description(&self) -> &'static str {
        "Return the full source of GameLibrary.js — the engine every game in \
         the collection uses. Read this ONCE per session to learn the API, then \
         keep it in context. ~4000 lines; do not re-fetch unless you forgot \
         something specific."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn call(&self, _input: Value) -> Result<String> {
        let root = local_root()?.ok_or_else(|| Error::Tool(format!("{ENV_VAR} is not set")))?;
        let lib = root.join("GameLibrary.js");
        std::fs::read_to_string(&lib)
            .map_err(|e| Error::Tool(format!("read {}: {e}", lib.display())))
    }
}

// ─────────────────────────────────────────────────────────────────────
// GamedevScaffold
// ─────────────────────────────────────────────────────────────────────

pub struct GamedevScaffoldTool;

#[async_trait]
impl Tool for GamedevScaffoldTool {
    fn name(&self) -> &'static str {
        "GamedevScaffold"
    }

    fn description(&self) -> &'static str {
        "Copy the `minimal` template into a target directory, renaming the \
         placeholder class `Minimal` to the requested new game name. Creates \
         index.html, style.css, and <Name>.js. You still need to add the PNG \
         assets (SplashScreen, Background, standard buttons) — copy them from \
         a reference game's directory or generate them."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "PascalCase game name (e.g. 'MyShooter'). Used as the class names AND the .js filename."
                },
                "target_dir": {
                    "type": "string",
                    "description": "Directory to create. Must not exist. Created as a sibling of GameLibrary.js so the relative `../GameLibrary.js` script path works."
                }
            },
            "required": ["name", "target_dir"]
        })
    }

    fn requires_approval(&self, _input: &Value) -> bool {
        true
    }

    async fn call(&self, input: Value) -> Result<String> {
        let name = input
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool("missing 'name' argument".into()))?;
        let target_raw = input
            .get("target_dir")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool("missing 'target_dir' argument".into()))?;
        if name.is_empty() || !name.chars().next().unwrap().is_ascii_uppercase() {
            return Err(Error::Tool(
                "'name' must start with an uppercase ASCII letter (PascalCase)".into(),
            ));
        }
        if name.chars().any(|c| !c.is_ascii_alphanumeric()) {
            return Err(Error::Tool(
                "'name' may only contain ASCII letters and digits".into(),
            ));
        }
        let target = crate::sandbox::Sandbox::check(target_raw)?;
        if target.exists() {
            return Err(Error::Tool(format!(
                "target {} already exists",
                target.display()
            )));
        }
        let root = local_root()?.ok_or_else(|| Error::Tool(format!("{ENV_VAR} is not set")))?;
        let tmpl_dir = root.join("_templates").join("minimal");
        if !tmpl_dir.is_dir() {
            return Err(Error::Tool(format!(
                "template dir {} is missing",
                tmpl_dir.display()
            )));
        }
        std::fs::create_dir_all(&target)
            .map_err(|e| Error::Tool(format!("mkdir {}: {e}", target.display())))?;
        let lower = first_lower(name);
        let mut created: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&tmpl_dir)
            .map_err(|e| Error::Tool(format!("read {}: {e}", tmpl_dir.display())))?
            .flatten()
        {
            let src = entry.path();
            let Some(src_name) = src.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            // Map source filename onto the new name.
            let dst_name = if src_name == "Minimal.js" {
                format!("{name}.js")
            } else {
                src_name.to_string()
            };
            let dst = target.join(&dst_name);
            let body = std::fs::read_to_string(&src)
                .map_err(|e| Error::Tool(format!("read {}: {e}", src.display())))?;
            // Substitution rules:
            //   Minimal           → <name>          (class names, file ref in HTML)
            //   minimal           → <lower(name)>   (var names)
            // Order matters — replace PascalCase first so we don't
            // corrupt the substring inside `MinimalLibrary` etc.
            let body = body.replace("Minimal", name).replace("minimal", &lower);
            std::fs::write(&dst, body)
                .map_err(|e| Error::Tool(format!("write {}: {e}", dst.display())))?;
            created.push(dst_name);
        }
        created.sort();
        // The scaffolded index.html references `../GameLibrary.js`.
        // If the user scaffolded outside $LOCAL, that relative path
        // won't resolve to the actual engine. Drop a symlink in the
        // parent dir of the target so the game loads anywhere.
        let parent = target.parent().unwrap_or(&target);
        let parent_engine = parent.join("GameLibrary.js");
        let engine_src = root.join("GameLibrary.js");
        let mut engine_link_msg = String::new();
        if !parent_engine.exists() && engine_src.is_file() {
            #[cfg(unix)]
            let linked = std::os::unix::fs::symlink(&engine_src, &parent_engine).is_ok();
            #[cfg(not(unix))]
            let linked = std::fs::copy(&engine_src, &parent_engine).is_ok();
            if linked {
                engine_link_msg = format!(
                    "\n  + linked GameLibrary.js into {} so `../GameLibrary.js` resolves",
                    parent.display()
                );
            }
        }
        Ok(format!(
            "scaffolded {name} at {}\nfiles created:\n{}{}\n\nnext steps:\n  1. add PNG assets (SplashScreen, Background, Play0/1, EnterFullscreen0/1, ExitFullscreen0/1) — copy from a reference game via GamedevExample, or generate\n  2. fill in TODOs in {name}.js (start with onLoad to register the assets)\n  3. call GamedevPreview(target_dir={}) to view it live in chat",
            target.display(),
            created
                .iter()
                .map(|f| format!("  - {f}"))
                .collect::<Vec<_>>()
                .join("\n"),
            engine_link_msg,
            target.display(),
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────
// GamedevPreview
// ─────────────────────────────────────────────────────────────────────
//
// Renders a game's index.html in the chat surface as an MCP-Apps style
// widget. Uses the `fetch_ui_resource` trait hook which agent.rs:1558
// invokes after every successful tool call — the returned `UiResource`
// rides alongside the tool-result event and the frontend's
// `McpAppIframe` component mounts it as a sandboxed iframe (with
// detach-to-side-panel via portal-move, no reload on detach).
//
// The widget HTML is a tiny wrapper whose body is a single iframe
// pointing at `thclaws://localhost/file-asset/<abs>/index.html` — the
// wry custom protocol registered in `gui.rs:726` reads the file off
// disk after sandbox-checking the path, and relative URLs inside the
// game (`../GameLibrary.js`, `Background.png`, `Player.wav`) resolve
// through the same protocol because the iframe's base URL is the
// custom scheme. No HTTP server, no port allocation, no extra deps.
//
// Caveat: `fetch_ui_resource` has no per-call argument (see the trait
// definition in `tools/mod.rs:106`), so the target_dir from the last
// `call()` is stashed in an interior-mutable field and re-read by the
// fetch. The agent loop calls them back-to-back (agent.rs:1557-1561),
// so for a single agent the read sees the right path. Parallel
// preview calls on the same tool instance (e.g. two subagents) would
// race — acceptable for single-user dev use; revisit when teams need it.

/// Live state for the embedded preview HTTP server. Holds the bound
/// port, the directory it's serving, and the shutdown signal. Dropping
/// it triggers a graceful shutdown of the axum task.
struct PreviewServerState {
    /// Canonical parent dir we're serving (one above the game dir, so
    /// `../GameLibrary.js` from inside `<game>/index.html` resolves).
    serve_root: PathBuf,
    /// Bound TCP port — embedded in the iframe URL.
    port: u16,
    /// Game directory name relative to `serve_root`. The widget URL is
    /// `http://127.0.0.1:<port>/<game_subdir>/index.html`.
    game_subdir: String,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for PreviewServerState {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

pub struct GamedevPreviewTool {
    state: TokioMutex<Option<PreviewServerState>>,
}

impl Default for GamedevPreviewTool {
    fn default() -> Self {
        Self {
            state: TokioMutex::new(None),
        }
    }
}

#[async_trait]
impl Tool for GamedevPreviewTool {
    fn name(&self) -> &'static str {
        "GamedevPreview"
    }

    fn description(&self) -> &'static str {
        "Render a game's index.html in an iframe widget alongside the chat. \
         Use after writing or editing a game to see it run live. The iframe \
         loads via thClaws's sandbox-checked file protocol, so relative \
         asset paths (../GameLibrary.js, Background.png, sprite PNGs, \
         .wav sounds) resolve correctly. Click the expand icon in the \
         widget to detach the preview into the side panel — the iframe \
         keeps running through the detach, so any in-progress game state \
         survives. Re-call after every meaningful edit; the widget refreshes."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target_dir": {
                    "type": "string",
                    "description": "Path to a directory that contains index.html. Absolute or relative to the cwd."
                }
            },
            "required": ["target_dir"]
        })
    }

    async fn call(&self, input: Value) -> Result<String> {
        let target_raw = input
            .get("target_dir")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool("missing 'target_dir' argument".into()))?;
        let target = crate::sandbox::Sandbox::check(target_raw)?;
        if !target.is_dir() {
            return Err(Error::Tool(format!(
                "{} is not a directory",
                target.display()
            )));
        }
        let index = target.join("index.html");
        if !index.is_file() {
            return Err(Error::Tool(format!(
                "{} has no index.html — nothing to preview yet",
                target.display()
            )));
        }
        let abs = target
            .canonicalize()
            .map_err(|e| Error::Tool(format!("canonicalize {}: {e}", target.display())))?;
        let parent = abs
            .parent()
            .ok_or_else(|| Error::Tool("target_dir has no parent".into()))?
            .to_path_buf();
        let game_subdir = abs
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::Tool("target_dir has no name".into()))?
            .to_string();
        // Self-heal `../GameLibrary.js` — symlink the engine into the
        // serve root if the agent scaffolded outside $LOCAL.
        let mut engine_msg = String::new();
        let parent_engine = parent.join("GameLibrary.js");
        if !parent_engine.exists() {
            if let Ok(Some(local)) = local_root() {
                let engine_src = local.join("GameLibrary.js");
                if engine_src.is_file() {
                    #[cfg(unix)]
                    let linked = std::os::unix::fs::symlink(&engine_src, &parent_engine).is_ok();
                    #[cfg(not(unix))]
                    let linked = std::fs::copy(&engine_src, &parent_engine).is_ok();
                    if linked {
                        engine_msg = format!(
                            "\n  (linked GameLibrary.js into {} for relative resolution)",
                            parent.display()
                        );
                    }
                }
            }
        }
        // Reuse existing server iff it's already serving this parent.
        // Otherwise replace (Drop on the old state graceful-shutdowns
        // the old axum task).
        let mut guard = self.state.lock().await;
        let needs_new = match guard.as_ref() {
            Some(s) => s.serve_root != parent,
            None => true,
        };
        if needs_new {
            let (port, shutdown) = spawn_preview_server(parent.clone()).await?;
            *guard = Some(PreviewServerState {
                serve_root: parent.clone(),
                port,
                game_subdir: game_subdir.clone(),
                shutdown: Some(shutdown),
            });
        } else if let Some(s) = guard.as_mut() {
            // Same parent — just update which game we point the
            // widget at. Server keeps running.
            s.game_subdir = game_subdir.clone();
        }
        let port = guard.as_ref().map(|s| s.port).unwrap_or(0);
        drop(guard);
        Ok(format!(
            "preview mounted for {}\n  server: http://127.0.0.1:{port}/{game_subdir}/index.html{}",
            abs.display(),
            engine_msg,
        ))
    }

    async fn fetch_ui_resource(&self) -> Option<UiResource> {
        let guard = self.state.lock().await;
        let s = guard.as_ref()?;
        let game_url = format!(
            "http://127.0.0.1:{}/{}/index.html",
            s.port, s.game_subdir
        );
        let html = format!(
            r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>gamedev preview</title><style>
html, body, iframe {{
    margin: 0; padding: 0;
    height: 100vh; width: 100vw;
    border: 0; display: block;
    background: #000;
}}
</style></head>
<body><iframe src="{game_url}"
    allow="autoplay; gamepad; fullscreen"
    title="game preview"></iframe></body>
</html>"#
        );
        Some(UiResource {
            uri: format!("gamedev://preview/{}", s.game_subdir),
            html,
            mime: Some("text/html;profile=mcp-app".into()),
            // First-party preview iframe needs to load `<script src>`
            // from the localhost preview server + p5 CDN. Opaque
            // origin (sandbox without allow-same-origin) blocks both
            // — the canvas stays black. Trust is implicit: the tool
            // ships inside the thClaws binary and only serves files
            // out of a directory the user pointed at.
            allow_same_origin: true,
        })
    }
}

// ─── Embedded preview server ────────────────────────────────────────
//
// A loopback-only static file server scoped to a single directory tree.
// Used by GamedevPreview because the McpAppIframe sandbox
// (`allow-scripts allow-popups allow-forms`, intentionally no
// `allow-same-origin`) blocks script execution from custom-scheme
// nested iframes. Loading the game from `http://127.0.0.1:<port>/`
// keeps scripts on a normal origin and CORS-classifies CDN script
// loads as standard cross-origin (which is unrestricted for
// `<script src>`).
//
// One server per active preview target. Replacing the
// `PreviewServerState` drops the previous server, which shuts down
// gracefully via the oneshot signal.

async fn spawn_preview_server(serve_root: PathBuf) -> Result<(u16, oneshot::Sender<()>)> {
    let canonical_root = serve_root
        .canonicalize()
        .map_err(|e| Error::Tool(format!("canonicalize serve root: {e}")))?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| Error::Tool(format!("bind preview server: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::Tool(format!("preview server local_addr: {e}")))?
        .port();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let app = Router::new()
        .route("/{*path}", get(serve_file))
        .with_state(Arc::new(canonical_root));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    Ok((port, shutdown_tx))
}

async fn serve_file(
    AxumPath(path): AxumPath<String>,
    State(root): State<Arc<PathBuf>>,
) -> Response {
    let full = root.join(&path);
    let canonical = match full.canonicalize() {
        Ok(c) => c,
        Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    // Containment check — prevent `../` escapes via URL path. Symlinks
    // followed by canonicalize, so a symlink that points outside the
    // serve root is also rejected here. Exception: the
    // GameLibrary.js symlink we plant in the serve root targets the
    // user's $LOCAL pack and is intentional, so the check passes
    // (target was canonical-checked too by symlink resolution).
    if !canonical.starts_with(&*root)
        && canonical
            .file_name()
            .map(|n| n != "GameLibrary.js")
            .unwrap_or(true)
    {
        return (StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let bytes = match tokio::fs::read(&canonical).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    let mime = mime_from_ext(&canonical);
    Response::builder()
        .header("content-type", mime)
        .body(axum::body::Body::from(bytes))
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "build response").into_response())
}

fn mime_from_ext(p: &std::path::Path) -> &'static str {
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

fn first_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    if let Some(c) = chars.next() {
        for c in c.to_lowercase() {
            out.push(c);
        }
    }
    out.extend(chars);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_lower_basic() {
        assert_eq!(first_lower("Minimal"), "minimal");
        assert_eq!(first_lower("SpaceShooter"), "spaceShooter");
        assert_eq!(first_lower(""), "");
    }

    #[test]
    fn is_game_dir_filters_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Build a fake game dir.
        let game = root.join("FakeGame");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("index.html"), "").unwrap();
        std::fs::write(game.join("FakeGame.js"), "").unwrap();
        // And a non-game dir.
        std::fs::create_dir_all(root.join("_templates")).unwrap();
        // And a junk dir.
        std::fs::create_dir_all(root.join("Empty")).unwrap();

        assert!(is_game_dir(&game));
        assert!(!is_game_dir(&root.join("_templates")));
        assert!(!is_game_dir(&root.join("Empty")));
        assert!(!is_game_dir(&root.join("Nonexistent")));
    }
}
