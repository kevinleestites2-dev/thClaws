# PANTHEON.md

This is the Pantheon fork of thClaws (https://github.com/thClaws/thClaws).

## Role in the Pantheon

thClaws is Agent Zero's Execution Harness.

Agent Zero's intelligence layers (SAFLA, FluxPrime, Layer 20 absorber) decide WHAT to do.
thClaws is HOW it gets done — the sovereign, Rust-native engine that runs the work.

## Integration

Bridge file: pantheon/thclaws_bridge.py

    from thclaws_bridge import ThClawsHarness

    harness = ThClawsHarness()

    # Single task
    result = harness.execute("Summarize all .rs files in crates/core/src")

    # LLM-authored workflow (Boa sandbox)
    result = harness.workflow("Fan out analysis of every Prime in the Pantheon")

    # Parallel subagents
    results = harness.fanout(["task A", "task B", "task C"])

## Key Capabilities

| Capability         | thClaws Feature          | Pantheon Use                  |
|--------------------|--------------------------|-------------------------------|
| Task execution     | run "<task>"             | Prime delegation              |
| Parallel agents    | workflow run + Boa fanout| Multi-Prime coordination      |
| Code tasks         | Built-in code tools      | Self-modification, absorb     |
| Memory             | Native persistence       | Cross-session continuity      |
| Sovereign          | One binary, no cloud     | Ghost Operator compliance     |

## Upstream

Synced from: https://github.com/thClaws/thClaws
Fork owner: kevinleestites2-dev
