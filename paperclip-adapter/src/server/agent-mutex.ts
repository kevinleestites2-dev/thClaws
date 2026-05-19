/**
 * Per-agent in-process mutex.
 *
 * dev-plan/25 Phase D: even though thcompany's heartbeat already
 * serializes runs of the same agent via withAgentStartLock +
 * maxConcurrentRuns, the daemon process can host multiple adapter
 * instances (different agents) concurrently. When agent A and agent B
 * both materialize their own skill sets into their own workspace dirs
 * we're fine — different paths. But if the same agent's
 * maxConcurrentRuns goes >1, the materialize → POST window must
 * serialize so the second run doesn't write skills before the first
 * has issued the request.
 *
 * Scope is the adapter package's lifetime (in-process Map). Multiple
 * thcompany pods don't share this — that's heartbeat's
 * `FOR UPDATE SKIP LOCKED` job.
 */

type Resolver = () => void;

const queues: Map<string, Promise<void>> = new Map();

/**
 * Acquire the per-agent lock. Returns a release function. Callers
 * MUST call release in a finally block — leaking the lock wedges
 * every future run for the same agent.
 */
export async function acquireAgentLock(agentId: string): Promise<Resolver> {
  // Chain onto whatever's currently queued for this agent (or a
  // resolved promise if nothing is). The chain's tail becomes the new
  // "current"; the next acquire chains onto us.
  const previous = queues.get(agentId) ?? Promise.resolve();
  let release!: Resolver;
  const next = new Promise<void>((resolve) => {
    release = () => {
      // Drop ourselves from the map if we're still the tail. (If
      // someone chained behind us, they're the tail now — leave them.)
      if (queues.get(agentId) === next) queues.delete(agentId);
      resolve();
    };
  });
  queues.set(agentId, previous.then(() => next));
  // Wait for our turn.
  await previous;
  return release;
}

/** Test-only: drop all queued promises. */
export function _resetAgentLocksForTests(): void {
  queues.clear();
}
