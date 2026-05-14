//! GameDev vertical pack — `/gamedev` mode for building p5.js web games.
//!
//! Ships in three pieces, in order of completeness:
//!
//! 1. **System-prompt fragment** (this file) — short framing that tells
//!    the agent it's in `/gamedev` mode and to consult the bundled
//!    skill. The verbose conventions live in the skill body, not here.
//! 2. **Universal skill** (`gamedev_skill.md`, embedded via
//!    `include_str!`) — the p5.js conventions, recipe index, workflow,
//!    and anti-patterns. Materialized to a cache directory on
//!    `on_enter` so `SkillStore` can pick it up through the normal
//!    on-disk discovery path.
//! 3. **Bundled MCP server** (future) — serves recipes, templates,
//!    and asset bytes. Out of scope for this commit; the pack reports
//!    an empty `mcp_servers` list until the server lands.
//!
//! The cache lives at `$XDG_CACHE_HOME/thclaws/verticals/gamedev/` (or
//! `~/.cache/...` if XDG isn't set). We always rewrite the skill body
//! on `on_enter` — the file is ~5 KB and writing it unconditionally
//! avoids stale content if the binary was updated since the last
//! activation.

use super::{VerticalPack, VerticalPackResources};
use crate::error::{Error, Result};
use std::path::PathBuf;
use std::sync::OnceLock;

const MODE_NAME: &str = "gamedev";
const DESCRIPTION: &str = "p5.js web game development";
const SKILL_NAME: &str = "gamedev";
const SKILL_BODY: &str = include_str!("gamedev_skill.md");

const SYSTEM_PROMPT: &str = r#"You are in `/gamedev` mode. The user is building a web game with p5.js.

Read the bundled `gamedev` skill before writing any game code. It defines the project conventions (entity model, state location, time, input) that all generated code must follow.

If the project root has a `./SKILL.md`, treat it as the source of truth for game-specific conventions — the universal skill is the default, the project file overrides.

Game feel matters. After a visible change, take a snap of the canvas (when the snap tool is available) and confirm the render before continuing. Code that parses is not code that runs."#;

pub struct GameDevPack {
    /// Set by `on_enter` once the skill body is materialized. The
    /// cache survives `exit_mode` so re-entering is a no-op write.
    cache_root: OnceLock<PathBuf>,
    /// Test seam: when set, `resolve_cache_root` returns this instead
    /// of the XDG-derived path. Production callers go through
    /// `GameDevPack::new()` which leaves it `None`. Avoids
    /// `std::env::set_var` in tests, which races with cargo's
    /// parallel test runner.
    cache_root_override: Option<PathBuf>,
}

impl GameDevPack {
    pub fn new() -> Self {
        Self {
            cache_root: OnceLock::new(),
            cache_root_override: None,
        }
    }

    /// Build a pack rooted at an explicit cache directory. Used by
    /// tests to isolate per-test state without touching environment
    /// variables. Not exposed publicly: vertical-pack consumers in
    /// production should always use `new()` so cache layout stays
    /// consistent across the binary.
    #[cfg(test)]
    fn with_cache_root(root: PathBuf) -> Self {
        Self {
            cache_root: OnceLock::new(),
            cache_root_override: Some(root),
        }
    }

    /// Resolve the on-disk cache root. Mirrors how
    /// `mcp::mcp_allowlist_path` resolves config paths: prefer
    /// `XDG_CACHE_HOME` when set, fall back to `~/.cache`. Returns an
    /// error only on a system without either, which would mean we
    /// can't write any user state.
    fn resolve_cache_root(&self) -> Result<PathBuf> {
        if let Some(o) = &self.cache_root_override {
            return Ok(o.clone());
        }
        let base = if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
            PathBuf::from(xdg)
        } else {
            crate::util::home_dir()
                .ok_or_else(|| {
                    Error::Config("no home directory; cannot cache gamedev skill".into())
                })?
                .join(".cache")
        };
        Ok(base.join("thclaws").join("verticals").join(MODE_NAME))
    }

    /// Write the embedded skill body to the cache. Unconditional —
    /// see module docs for why we don't gate on existence.
    fn materialize(&self, root: &std::path::Path) -> Result<()> {
        let skill_dir = root.join("skills").join(SKILL_NAME);
        std::fs::create_dir_all(&skill_dir)?;
        std::fs::write(skill_dir.join("SKILL.md"), SKILL_BODY)?;
        Ok(())
    }
}

impl Default for GameDevPack {
    fn default() -> Self {
        Self::new()
    }
}

impl VerticalPack for GameDevPack {
    fn mode_name(&self) -> &str {
        MODE_NAME
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn system_prompt(&self) -> String {
        SYSTEM_PROMPT.to_string()
    }

    fn resources(&self) -> VerticalPackResources {
        let skill_dirs = match self.cache_root.get() {
            Some(root) => vec![root.join("skills")],
            // Before `on_enter` runs there's no cache dir to point at.
            // Returning empty is correct — flatteners check `active`
            // before consulting `resources`, and `active` is only set
            // after `on_enter` succeeds.
            None => Vec::new(),
        };
        VerticalPackResources {
            skill_dirs,
            command_dirs: Vec::new(),
            agent_dirs: Vec::new(),
            mcp_servers: Vec::new(),
        }
    }

    fn on_enter(&self) -> Result<()> {
        let root = self.resolve_cache_root()?;
        self.materialize(&root)?;
        // OnceLock::set returns Err on subsequent calls; that's fine
        // because the path is deterministic — same value either way.
        let _ = self.cache_root.set(root);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn pack_metadata() {
        let pack = GameDevPack::new();
        assert_eq!(pack.mode_name(), "gamedev");
        assert!(!pack.description().is_empty());
        assert!(pack.system_prompt().contains("gamedev"));
    }

    #[test]
    fn resources_empty_before_enter() {
        let pack = GameDevPack::new();
        let res = pack.resources();
        assert!(res.skill_dirs.is_empty());
        assert!(res.mcp_servers.is_empty());
    }

    #[test]
    fn on_enter_materializes_skill_to_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let pack = GameDevPack::with_cache_root(tmp.path().to_path_buf());
        pack.on_enter().unwrap();

        let expected = tmp.path().join("skills").join("gamedev").join("SKILL.md");
        assert!(
            expected.exists(),
            "SKILL.md not materialized at {expected:?}"
        );
        let body = std::fs::read_to_string(&expected).unwrap();
        assert!(body.contains("p5.js"), "embedded skill body looks wrong");
    }

    #[test]
    fn resources_after_enter_point_at_cached_skill_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let pack = GameDevPack::with_cache_root(tmp.path().to_path_buf());
        pack.on_enter().unwrap();
        let res = pack.resources();
        assert_eq!(res.skill_dirs.len(), 1);
        // SkillStore::load_dir walks the immediate children of each
        // entry looking for `<child>/SKILL.md`. So we return the
        // `skills/` directory, not `skills/gamedev/`.
        let dir = &res.skill_dirs[0];
        assert!(dir.ends_with("skills"));
        assert!(dir.join("gamedev").join("SKILL.md").exists());
    }

    #[test]
    fn on_enter_is_idempotent_and_overwrites_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let pack = GameDevPack::with_cache_root(tmp.path().to_path_buf());
        pack.on_enter().unwrap();
        let body_path = tmp.path().join("skills/gamedev/SKILL.md");
        std::fs::write(&body_path, "stale").unwrap();
        pack.on_enter().unwrap();
        let body = std::fs::read_to_string(&body_path).unwrap();
        assert!(
            body.contains("p5.js"),
            "on_enter should overwrite stale content"
        );
    }

    #[test]
    fn pack_works_through_registry() {
        let tmp = tempfile::tempdir().unwrap();
        let mut reg = super::super::VerticalPackRegistry::new();
        reg.register(Arc::new(GameDevPack::with_cache_root(
            tmp.path().to_path_buf(),
        )));
        reg.enter("gamedev").unwrap();
        let pack = reg.active().unwrap();
        let res = pack.resources();
        assert_eq!(res.skill_dirs.len(), 1);
        assert!(!pack.system_prompt().is_empty());
    }
}
