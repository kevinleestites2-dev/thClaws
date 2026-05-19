/**
 * Skills surface for the thClaws adapter.
 *
 * thClaws scans skill manifests from (per crates/core/src/skills.rs):
 *   - ~/.claude/skills/           (user Claude Code — compat path)
 *   - ~/.config/thclaws/skills/   (user thClaws)
 *   - .claude/skills/             (project Claude Code)
 *   - .thclaws/skills/            (project thClaws — highest priority)
 *
 * For v1, listSkills + syncSkills return a `mode: "external"` snapshot:
 * the customer manages their skill files inside the thClaws subprocess
 * (or pod) directly via thClaws's own settings UI / file editing.
 * Paperclip just declares that skills ARE supported so the UI shows the
 * Skills tab (capability flag) — actual sync semantics get richer in
 * later versions when we wire syncSkills to materialize Paperclip-managed
 * skills into `.thclaws/skills/`.
 *
 * Matches claude-local's "trust the host filesystem" approach.
 */

import type {
  AdapterSkillContext,
  AdapterSkillSnapshot,
} from "@paperclipai/adapter-utils";

const ADAPTER_TYPE = "thclaws_local";

function emptySnapshot(): AdapterSkillSnapshot {
  return {
    adapterType: ADAPTER_TYPE,
    supported: true,
    // "ephemeral" because the customer manages the underlying skill
    // files inside the thClaws subprocess directly — Paperclip doesn't
    // persist them. "persistent" would imply Paperclip-managed lifecycle.
    mode: "ephemeral",
    desiredSkills: [],
    entries: [],
    warnings: [],
  };
}

export async function listSkills(_ctx: AdapterSkillContext): Promise<AdapterSkillSnapshot> {
  return emptySnapshot();
}

export async function syncSkills(
  _ctx: AdapterSkillContext,
  _desiredSkills: string[],
): Promise<AdapterSkillSnapshot> {
  return emptySnapshot();
}
