'use strict';

const fs = require('node:fs');

function consumeConfig(env = process.env) {
  const password = env.MD_PG_PASSWORD;
  delete env.MD_PG_PASSWORD;
  const host = env.MD_PG_HOST;
  const database = env.MD_PG_DATABASE;
  const user = env.MD_PG_USER;
  const namespace = env.MD_PG_NAMESPACE;
  const port = Number(env.MD_PG_PORT || 5432);
  if (!host || !database || !user || !password || !namespace || !/^[A-Za-z0-9._-]{1,128}$/.test(namespace)) {
    throw new Error('missing or invalid MD_PG_* configuration');
  }
  const local = ['localhost', '127.0.0.1', '::1'].includes(host);
  const caPath = env.MD_PG_TLS_CA;
  if (!local && !caPath) throw new Error('remote PostgreSQL requires MD_PG_TLS_CA');
  return {
    namespace,
    pool: {
      host, database, user, password, port,
      ssl: local && !caPath ? false : { rejectUnauthorized: true, ca: fs.readFileSync(caPath, 'utf8') },
      max: 2,
      connectionTimeoutMillis: 4000,
      options: '-c statement_timeout=15000 -c lock_timeout=2000 -c search_path=pg_catalog'
    }
  };
}

module.exports = { consumeConfig };
