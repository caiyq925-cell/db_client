// ===== Connection types (mirror Rust models) =====

export type DbType = 'mysql' | 'mongo' | 'redis';

export interface SshTunnelConfig {
  enabled: boolean;
  host: string;
  port: number;
  username: string;
  key_path: string;
  password: string;
}

export type AuthMethod =
  | { kind: 'password'; username: string; password: string }
  | { kind: 'none' }
  | { kind: 'connection_string'; url: string };

export interface DatabaseConnection {
  id: string;
  name: string;
  kind: DbType;
  group: string;
  host: string;
  port: number;
  database: string;
  auth: AuthMethod;
  ssl: boolean;
  connection_timeout_secs: number;
  ssh_tunnel: SshTunnelConfig | null;
}

export const DEFAULT_PORTS: Record<DbType, number> = {
  mysql: 3306,
  mongo: 27017,
  redis: 6379,
};

export function newConnection(kind: DbType): DatabaseConnection {
  return {
    id: crypto.randomUUID(),
    name: '',
    kind,
    group: 'Default',
    host: '127.0.0.1',
    port: DEFAULT_PORTS[kind],
    database: '',
    auth: { kind: 'password', username: kind === 'mysql' ? 'root' : '', password: '' },
    ssl: false,
    connection_timeout_secs: 30,
    ssh_tunnel: null,
  };
}

// ===== Query result types =====

export interface QueryResult {
  columns: string[];
  rows: Record<string, unknown>[];
  rows_affected: number | null;
  execution_time_ms: number;
  truncated: boolean;
  total_rows: number | null;
  message: string | null;
}

export interface QueryHistoryEntry {
  id: string;
  connection_id: string;
  query: string;
  execution_time_ms: number;
  success: boolean;
  error: string | null;
  timestamp: string;
}

// ===== Schema types =====

export interface ColumnInfo {
  name: string;
  data_type: string;
  is_nullable: boolean;
  default_value: string | null;
  is_primary_key: boolean;
  /** Column COMMENT, when present. */
  comment?: string | null;
}

export interface IndexInfo {
  name: string;
  columns: string[];
  is_unique: boolean;
}

export interface TableInfo {
  name: string;
  schema: string;
  row_count: number | null;
  columns: ColumnInfo[];
  indexes: IndexInfo[];
  /** Table COMMENT, when present. */
  comment?: string | null;
  /** Trigger names attached to this table. */
  triggers?: string[];
}

/** A single row edit submitted for a bulk table update. */
export interface RowUpdate {
  /** column -> original value used to locate the row. */
  where_key: Record<string, unknown>;
  /** column -> new value to write. */
  set: Record<string, unknown>;
}

/** Lightweight table reference for cross-schema search hits. */
export interface TableRef {
  schema: string;
  /** Empty string when the hit is a database-level entry (Mongo db search). */
  name: string;
  row_count: number | null;
}

// ===== Schema object grouping (under a database) =====

export interface SchemaObject {
  name: string;
  summary: string | null;
}

export interface SchemaObjects {
  tables: TableInfo[];
  views: SchemaObject[];
  procedures: SchemaObject[];
  functions: SchemaObject[];
  events: SchemaObject[];
  triggers: SchemaObject[];
}

export type ObjectKind = 'table' | 'view' | 'procedure' | 'function' | 'event' | 'trigger';

export interface DdlInfo {
  object: string;
  kind: ObjectKind;
  ddl: string;
}
