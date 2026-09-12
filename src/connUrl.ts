import type { DatabaseConnection } from './types';

/**
 * Build a copyable connection string for a saved connection.
 * Used by the "复制链接" (copy connection string) action in the connection menu.
 */
export function buildConnectionUrl(conn: DatabaseConnection): string {
  const host = conn.host || '127.0.0.1';
  const port = conn.port;

  switch (conn.kind) {
    case 'mysql': {
      let user = '';
      let pass = '';
      if (conn.auth.kind === 'password') {
        user = conn.auth.username;
        pass = conn.auth.password;
      }
      const cred = user ? (pass ? `${user}:${pass}@` : `${user}@`) : '';
      return `mysql://${cred}${host}:${port}${conn.database ? `/${conn.database}` : ''}`;
    }
    case 'mongo': {
      if (conn.auth.kind === 'connection_string') return conn.auth.url;
      let user = '';
      let pass = '';
      if (conn.auth.kind === 'password') {
        user = conn.auth.username;
        pass = conn.auth.password;
      }
      const cred = user ? (pass ? `${user}:${pass}@` : `${user}@`) : '';
      return `mongodb://${cred}${host}:${port}${conn.database ? `/${conn.database}` : ''}`;
    }
    case 'redis': {
      let user = '';
      let pass = '';
      if (conn.auth.kind === 'password') {
        user = conn.auth.username;
        pass = conn.auth.password;
      }
      let cred = '';
      if (user && pass) cred = `${user}:${pass}@`;
      else if (user) cred = `${user}@`;
      const dbSuffix = conn.database ? `/${conn.database}` : '';
      return `redis://${cred}${host}:${port}${dbSuffix}`;
    }
  }
}
