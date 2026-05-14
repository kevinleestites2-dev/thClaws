//! Vertical packs — first-party "modes" that swap in a tailored
//! system prompt, set of skills/commands/agents, MCP servers, and
//! (optionally) GUI panels.
//!
//! A vertical pack is to thClaws what `/gamedev`, `/webdev`, or
//! `/datasci` are to the user: a single entry point that reshapes the
//! agent for a specific kind of work. Compared to plugins:
//!
//! - **Plugins** are on-disk data (manifest + folders) installed by the
//!   user. They contribute resources unconditionally to every session.
//! - **Vertical packs** are Rust trait objects registered at startup.
//!   Their resources are only mounted while the pack is *active*
//!   (entered via `enter_mode`), and a pack can also inject a system
//!   prompt fragment and declare UI panels.
//!
//! The interface is intentionally close to `plugins::PluginManifest` so
//! the flatteners look familiar: a pack returns directories that feed
//! `SkillStore::discover_with_extra` etc., plus a list of
//! `McpServerConfig`. The transport (stdio for bundled, http for remote
//! enterprise) is opaque to the registry — swapping bundled assets for
//! a hosted asset service is a config change inside the pack, not a
//! rewrite of the harness.
//!
//! ## Lifecycle
//!
//! Exactly one mode is active at a time. `enter_mode("gamedev")`:
//!
//! 1. Looks up the pack by `mode_name`.
//! 2. Calls `on_exit` on the previously-active pack (if any).
//! 3. Calls `on_enter` on the new pack. If it errors, no mode is active.
//! 4. Stores the new active mode name.
//!
//! Resource flatteners (`vertical_pack_skill_dirs` etc.) read the
//! *currently active* pack only. Callers — the SkillStore, the MCP
//! spawner, the agent's system-prompt builder — pull from these
//! alongside the existing plugin flatteners.

use crate::error::{Error, Result};
use crate::mcp::McpServerConfig;
use crate::tools::Tool;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

pub mod gamedev;
pub mod gamedev_tools;

/// A registered vertical pack. Implementors are first-party Rust types
/// (e.g. `GameDevPack`) registered at startup via [`register_pack`].
///
/// All methods must be cheap and side-effect-free except `on_enter` /
/// `on_exit`, which are the documented lifecycle hooks.
pub trait VerticalPack: Send + Sync {
    /// Stable identifier — the name typed after the slash
    /// (`/gamedev` ⇒ `"gamedev"`). Used as the registry key and the
    /// `mode_name` returned by `active_mode_name`.
    fn mode_name(&self) -> &str;

    /// One-line description shown in `/mode` listings.
    fn description(&self) -> &str;

    /// System-prompt fragment to inject while this mode is active.
    /// The agent appends this to the base system prompt under a
    /// `## Mode: <name>` heading — it never replaces the base prompt,
    /// so CLAUDE.md and project conventions still apply.
    fn system_prompt(&self) -> String;

    /// Resources contributed while this mode is active.
    /// Called on `enter` and re-read by the flatteners; should be
    /// cheap (paths and config structs only, no IO).
    fn resources(&self) -> VerticalPackResources;

    /// Optional UI panels the GUI should mount while active. The
    /// backend only *declares* panels here — the frontend resolves
    /// `content_url` and decides how to render. CLI sessions ignore
    /// this entirely.
    fn ui_panels(&self) -> Vec<UiPanel> {
        Vec::new()
    }

    /// Called once when the mode transitions to active. Default no-op.
    /// Errors abort the transition: no mode becomes active and the
    /// previously-active pack stays exited.
    fn on_enter(&self) -> Result<()> {
        Ok(())
    }

    /// Called once when the mode transitions away. Default no-op.
    /// Errors are logged but do not block the new mode from entering;
    /// a broken exit hook must not leave the harness stuck.
    fn on_exit(&self) -> Result<()> {
        Ok(())
    }
}

/// Resource bundle returned by [`VerticalPack::resources`]. Shape
/// mirrors what `plugins::plugin_*_dirs()` produce so the discovery
/// callers can merge both sources without special-casing.
#[derive(Default, Clone)]
pub struct VerticalPackResources {
    /// Each entry is a directory whose immediate children are skill
    /// dirs containing `SKILL.md`. Fed into
    /// `SkillStore::discover_with_extra`.
    pub skill_dirs: Vec<PathBuf>,
    /// Each entry is a directory of `.md` command files. Fed into
    /// `CommandStore::discover_with_extra`.
    pub command_dirs: Vec<PathBuf>,
    /// Each entry is a directory of agent definition `.md` files. Fed
    /// into `AgentDefsConfig::load_with_extra`.
    pub agent_dirs: Vec<PathBuf>,
    /// MCP servers spawned (or connected to) while this mode is
    /// active. The `transport` field on each config (stdio vs http) is
    /// what makes a bundled-in-process vs remote/enterprise server
    /// swap transparent to the registry.
    pub mcp_servers: Vec<McpServerConfig>,
    /// In-process tools contributed by the pack. Registered into the
    /// agent's `ToolRegistry` at REPL startup alongside builtins.
    /// Use this for fast iteration when the swap-to-remote story
    /// isn't yet needed; promote to an `McpServerConfig` later when
    /// the same tools need to be reachable from outside thClaws.
    pub tools: Vec<Arc<dyn Tool>>,
}

impl std::fmt::Debug for VerticalPackResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerticalPackResources")
            .field("skill_dirs", &self.skill_dirs)
            .field("command_dirs", &self.command_dirs)
            .field("agent_dirs", &self.agent_dirs)
            .field("mcp_servers", &self.mcp_servers.len())
            .field(
                "tools",
                &self.tools.iter().map(|t| t.name()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Declarative UI panel description. The Rust side never renders
/// panels; it only tells the frontend what to mount via IPC.
#[derive(Debug, Clone)]
pub struct UiPanel {
    /// Stable, hierarchical ID — e.g. `"gamedev.asset-browser"`.
    pub id: String,
    /// Human-readable tab/panel title.
    pub title: String,
    /// URL the frontend should load. Either an in-app scheme
    /// (e.g. `"thclaws://vertical/gamedev/asset-browser"`) or
    /// `https://` for a hosted panel.
    pub content_url: String,
    /// Hint to the frontend about where to mount the panel.
    pub mount: UiPanelMount,
}

/// Mount slots the frontend understands. New variants must be added
/// in lockstep with the frontend's panel shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiPanelMount {
    SidebarLeft,
    SidebarRight,
    Bottom,
    /// Floating, user-positionable window.
    Floating,
}

/// Registry of available packs + the currently active mode name.
///
/// Construct directly in tests; production code goes through the
/// process-global accessor functions below (`register_pack`,
/// `enter_mode`, etc.) so every caller sees the same state.
#[derive(Default)]
pub struct VerticalPackRegistry {
    packs: HashMap<String, Arc<dyn VerticalPack>>,
    active: Option<String>,
}

impl std::fmt::Debug for VerticalPackRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerticalPackRegistry")
            .field("packs", &self.packs.keys().collect::<Vec<_>>())
            .field("active", &self.active)
            .finish()
    }
}

impl VerticalPackRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a pack. Re-registering the same `mode_name` replaces the
    /// prior entry; this is intentional so a test or plugin can
    /// shadow a built-in pack without an unregister dance.
    pub fn register(&mut self, pack: Arc<dyn VerticalPack>) {
        let name = pack.mode_name().to_string();
        self.packs.insert(name, pack);
    }

    pub fn get(&self, mode_name: &str) -> Option<Arc<dyn VerticalPack>> {
        self.packs.get(mode_name).cloned()
    }

    /// List `(mode_name, description)` for every registered pack,
    /// sorted by `mode_name` for stable UI output.
    pub fn list(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .packs
            .values()
            .map(|p| (p.mode_name().to_string(), p.description().to_string()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn active(&self) -> Option<Arc<dyn VerticalPack>> {
        self.active.as_deref().and_then(|n| self.get(n))
    }

    pub fn active_name(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Transition to the named mode. The previously-active pack (if
    /// any) gets `on_exit`; the new pack gets `on_enter`. If
    /// `on_enter` errors, neither pack is active afterward — the user
    /// sees a clean "no mode" state rather than the prior mode still
    /// being half-attached.
    pub fn enter(&mut self, mode_name: &str) -> Result<()> {
        let pack = self
            .get(mode_name)
            .ok_or_else(|| Error::Config(format!("vertical pack not registered: {mode_name}")))?;
        if let Some(prev) = self.active() {
            // Exit hook errors are non-fatal — log and continue, so a
            // broken exit can't strand the user in a mode.
            if let Err(e) = prev.on_exit() {
                eprintln!("[vertical] pack {} on_exit failed: {e}", prev.mode_name());
            }
        }
        self.active = None;
        pack.on_enter()?;
        self.active = Some(mode_name.to_string());
        Ok(())
    }

    /// Leave the current mode. No-op if none is active.
    pub fn exit(&mut self) -> Result<()> {
        if let Some(prev) = self.active() {
            if let Err(e) = prev.on_exit() {
                eprintln!("[vertical] pack {} on_exit failed: {e}", prev.mode_name());
            }
        }
        self.active = None;
        Ok(())
    }
}

// ── Process-global accessor ───────────────────────────────────────────
//
// We model registration like `plugins`: a single process-wide registry
// that every caller queries. Packs are Rust trait objects (not on-disk
// data), so they must be `register_pack`'d at startup before they're
// reachable.

fn global() -> &'static RwLock<VerticalPackRegistry> {
    static GLOBAL: OnceLock<RwLock<VerticalPackRegistry>> = OnceLock::new();
    GLOBAL.get_or_init(|| RwLock::new(VerticalPackRegistry::new()))
}

pub fn register_pack(pack: Arc<dyn VerticalPack>) {
    global()
        .write()
        .expect("vertical pack registry poisoned")
        .register(pack);
}

pub fn enter_mode(mode_name: &str) -> Result<()> {
    global()
        .write()
        .expect("vertical pack registry poisoned")
        .enter(mode_name)
}

pub fn exit_mode() -> Result<()> {
    global()
        .write()
        .expect("vertical pack registry poisoned")
        .exit()
}

pub fn active_mode_name() -> Option<String> {
    global()
        .read()
        .expect("vertical pack registry poisoned")
        .active_name()
        .map(str::to_string)
}

pub fn list_modes() -> Vec<(String, String)> {
    global()
        .read()
        .expect("vertical pack registry poisoned")
        .list()
}

/// Returns the system-prompt fragment of the active pack, if any.
/// The agent appends this under a `## Mode: <name>` heading; here we
/// return only the raw body so the agent owns the framing.
pub fn vertical_pack_system_prompt() -> Option<String> {
    let g = global().read().expect("vertical pack registry poisoned");
    g.active().map(|p| p.system_prompt())
}

pub fn vertical_pack_skill_dirs() -> Vec<PathBuf> {
    let g = global().read().expect("vertical pack registry poisoned");
    g.active()
        .map(|p| p.resources().skill_dirs)
        .unwrap_or_default()
}

pub fn vertical_pack_command_dirs() -> Vec<PathBuf> {
    let g = global().read().expect("vertical pack registry poisoned");
    g.active()
        .map(|p| p.resources().command_dirs)
        .unwrap_or_default()
}

pub fn vertical_pack_agent_dirs() -> Vec<PathBuf> {
    let g = global().read().expect("vertical pack registry poisoned");
    g.active()
        .map(|p| p.resources().agent_dirs)
        .unwrap_or_default()
}

pub fn vertical_pack_mcp_servers() -> Vec<McpServerConfig> {
    let g = global().read().expect("vertical pack registry poisoned");
    g.active()
        .map(|p| p.resources().mcp_servers)
        .unwrap_or_default()
}

pub fn vertical_pack_ui_panels() -> Vec<UiPanel> {
    let g = global().read().expect("vertical pack registry poisoned");
    g.active().map(|p| p.ui_panels()).unwrap_or_default()
}

/// Tools contributed by the currently active pack. Returns an empty
/// Vec when no mode is active. The REPL folds these into the
/// `ToolRegistry` at startup, parallel to the MCP server merge.
pub fn vertical_pack_tools() -> Vec<Arc<dyn Tool>> {
    let g = global().read().expect("vertical pack registry poisoned");
    g.active().map(|p| p.resources().tools).unwrap_or_default()
}

/// Register the first-party vertical packs that ship with the binary.
///
/// Called once at startup by every entry point that wants vertical
/// modes available (REPL, GUI, server). Safe to call multiple times:
/// re-registration replaces the prior entry by `mode_name`, so a test
/// can re-register without an unregister step.
///
/// If `THCLAWS_GAMEDEV_LOCAL` is set we also auto-enter `gamedev` mode
/// — the env var is an explicit signal that the user wants the pro
/// pack live, and forcing them to also type `/gamedev` would be
/// redundant. `/mode exit` still works to drop back to the OSS shell.
pub fn register_builtin_packs() {
    register_pack(Arc::new(gamedev::GameDevPack::new()));
    if std::env::var(gamedev_tools::ENV_VAR)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
    {
        if let Err(e) = enter_mode("gamedev") {
            eprintln!("[vertical] auto-enter gamedev failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct StubPack {
        name: &'static str,
        desc: &'static str,
        prompt: &'static str,
        enters: Arc<AtomicU32>,
        exits: Arc<AtomicU32>,
        enter_should_fail: bool,
    }

    impl StubPack {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                desc: "stub pack for tests",
                prompt: "stub prompt",
                enters: Arc::new(AtomicU32::new(0)),
                exits: Arc::new(AtomicU32::new(0)),
                enter_should_fail: false,
            }
        }
    }

    impl VerticalPack for StubPack {
        fn mode_name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            self.desc
        }
        fn system_prompt(&self) -> String {
            self.prompt.to_string()
        }
        fn resources(&self) -> VerticalPackResources {
            VerticalPackResources {
                skill_dirs: vec![PathBuf::from(format!("/fake/{}/skills", self.name))],
                command_dirs: vec![PathBuf::from(format!("/fake/{}/cmds", self.name))],
                agent_dirs: vec![],
                mcp_servers: vec![],
                tools: vec![],
            }
        }
        fn on_enter(&self) -> Result<()> {
            self.enters.fetch_add(1, Ordering::SeqCst);
            if self.enter_should_fail {
                Err(Error::Config("boom".into()))
            } else {
                Ok(())
            }
        }
        fn on_exit(&self) -> Result<()> {
            self.exits.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn register_and_list_is_sorted() {
        let mut reg = VerticalPackRegistry::new();
        reg.register(Arc::new(StubPack::new("zeta")));
        reg.register(Arc::new(StubPack::new("alpha")));
        reg.register(Arc::new(StubPack::new("mu")));
        let list: Vec<_> = reg.list().into_iter().map(|(n, _)| n).collect();
        assert_eq!(list, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn enter_calls_on_enter_and_sets_active() {
        let pack = Arc::new(StubPack::new("gamedev"));
        let enters = pack.enters.clone();
        let mut reg = VerticalPackRegistry::new();
        reg.register(pack);
        reg.enter("gamedev").unwrap();
        assert_eq!(reg.active_name(), Some("gamedev"));
        assert_eq!(enters.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn enter_unknown_mode_errors_and_leaves_state_untouched() {
        let mut reg = VerticalPackRegistry::new();
        reg.register(Arc::new(StubPack::new("gamedev")));
        reg.enter("gamedev").unwrap();
        let res = reg.enter("does-not-exist");
        assert!(res.is_err());
        assert_eq!(reg.active_name(), Some("gamedev"));
    }

    #[test]
    fn entering_new_mode_exits_prior() {
        let a = Arc::new(StubPack::new("a"));
        let b = Arc::new(StubPack::new("b"));
        let a_exits = a.exits.clone();
        let b_enters = b.enters.clone();
        let mut reg = VerticalPackRegistry::new();
        reg.register(a);
        reg.register(b);
        reg.enter("a").unwrap();
        reg.enter("b").unwrap();
        assert_eq!(reg.active_name(), Some("b"));
        assert_eq!(a_exits.load(Ordering::SeqCst), 1);
        assert_eq!(b_enters.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failing_on_enter_leaves_no_active_mode() {
        let mut bad = StubPack::new("bad");
        bad.enter_should_fail = true;
        let mut reg = VerticalPackRegistry::new();
        reg.register(Arc::new(StubPack::new("good")));
        reg.register(Arc::new(bad));
        reg.enter("good").unwrap();
        let res = reg.enter("bad");
        assert!(res.is_err());
        assert_eq!(reg.active_name(), None);
    }

    #[test]
    fn exit_clears_active() {
        let mut reg = VerticalPackRegistry::new();
        reg.register(Arc::new(StubPack::new("gamedev")));
        reg.enter("gamedev").unwrap();
        reg.exit().unwrap();
        assert_eq!(reg.active_name(), None);
    }

    #[test]
    fn re_registering_replaces_prior_pack() {
        let mut reg = VerticalPackRegistry::new();
        let first = StubPack::new("gamedev");
        let mut second = StubPack::new("gamedev");
        second.desc = "second";
        reg.register(Arc::new(first));
        reg.register(Arc::new(second));
        let entry = reg.get("gamedev").unwrap();
        assert_eq!(entry.description(), "second");
    }

    #[test]
    fn resources_flatten_when_active_and_empty_otherwise() {
        let mut reg = VerticalPackRegistry::new();
        reg.register(Arc::new(StubPack::new("gamedev")));
        assert!(reg.active().is_none());
        reg.enter("gamedev").unwrap();
        let res = reg.active().unwrap().resources();
        assert_eq!(res.skill_dirs.len(), 1);
        assert_eq!(res.command_dirs.len(), 1);
    }
}
