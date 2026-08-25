const EPS = 1e-9;

export interface CostPoint { agentId: string; sessionId: string; usd: number }

/** SQL-backed lifetime totals adapter. PostgreSQL is the only durable authority. */
export class CostLedgerTotals {
  private totals = new Map<string, number>();
  private warm = false;
  private refreshing = false;

  constructor(private readonly load: () => Promise<Map<string, number>>) {}

  usdFor(agentId: string): number | null { return this.warm ? (this.totals.get(agentId) ?? 0) : null; }
  all(): Map<string, number> { return new Map(this.totals); }
  get ready(): boolean { return this.warm; }
  floorTotal(): number { return [...this.totals.values()].reduce((sum, value) => sum + value, 0); }

  async refresh(): Promise<void> {
    if (this.refreshing) return;
    this.refreshing = true;
    try { this.totals = await this.load(); this.warm = true; }
    catch { /* retain the last complete totals */ }
    finally { this.refreshing = false; }
  }
}

/** Pure reference fold used by the 12 reset/agent/session behavior vectors. */
export function lifetimeUsdFromRows(rows: readonly CostPoint[]): Map<string, number> {
  const segments = new Map<string, { committed: number; peak: number }>();
  for (const row of rows) {
    const usd = Number.isFinite(row.usd) && row.usd >= 0 ? row.usd : 0;
    const key = `${row.agentId}\t${row.sessionId}`;
    const state = segments.get(key) ?? { committed: 0, peak: 0 };
    if (usd < state.peak - EPS) { state.committed += state.peak; state.peak = usd; }
    else if (usd > state.peak) state.peak = usd;
    segments.set(key, state);
  }
  const totals = new Map<string, number>();
  for (const [key, state] of segments) {
    const agentId = key.slice(0, key.indexOf('\t'));
    totals.set(agentId, (totals.get(agentId) ?? 0) + state.committed + state.peak);
  }
  return totals;
}
