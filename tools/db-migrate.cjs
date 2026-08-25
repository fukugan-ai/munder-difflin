'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { Pool } = require('pg');
const { consumeConfig } = require('./db-env.cjs');

async function main() {
  const { namespace, pool: config } = consumeConfig();
  const pool = new Pool(config);
  const client = await pool.connect();
  try {
    const lock = await client.query('SELECT pg_try_advisory_lock(hashtext($1), hashtext($2)) AS locked', ['munder-difflin', namespace]);
    if (lock.rows[0]?.locked !== true) throw new Error('namespace busy');
    const sql = fs.readFileSync(path.join(__dirname, '..', 'db/migrations/001_initial.sql'), 'utf8');
    await client.query(sql);
    process.stdout.write('PostgreSQL schema migration completed.\n');
  } finally { client.release(); await pool.end(); }
}

main().catch(() => { process.stderr.write('PostgreSQL migration failed.\n'); process.exitCode = 1; });
