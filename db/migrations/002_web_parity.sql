BEGIN;

-- Serialize forward-only schema changes without depending on search_path.
SELECT pg_advisory_xact_lock(hashtext('munder-difflin'), hashtext('schema-migrations'));

CREATE TABLE IF NOT EXISTS munder_difflin.web_app_config (
  namespace text PRIMARY KEY,
  revision bigint NOT NULL CHECK (revision >= 0),
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (namespace ~ '^[A-Za-z0-9._-]{1,128}$')
);

-- Lossless JSON documents let each Web domain preserve forward-compatible fields.
-- `domain` is deliberately constrained; `kind` remains extensible inside a domain.
CREATE TABLE IF NOT EXISTS munder_difflin.web_durable_records (
  namespace text NOT NULL,
  domain text NOT NULL CHECK (domain IN ('tasks', 'hive', 'connections', 'triggers', 'floors')),
  kind text NOT NULL CHECK (char_length(kind) BETWEEN 1 AND 64),
  record_id text NOT NULL CHECK (char_length(record_id) BETWEEN 1 AND 256),
  revision bigint NOT NULL CHECK (revision >= 1),
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (namespace, domain, kind, record_id),
  CHECK (namespace ~ '^[A-Za-z0-9._-]{1,128}$')
);
CREATE INDEX IF NOT EXISTS web_durable_records_namespace_domain_updated
  ON munder_difflin.web_durable_records(namespace, domain, kind, updated_at DESC, record_id);

-- One locked head row allocates a gap-free, namespace-local sequence per stream.
CREATE TABLE IF NOT EXISTS munder_difflin.web_event_stream_heads (
  namespace text NOT NULL,
  stream text NOT NULL CHECK (char_length(stream) BETWEEN 1 AND 64),
  last_sequence bigint NOT NULL CHECK (last_sequence >= 0),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (namespace, stream),
  CHECK (namespace ~ '^[A-Za-z0-9._-]{1,128}$')
);

CREATE TABLE IF NOT EXISTS munder_difflin.web_event_replay (
  namespace text NOT NULL,
  stream text NOT NULL,
  sequence bigint NOT NULL CHECK (sequence >= 1),
  event_id uuid NOT NULL,
  occurred_at timestamptz NOT NULL,
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
  PRIMARY KEY (namespace, stream, sequence),
  UNIQUE (namespace, stream, event_id),
  FOREIGN KEY (namespace, stream)
    REFERENCES munder_difflin.web_event_stream_heads(namespace, stream)
    ON DELETE CASCADE,
  CHECK (namespace ~ '^[A-Za-z0-9._-]{1,128}$')
);
CREATE INDEX IF NOT EXISTS web_event_replay_namespace_stream_latest
  ON munder_difflin.web_event_replay(namespace, stream, sequence DESC);

-- Trigger history is append-only and idempotent by event_id. The repository
-- prunes each namespace to the browser contract's newest 500 rows in the same
-- transaction as an insert.
CREATE TABLE IF NOT EXISTS munder_difflin.web_trigger_history (
  namespace text NOT NULL,
  event_id uuid NOT NULL,
  source text NOT NULL CHECK (char_length(source) BETWEEN 1 AND 32),
  source_id text NOT NULL CHECK (char_length(source_id) BETWEEN 1 AND 256),
  occurred_at timestamptz NOT NULL,
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
  PRIMARY KEY (namespace, event_id),
  CHECK (namespace ~ '^[A-Za-z0-9._-]{1,128}$')
);
CREATE INDEX IF NOT EXISTS web_trigger_history_namespace_latest
  ON munder_difflin.web_trigger_history(namespace, occurred_at DESC, event_id DESC);
CREATE INDEX IF NOT EXISTS web_trigger_history_namespace_source_latest
  ON munder_difflin.web_trigger_history(namespace, source, occurred_at DESC, event_id DESC);

INSERT INTO munder_difflin.schema_migrations(version) VALUES (2)
ON CONFLICT(version) DO NOTHING;

COMMIT;
