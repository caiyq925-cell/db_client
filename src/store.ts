import { create } from 'zustand';
import { notifications } from '@mantine/notifications';
import type {
  ColumnInfo,
  DatabaseConnection,
  ObjectKind,
  QueryResult,
  RowUpdate,
  SchemaObjects,
} from './types';
import { api, errMessage } from './api';
import type { PageNav } from './api';

/** Auto-dismiss timer for the transient toast (module-scoped). */
let toastTimer: number | null = null;

export type TabKind = 'query' | 'schema' | 'datatable' | 'history';

/** Per-connection status for the sidebar indicator dot. */
export type ConnStatus = 'idle' | 'connecting' | 'connected' | 'error';

export interface QueryTab {
  id: string;
  kind: 'query';
  title: string;
  connectionId: string;
  queryText: string;
  result: QueryResult | null;
  running: boolean;
  error: string | null;
}

/** Keyset page size for editable data tables. */
export const DATA_PAGE_SIZE = 200;

/** Boundary marks for one visited data-table page. */
export interface DataTablePageMark {
  /** PK values of this page's first row (keyset mode). */
  firstKey: unknown[] | null;
  /** PK values of this page's last row (keyset mode). */
  lastKey: unknown[] | null;
  /** lastKey of the page before this one — used to re-fetch this page after saves. */
  prevLastKey: unknown[] | null;
  /** Row offset of this page (no-PK OFFSET fallback). */
  offset: number;
}

/**
 * Editable data-grid tab opened by double-clicking a table.
 *
 * `rows` / `originalRows` stay row-aligned by index so edits can be diffed:
 * changed rows are submitted as `RowUpdate`s keyed by primary key.
 */
export interface DataTableTab {
  id: string;
  kind: 'datatable';
  title: string;
  connectionId: string;
  schema: string;
  table: string;
  columns: ColumnInfo[];
  /** Current (editable) cell values, index-aligned with originalRows. */
  rows: Record<string, unknown>[];
  /** Values as fetched from the DB, used for the diff. */
  originalRows: Record<string, unknown>[];
  /** Primary-key column names used to locate rows on save. */
  pkColumns: string[];
  editable: boolean;
  /** Estimated row count from INFORMATION_SCHEMA (instant). */
  totalRows: number | null;
  /** Exact COUNT(*) fetched on demand (slow on huge tables). */
  exactCount: number | null;
  counting: boolean;
  /** Keyset page marks, index === page index. */
  pageStack: DataTablePageMark[];
  pageIndex: number;
  hasMore: boolean;
  loadingPage: boolean;
  /** No primary key → OFFSET fallback navigation (slow deep pages). */
  noPk: boolean;
  loaded: boolean;
  error: string | null;
  saving: boolean;
}

export type AppTab =
  | QueryTab
  | DataTableTab
  | { id: string; kind: 'schema'; title: string; connectionId: string }
  | { id: string; kind: 'history'; title: string; connectionId?: undefined };

/** Target of the DDL modal (view / edit a schema object's DDL). */
export interface DdlModalTarget {
  connectionId: string;
  schema: string;
  object: string;
  kind: ObjectKind;
  title: string;
  /** open directly in edit mode (view structure / alter) */
  edit?: boolean;
}

/** Context menu entry for a schema object (right-click). */
export interface ContextMenuState {
  x: number;
  y: number;
  connectionId: string;
  /** 所属库；database 菜单时为空串 */
  schema: string;
  /** 对象名；database 菜单=库名，group 菜单=组 key（如 procedure） */
  object: string;
  /** 节点类型：schema 对象 / 数据库节点 / 对象分组节点 */
  kind: ObjectKind | 'database' | 'group';
}

/** Right-click context menu for a connection. */
export interface ConnMenuState {
  x: number;
  y: number;
  connId: string;
}

interface AppState {
  connections: DatabaseConnection[];
  activeConnectionId: string | null;
  tabs: AppTab[];
  activeTabId: string | null;

  /** Per-connection status for the sidebar dot (idle/connecting/connected/error). */
  connStatus: Record<string, ConnStatus>;
  /** Resizable sidebar width in pixels. */
  sidebarWidth: number;

  // === schema tree state ===
  /** connectionId -> loaded database names */
  dbList: Record<string, string[]>;
  /** `${connId}::${db}` -> schema objects (tables/views/procs/...) */
  objectsByDb: Record<string, SchemaObjects>;
  /** `${connId}::${db}` -> 对象列表拉取中（侧栏库节点后的小 loading） */
  objectsLoading: Record<string, boolean>;
  /** expansion state, keyed by node path */
  expanded: Record<string, boolean>;
  /** `${connId}::${db}::${group}::${table}` -> per-table expansion */
  tableExpanded: Record<string, boolean>;
  /** connectionId -> database-list filter text (persisted; blank = show all) */
  dbFilter: Record<string, string>;
  /** connectionId -> 当前数据库（侧栏点击/查询页下拉选定；持久化；空 = 未选） */
  activeDb: Record<string, string>;
  /** `${connId}::${db}::${table}` -> lazily-loaded full table detail (columns/indexes/triggers) */
  tableDetails: Record<string, import('./types').TableInfo>;
  /** `${connId}::${db}::${table}` -> detail currently loading */
  tableDetailLoading: Record<string, boolean>;
  /** `${detailKey}::fields|indexes|triggers` -> which detail sections are unfolded */
  detailSections: Record<string, boolean>;
  /** `${connId}` -> last connection error message (shown under the connection row). */
  connErrors: Record<string, string>;
  /** Global app settings (connect timeout), persisted to localStorage. */
  settings: { connectTimeoutSecs: number };
  setConnectTimeoutSecs: (secs: number) => void;
  /** `${connId}` -> database-list filter text, persisted to localStorage. */
  setDbFilter: (connId: string, value: string) => void;
  setActiveDb: (connId: string, db: string) => void;
  /** Transient toast message (auto-dismisses). */
  toast: { text: string; kind: 'success' | 'error' | 'info' } | null;
  showToast: (text: string, kind?: 'success' | 'error' | 'info') => void;
  /** 二次确认弹窗（危险操作：删除存储过程等）；onConfirm 在用户确认后执行 */
  confirmState: { title: string; message: string; onConfirm: () => void } | null;
  openConfirm: (state: { title: string; message: string; onConfirm: () => void }) => void;
  closeConfirm: () => void;
  /** active DDL modal target (view/edit) */
  ddlTarget: DdlModalTarget | null;
  /** right-click context menu */
  contextMenu: ContextMenuState | null;
  /** right-click context menu for a connection */
  connMenu: ConnMenuState | null;

  loadConnections: () => Promise<void>;
  upsertConnection: (conn: DatabaseConnection) => Promise<void>;
  removeConnection: (id: string) => Promise<void>;
  /** Move a connection into another group, creating the group if new. */
  moveConnectionToGroup: (connId: string, group: string) => Promise<void>;
  setActiveConnection: (id: string) => void;
  getActiveConnection: () => DatabaseConnection | null;

  /** Run a live connectivity test and update the sidebar status dot. */
  testConnection: (conn: DatabaseConnection) => Promise<void>;
  /** Reset the status dot to idle (logical "close" - no persistent connection). */
  closeConnection: (id: string) => void;

  openQueryTab: (
    connectionId: string,
    opts?: { database?: string; sql?: string },
  ) => void;
  openSchemaTab: (connectionId: string) => void;
  openHistoryTab: () => void;
  /** Open an editable data-grid tab for a table. */
  openDataTableTab: (conn: DatabaseConnection, schema: string, table: string, estimate?: number | null) => Promise<void>;
  nextDataTablePage: (id: string) => Promise<void>;
  prevDataTablePage: (id: string) => Promise<void>;
  firstDataTablePage: (id: string) => Promise<void>;
  countDataTableRows: (id: string) => Promise<void>;
  closeTab: (id: string) => void;
  setActiveTab: (id: string) => void;
  updateQueryTab: (id: string, patch: Partial<QueryTab>) => void;
  /** Update a data-grid tab (cell edits, save state, etc.). */
  updateDataTableTab: (id: string, patch: Partial<DataTableTab>) => void;
  /** Diff current rows against originals and submit the changes. */
  saveDataTableTab: (id: string) => Promise<void>;

  // === schema tree actions ===
  loadDatabases: (conn: DatabaseConnection) => Promise<void>;
  refreshDatabases: (conn: DatabaseConnection) => Promise<void>;
  loadObjects: (conn: DatabaseConnection, db: string, force?: boolean) => Promise<void>;
  isExpanded: (key: string) => boolean;
  toggleExpanded: (key: string) => void;
  /** Idempotent expand: always sets the key to true (double-click safe). */
  expandNode: (key: string) => void;
  toggleTableExpanded: (key: string) => void;
  /** Toggle a table-detail section (fields/indexes/triggers). */
  toggleDetailSection: (key: string) => void;
  /** Lazily fetch a table's columns/indexes/triggers (called on table expand). */
  loadTableDetail: (conn: DatabaseConnection, db: string, table: string) => Promise<void>;
  setSidebarWidth: (w: number) => void;
  openDdl: (target: DdlModalTarget) => void;
  closeDdl: () => void;
  openContextMenu: (menu: ContextMenuState) => void;
  closeContextMenu: () => void;
  openConnMenu: (menu: ConnMenuState) => void;
  closeConnMenu: () => void;
  /** Connection editor driven by the store (shared by sidebar & conn menu). */
  editingConn: DatabaseConnection | null;
  showConnEditor: boolean;
  openConnEditor: (conn: DatabaseConnection | null) => void;
  closeConnEditor: () => void;
  /** Move-to-group target driven by the store (shared by sidebar & conn menu). */
  moveTarget: DatabaseConnection | null;
  openMoveGroup: (conn: DatabaseConnection) => void;
  closeMoveGroup: () => void;
}

export const useAppStore = create<AppState>((set, get) => ({
  connections: [],
  activeConnectionId: null,
  tabs: [],
  activeTabId: null,
  connStatus: {},
  connErrors: {},
  sidebarWidth: 260,
  dbList: {},
  objectsByDb: {},
  objectsLoading: {},
  expanded: {},
  tableExpanded: {},
  tableDetails: {},
  tableDetailLoading: {},
  detailSections: {},
  settings: (() => {
    try {
      const raw = localStorage.getItem('dbclient.settings');
      if (raw) {
        const parsed = JSON.parse(raw) as { connectTimeoutSecs?: number };
        return { connectTimeoutSecs: parsed.connectTimeoutSecs ?? 15 };
      }
    } catch {
      /* corrupted settings fall back to default */
    }
    return { connectTimeoutSecs: 15 };
  })(),
  ddlTarget: null,
  contextMenu: null,
  connMenu: null,
  editingConn: null,
  showConnEditor: false,
  moveTarget: null,

  loadConnections: async () => {
    const connections = await api.listConnections();
    set({ connections });
  },

  upsertConnection: async (conn) => {
    await api.saveConnection(conn);
    await get().loadConnections();
    set((s) => ({
      activeConnectionId: s.activeConnectionId ?? conn.id,
    }));
  },

  removeConnection: async (id) => {
    await api.deleteConnection(id);
    const prev = get();
    await prev.loadConnections();
    set((s) => {
      const remaining = s.tabs.filter((t) => t.connectionId !== id);
      const wasActive = s.tabs.find((t) => t.id === s.activeTabId)?.connectionId === id;
      const idx = s.tabs.findIndex((t) => t.id === s.activeTabId);
      const connStatus = { ...s.connStatus };
      delete connStatus[id];
      return {
        activeConnectionId: s.activeConnectionId === id ? null : s.activeConnectionId,
        tabs: remaining,
        activeTabId: wasActive ? remaining[Math.max(0, idx - 1)]?.id ?? null : s.activeTabId,
        connStatus,
      };
    });
  },

  moveConnectionToGroup: async (connId, group) => {
    const conn = get().connections.find((c) => c.id === connId);
    if (!conn || conn.group === group) return;
    await api.saveConnection({ ...conn, group });
    await get().loadConnections();
  },

  setActiveConnection: (id) => set({ activeConnectionId: id }),

  testConnection: async (conn) => {
    set((s) => ({ connStatus: { ...s.connStatus, [conn.id]: 'connecting' } }));
    try {
      await api.testConnection(conn);
      set((s) => ({ connStatus: { ...s.connStatus, [conn.id]: 'connected' } }));
      notifications.show({ color: 'green', title: '连接成功', message: `${conn.name || conn.host} 连接正常` });
    } catch (e) {
      notifications.show({ color: 'red', title: '连接失败', message: errMessage(e) });
      set((s) => ({ connStatus: { ...s.connStatus, [conn.id]: 'error' } }));
    }
  },

  closeConnection: (id) => {
    // 逻辑关闭：状态点复位为灰，并丢弃该连接的全部结构缓存
    // （数据库列表 / 对象树 / 表详情），收起节点。
    // 再次展开时 loadDatabases 缓存未命中 -> 重新连接并拉取最新表清单。
    set((s) => {
      const connStatus = { ...s.connStatus, [id]: 'idle' as ConnStatus };
      const dbList = { ...s.dbList };
      delete dbList[id];
      const dropForConn = <T,>(rec: Record<string, T>) =>
        Object.fromEntries(Object.entries(rec).filter(([k]) => !k.startsWith(`${id}::`)));
      const expanded = dropForConn(s.expanded);
      expanded[id] = false;
      return {
        connStatus,
        dbList,
        objectsByDb: dropForConn(s.objectsByDb),
        objectsLoading: dropForConn(s.objectsLoading),
        tableDetails: dropForConn(s.tableDetails),
        tableDetailLoading: dropForConn(s.tableDetailLoading),
        detailSections: dropForConn(s.detailSections),
        tableExpanded: dropForConn(s.tableExpanded),
        expanded,
      };
    });
  },

  getActiveConnection: () => {
    const { connections, activeConnectionId } = get();
    return connections.find((c) => c.id === activeConnectionId) ?? null;
  },

  openQueryTab: (connectionId, opts) => {
    const conn = get().connections.find((c) => c.id === connectionId);
    const id = crypto.randomUUID();
    // 右键数据库/表新建查询时显式指定库，并同步为该连接的"当前数据库"
    if (opts?.database) get().setActiveDb(connectionId, opts.database);
    // Tab 与工具栏去重（T-03）：Tab 仅留「查询」+ 库名缩写，连接全称由工具栏展示
    const db = (conn && (opts?.database || get().activeDb[connectionId] || conn.database)) || '';
    set((s) => ({
      tabs: [
        ...s.tabs,
        {
          id,
          kind: 'query',
          title: db ? `查询·${db}` : '查询',
          connectionId,
          queryText: opts?.sql ?? '',
          result: null,
          running: false,
          error: null,
        },
      ],
      activeTabId: id,
      activeConnectionId: connectionId,
    }));
  },

  openSchemaTab: (connectionId) => {
    const conn = get().connections.find((c) => c.id === connectionId);
    const id = crypto.randomUUID();
    set((s) => ({
      tabs: [...s.tabs, { id, kind: 'schema', title: `结构 - ${conn?.name ?? ''}`, connectionId }],
      activeTabId: id,
      activeConnectionId: connectionId,
    }));
  },

  openDataTableTab: async (conn, schema, table, estimate) => {
    // Reuse an already-open data tab for the same table if present.
    const existing = get().tabs.find(
      (t) =>
        t.kind === 'datatable' &&
        t.connectionId === conn.id &&
        t.schema === schema &&
        t.table === table,
    );
    if (existing) {
      set({ activeTabId: existing.id, activeConnectionId: conn.id });
      return;
    }

    const id = crypto.randomUUID();
    const tab: DataTableTab = {
      id,
      kind: 'datatable',
      title: `数据 - ${table}`,
      connectionId: conn.id,
      schema,
      table,
      columns: [],
      rows: [],
      originalRows: [],
      pkColumns: [],
      editable: conn.kind === 'mysql',
      totalRows: estimate ?? null,
      exactCount: null,
      counting: false,
      pageStack: [],
      pageIndex: 0,
      hasMore: false,
      loadingPage: false,
      noPk: false,
      loaded: false,
      error: null,
      saving: false,
    };
    set((s) => ({
      tabs: [...s.tabs, tab],
      activeTabId: id,
      activeConnectionId: conn.id,
    }));

    // Load columns + first page.
    try {
      const columns = await api.getTableColumns(conn, schema, table);
      const pkColumns = columns.filter((c) => c.is_primary_key).map((c) => c.name);
      const editable = conn.kind === 'mysql' && pkColumns.length > 0;
      const noPk = conn.kind === 'mysql' && pkColumns.length === 0;

      let pageRows: Record<string, unknown>[] = [];
      let hasMore = false;
      let pageStack: DataTablePageMark[] = [];
      if (conn.kind === 'mysql') {
        // Keyset first page (OFFSET fallback for no-PK tables) keeps huge
        // tables responsive; the old fixed `LIMIT 200` scan is gone.
        const page = await api.tablePage(
          conn,
          schema,
          table,
          pkColumns,
          noPk ? { mode: 'offset', offset: 0 } : { mode: 'first' },
          DATA_PAGE_SIZE,
        );
        pageRows = page.rows.map((r) => ({ ...r }));
        hasMore = page.has_more;
        pageStack = [
          { firstKey: page.first_key, lastKey: page.last_key, prevLastKey: null, offset: 0 },
        ];
      } else {
        const result = await api.executeQuery(
          conn,
          `SELECT * FROM \`${schema}\`.\`${table}\` LIMIT ${DATA_PAGE_SIZE}`,
          DATA_PAGE_SIZE,
        );
        pageRows = result.rows.map((r) => ({ ...r }));
      }

      const live = get().tabs.find((t) => t.id === id);
      if (!live || live.kind !== 'datatable') return; // tab closed meanwhile
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === id && t.kind === 'datatable'
            ? {
                ...t,
                columns,
                rows: pageRows,
                originalRows: pageRows.map((r) => ({ ...r })),
                pkColumns,
                editable,
                noPk,
                pageStack,
                hasMore,
                loaded: true,
              }
            : t,
        ),
      }));
    } catch (e) {
      const msg = errMessage(e);
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === id && t.kind === 'datatable' ? { ...t, loaded: true, error: msg } : t,
        ),
      }));
    }
  },

  nextDataTablePage: async (id) => {
    const live = get().tabs.find((t) => t.id === id);
    if (!live || live.kind !== 'datatable' || live.loadingPage) return;
    const conn = get().connections.find((c) => c.id === live.connectionId);
    const mark = live.pageStack[live.pageIndex];
    if (!conn || !mark) return;

    const nav: PageNav = mark.lastKey
      ? { mode: 'next', key: mark.lastKey }
      : { mode: 'offset', offset: (live.pageIndex + 1) * DATA_PAGE_SIZE };
    get().updateDataTableTab(id, { loadingPage: true, error: null });
    try {
      const page = await api.tablePage(conn, live.schema, live.table, live.pkColumns, nav, DATA_PAGE_SIZE);
      set((s) => ({
        tabs: s.tabs.map((t) => {
          if (t.id !== id || t.kind !== 'datatable') return t;
          const stack = t.pageStack.slice(0, t.pageIndex + 1); // drop forward history
          stack.push({
            firstKey: page.first_key,
            lastKey: page.last_key,
            prevLastKey: mark.lastKey,
            offset: (t.pageIndex + 1) * DATA_PAGE_SIZE,
          });
          return {
            ...t,
            rows: page.rows.map((r) => ({ ...r })),
            originalRows: page.rows.map((r) => ({ ...r })),
            pageStack: stack,
            pageIndex: t.pageIndex + 1,
            hasMore: page.has_more,
            loadingPage: false,
          };
        }),
      }));
    } catch (e) {
      get().updateDataTableTab(id, { loadingPage: false, error: errMessage(e) });
    }
  },

  prevDataTablePage: async (id) => {
    const live = get().tabs.find((t) => t.id === id);
    if (!live || live.kind !== 'datatable' || live.loadingPage || live.pageIndex === 0) return;
    const conn = get().connections.find((c) => c.id === live.connectionId);
    const target = live.pageStack[live.pageIndex - 1];
    if (!conn || !target) return;

    const nav: PageNav = target.firstKey
      ? { mode: 'prev', key: target.firstKey }
      : { mode: 'offset', offset: (live.pageIndex - 1) * DATA_PAGE_SIZE };
    get().updateDataTableTab(id, { loadingPage: true, error: null });
    try {
      const page = await api.tablePage(conn, live.schema, live.table, live.pkColumns, nav, DATA_PAGE_SIZE);
      set((s) => ({
        tabs: s.tabs.map((t) => {
          if (t.id !== id || t.kind !== 'datatable') return t;
          // Refresh the previous page's boundaries in place.
          const stack = t.pageStack.slice();
          stack[t.pageIndex - 1] = {
            firstKey: page.first_key,
            lastKey: page.last_key,
            prevLastKey: target.prevLastKey,
            offset: target.offset,
          };
          return {
            ...t,
            rows: page.rows.map((r) => ({ ...r })),
            originalRows: page.rows.map((r) => ({ ...r })),
            pageStack: stack,
            pageIndex: t.pageIndex - 1,
            hasMore: true, // we came from a later page, so a next page exists
            loadingPage: false,
          };
        }),
      }));
    } catch (e) {
      get().updateDataTableTab(id, { loadingPage: false, error: errMessage(e) });
    }
  },

  firstDataTablePage: async (id) => {
    const live = get().tabs.find((t) => t.id === id);
    if (!live || live.kind !== 'datatable' || live.loadingPage || live.pageIndex === 0) return;
    const conn = get().connections.find((c) => c.id === live.connectionId);
    if (!conn) return;

    get().updateDataTableTab(id, { loadingPage: true, error: null });
    try {
      const page = await api.tablePage(
        conn,
        live.schema,
        live.table,
        live.pkColumns,
        live.noPk ? { mode: 'offset', offset: 0 } : { mode: 'first' },
        DATA_PAGE_SIZE,
      );
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === id && t.kind === 'datatable'
            ? {
                ...t,
                rows: page.rows.map((r) => ({ ...r })),
                originalRows: page.rows.map((r) => ({ ...r })),
                pageStack: [
                  { firstKey: page.first_key, lastKey: page.last_key, prevLastKey: null, offset: 0 },
                ],
                pageIndex: 0,
                hasMore: page.has_more,
                loadingPage: false,
              }
            : t,
        ),
      }));
    } catch (e) {
      get().updateDataTableTab(id, { loadingPage: false, error: errMessage(e) });
    }
  },

  countDataTableRows: async (id) => {
    const live = get().tabs.find((t) => t.id === id);
    if (!live || live.kind !== 'datatable' || live.counting) return;
    const conn = get().connections.find((c) => c.id === live.connectionId);
    if (!conn) return;

    get().updateDataTableTab(id, { counting: true });
    try {
      const n = await api.countTableRows(conn, live.schema, live.table);
      get().updateDataTableTab(id, { exactCount: n, counting: false });
    } catch (e) {
      get().showToast(`精确计数失败：${errMessage(e)}`, 'error');
      get().updateDataTableTab(id, { counting: false });
    }
  },

  openHistoryTab: () => {
    // Only one history tab at a time
    const existing = get().tabs.find((t) => t.kind === 'history');
    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }
    const id = crypto.randomUUID();
    set((s) => ({
      tabs: [...s.tabs, { id, kind: 'history', title: '查询历史' }],
      activeTabId: id,
    }));
  },

  closeTab: (id) =>
    set((s) => {
      const idx = s.tabs.findIndex((t) => t.id === id);
      const tabs = s.tabs.filter((t) => t.id !== id);
      const activeTabId =
        s.activeTabId === id ? tabs[Math.max(0, idx - 1)]?.id ?? null : s.activeTabId;
      return { tabs, activeTabId };
    }),

  setActiveTab: (id) => set({ activeTabId: id }),

  updateQueryTab: (id, patch) =>
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === id && t.kind === 'query' ? { ...t, ...patch } : t)),
    })),

  updateDataTableTab: (id, patch) =>
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === id && t.kind === 'datatable' ? { ...t, ...patch } : t)),
    })),

  saveDataTableTab: async (id) => {
    const live = get().tabs.find((t) => t.id === id);
    if (!live || live.kind !== 'datatable') return;
    const conn = get().connections.find((c) => c.id === live.connectionId);
    if (!conn) return;

    // Diff current rows against originals (index-aligned).
    const pk = live.pkColumns;
    const updates: RowUpdate[] = [];
    live.rows.forEach((row, i) => {
      const orig = live.originalRows[i];
      if (!orig) return;
      const whereKey: Record<string, unknown> = {};
      for (const c of pk) whereKey[c] = orig[c];
      const setCols: Record<string, unknown> = {};
      for (const col of live.columns) {
        if (col.name in whereKey) continue; // don't update the locating key
        if (String(row[col.name] ?? '') !== String(orig[col.name] ?? '')) {
          setCols[col.name] = row[col.name] ?? null;
        }
      }
      if (Object.keys(setCols).length > 0) {
        updates.push({ where_key: whereKey, set: setCols });
      }
    });

    if (updates.length === 0) {
      get().showToast('没有需要保存的修改', 'info');
      get().updateDataTableTab(id, { saving: false });
      return;
    }

    // 供查询历史展示的等价 SQL 文本（每行一条参数化 UPDATE）。
    const q = (s: string) => `\`${s.replace(/`/g, '``')}\``;
    const lit = (v: unknown) =>
      v === null || v === undefined ? 'NULL' : `'${String(v).replace(/'/g, "''")}'`;
    const sqlText = updates
      .map((u) => {
        const sets = Object.entries(u.set)
          .map(([c, v]) => `${q(c)} = ${lit(v)}`)
          .join(', ');
        const wheres = Object.entries(u.where_key)
          .map(([c, v]) => `${q(c)} = ${lit(v)}`)
          .join(' AND ');
        return `UPDATE ${q(live.schema)}.${q(live.table)} SET ${sets} WHERE ${wheres};`;
      })
      .join('\n');

    get().updateDataTableTab(id, { saving: true, error: null });
    const startedAt = Date.now();
    try {
      const affected = await api.updateTableRows(conn, live.schema, live.table, updates);
      // 成功执行的 SQL 记入查询历史，便于审计/复用。
      void api.addQueryHistory(conn.id, sqlText, true, null, Date.now() - startedAt).catch(() => {});
      get().showToast(
        affected > 0
          ? `已保存 ${affected} 行`
          : '已保存 0 行：条件未匹配到数据，表数据可能有变化，请刷新后重试',
        affected > 0 ? 'success' : 'error',
      );
      // Reload to reflect the committed state.
      const columns = await api.getTableColumns(conn, live.schema, live.table);
      const result = await api.executeQuery(
        conn,
        `SELECT * FROM \`${live.schema}\`.\`${live.table}\` LIMIT 200`,
        200,
      );
      const pkColumns = columns.filter((c) => c.is_primary_key).map((c) => c.name);
      const editable = conn.kind === 'mysql' && pkColumns.length > 0;
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === id && t.kind === 'datatable'
            ? {
                ...t,
                columns,
                rows: result.rows.map((r) => ({ ...r })),
                originalRows: result.rows,
                pkColumns,
                editable,
                totalRows: result.total_rows,
                saving: false,
              }
            : t,
        ),
      }));
      // Refresh schema object metadata so row counts stay accurate.
      await get().loadObjects(conn, live.schema);
    } catch (e) {
      const msg = errMessage(e);
      void api.addQueryHistory(conn.id, sqlText, false, msg, Date.now() - startedAt).catch(() => {});
      get().showToast(`保存失败：${msg}`, 'error');
      get().updateDataTableTab(id, {
        saving: false,
        error: msg,
      });
    }
  },

  // ===== schema tree actions =====

  loadDatabases: async (conn) => {
    if (get().dbList[conn.id]) return; // already loaded
    set((s) => ({
      connStatus: { ...s.connStatus, [conn.id]: 'connecting' },
      connErrors: { ...s.connErrors, [conn.id]: '' },
    }));
    try {
      const dbs = await api.getSchemaList(conn);
      set((s) => ({
        dbList: { ...s.dbList, [conn.id]: dbs },
        // A successful schema query means the connection is live -> green dot.
        connStatus: { ...s.connStatus, [conn.id]: 'connected' },
        connErrors: { ...s.connErrors, [conn.id]: '' },
      }));
      // Auto-expand this connection so its databases show up.
      set((s) => ({ expanded: { ...s.expanded, [conn.id]: true } }));
    } catch (e) {
      console.error('loadDatabases failed:', e);
      notifications.show({ color: 'red', title: '连接失败', message: errMessage(e) });
      set((s) => ({
        connStatus: { ...s.connStatus, [conn.id]: 'error' },
        connErrors: { ...s.connErrors, [conn.id]: errMessage(e) },
      }));
    }
  },

  loadObjects: async (conn, db, force = false) => {
    const key = `${conn.id}::${db}`;
    if (!force && get().objectsByDb[key]) return;
    set((s) => ({ objectsLoading: { ...s.objectsLoading, [key]: true } }));
    try {
      const objects = await api.listSchemaObjects(conn, db);
      set((s) => ({ objectsByDb: { ...s.objectsByDb, [key]: objects } }));
      // Mongo 场景只有 tables 一个非空组：自动展开组，集合直接可见
      //（多一层"表 N"分组点击对单组对象是多余交互）。
      const onlyTables =
        objects.tables.length > 0 &&
        objects.views.length === 0 &&
        objects.procedures.length === 0 &&
        objects.functions.length === 0 &&
        objects.events.length === 0 &&
        objects.triggers.length === 0;
      if (onlyTables) {
        set((s) => ({ expanded: { ...s.expanded, [`${key}::table`]: true } }));
      }
    } catch (e) {
      console.error('loadObjects failed:', e);
    } finally {
      set((s) => ({ objectsLoading: { ...s.objectsLoading, [key]: false } }));
    }
  },

  refreshDatabases: async (conn) => {
    // Force-reload the database list and drop any cached object lists for this
    // connection so the tree re-fetches fresh structure on next expand.
    set((s) => ({ connErrors: { ...s.connErrors, [conn.id]: '' } }));
    try {
      const dbs = await api.getSchemaList(conn);
      set((s) => ({
        dbList: { ...s.dbList, [conn.id]: dbs },
        objectsByDb: Object.fromEntries(
          Object.entries(s.objectsByDb).filter(([k]) => !k.startsWith(`${conn.id}::`)),
        ),
        expanded: { ...s.expanded, [conn.id]: true },
        connStatus: { ...s.connStatus, [conn.id]: 'connected' },
        connErrors: { ...s.connErrors, [conn.id]: '' },
      }));
    } catch (e) {
      console.error('refreshDatabases failed:', e);
      notifications.show({ color: 'red', title: '刷新失败', message: errMessage(e) });
      set((s) => ({
        connStatus: { ...s.connStatus, [conn.id]: 'error' },
        connErrors: { ...s.connErrors, [conn.id]: errMessage(e) },
      }));
    }
  },

  isExpanded: (key) => !!get().expanded[key],

  toggleExpanded: (key) =>
    set((s) => ({ expanded: { ...s.expanded, [key]: !s.expanded[key] } })),

  // 幂等展开：双击 = 2 次同步 click，React 批处理下两次 toggle 会 net-zero，
  // 因此行点击展开必须恒置 true 而不是翻转。
  expandNode: (key) =>
    set((s) => ({ expanded: { ...s.expanded, [key]: true } })),

  toggleTableExpanded: (key) =>
    set((s) => ({ tableExpanded: { ...s.tableExpanded, [key]: !s.tableExpanded[key] } })),

  toggleDetailSection: (key) =>
    set((s) => ({ detailSections: { ...s.detailSections, [key]: !s.detailSections[key] } })),

  setConnectTimeoutSecs: (secs) => {
    const settings = { connectTimeoutSecs: Math.max(1, Math.min(300, Math.round(secs) || 15)) };
    set({ settings });
    try {
      localStorage.setItem('dbclient.settings', JSON.stringify(settings));
    } catch {
      /* localStorage unavailable; setting stays in memory */
    }
  },

  dbFilter: (() => {
    try {
      const raw = localStorage.getItem('dbclient.dbFilters');
      if (raw) return JSON.parse(raw) as Record<string, string>;
    } catch {
      /* corrupted filters fall back to empty */
    }
    return {};
  })(),

  setDbFilter: (connId, value) => {
    set((s) => {
      const next = { ...s.dbFilter, [connId]: value };
      try {
        localStorage.setItem('dbclient.dbFilters', JSON.stringify(next));
      } catch {
        /* localStorage unavailable; filter stays in memory */
      }
      return { dbFilter: next };
    });
  },

  activeDb: (() => {
    try {
      const raw = localStorage.getItem('dbclient.activeDb');
      if (raw) return JSON.parse(raw) as Record<string, string>;
    } catch {
      /* corrupted state falls back to empty */
    }
    return {};
  })(),

  setActiveDb: (connId, db) => {
    set((s) => {
      const next = { ...s.activeDb, [connId]: db };
      try {
        localStorage.setItem('dbclient.activeDb', JSON.stringify(next));
      } catch {
        /* localStorage unavailable; selection stays in memory */
      }
      return { activeDb: next };
    });
  },

  toast: null,

  showToast: (text, kind = 'info') => {
    if (toastTimer) window.clearTimeout(toastTimer);
    set({ toast: { text, kind } });
    toastTimer = window.setTimeout(() => set({ toast: null }), 3000);
  },

  confirmState: null,

  openConfirm: (state) => set({ confirmState: state }),

  closeConfirm: () => set({ confirmState: null }),

  loadTableDetail: async (conn, db, table) => {
    const key = `${conn.id}::${db}::${table}`;
    if (get().tableDetails[key] || get().tableDetailLoading[key]) return;
    set((s) => ({ tableDetailLoading: { ...s.tableDetailLoading, [key]: true } }));
    try {
      const detail = await api.getTableDetail(conn, db, table);
      set((s) => ({ tableDetails: { ...s.tableDetails, [key]: detail } }));
    } catch (e) {
      console.error('loadTableDetail failed:', e);
    } finally {
      set((s) => ({ tableDetailLoading: { ...s.tableDetailLoading, [key]: false } }));
    }
  },

  setSidebarWidth: (w) =>
    set({ sidebarWidth: Math.max(200, Math.min(520, w)) }),

  openDdl: (target) => set({ ddlTarget: target, contextMenu: null }),
  closeDdl: () => set({ ddlTarget: null }),

  openContextMenu: (menu) => set({ contextMenu: menu }),
  closeContextMenu: () => set({ contextMenu: null }),

  openConnMenu: (menu) => set({ connMenu: menu, contextMenu: null }),
  closeConnMenu: () => set({ connMenu: null }),

  openConnEditor: (conn) => set({ editingConn: conn, showConnEditor: true, connMenu: null }),
  closeConnEditor: () => set({ editingConn: null, showConnEditor: false }),

  openMoveGroup: (conn) => set({ moveTarget: conn, connMenu: null }),
  closeMoveGroup: () => set({ moveTarget: null }),
}));
