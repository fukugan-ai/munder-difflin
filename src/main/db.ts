import { randomUUID } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { Pool, types, type PoolClient, type PoolConfig, type QueryResult } from 'pg';
import type { AgentUsageSample } from './usage';

const SCHEMA = 'munder_difflin';
const SCHEMA_VERSION = 1;
types.setTypeParser(20, (value) => value);

export interface CommandHistoryRow {
  id: string;
  agentId: string;
  cwd: string | null;
  text: string;
  ts: number;
}

export type PersistStatus =
  | { state: 'closed' }
  | { state: 'ready'; writes: true }
  | { state: 'degraded'; writes: false; code: 'missing_config' | 'config_invalid' | 'unreachable' | 'schema_mismatch' | 'namespace_locked' | 'write_failed' };

interface PoolLike {
  query<T extends Record<string, unknown> = Record<string, unknown>>(text: string, values?: unknown[]): Promise<QueryResult<T>>;
  connect(): Promise<PoolClient>;
  end(): Promise<void>;
}

export interface PgRuntimeConfig { namespace: string; pool: PoolConfig; invalid?: boolean }
export interface PersistOptions { env?: NodeJS.ProcessEnv; pool?: PoolLike }

/** Reads only app-specific settings and removes the password before child spawn. */
export function consumePgConfig(env: NodeJS.ProcessEnv = process.env): PgRuntimeConfig | null {
  const password = env.MD_PG_PASSWORD;
  const host = env.MD_PG_HOST?.trim();
  const database = env.MD_PG_DATABASE?.trim();
  const user = env.MD_PG_USER?.trim();
  const namespace = env.MD_PG_NAMESPACE?.trim();
  const caPath = env.MD_PG_TLS_CA?.trim();
  const portValue = env.MD_PG_PORT;
  for (const key of Object.keys(env)) if (key.startsWith('MD_PG_')) delete env[key];
  if (!host || !database || !user || !password || !namespace || namespace.length > 128 || !/^[A-Za-z0-9._-]+$/.test(namespace)) return null;
  const port = Number(portValue ?? 5432);
  if (!Number.isInteger(port) || port < 1 || port > 65535) return null;
  const local = host === 'localhost' || host === '127.0.0.1' || host === '::1';
  if (!local && !caPath) return null;
  let ssl: PoolConfig['ssl'];
  try { ssl = local && !caPath ? false : { rejectUnauthorized: true, ca: caPath ? readFileSync(caPath, 'utf8') : undefined }; }
  catch { return { namespace, pool: {}, invalid: true }; }
  return { namespace, pool: {
    host, port, database, user, password, ssl, max: 4,
    connectionTimeoutMillis: 4_000, idleTimeoutMillis: 30_000,
    options: '-c statement_timeout=5000 -c lock_timeout=2000 -c search_path=pg_catalog'
  } };
}

function retryable(error: unknown): boolean {
  const code = (error as { code?: string } | null)?.code;
  return code === '40001' || code === '40P01' || code === 'ECONNRESET' || code === 'ETIMEDOUT' || code === 'EPIPE';
}

export function clampLimit(n: number | undefined, fallback: number): number {
  const value = Math.floor(Number(n));
  return Number.isFinite(value) && value > 0 ? Math.min(1000, value) : fallback;
}

export class PersistStore {
  private readonly config: PgRuntimeConfig | null;
  private pool: PoolLike | null;
  private lockClient: PoolClient | null = null;
  private cache = new Map<string, unknown>();
  private writeQueue: Promise<void> = Promise.resolve();
  private acceptingWrites = false;
  private queueFailed = false;
  private currentStatus: PersistStatus = { state: 'closed' };

  constructor(options: PersistOptions = {}) {
    this.config = consumePgConfig(options.env ?? process.env);
    this.pool = options.pool ?? (this.config && !this.config.invalid ? new Pool(this.config.pool) : null);
  }

  get status(): PersistStatus { return this.currentStatus; }
  get isOpen(): boolean { return this.currentStatus.state === 'ready'; }

  async open(): Promise<PersistStatus> {
    if (this.isOpen) return this.currentStatus;
    if (this.config?.invalid) return this.degrade('config_invalid');
    if (!this.config || !this.pool) return this.degrade('missing_config');
    try {
      const client = await this.pool.connect();
      const lock = await client.query<{ locked: boolean }>(
        'SELECT pg_try_advisory_lock(hashtext($1), hashtext($2)) AS locked', ['munder-difflin', this.config.namespace]
      );
      if (lock.rows[0]?.locked !== true) { client.release(); return this.degrade('namespace_locked'); }
      this.lockClient = client;
      const version = await this.pool.query<{ version: number }>(
        `SELECT COALESCE(MAX(version), 0)::int AS version FROM ${SCHEMA}.schema_migrations`
      );
      if (version.rows[0]?.version !== SCHEMA_VERSION) {
        await this.releaseLock();
        return this.degrade('schema_mismatch');
      }
      const kv = await this.pool.query<{ key: string; value: unknown }>(
        `SELECT key, value FROM ${SCHEMA}.kv WHERE namespace = $1`, [this.config.namespace]
      );
      this.cache = new Map(kv.rows.map((row) => [row.key, row.value]));
      this.currentStatus = { state: 'ready', writes: true };
      this.acceptingWrites = true;
      this.queueFailed = false;
      return this.currentStatus;
    } catch {
      await this.releaseLock();
      return this.degrade('unreachable');
    }
  }

  private degrade(code: Extract<PersistStatus, { state: 'degraded' }>['code']): PersistStatus {
    this.currentStatus = { state: 'degraded', writes: false, code };
    return this.currentStatus;
  }

  private async releaseLock(): Promise<void> {
    const client = this.lockClient;
    this.lockClient = null;
    if (!client || !this.config) return;
    try { await client.query('SELECT pg_advisory_unlock(hashtext($1), hashtext($2))', ['munder-difflin', this.config.namespace]); } catch { /* lost connection */ }
    client.release();
  }

  private destroyLockConnection(): void {
    const client = this.lockClient;
    this.lockClient = null;
    if (client) client.release(true);
  }

  private enqueue(operation: () => Promise<void>): boolean {
    if (!this.isOpen || !this.acceptingWrites || this.queueFailed) return false;
    this.writeQueue = this.writeQueue.then(operation).catch(() => {
      this.queueFailed = true;
      this.acceptingWrites = false;
      this.degrade('write_failed');
    });
    return true;
  }

  private async idempotentQuery(text: string, values: unknown[]): Promise<QueryResult<Record<string, unknown>>> {
    if (!this.pool) throw new Error('database unavailable');
    for (let attempt = 1; ; attempt++) {
      try { return await this.pool.query(text, values); }
      catch (error) { if (attempt >= 3 || !retryable(error)) throw error; }
    }
  }

  getKv<T = unknown>(key: string): T | undefined { return this.cache.get(key) as T | undefined; }

  setKv(key: string, value: unknown): boolean {
    if (!this.isOpen || !this.config || !key) return false;
    this.cache.set(key, value);
    const namespace = this.config.namespace;
    return this.enqueue(async () => { await this.idempotentQuery(
      `INSERT INTO ${SCHEMA}.kv(namespace,key,value,updated_at) VALUES($1,$2,$3::jsonb,now())
       ON CONFLICT(namespace,key) DO UPDATE SET value=EXCLUDED.value,updated_at=EXCLUDED.updated_at`,
      [namespace, key, JSON.stringify(value)]
    ); });
  }

  async addHistory(entry: { agentId: string; cwd?: string | null; text: string }): Promise<boolean> {
    const text = entry.text?.trim();
    if (!this.isOpen || !this.config || !entry.agentId || !text) return false;
    const namespace = this.config.namespace;
    const eventId = randomUUID();
    let committed = false;
    const queued = this.enqueue(async () => { await this.idempotentQuery(
      `INSERT INTO ${SCHEMA}.command_history(namespace,event_id,agent_id,cwd,text,occurred_at)
       VALUES($1,$2,$3,$4,$5,now()) ON CONFLICT(namespace,event_id) DO NOTHING`,
      [namespace, eventId, entry.agentId, entry.cwd ?? null, text]
    ); committed = true; });
    if (!queued) return false;
    await this.writeQueue;
    return committed && !this.queueFailed;
  }

  async listHistory(agentId?: string, limit = 100): Promise<CommandHistoryRow[]> {
    if (!this.isOpen || !this.pool || !this.config) return [];
    const values: unknown[] = [this.config.namespace];
    let where = 'namespace=$1';
    if (agentId) { values.push(agentId); where += ` AND agent_id=$${values.length}`; }
    values.push(clampLimit(limit, 100));
    try {
      const result = await this.pool.query<{ id: string; agentId: string; cwd: string | null; text: string; ts: string }>(
        `SELECT id::text AS id,agent_id AS "agentId",cwd,text,(extract(epoch from occurred_at)*1000)::bigint::text AS ts
         FROM ${SCHEMA}.command_history WHERE ${where} ORDER BY occurred_at DESC,id DESC LIMIT $${values.length}`, values
      );
      return result.rows.map((row) => ({ ...row, ts: Number(row.ts) }));
    } catch { this.degrade('unreachable'); this.acceptingWrites = false; return []; }
  }

  async searchHistory(query: string, limit = 50): Promise<CommandHistoryRow[]> {
    const q = query?.trim();
    if (!q || !this.isOpen || !this.pool || !this.config) return [];
    try {
      const result = await this.pool.query<{ id: string; agentId: string; cwd: string | null; text: string; ts: string }>(
        `SELECT id::text AS id,agent_id AS "agentId",cwd,text,(extract(epoch from occurred_at)*1000)::bigint::text AS ts
         FROM ${SCHEMA}.command_history WHERE namespace=$1 AND text ILIKE $2 ESCAPE '\\'
         ORDER BY occurred_at DESC,id DESC LIMIT $3`, [this.config.namespace, `%${q.replace(/[\\%_]/g, '\\$&')}%`, clampLimit(limit, 50)]
      );
      return result.rows.map((row) => ({ ...row, ts: Number(row.ts) }));
    } catch { this.degrade('unreachable'); this.acceptingWrites = false; return []; }
  }

  appendCost(sample: AgentUsageSample): boolean {
    if (!this.isOpen || !this.config || !sample.sessionId) return false;
    const namespace = this.config.namespace;
    const eventId = randomUUID();
    return this.enqueue(async () => { await this.idempotentQuery(
      `INSERT INTO ${SCHEMA}.cost_ledger
       (namespace,event_id,agent_id,session_id,occurred_at,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,model,usd)
       VALUES($1,$2,$3,$4,to_timestamp($5/1000.0),$6,$7,$8,$9,$10,$11)
       ON CONFLICT(namespace,event_id) DO NOTHING`,
      [namespace,eventId,sample.agentId,sample.sessionId,sample.ts,sample.input,sample.output,sample.cacheRead,sample.cacheCreation,sample.model,sample.usd]
    ); });
  }

  async lifetimeCostTotals(): Promise<Map<string, number>> {
    if (!this.isOpen || !this.pool || !this.config) return new Map();
    const result = await this.pool.query<{ agentId: string; usd: string }>(
      `WITH ordered AS (
         SELECT id,agent_id,session_id,usd,lag(usd) OVER(PARTITION BY agent_id,session_id ORDER BY id) prior
         FROM ${SCHEMA}.cost_ledger WHERE namespace=$1
       ), segmented AS (
         SELECT *,sum(CASE WHEN prior IS NOT NULL AND usd<prior-1e-9 THEN 1 ELSE 0 END)
         OVER(PARTITION BY agent_id,session_id ORDER BY id) segment FROM ordered
       ), peaks AS (
         SELECT agent_id,session_id,segment,max(usd) peak FROM segmented GROUP BY agent_id,session_id,segment
       ) SELECT agent_id AS "agentId",sum(peak)::text usd FROM peaks GROUP BY agent_id`, [this.config.namespace]
    );
    return new Map(result.rows.map((row) => [row.agentId, Number(row.usd)]));
  }

  async resetNamespace(): Promise<boolean> {
    if (!this.isOpen || !this.pool || !this.config) return false;
    this.acceptingWrites = false;
    if (!await this.drain()) return false;
    let client: PoolClient;
    try { client = await this.pool.connect(); } catch { return false; }
    try {
      await client.query('BEGIN');
      for (const table of ['cost_ledger', 'command_history', 'kv', 'legacy_imports']) {
        await client.query(`DELETE FROM ${SCHEMA}.${table} WHERE namespace=$1`, [this.config.namespace]);
      }
      await client.query('COMMIT');
      this.cache.clear();
      return true;
    } catch {
      try { await client.query('ROLLBACK'); } catch { /* noop */ }
      this.acceptingWrites = true;
      return false;
    } finally { client.release(); }
  }

  async drain(timeoutMs = 2_000): Promise<boolean> {
    let timer: NodeJS.Timeout | undefined;
    try {
      await Promise.race([this.writeQueue, new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error('timeout')), timeoutMs); })]);
      return !this.queueFailed;
    } catch { return false; }
    finally { if (timer) clearTimeout(timer); }
  }

  async close(): Promise<void> {
    this.acceptingWrites = false;
    const drained = await this.drain();
    // A timed-out operation may still commit. Keep the session lock until the
    // caller's bounded process exit releases every connection.
    if (!drained && !this.queueFailed) { this.degrade('write_failed'); return; }
    if (drained) await this.releaseLock(); else this.destroyLockConnection();
    const pool = this.pool;
    this.pool = null;
    try { await pool?.end(); } catch { /* best effort */ }
    this.currentStatus = { state: 'closed' };
  }
}
