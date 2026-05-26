//! Curated reference-game launcher for the `/games` and `/game` slash
//! commands.
//!
//! The list is intentionally hardcoded — the user asked for a small
//! curated showcase, not the full 50+ reference catalog from gamedev-mcp.
//! `/game <name>` calls the `GamedevPlayReference` MCP tool which serves
//! the reference's source directly from the gamedev-mcp Docker image's
//! read-only layer (no clone into the user's workspace), so the source
//! is unmodifiable during play.

use crate::error::{Error, Result};
use crate::mcp::MCP_NAME_SEPARATOR;
use crate::tools::{ToolRegistry, UiResource};

/// One entry in the curated showcase list. `display_name` is what the
/// user types after `/game` AND the name forwarded to the MCP tool —
/// gamedev-mcp resolves it against its reference index. `reference_id`
/// is recorded for documentation only; we don't translate names here.
pub struct CuratedGame {
    pub display_name: &'static str,
    pub reference_id: &'static str,
    pub description: &'static str,
}

pub const CURATED_GAMES: &[CuratedGame] = &[
    CuratedGame {
        display_name: "ThaiChecker",
        reference_id: "thai-checker-game",
        description: "หมากฮอสไทย — เริ่ม 2 แถว เดินทแยง กินกระโดด",
    },
    CuratedGame {
        display_name: "Othello",
        reference_id: "othello-game",
        description: "เกมพลิกหมาก 8×8 — กลยุทธ์ครองมุมและขอบ",
    },
    CuratedGame {
        display_name: "Chess",
        reference_id: "chess-game",
        description: "หมากรุกสากล — ครบทั้ง 6 ตัวหมาก",
    },
    CuratedGame {
        display_name: "IsometricMatch3",
        reference_id: "IsometricMatch3",
        description: "จับคู่สามชิ้นในมุมมอง isometric",
    },
    CuratedGame {
        display_name: "VeggieMerge",
        reference_id: "VeggieMerge",
        description: "ปลูก–รวม–เก็บเกี่ยวผัก เพิ่ม tier ตามการ merge",
    },
];

const PLAY_TOOL_BARE: &str = "GamedevPlayReference";

/// Render the `/games` output: a short header plus one line per curated
/// game with its description. Same plain-text shape `/help` and `/cost`
/// emit — the CLI prints it directly, the GUI passes it through
/// `ViewEvent::SlashOutput`.
pub fn render_games_list() -> String {
    let mut out = String::from("Playable reference games:\n");
    let name_width = CURATED_GAMES
        .iter()
        .map(|g| g.display_name.chars().count())
        .max()
        .unwrap_or(0);
    for g in CURATED_GAMES {
        out.push_str(&format!(
            "  /game {:<width$}  {}\n",
            g.display_name,
            g.description,
            width = name_width
        ));
    }
    out.push_str("\nGames are served read-only from the gamedev-mcp \
                  reference library — they cannot be modified during play.");
    out
}

/// Outcome of a successful `/game` dispatch: the qualified MCP tool
/// name that produced the iframe (the GUI passes this through as the
/// `name` field on the synthetic ToolCallStart/Result events so the
/// frontend's McpAppIframe can derive the right serverPrefix for
/// widget→host tool calls), the tool's text output, and the resolved
/// UI resource (when the server is trusted).
#[derive(Debug)]
pub struct PlayOutcome {
    pub qualified_tool_name: String,
    pub output: String,
    pub ui_resource: Option<UiResource>,
}

/// Look up the gamedev-mcp `GamedevPlayReference` tool in `registry`
/// (qualified name = `<sanitized_server>__GamedevPlayReference`) and
/// invoke it for `display_name`. Returns a [`PlayOutcome`] so the GUI
/// dispatcher can emit a synthetic ToolCallResult event with both the
/// iframe widget and the qualified name attached, and the CLI can
/// print just the text.
///
/// Errors are user-facing: unknown game name, MCP not connected,
/// gamedev-mcp's own tool error (game not found in references, etc.).
pub async fn play(
    registry: &ToolRegistry,
    display_name: &str,
) -> Result<PlayOutcome> {
    let game = CURATED_GAMES
        .iter()
        .find(|g| g.display_name.eq_ignore_ascii_case(display_name))
        .ok_or_else(|| {
            Error::Tool(format!(
                "unknown game '{display_name}'. Available: {}",
                CURATED_GAMES
                    .iter()
                    .map(|g| g.display_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;

    // Find the qualified tool name. MCP tools are registered as
    // `<sanitized_server>__GamedevPlayReference`; we don't know the
    // server's sanitized name a priori (the user could mount gamedev-mcp
    // under any alias in mcp.json), so match by suffix.
    let suffix = format!("{MCP_NAME_SEPARATOR}{PLAY_TOOL_BARE}");
    let qualified: String = registry
        .names()
        .into_iter()
        .find(|n| n.ends_with(&suffix))
        .ok_or_else(|| {
            Error::Tool(
                "gamedev MCP not connected — expected a tool ending in \
                 '__GamedevPlayReference'. Make sure the gamedev-mcp \
                 server is registered in mcp.json and running."
                    .into(),
            )
        })?
        .to_string();

    let tool = registry
        .get(&qualified)
        .ok_or_else(|| Error::Tool(format!("tool '{qualified}' vanished mid-lookup")))?;

    // Send the canonical reference id (kebab-case for board games,
    // PascalCase for the engine-style ones) — the user-facing display
    // name may differ (e.g. "Chess" → "chess-game", "ThaiChecker" →
    // "thai-checker-game"). gamedev-mcp resolves by exact directory
    // name, not by case-insensitive match.
    let args = serde_json::json!({ "name": game.reference_id });
    let output = tool.call(args).await?;
    let ui_resource = tool.fetch_ui_resource().await;
    Ok(PlayOutcome {
        qualified_tool_name: qualified,
        output,
        ui_resource,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_list_has_all_five_games() {
        let names: Vec<_> = CURATED_GAMES.iter().map(|g| g.display_name).collect();
        assert_eq!(
            names,
            ["ThaiChecker", "Othello", "Chess", "IsometricMatch3", "VeggieMerge"]
        );
    }

    #[test]
    fn render_lists_every_game_and_includes_readonly_notice() {
        let s = render_games_list();
        for g in CURATED_GAMES {
            assert!(s.contains(g.display_name), "missing {}", g.display_name);
            assert!(s.contains(g.description), "missing desc for {}", g.display_name);
        }
        assert!(s.contains("read-only"));
    }

    #[tokio::test]
    async fn play_rejects_unknown_game() {
        let registry = ToolRegistry::new();
        let err = play(&registry, "NoSuchGame").await.unwrap_err();
        assert!(err.to_string().contains("unknown game"));
    }

    #[tokio::test]
    async fn play_reports_missing_mcp_when_registry_lacks_tool() {
        let registry = ToolRegistry::new();
        let err = play(&registry, "Othello").await.unwrap_err();
        assert!(err.to_string().contains("gamedev MCP not connected"));
    }
}
