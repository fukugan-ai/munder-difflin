'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { Pool } = require('pg');
const { consumeConfig } = require('./db-env.cjs');

const MIGRATIONS_DIRECTORY = path.join(__dirname, '..', 'db', 'migrations');
const MIGRATION_NAME = /^(\d{3})_[a-z0-9_]+\.sql$/;

function loadOrderedMigrations(directory = MIGRATIONS_DIRECTORY) {
  const migrations = fs.readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && MIGRATION_NAME.test(entry.name))
    .map((entry) => ({
      name: entry.name,
      version: Number(MIGRATION_NAME.exec(entry.name)[1]),
      sql: fs.readFileSync(path.join(directory, entry.name), 'utf8')
    }))
    .sort((left, right) => left.version - right.version);
  if (migrations.length === 0) throw new Error('no PostgreSQL migrations found');
  migrations.forEach((migration, index) => {
    if (migration.version !== index + 1) throw new Error('PostgreSQL migration sequence is not contiguous');
    const sql = migration.sql.trim();
    if (!sql.startsWith('BEGIN;') || !sql.endsWith('COMMIT;')) {
      throw new Error('PostgreSQL migration must own one transaction');
    }
  });
  return migrations;
}

async function applyMigrations(client, migrations) {
  for (const migration of migrations) await client.query(migration.sql);
}

async function main() {
  const { pool: config } = consumeConfig();
  const pool = new Pool(config);
  const client = await pool.connect();
  let locked = false;
  try {
    const lock = await client.query(
      'SELECT pg_try_advisory_lock(hashtext($1), hashtext($2)) AS locked',
      ['munder-difflin', 'schema-migrations']
    );
    if (lock.rows[0]?.locked !== true) throw new Error('schema migration busy');
    locked = true;
    try { await applyMigrations(client, loadOrderedMigrations()); }
    catch (error) {
      try { await client.query('ROLLBACK'); } catch { /* connection may already be unusable */ }
      throw error;
    }
    process.stdout.write('PostgreSQL schema migration completed.\n');
  } finally {
    if (locked) {
      try {
        await client.query(
          'SELECT pg_advisory_unlock(hashtext($1), hashtext($2))',
          ['munder-difflin', 'schema-migrations']
        );
      } catch { /* pool shutdown releases a lost or failed session lock */ }
    }
    client.release();
    await pool.end();
  }
}

if (require.main === module) {
  main().catch(() => { process.stderr.write('PostgreSQL migration failed.\n'); process.exitCode = 1; });
}

module.exports = { applyMigrations, loadOrderedMigrations };
