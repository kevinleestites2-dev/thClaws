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
use crate::tools::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

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
        Ok(format!(
            "scaffolded {name} at {}\nfiles created:\n{}\n\nnext steps:\n  1. add PNG assets (SplashScreen, Background, Play0/1, EnterFullscreen0/1, ExitFullscreen0/1)\n  2. fill in TODOs in {name}.js (start with onLoad to register the assets)\n  3. open index.html in a browser to test",
            target.display(),
            created
                .iter()
                .map(|f| format!("  - {f}"))
                .collect::<Vec<_>>()
                .join("\n")
        ))
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
