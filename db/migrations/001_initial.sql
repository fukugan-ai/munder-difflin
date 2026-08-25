BEGIN;
CREATE SCHEMA IF NOT EXISTS munder_difflin;

CREATE TABLE IF NOT EXISTS munder_difflin.schema_migrations (
  version integer PRIMARY KEY,
  applied_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS munder_difflin.kv (
  namespace text NOT NULL,
  key text NOT NULL,
  value jsonb NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (namespace, key)
);

CREATE TABLE IF NOT EXISTS munder_difflin.command_history (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  namespace text NOT NULL,
  event_id uuid NOT NULL,
  agent_id text NOT NULL,
  cwd text,
  text text NOT NULL,
  occurred_at timestamptz NOT NULL,
  UNIQUE (namespace, event_id)
);
CREATE INDEX IF NOT EXISTS command_history_namespace_agent_time
  ON munder_difflin.command_history(namespace, agent_id, occurred_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS command_history_namespace_time
  ON munder_difflin.command_history(namespace, occurred_at DESC, id DESC);

CREATE TABLE IF NOT EXISTS munder_difflin.cost_ledger (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  namespace text NOT NULL,
  event_id uuid NOT NULL,
  agent_id text NOT NULL,
  session_id text NOT NULL,
  occurred_at timestamptz NOT NULL,
  input_tokens bigint NOT NULL CHECK (input_tokens >= 0),
  output_tokens bigint NOT NULL CHECK (output_tokens >= 0),
  cache_read_tokens bigint NOT NULL CHECK (cache_read_tokens >= 0),
  cache_creation_tokens bigint NOT NULL CHECK (cache_creation_tokens >= 0),
  model text,
  usd numeric(20, 10) NOT NULL CHECK (usd >= 0),
  UNIQUE (namespace, event_id)
);
CREATE INDEX IF NOT EXISTS cost_ledger_namespace_agent_session_time
  ON munder_difflin.cost_ledger(namespace, agent_id, session_id, id);

CREATE TABLE IF NOT EXISTS munder_difflin.legacy_imports (
  namespace text NOT NULL,
  source_id text NOT NULL,
  source_kind text NOT NULL CHECK (source_kind IN ('sqlite', 'cost_jsonl')),
  content_fingerprint text NOT NULL,
  checkpoint text,
  completed_at timestamptz,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (namespace, source_id)
);

INSERT INTO munder_difflin.schema_migrations(version) VALUES (1)
ON CONFLICT(version) DO NOTHING;
COMMIT;
