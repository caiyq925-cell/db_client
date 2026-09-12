import { useEffect, useMemo, useState } from 'react';
import {
  ActionIcon,
  Badge,
  Group,
  Modal,
  Stack,
  Table,
  Text,
  TextInput,
  Tooltip,
} from '@mantine/core';
import { useDebouncedValue } from '@mantine/hooks';
import { IconDatabase, IconSearch } from '@tabler/icons-react';
import { api, errMessage } from './api';
import { useAppStore } from './store';
import type { ColumnInfo, TableInfo, TableRef } from './types';
import { VirtualizedList } from './VirtualizedList';

/** Height reserved around the two list columns so they fill the tab. */
const LIST_HEIGHT = 'calc(100vh - 130px)';

export function SchemaTabView({ connectionId }: { connectionId: string }) {
  const connections = useAppStore((s) => s.connections);
  const conn = connections.find((c) => c.id === connectionId);

  const [schemas, setSchemas] = useState<string[]>([]);
  const [activeSchema, setActiveSchema] = useState<string | null>(null);
  const [tables, setTables] = useState<TableInfo[]>([]);
  const [activeTable, setActiveTable] = useState<TableInfo | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // 表名过滤（客户端 substring，3000 项内存过滤瞬时）
  const [tableFilter, setTableFilter] = useState('');
  const [debouncedFilter] = useDebouncedValue(tableFilter, 150);
  const shownTables = useMemo(() => {
    const q = debouncedFilter.trim().toLowerCase();
    if (!q) return tables;
    return tables.filter((t) => t.name.toLowerCase().includes(q));
  }, [tables, debouncedFilter]);

  useEffect(() => {
    if (!conn) return;
    setLoading(true);
    setError(null);
    api
      .getSchemaList(conn)
      .then((list) => {
        setSchemas(list);
        if (list.length > 0) setActiveSchema(list[0]);
      })
      .catch((e) => setError(errMessage(e)))
      .finally(() => setLoading(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connectionId]);

  useEffect(() => {
    if (!conn || !activeSchema) return;
    setLoading(true);
    setError(null);
    setActiveTable(null);
    api
      .listTables(conn, activeSchema)
      .then(setTables)
      .catch((e) => setError(errMessage(e)))
      .finally(() => setLoading(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connectionId, activeSchema]);

  /** Open a table by (schema, name); MySQL loads full detail (columns/indexes/comment), Mongo loads sampled columns. */
  const openTableRef = async (schema: string, name: string) => {
    if (!conn) return;
    setActiveTable({
      name,
      schema,
      row_count: null,
      columns: [],
      indexes: [],
      comment: null,
      triggers: [],
    });
    try {
      if (conn.kind === 'mysql') {
        const detail = await api.getTableDetail(conn, schema, name);
        setActiveTable(detail);
      } else if (conn.kind === 'mongo') {
        const cols = await api.getTableColumns(conn, schema, name);
        setActiveTable((prev) =>
          prev && prev.schema === schema && prev.name === name
            ? { ...prev, columns: cols }
            : prev,
        );
      }
    } catch (e) {
      setError(errMessage(e));
    }
  };

  const openTable = (table: TableInfo) => openTableRef(table.schema, table.name);

  // ----- 跨库搜表 -----
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchInput, setSearchInput] = useState('');
  const [debouncedSearch] = useDebouncedValue(searchInput, 300);
  const [searchResults, setSearchResults] = useState<TableRef[] | null>(null);
  const [searching, setSearching] = useState(false);

  useEffect(() => {
    if (!searchOpen) {
      setSearchInput('');
      setSearchResults(null);
      return;
    }
    if (!conn) return;
    const q = debouncedSearch.trim();
    if (!q) {
      setSearchResults(null);
      return;
    }
    let cancelled = false;
    setSearching(true);
    api
      .searchTables(conn, q)
      .then((r) => {
        if (!cancelled) setSearchResults(r);
      })
      .catch((e) => {
        if (!cancelled) {
          setError(errMessage(e));
          setSearchResults([]);
        }
      })
      .finally(() => {
        if (!cancelled) setSearching(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [debouncedSearch, searchOpen]);

  const pickSearchResult = (r: TableRef) => {
    setSearchOpen(false);
    if (r.name === '') {
      // Mongo 库名命中：直接切换库
      setActiveSchema(r.schema);
    } else {
      openTableRef(r.schema, r.name);
    }
  };

  if (!conn) return <div className="empty-state"><Text>连接不存在</Text></div>;

  const isMongo = conn.kind === 'mongo';
  const isRedis = conn.kind === 'redis';

  return (
    <div className="schema-tab">
      {error && <div className="error-banner">{error}</div>}

      {isRedis ? (
        <Text size="sm" c="dimmed">
          Redis 是键值存储，没有表结构。使用查询标签页的 <code>KEYS *</code> 命令浏览键。
        </Text>
      ) : (
        <Group align="flex-start" gap="lg" wrap="nowrap">
          {/* Schema list */}
          {schemas.length > 1 && (
            <Stack gap={4} w={180} style={{ height: LIST_HEIGHT }}>
              <Text size="xs" c="dimmed" mb={4}>数据库 {schemas.length}</Text>
              <VirtualizedList
                items={schemas}
                getKey={(s) => s}
                renderItem={(s) => (
                  <div
                    className={`schema-list-item schema-row ${s === activeSchema ? 'active' : ''}`}
                    onClick={() => setActiveSchema(s)}
                    title={s}
                  >
                    {s}
                  </div>
                )}
                style={{ flex: 1 }}
              />
            </Stack>
          )}

          {/* Table list */}
          <Stack gap={4} w={240} style={{ height: LIST_HEIGHT }}>
            <Group gap="xs" wrap="nowrap">
              <Text size="xs" c="dimmed" style={{ flex: 1 }}>
                {isMongo ? '集合' : '表'} {tables.length > 0 && `· ${tables.length}`} {loading && '...'}
              </Text>
              <Tooltip label="跨库搜表">
                <ActionIcon size="sm" variant="subtle" color="gray" onClick={() => setSearchOpen(true)}>
                  <IconSearch size={14} />
                </ActionIcon>
              </Tooltip>
            </Group>
            <TextInput
              size="xs"
              placeholder={isMongo ? '过滤集合名…' : '过滤表名…'}
              value={tableFilter}
              onChange={(e) => setTableFilter(e.currentTarget.value)}
              spellCheck={false}
            />
            <VirtualizedList
              items={shownTables}
              getKey={(t) => t.name}
              renderItem={(t) => (
                <div
                  className={`schema-list-item schema-row ${activeTable?.name === t.name ? 'active' : ''}`}
                  onClick={() => openTable(t)}
                  title={t.name}
                >
                  <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis' }}>{t.name}</span>
                  {t.row_count != null && (
                    <Badge size="xs" variant="light">{t.row_count}</Badge>
                  )}
                </div>
              )}
              style={{ flex: 1 }}
            />
            {shownTables.length === 0 && !loading && (
              <Text size="xs" c="dimmed">
                {tables.length === 0 ? '（空）' : '无匹配表'}
              </Text>
            )}
          </Stack>

          {/* Table detail */}
          <div className="schema-detail" style={{ flex: 1, minWidth: 0, overflowX: 'auto' }}>
            {activeTable ? (
              <Stack gap="md">
                <Group>
                  <h4 style={{ margin: 0 }}>
                    {isMongo ? '集合' : '表'}: {activeTable.name}
                  </h4>
                  {activeTable.row_count != null && (
                    <Badge variant="light" color="blue">{activeTable.row_count} 行</Badge>
                  )}
                </Group>

                {activeTable.columns.length > 0 && (
                  <div>
                    <Text size="xs" c="dimmed" mb={4}>
                      {isMongo ? '采样推断字段' : '列'}
                    </Text>
                    <div style={{ maxHeight: 'calc(100vh - 260px)', overflowY: 'auto' }}>
                      <Table striped highlightOnHover withTableBorder>
                        <Table.Thead>
                          <Table.Tr>
                            <Table.Th>名称</Table.Th>
                            <Table.Th>类型</Table.Th>
                            <Table.Th>可空</Table.Th>
                            <Table.Th>默认值</Table.Th>
                            <Table.Th>主键</Table.Th>
                          </Table.Tr>
                        </Table.Thead>
                        <Table.Tbody>
                          {activeTable.columns.map((col: ColumnInfo) => (
                            <Table.Tr key={col.name}>
                              <Table.Td>{col.name}</Table.Td>
                              <Table.Td>{col.data_type}</Table.Td>
                              <Table.Td>{col.is_nullable ? '是' : '否'}</Table.Td>
                              <Table.Td>{col.default_value ?? '-'}</Table.Td>
                              <Table.Td>{col.is_primary_key ? '✓' : ''}</Table.Td>
                            </Table.Tr>
                          ))}
                        </Table.Tbody>
                      </Table>
                    </div>
                  </div>
                )}

                {activeTable.indexes.length > 0 && (
                  <div>
                    <Text size="xs" c="dimmed" mb={4}>索引</Text>
                    <Table striped highlightOnHover withTableBorder>
                      <Table.Thead>
                        <Table.Tr>
                          <Table.Th>名称</Table.Th>
                          <Table.Th>列</Table.Th>
                          <Table.Th>唯一</Table.Th>
                        </Table.Tr>
                      </Table.Thead>
                      <Table.Tbody>
                        {activeTable.indexes.map((idx) => (
                          <Table.Tr key={idx.name}>
                            <Table.Td>{idx.name}</Table.Td>
                            <Table.Td>{idx.columns.join(', ')}</Table.Td>
                            <Table.Td>{idx.is_unique ? '✓' : ''}</Table.Td>
                          </Table.Tr>
                        ))}
                      </Table.Tbody>
                    </Table>
                  </div>
                )}
              </Stack>
            ) : (
              <div className="empty-state">
                <Text size="sm" c="dimmed">选择一个{isMongo ? '集合' : '表'}查看结构</Text>
              </div>
            )}
          </div>
        </Group>
      )}

      {/* 跨库搜表弹层 */}
      <Modal
        opened={searchOpen}
        onClose={() => setSearchOpen(false)}
        title={
          <Group gap={6}>
            <IconDatabase size={15} style={{ color: 'var(--text-tertiary)' }} />
            <Text size="sm" fw={600}>跨库搜表</Text>
          </Group>
        }
        overlayProps={{ opacity: 0.55, color: '#000' }}
        classNames={{ root: 'app-modal' }}
      >
        <TextInput
          data-autofocus
          placeholder="输入表名关键字，跨全部数据库搜索…"
          value={searchInput}
          onChange={(e) => setSearchInput(e.currentTarget.value)}
          leftSection={<IconSearch size={14} />}
          spellCheck={false}
          mb="sm"
        />
        {isMongo && (
          <Text size="xs" c="dimmed" mb="sm">
            MongoDB 无跨库集合元数据查询：命中的为库名，点击切换到该库。
          </Text>
        )}
        {searching && <Text size="xs" c="dimmed">搜索中…</Text>}
        {searchResults != null && !searching && (
          searchResults.length === 0 ? (
            <Text size="xs" c="dimmed">无结果</Text>
          ) : (
            <div className="search-results">
              {searchResults.map((r) => (
                <div
                  key={`${r.schema}.${r.name}`}
                  className="schema-list-item"
                  onClick={() => pickSearchResult(r)}
                >
                  <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis' }}>
                    {r.name === '' ? r.schema : `${r.schema}.${r.name}`}
                  </span>
                  {r.row_count != null && (
                    <Badge size="xs" variant="light">{r.row_count}</Badge>
                  )}
                </div>
              ))}
            </div>
          )
        )}
      </Modal>
    </div>
  );
}
