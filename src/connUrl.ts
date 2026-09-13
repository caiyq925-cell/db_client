import type { DatabaseConnection } from './types';

/**
 * Build a copyable connection string for a saved connection.
 * Used by the "复制链接" (copy connection string) action in the connection menu.
 */
/** 对 URL 的 userinfo 部分做百分号转义，避免 `@ : / #` 等字符破坏解析。 */
function encodeUserInfo(s: string): string {
  return encodeURIComponent(s).replace(/[!'()*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`);
}

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
      const cred =
        user || pass ? `${encodeUserInfo(user)}${pass ? `:${encodeUserInfo(pass)}` : ''}@` : '';
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
      const cred =
        user || pass ? `${encodeUserInfo(user)}${pass ? `:${encodeUserInfo(pass)}` : ''}@` : '';
      return `mongodb://${cred}${host}:${port}${conn.database ? `/${conn.database}` : ''}`;
    }
    case 'redis': {
      let user = '';
      let pass = '';
      if (conn.auth.kind === 'password') {
        user = conn.auth.username;
        pass = conn.auth.password;
      }
      // Redis 默认用户无用户名：redis://:password@host:port，密码必须保留。
      const cred =
        user || pass ? `${encodeUserInfo(user)}${pass ? `:${encodeUserInfo(pass)}` : ''}@` : '';
      const dbSuffix = conn.database ? `/${conn.database}` : '';
      return `redis://${cred}${host}:${port}${dbSuffix}`;
    }
  }
}
