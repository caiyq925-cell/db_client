import { useEffect, useMemo, useRef, useState } from 'react';
import { ActionIcon, Badge, CloseButton, Loader, Text, TextInput } from '@mantine/core';
import {
  IconBolt,
  IconCode,
  IconCrown,
  IconDatabase,
  IconFolder,
  IconKey,
  IconLeaf,
  IconPlus,
  IconRefresh,
  IconSearch,
  IconShield,
  IconTable,
  IconVariable,
} from '@tabler/icons-react';
import { useAppStore } from './store';
import type { ReactElement } from 'react';
import type { DatabaseConnection, SchemaObjects, ObjectKind, DbType } from './types';

// ----- connection status dot (task 6: connected = green) -----
// 语义色对齐设计令牌：success #34D399 / warning #FBBF24 / danger #F87171

const STATUS_DOT: Record<string, { color: string; label: string }> = {
  idle: { color: '#4a4a54', label: '未连接' },
  connecting: { color: '#fbbf24', label: '连接中…' },
  connected: { color: '#34d399', label: '已连接' },
  error: { color: '#f87171', label: '连接失败' },
};

// ----- 连接行图标按数据库类型区分（bug 2：Mongo 与 MySQL 图标相同） -----
const CONN_KIND_ICON: Record<DbType, { icon: ReactElement; color: string }> = {
  mysql: { icon: <IconDatabase size={14} />, color: '#81A1C1' },
  mongo: { icon: <IconLeaf size={14} />, color: '#A3BE8C' },
  redis: { icon: <IconBolt size={14} />, color: '#D08770' },
};

// 同色 20% 透明光晕：暗示「活跃」
const dotGlow = (color: string): string => `0 0 0 2px ${color}33`;

// ----- db icon prefix tone (K-01: 前缀语义着色，30 库可分组扫视) -----
// ai_* 系用青、activity* 用绿、archive* 用赭；其余保持中性灰。

const DB_ICON_TONES: Array<{ pattern: RegExp; chip: string }> = [
  { pattern: /^ai[_-]/i, chip: 'chip-cyan' },
  { pattern: /^activity/i, chip: 'chip-green' },
  { pattern: /^archive/i, chip: 'chip-ochre' },
];

function dbIconTone(db: string): string | null {
  for (const t of DB_ICON_TONES) if (t.pattern.test(db)) return t.chip;
  return null;
}

/** 数据库图标：命中前缀语义色时以 12% 底色 chip 包裹，其余中性灰 */
function DbIcon({ db }: { db: string }) {
  const tone = dbIconTone(db);
  if (!tone) return <IconDatabase size={13} style={{ opacity: 0.8 }} />;
  return (
    <span className={`db-icon-chip ${tone}`}>
      <IconDatabase size={12} />
    </span>
  );
}

// ----- object type group config -----

interface ObjGroup {
  key: ObjectKind;
  label: string;
  icon: ReactElement;
}

const OBJ_GROUPS: ObjGroup[] = [
  { key: 'table', label: '表', icon: <IconTable size={13} /> },
  { key: 'view', label: '视图', icon: <IconCrown size={13} /> },
  { key: 'procedure', label: '存储过程', icon: <IconCode size={13} /> },
  { key: 'function', label: '函数', icon: <IconVariable size={13} /> },
  { key: 'event', label: '事件', icon: <IconBolt size={13} /> },
  { key: 'trigger', label: '触发器', icon: <IconKey size={13} /> },
];

function totalObjects(o: SchemaObjects): number {
  return (
    o.tables.length +
    o.views.length +
    o.procedures.length +
    o.functions.length +
    o.events.length +
    o.triggers.length
  );
}

/** Resolve the object list for a given kind key (singular) on SchemaObjects. */
function objectsFor(objects: SchemaObjects, key: ObjectKind): string[] {
  switch (key) {
    case 'table':
      return objects.tables.map((t) => t.name);
    case 'view':
      return objects.views.map((o) => o.name);
    case 'procedure':
      return objects.procedures.map((o) => o.name);
    case 'function':
      return objects.functions.map((o) => o.name);
    case 'event':
      return objects.events.map((o) => o.name);
    case 'trigger':
      return objects.triggers.map((o) => o.name);
  }
}

/** Resolve the raw schema-object entries (with summary) for a given kind key. */
function objectsForWithSummary(objects: SchemaObjects, key: ObjectKind) {
  switch (key) {
    case 'table':
      return objects.tables.map((t) => ({ name: t.name, summary: null as string | null }));
    case 'view':
      return objects.views;
    case 'procedure':
      return objects.procedures;
    case 'function':
      return objects.functions;
    case 'event':
      return objects.events;
    case 'trigger':
      return objects.triggers;
  }
}

// ----- per-table detail (two-level lazy load: 点开具体分组才拉取数据) -----

const DETAIL_GROUPS = [
  { key: 'fields', label: '字段' },
  { key: 'indexes', label: '索引' },
  { key: 'triggers', label: '触发器' },
] as const;

function TableDetail({
  conn,
  db,
  table,
}: {
  conn: DatabaseConnection;
  db: string;
  table: SchemaObjects['tables'][number];
}) {
  const detailKey = `${conn.id}::${db}::${table.name}`;
  const detail = useAppStore((s) => s.tableDetails[detailKey]);
  const loading = useAppStore((s) => !!s.tableDetailLoading[detailKey]);
  const sections = useAppStore((s) => s.detailSections);
  const toggleSection = useAppStore((s) => s.toggleDetailSection);
  const loadTableDetail = useAppStore((s) => s.loadTableDetail);

  // 首次展开任一分组时才拉取表详情（字段/索引/触发器一次加载，共享缓存）。
  const openSection = (sk: string) => {
    const sKey = `${detailKey}::${sk}`;
    if (!sections[sKey] && !detail && !loading) void loadTableDetail(conn, db, table.name);
    toggleSection(sKey);
  };

  const countOf = (sk: string): number | null => {
    if (!detail) return null;
    if (sk === 'fields') return detail.columns.length;
    if (sk === 'indexes') return detail.indexes.length;
    return detail.triggers?.length ?? 0;
  };

  return (
    <div className="table-detail" style={{ paddingLeft: 12 }}>
      {DETAIL_GROUPS.map((g) => {
        const sKey = `${detailKey}::${g.key}`;
        const open = !!sections[sKey];
        const count = countOf(g.key);
        return (
          <div key={g.key}>
            <div className="tree-row detail-group-row" onClick={() => openSection(g.key)}>
              <span className="caret">{open ? '▾' : '▸'}</span>
              <span className="row-name">{g.label}</span>
              {loading && !detail && <Loader size={10} aria-label="加载中" />}
              {count != null ? (
                <Text size="xs" c="dimmed">
                  {count}
                </Text>
              ) : null}
            </div>
            {open && (
              <div className="table-detail-section" style={{ paddingLeft: 16 }}>
                {!detail && loading && (
                  <Text size="xs" c="dimmed" style={{ padding: '2px 6px' }}>
                    加载中…
                  </Text>
                )}
                {detail && g.key === 'fields' && (
                  detail.columns.length > 0 ? (
                    detail.columns.map((c) => (
                      <div
                        key={c.name}
                        className="field-row"
                        title={`${c.data_type}${c.comment ? '  ·  ' + c.comment : ''}`}
                      >
                        <span className={`field-name ${c.is_primary_key ? 'pk' : ''}`}>{c.name}</span>
                        <span className="field-type" title={c.comment || c.data_type}>
                          {c.data_type}
                        </span>
                        {c.is_primary_key ? (
                          <Badge size="xs" variant="light" color="yellow" ml="auto">
                            PK
                          </Badge>
                        ) : null}
                        {c.comment ? <span className="field-comment">{c.comment}</span> : null}
                      </div>
                    ))
                  ) : (
                    <Text size="xs" c="dimmed" style={{ padding: '2px 6px' }}>
                      （无）
                    </Text>
                  )
                )}
                {detail && g.key === 'indexes' && (
                  detail.indexes.length > 0 ? (
                    detail.indexes.map((ix) => (
                      <div key={ix.name} className="index-row" title={ix.columns.join(', ')}>
                        <span className="field-name">{ix.name}</span>
                        <span className="field-type">{ix.columns.join(', ')}</span>
                        {ix.is_unique ? (
                          <Badge size="xs" variant="light" color="blue" ml="auto">
                            唯一
                          </Badge>
                        ) : null}
                      </div>
                    ))
                  ) : (
                    <Text size="xs" c="dimmed" style={{ padding: '2px 6px' }}>
                      （无）
                    </Text>
                  )
                )}
                {detail && g.key === 'triggers' && (
                  (detail.triggers?.length ?? 0) > 0 ? (
                    detail.triggers!.map((t) => (
                      <div key={t} className="field-row">
                        <span className="field-name">{t}</span>
                      </div>
                    ))
                  ) : (
                    <Text size="xs" c="dimmed" style={{ padding: '2px 6px' }}>
                      （无）
                    </Text>
                  )
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

// ----- leaf node with right-click (non-table objects: view/procedure/...) -----

function ObjectLeaf({
  name,
  connectionId,
  schema,
  kind,
  summary,
}: {
  name: string;
  connectionId: string;
  schema: string;
  kind: ObjectKind;
  summary?: string | null;
}) {
  const openDdl = useAppStore((s) => s.openDdl);
  const openContextMenu = useAppStore((s) => s.openContextMenu);

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    openContextMenu({
      x: e.clientX,
      y: e.clientY,
      connectionId,
      schema,
      object: name,
      kind,
    });
  };

  const onDouble = () => {
    openDdl({ connectionId, schema, object: name, kind, title: `${kind} ${name}` });
  };

  return (
    <div
      className="schema-leaf"
      onContextMenu={handleContextMenu}
      onDoubleClick={onDouble}
      title={`双击查看 DDL / 编辑结构${summary ? '  ·  ' + summary : ''}`}
    >
      <IconShield size={11} style={{ opacity: 0.5 }} />
      <span className="leaf-name">{name}</span>
    </div>
  );
}

// ----- connection row + its tree -----

function ConnectionTree({ conn }: { conn: DatabaseConnection }) {
  const dbList = useAppStore((s) => s.dbList);
  const objectsByDb = useAppStore((s) => s.objectsByDb);
  const objectsLoading = useAppStore((s) => s.objectsLoading);
  const expanded = useAppStore((s) => s.expanded);
  const toggleExpanded = useAppStore((s) => s.toggleExpanded);
  const expandNode = useAppStore((s) => s.expandNode);
  const loadDatabases = useAppStore((s) => s.loadDatabases);
  const loadObjects = useAppStore((s) => s.loadObjects);
  const activeConnectionId = useAppStore((s) => s.activeConnectionId);
  const setActiveConnection = useAppStore((s) => s.setActiveConnection);
  const connStatus = useAppStore((s) => s.connStatus);
  const openContextMenu = useAppStore((s) => s.openContextMenu);
  const openConnMenu = useAppStore((s) => s.openConnMenu);
  const openDataTableTab = useAppStore((s) => s.openDataTableTab);
  const tableExpanded = useAppStore((s) => s.tableExpanded);
  const toggleTableExpanded = useAppStore((s) => s.toggleTableExpanded);
  const dbFilter = useAppStore((s) => s.dbFilter);
  const setDbFilter = useAppStore((s) => s.setDbFilter);
  const activeDb = useAppStore((s) => s.activeDb);
  const setActiveDb = useAppStore((s) => s.setActiveDb);

  const connKey = conn.id;
  const connExpanded = !!expanded[connKey];
  const dbs = dbList[conn.id];
  const status = connStatus[conn.id] ?? 'idle';
  const dot = STATUS_DOT[status];
  // 库筛选（task 4）：即时过滤数据库列表，条件持久化到 localStorage。
  const dbFilterText = dbFilter[conn.id] ?? '';

  const toggleConn = () => {
    toggleExpanded(connKey);
    if (!connExpanded && !dbs) loadDatabases(conn);
  };

  const toggleDb = (db: string) => {
    const key = `${conn.id}::${db}`;
    toggleExpanded(key);
    if (!expanded[key] && !objectsByDb[key]) loadObjects(conn, db);
  };

  // 幂等展开：行点击 = 选中 + 展开（双击 = 2 次同步 click 仍保持展开——
  // React 批处理下两次 toggle 会 net-zero，必须用恒置 true 的 expandNode）。
  // 收起统一走行首箭头。
  const expandDb = (db: string) => {
    const key = `${conn.id}::${db}`;
    expandNode(key);
    if (!objectsByDb[key]) loadObjects(conn, db);
  };

  const toggleGroup = (db: string, group: string) => {
    toggleExpanded(`${conn.id}::${db}::${group}`);
  };

  // (Task 1) right-click on the connection row -> dedicated connection menu.
  const handleConnRowContext = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    openConnMenu({ x: e.clientX, y: e.clientY, connId: conn.id });
  };

  // 库筛选（task 4）：按库名即时过滤数据库列表（大小写不敏感）
  const matchDbFilter = (db: string) => {
    if (!dbFilterText.trim()) return true;
    const q = dbFilterText.trim().toLowerCase();
    return db.toLowerCase().includes(q);
  };
  const filteredDbs = useMemo(() => dbs?.filter(matchDbFilter) ?? [], [dbs, dbFilterText]);

  return (
    <div>
      {/* connection row */}
      <div
        className={`tree-row conn-row ${conn.id === activeConnectionId ? 'active' : ''}`}
        onClick={() => setActiveConnection(conn.id)}
        onContextMenu={handleConnRowContext}
        title="单击选中 · 点箭头展开数据库列表 · 右键连接操作"
      >
        <span
          className="caret"
          onClick={(e) => {
            e.stopPropagation();
            toggleConn();
          }}
        >
          {connExpanded ? '▾' : '▸'}
        </span>
        {(() => {
          const ki = CONN_KIND_ICON[conn.kind];
          return (
            <span
              style={{
                color: status === 'idle' ? ki.color : dot.color,
                display: 'inline-flex',
              }}
            >
              {ki.icon}
            </span>
          );
        })()}
        <span className="row-name">{conn.name || `${conn.host}:${conn.port}`}</span>
        {status === 'connecting' && <Loader size={10} aria-label="加载中" />}
        <span
          className="status-dot"
          title={`连接状态：${dot.label}`}
          style={{
            backgroundColor: dot.color,
            boxShadow: status === 'idle' ? undefined : dotGlow(dot.color),
          }}
        />
      </div>

      {/* 连接失败 -> toast 提示（store），这里只留重试线索，不常驻红条 */}

      {/* task 4: database-list filter（即时过滤，条件持久化） */}
      {connExpanded && conn.kind !== 'redis' && dbs && dbs.length > 0 && (
        <div className="conn-filter-row" onClick={(e) => e.stopPropagation()}>
          <TextInput
            className="db-filter-input"
            size="xs"
            placeholder="筛选数据库…"
            aria-label="筛选数据库"
            leftSection={<IconSearch size={12} />}
            rightSection={
              dbFilterText.trim() ? (
                <CloseButton
                  size="xs"
                  onClick={() => setDbFilter(conn.id, '')}
                  title="清除筛选（显示全部）"
                  aria-label="清除筛选"
                />
              ) : undefined
            }
            value={dbFilterText}
            onChange={(e) => setDbFilter(conn.id, e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') setDbFilter(conn.id, '');
            }}
            title="输入关键词即时过滤数据库列表；条件会保存，重启后仍生效"
          />
          {dbFilterText.trim() && (
            <Text size="xs" c="dimmed" className="db-filter-count">
              匹配 {(dbs ?? []).filter(matchDbFilter).length} / {dbs?.length ?? 0} 个库
            </Text>
          )}
        </div>
      )}

      {/* databases under this connection */}
      {connExpanded && conn.kind !== 'redis' && (
        <div className="tree-children" style={{ paddingLeft: 14 }}>
          {connExpanded && !dbs && status !== 'error' && (
            <Text size="xs" c="dimmed" style={{ padding: '2px 6px' }}>
              加载中…
            </Text>
          )}
          {connExpanded && !dbs && status === 'error' && (
            <Text size="xs" c="dimmed" style={{ padding: '2px 6px' }}>
              连接失败（收起后重新展开可重试）
            </Text>
          )}
          {dbs && dbs.length === 0 && (
            <Text size="xs" c="dimmed" style={{ padding: '2px 6px' }}>
              （无数据库）
            </Text>
          )}
          {dbs && dbs.length > 0 && dbFilterText.trim() && filteredDbs.length === 0 && (
            <Text size="xs" c="dimmed" style={{ padding: '2px 6px' }}>
              无匹配数据库
            </Text>
          )}
          {filteredDbs.map((db) => {
            const dbKey = `${conn.id}::${db}`;
            const dbExpanded = !!expanded[dbKey];
            const objects = objectsByDb[dbKey];
            return (
              <div key={db}>
                <div
                  className={`tree-row db-row${activeDb[conn.id] === db ? ' db-row-active' : ''}`}
                  onClick={() => {
                    setActiveDb(conn.id, db);
                    expandDb(db);
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    openContextMenu({
                      x: e.clientX,
                      y: e.clientY,
                      connectionId: conn.id,
                      schema: '',
                      object: db,
                      kind: 'database',
                    });
                  }}
                  title="点击设为当前数据库并展开对象 · 右键新建查询"
                >
                  <span
                    className="caret"
                    onClick={(e) => {
                      e.stopPropagation();
                      toggleDb(db);
                    }}
                    title="展开/收起"
                  >
                    {dbExpanded ? '▾' : '▸'}
                  </span>
                  <DbIcon db={db} />
                  <span className="row-name">{db}</span>
                  {objectsLoading[dbKey] && <Loader size={10} aria-label="加载中" />}
                  {activeDb[conn.id] === db && (
                    <Badge size="xs" variant="light" color="blue">
                      当前
                    </Badge>
                  )}
                  {dbExpanded && objects && (
                    <Badge size="xs" variant="light">
                      {totalObjects(objects)}
                    </Badge>
                  )}
                </div>

                {/* object groups under this db */}
                {dbExpanded && objects && (
                  <div className="tree-children" style={{ paddingLeft: 14 }}>
                    {OBJ_GROUPS.map((g) => {
                      const names = objectsFor(objects, g.key);
                      if (names.length === 0) return null;
                      const gKey = `${conn.id}::${db}::${g.key}`;
                      const gExpanded = !!expanded[gKey];
                      return (
                        <div key={g.key}>
                          <div
                            className="tree-row group-row"
                            onClick={() => toggleGroup(db, g.key)}
                            onContextMenu={(e) => {
                              e.preventDefault();
                              openContextMenu({
                                x: e.clientX,
                                y: e.clientY,
                                connectionId: conn.id,
                                schema: db,
                                object: g.key,
                                kind: 'group',
                              });
                            }}
                            title="左键展开/收起 · 右键更多操作"
                          >
                            <span className="caret">{gExpanded ? '▾' : '▸'}</span>
                            {g.icon}
                            <span className="row-name">{g.label}</span>
                            <Text size="xs" c="dimmed">
                              {names.length}
                            </Text>
                          </div>
                          {gExpanded && (
                            <div style={{ paddingLeft: 18 }}>
                              {g.key === 'table'
                                ? objects.tables
                                    .map((t) => {
                                      const tKey = `${conn.id}::${db}::table::${t.name}`;
                                      const tExpanded = !!tableExpanded[tKey];
                                      return (
                                        <div key={t.name}>
                                          <div
                                            className="table-row"
                                            onDoubleClick={() => openDataTableTab(conn, db, t.name)}
                                            onContextMenu={(e) => {
                                              e.preventDefault();
                                              openContextMenu({
                                                x: e.clientX,
                                                y: e.clientY,
                                                connectionId: conn.id,
                                                schema: db,
                                                object: t.name,
                                                kind: 'table',
                                              });
                                            }}
                                            title="双击打开数据 · 点箭头展开字段/索引/触发器 · 右键更多操作"
                                          >
                                            <span
                                              className="caret"
                                              onClick={(e) => {
                                                e.stopPropagation();
                                                toggleTableExpanded(tKey);
                                              }}
                                              title="展开/收起"
                                            >
                                              {tExpanded ? '▾' : '▸'}
                                            </span>
                                            <IconTable size={12} style={{ opacity: 0.7 }} />
                                            <span className="leaf-name">{t.name}</span>
                                            {t.comment ? (
                                              <span className="table-comment" title={t.comment}>
                                                {t.comment}
                                              </span>
                                            ) : null}
                                          </div>
                                          {tExpanded && <TableDetail conn={conn} db={db} table={t} />}
                                        </div>
                                      );
                                    })
                                : objectsForWithSummary(objects, g.key).map((o) => (
                                    <ObjectLeaf
                                      key={o.name}
                                      name={o.name}
                                      summary={o.summary}
                                      connectionId={conn.id}
                                      schema={db}
                                      kind={g.key}
                                    />
                                  ))}
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

// ----- main sidebar -----

export function Sidebar() {
  const connections = useAppStore((s) => s.connections);
  const loadConnections = useAppStore((s) => s.loadConnections);
  const openConnEditor = useAppStore((s) => s.openConnEditor);
  const sidebarWidth = useAppStore((s) => s.sidebarWidth);
  const setSidebarWidth = useAppStore((s) => s.setSidebarWidth);
  const [loading, setLoading] = useState(false);
  const dragRef = useRef(false);
  // 分组默认收起，点击分组头展开/收起（task 5）。
  const [groupOpen, setGroupOpen] = useState<Record<string, boolean>>({});

  useEffect(() => {
    setLoading(true);
    loadConnections()
      .catch((e) => console.error('Failed to load connections:', e))
      .finally(() => setLoading(false));
  }, [loadConnections]);

  const grouped = useMemo(() => {
    const map = new Map<string, DatabaseConnection[]>();
    for (const conn of connections) {
      const g = conn.group || 'Default';
      if (!map.has(g)) map.set(g, []);
      map.get(g)!.push(conn);
    }
    return [...map.entries()];
  }, [connections]);

  // task 5: drag the resizer to change the sidebar width
  const startDrag = (e: React.MouseEvent) => {
    e.preventDefault();
    dragRef.current = true;
    const move = (ev: MouseEvent) => {
      if (dragRef.current) setSidebarWidth(ev.clientX);
    };
    const up = () => {
      dragRef.current = false;
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  };

  return (
    <div
      className="sidebar"
      style={{ width: sidebarWidth, minWidth: sidebarWidth, position: 'relative' }}
    >
      <div className="sidebar-header">
        <IconDatabase size={16} />
        <span style={{ flex: 1 }}>数据库连接</span>
        <ActionIcon variant="subtle" onClick={() => loadConnections()} title="刷新">
          <IconRefresh size={14} />
        </ActionIcon>
        <ActionIcon variant="subtle" onClick={() => openConnEditor(null)} title="新建连接">
          <IconPlus size={14} />
        </ActionIcon>
      </div>

      <div className="sidebar-content">
        {connections.length === 0 && (
          <Text size="xs" c="dimmed" ta="center" mt="xl">
            {loading ? '正在加载连接…' : '还没有连接'}
            {!loading && (
              <>
                <br />
                点击右上角 + 新建
              </>
            )}
          </Text>
        )}

        {grouped.map(([group, conns]) => (
          <div key={group}>
            <div
              className={`group-header ${groupOpen[group] ? 'open' : ''}`}
              onClick={() => setGroupOpen((m) => ({ ...m, [group]: !m[group] }))}
              title={`分组「${group}」· ${conns.length} 个连接 · 点击展开/收起`}
            >
              <span className="caret">{groupOpen[group] ? '▾' : '▸'}</span>
              <IconFolder size={12} />
              <span className="group-name">{group}</span>
              <span className="group-count">{conns.length}</span>
            </div>
            {groupOpen[group] &&
              conns.map((conn) => <ConnectionTree key={conn.id} conn={conn} />)}
          </div>
        ))}
      </div>

      {/* task 5: resizer handle on the right edge of the sidebar */}
      <div className="sidebar-resizer" onMouseDown={startDrag} title="拖动调整宽度" />
    </div>
  );
}
