import type {
  ColumnInfo,
  DatabaseConnection,
  DdlInfo,
  ObjectKind,
  QueryHistoryEntry,
  QueryResult,
  RowUpdate,
  SchemaObjects,
  TableInfo,
  TableRef,
} from './types';

/** Table paging navigation (mirrors Rust PageNav). */
export type PageNav =
  | { mode: 'first' }
  | { mode: 'next'; key: unknown[] }
  | { mode: 'prev'; key: unknown[] }
  | { mode: 'offset'; offset: number };

/** One page of table rows plus keyset boundaries (mirrors Rust TablePage). */
export interface TablePage {
  columns: string[];
  rows: Record<string, unknown>[];
  has_more: boolean;
  first_key: unknown[] | null;
  last_key: unknown[] | null;
}

// ===== Streaming export (mirrors Rust export models) =====

export type ExportSource =
  | { source: 'table'; schema: string; table: string }
  | { source: 'query'; sql: string };

export type ExportFormat = 'csv' | 'jsonlines';

export interface ExportSummary {
  rows: number;
  bytes: number;
  elapsed_ms: number;
  cancelled: boolean;
  truncated: boolean;
}
import { useAppStore } from './store';

// Tauri v2 injects __TAURI_INTERNALS__ in the webview; in dev via browser it's absent.
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const w = window as unknown as {
    __TAURI_INTERNALS__?: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<T> };
  };
  if (w.__TAURI_INTERNALS__) {
    return w.__TAURI_INTERNALS__.invoke(cmd, args);
  }
  throw new Error('Not running inside Tauri. Start with `npm run tauri dev`.');
}

/**
 * Normalize errors thrown by Tauri commands: the Rust side rejects with a
 * plain string (Err(String)), not an Error object, so `(e as Error).message`
 * is undefined. Also maps driver timeout errors to a friendly message.
 */
export function errMessage(e: unknown): string {
  let raw: string;
  if (typeof e === 'string') raw = e;
  else if (e instanceof Error) raw = e.message;
  else if (e && typeof e === 'object' && 'message' in e) raw = String((e as { message: unknown }).message);
  else raw = String(e);
  if (/timed?\s*out|PoolTimedOut|连接超时/i.test(raw)) {
    const secs = useAppStore.getState().settings.connectTimeoutSecs;
    return `连接超时（${secs}s）：请检查主机/端口/网络，或在设置中调大超时时间`;
  }
  return raw;
}

/**
 * Inject the global connect-timeout setting into the connection payload so
 * every backend connect honours the user's preference (overrides the
 * per-connection value for this session).
 */
function withTimeout(conn: DatabaseConnection): DatabaseConnection {
  const secs = useAppStore.getState().settings.connectTimeoutSecs;
  return { ...conn, connection_timeout_secs: secs };
}

/**
 * Connection-scoped invoke: injects the global timeout into the connection
 * payload AND enforces it client-side. The backend timeout can hang on
 * black-holed hosts (TCP connect never fails), so a frontend race guarantees
 * the UI reports "连接超时" after the configured seconds instead of spinning.
 */
function connectInvoke<T>(
  cmd: string,
  args: Record<string, unknown>,
  conn: DatabaseConnection,
): Promise<T> {
  const secs = useAppStore.getState().settings.connectTimeoutSecs;
  const timer = new Promise<never>((_, reject) => {
    setTimeout(() => reject(new Error(`Connection timed out after ${secs}s`)), secs * 1000);
  });
  return Promise.race([invoke<T>(cmd, { ...args, conn: withTimeout(conn) }), timer]);
}

// ===== Connection commands =====

export const api = {
  testConnection: (conn: DatabaseConnection) =>
    connectInvoke<string>('test_connection', {}, conn),

  saveConnection: (conn: DatabaseConnection) =>
    invoke<string>('save_connection', { conn }),

  deleteConnection: (id: string) =>
    invoke<void>('delete_connection', { id }),

  listConnections: () =>
    invoke<DatabaseConnection[]>('list_connections'),

  getConnection: (id: string) =>
    invoke<DatabaseConnection | null>('get_connection', { id }),

  // ===== Query commands =====

  executeQuery: (conn: DatabaseConnection, query: string, limit?: number, database?: string) =>
    connectInvoke<QueryResult>(
      'execute_query',
      { query, limit: limit ?? null, database: database ?? null },
      conn,
    ),

  updateTableRows: (conn: DatabaseConnection, schema: string, table: string, updates: RowUpdate[]) =>
    connectInvoke<number>('update_table_rows', { schema, table, updates }, conn),

  getSchemaList: (conn: DatabaseConnection) =>
    connectInvoke<string[]>('get_schemas', {}, conn),

  listTables: (conn: DatabaseConnection, schema: string) =>
    connectInvoke<TableInfo[]>('list_tables', { schema }, conn),

  getTableColumns: (conn: DatabaseConnection, schema: string, table: string) =>
    connectInvoke<ColumnInfo[]>('get_table_columns', { schema, table }, conn),

  getTableDetail: (conn: DatabaseConnection, schema: string, table: string) =>
    connectInvoke<TableInfo>('get_table_detail', { schema, table }, conn),

  listSchemaObjects: (conn: DatabaseConnection, schema: string) =>
    connectInvoke<SchemaObjects>('list_schema_objects', { schema }, conn),

  getObjectDdl: (conn: DatabaseConnection, schema: string, object: string, kind: ObjectKind) =>
    connectInvoke<DdlInfo>('get_object_ddl', { schema, object, kind }, conn),

  searchTables: (conn: DatabaseConnection, pattern: string) =>
    connectInvoke<TableRef[]>('search_tables', { pattern }, conn),

  tablePage: (conn: DatabaseConnection, schema: string, table: string, pkColumns: string[], nav: PageNav, pageSize?: number) =>
    connectInvoke<TablePage>(
      'table_page',
      { schema, table, pk_columns: pkColumns, nav, page_size: pageSize ?? null },
      conn,
    ),

  countTableRows: (conn: DatabaseConnection, schema: string, table: string) =>
    connectInvoke<number>('count_table_rows', { schema, table }, conn),

  // ===== Streaming export =====

  /**
   * Stream a table / readonly query to a local file. Deliberately uses the
   * raw invoke — exports legitimately run for minutes, so the
   * connect-timeout race of `connectInvoke` must not apply here.
   */
  exportStream: (conn: DatabaseConnection, source: ExportSource, format: ExportFormat, path: string, maxRows?: number) =>
    invoke<ExportSummary>('export_stream', {
      conn: withTimeout(conn),
      source,
      format,
      path,
      max_rows: maxRows ?? null,
    }),

  exportCancel: () => invoke<void>('export_cancel'),

  /** Subscribe to backend export progress (rows exported so far). */
  onExportProgress: async (cb: (rows: number) => void): Promise<() => void> => {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<{ rows: number }>('export-progress', (e) => cb(e.payload.rows));
  },

  // ===== History commands =====

  addQueryHistory: (connectionId: string, query: string, success: boolean, error: string | null, executionTimeMs: number) =>
    invoke<string>('add_query_history', {
      connection_id: connectionId,
      query,
      success,
      error,
      execution_time_ms: executionTimeMs,
    }),

  getQueryHistory: (connectionId: string | null, limit?: number) =>
    invoke<QueryHistoryEntry[]>('get_query_history', {
      connection_id: connectionId,
      limit: limit ?? null,
    }),

  clearQueryHistory: () =>
    invoke<void>('clear_query_history'),

  getStorageDir: () =>
    invoke<string>('get_storage_dir'),
};
