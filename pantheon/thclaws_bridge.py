#!/usr/bin/env python3
"""
Agent Zero — thClaws Execution Harness Bridge
Layer 20: Wires thClaws as Agent Zero's native execution engine.

thClaws handles:
  - Subagent spawning + parallel execution
  - Dynamic workflow orchestration (LLM-authored JS, Boa sandbox)
  - Tool use, memory, coding tasks
  - Terminal + desktop native (Rust binary, sovereign)

This bridge lets Agent Zero delegate execution tasks to thClaws
and receive structured results back into the Pantheon signal format.

Usage:
    python thclaws_bridge.py run "<task>"
    python thclaws_bridge.py workflow "<prompt>"
    python thclaws_bridge.py status
"""

import os
import sys
import json
import subprocess
import shutil
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional


# ─── CONFIG ──────────────────────────────────────────────────────────────────

THCLAWS_REPO    = "kevinleestites2-dev/thClaws"
THCLAWS_BIN     = os.environ.get("THCLAWS_BIN", "thclaws")   # path to binary after install
TELEGRAM_TOKEN  = os.environ.get("TELEGRAM_BOT_TOKEN", "8679655550:AAGUB1m5fmqHc8OHqqM24Vixz8FfwX-gqD4")
TELEGRAM_CHAT   = os.environ.get("TELEGRAM_CHAT_ID", "7135054241")


# ─── INSTALL / VERIFY ────────────────────────────────────────────────────────

def is_installed() -> bool:
    return shutil.which(THCLAWS_BIN) is not None


def install_from_release() -> bool:
    """
    Download the latest thClaws binary from GitHub releases.
    Detects platform and pulls the right asset.
    """
    import platform
    system = platform.system().lower()
    machine = platform.machine().lower()

    # Map to release asset names (from thClaws release naming convention)
    if system == "linux" and "aarch64" in machine:
        asset_pattern = "aarch64-unknown-linux"
    elif system == "linux":
        asset_pattern = "x86_64-unknown-linux"
    elif system == "darwin":
        asset_pattern = "aarch64-apple-darwin" if "arm" in machine else "x86_64-apple-darwin"
    elif system == "windows":
        asset_pattern = "x86_64-pc-windows"
    else:
        print(f"[thClaws Bridge] Unknown platform: {system}/{machine}")
        return False

    print(f"[thClaws Bridge] Fetching latest release for {asset_pattern}...")
    url = "https://api.github.com/repos/thClaws/thClaws/releases/latest"
    req = urllib.request.Request(url, headers={"Accept": "application/vnd.github.v3+json"})
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            release = json.loads(r.read())
    except Exception as e:
        print(f"[thClaws Bridge] Could not fetch release: {e}")
        return False

    # Find matching asset
    asset = next(
        (a for a in release.get("assets", []) if asset_pattern in a["name"]),
        None
    )
    if not asset:
        print(f"[thClaws Bridge] No asset found matching '{asset_pattern}'")
        print(f"  Available: {[a['name'] for a in release.get('assets', [])]}")
        return False

    download_url = asset["browser_download_url"]
    print(f"[thClaws Bridge] Downloading: {asset['name']} ({asset['size']//1024}KB)")

    dest = Path.home() / ".local" / "bin" / "thclaws"
    dest.parent.mkdir(parents=True, exist_ok=True)

    try:
        urllib.request.urlretrieve(download_url, dest)
        dest.chmod(0o755)
        print(f"[thClaws Bridge] Installed to {dest}")
        return True
    except Exception as e:
        print(f"[thClaws Bridge] Download failed: {e}")
        return False


def build_from_source() -> bool:
    """
    Fallback: build thClaws from the Pantheon fork using cargo.
    Requires Rust + cargo to be available.
    """
    if not shutil.which("cargo"):
        print("[thClaws Bridge] cargo not found — install Rust first")
        return False

    fork_dir = Path.home() / "pantheon" / "thClaws"
    if not fork_dir.exists():
        print(f"[thClaws Bridge] Cloning fork...")
        result = subprocess.run(
            ["git", "clone",
             "https://github.com/kevinleestites2-dev/thClaws.git",
             str(fork_dir)],
            capture_output=True, text=True
        )
        if result.returncode != 0:
            print(f"[thClaws Bridge] Clone failed: {result.stderr}")
            return False

    print("[thClaws Bridge] Building from source (this takes a few minutes)...")
    result = subprocess.run(
        ["cargo", "build", "--release"],
        cwd=fork_dir,
        capture_output=True, text=True
    )
    if result.returncode != 0:
        print(f"[thClaws Bridge] Build failed: {result.stderr[-500:]}")
        return False

    binary = fork_dir / "target" / "release" / "thclaws"
    dest = Path.home() / ".local" / "bin" / "thclaws"
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(binary, dest)
    dest.chmod(0o755)
    print(f"[thClaws Bridge] Built and installed to {dest}")
    return True


def setup() -> bool:
    """Ensure thClaws is available. Try release binary first, then source."""
    if is_installed():
        print(f"[thClaws Bridge] ✅ Binary found: {shutil.which(THCLAWS_BIN)}")
        return True

    print("[thClaws Bridge] Binary not found — attempting install...")
    if install_from_release():
        return True
    print("[thClaws Bridge] Release install failed — trying source build...")
    return build_from_source()


# ─── EXECUTION ───────────────────────────────────────────────────────────────

def run_task(task: str, timeout: int = 120) -> Dict:
    """
    Run a single task through thClaws.
    Returns a Pantheon signal dict with the result.
    """
    if not is_installed():
        return {"error": "thClaws not installed — run setup() first", "status": "failed"}

    print(f"[thClaws Bridge] Running task: {task[:80]}...")
    try:
        result = subprocess.run(
            [THCLAWS_BIN, "run", task],
            capture_output=True,
            text=True,
            timeout=timeout
        )
        return to_pantheon_signal({
            "task":      task,
            "stdout":    result.stdout,
            "stderr":    result.stderr,
            "exit_code": result.returncode,
            "status":    "ok" if result.returncode == 0 else "error",
        })
    except subprocess.TimeoutExpired:
        return {"error": f"Task timed out after {timeout}s", "status": "timeout"}
    except Exception as e:
        return {"error": str(e), "status": "failed"}


def run_workflow(prompt: str, auto_approve: bool = False, timeout: int = 300) -> Dict:
    """
    Trigger thClaws dynamic workflow mode.
    LLM authors a JS orchestration script → Boa sandbox executes it across subagents.

    :param prompt:       Natural language task description
    :param auto_approve: If True, skip the human review step (use carefully)
    :param timeout:      Max seconds to wait
    """
    if not is_installed():
        return {"error": "thClaws not installed", "status": "failed"}

    cmd = [THCLAWS_BIN, "workflow", "run", prompt]
    if auto_approve:
        cmd.append("--yes")   # skip interactive approval

    print(f"[thClaws Bridge] Workflow: {prompt[:80]}...")
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout
        )
        return to_pantheon_signal({
            "mode":      "workflow",
            "prompt":    prompt,
            "stdout":    result.stdout,
            "stderr":    result.stderr,
            "exit_code": result.returncode,
            "status":    "ok" if result.returncode == 0 else "error",
        })
    except subprocess.TimeoutExpired:
        return {"error": f"Workflow timed out after {timeout}s", "status": "timeout"}
    except Exception as e:
        return {"error": str(e), "status": "failed"}


def spawn_subagents(tasks: List[str], timeout: int = 300) -> List[Dict]:
    """
    Fan out multiple tasks across parallel thClaws subagents.
    Returns a list of Pantheon signal dicts, one per task.
    """
    import concurrent.futures
    print(f"[thClaws Bridge] Spawning {len(tasks)} parallel subagents...")

    def run_one(task):
        return run_task(task, timeout=timeout)

    with concurrent.futures.ThreadPoolExecutor(max_workers=len(tasks)) as ex:
        futures = {ex.submit(run_one, t): t for t in tasks}
        results = []
        for future in concurrent.futures.as_completed(futures):
            results.append(future.result())

    return results


def status() -> Dict:
    """Check thClaws installation status and version."""
    installed = is_installed()
    version = None
    if installed:
        try:
            r = subprocess.run([THCLAWS_BIN, "--version"], capture_output=True, text=True, timeout=5)
            version = r.stdout.strip()
        except Exception:
            pass
    return {
        "installed":  installed,
        "binary":     shutil.which(THCLAWS_BIN),
        "version":    version,
        "fork":       f"https://github.com/{THCLAWS_REPO}",
        "upstream":   "https://github.com/thClaws/thClaws",
    }


# ─── PANTHEON BRIDGE ─────────────────────────────────────────────────────────

def to_pantheon_signal(raw: Any) -> Dict:
    """Normalize thClaws output into a standard Pantheon signal dict."""
    return {
        "source":    "thClaws",
        "layer":     "execution_harness",
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "data":      raw,
    }


def relay_to_telegram(message: str) -> None:
    """Send a thClaws status update to the Pantheon Telegram channel."""
    payload = {
        "chat_id":    TELEGRAM_CHAT,
        "text":       f"[thClaws] {message}",
        "parse_mode": "Markdown"
    }
    data = json.dumps(payload).encode()
    req = urllib.request.Request(
        f"https://api.telegram.org/bot{TELEGRAM_TOKEN}/sendMessage",
        data=data,
        headers={"Content-Type": "application/json"}
    )
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            return json.loads(r.read())
    except Exception as e:
        print(f"[thClaws Bridge] Telegram relay failed: {e}")


# ─── AGENT ZERO WIRING ───────────────────────────────────────────────────────

class ThClawsHarness:
    """
    Agent Zero's execution harness interface.
    Drop this into any Prime that needs to run tasks, workflows, or subagents.

    Example:
        harness = ThClawsHarness()
        result = harness.execute("Summarize all .rs files in the repo")
        harness.fanout(["task A", "task B", "task C"])
    """

    def __init__(self, auto_setup: bool = True):
        if auto_setup and not is_installed():
            setup()

    def execute(self, task: str, timeout: int = 120) -> Dict:
        """Single task execution."""
        result = run_task(task, timeout)
        relay_to_telegram(f"Task complete: `{task[:60]}`\nStatus: {result.get('data', {}).get('status', '?')}")
        return result

    def workflow(self, prompt: str, auto_approve: bool = False) -> Dict:
        """LLM-authored workflow execution."""
        result = run_workflow(prompt, auto_approve)
        relay_to_telegram(f"Workflow complete: `{prompt[:60]}`")
        return result

    def fanout(self, tasks: List[str]) -> List[Dict]:
        """Parallel subagent execution."""
        results = spawn_subagents(tasks)
        relay_to_telegram(f"Fanout: {len(tasks)} tasks complete — {sum(1 for r in results if r.get('data', {}).get('status') == 'ok')} OK")
        return results

    def health(self) -> Dict:
        return status()


# ─── CLI ─────────────────────────────────────────────────────────────────────

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(0)

    cmd = sys.argv[1]

    if cmd == "status":
        print(json.dumps(status(), indent=2))

    elif cmd == "setup":
        ok = setup()
        print("✅ Ready" if ok else "❌ Setup failed")

    elif cmd == "run":
        if len(sys.argv) < 3:
            print("Usage: thclaws_bridge.py run \"<task>\"")
            sys.exit(1)
        result = run_task(sys.argv[2])
        print(json.dumps(result, indent=2))

    elif cmd == "workflow":
        if len(sys.argv) < 3:
            print("Usage: thclaws_bridge.py workflow \"<prompt>\"")
            sys.exit(1)
        result = run_workflow(sys.argv[2])
        print(json.dumps(result, indent=2))

    else:
        print(f"Unknown command: {cmd}")
        sys.exit(1)
