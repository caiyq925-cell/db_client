import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { ActionIcon, Button, Drawer, Group, SegmentedControl, Text } from '@mantine/core';
import {
  IconCheck,
  IconChevronLeft,
  IconChevronRight,
  IconChevronsLeft,
  IconDatabase,
  IconDownload,
  IconInfoCircle,
  IconListNumbers,
} from '@tabler/icons-react';
import { useAppStore, type DataTableTab } from './store';
import type { ColumnInfo } from './types';
import { ExportDialog } from './ExportDialog';

/** Render a cell value for display when not editing. */
function displayCell(v: unknown): string {
  if (v === null || v === undefined) return '';
  if (typeof v === 'object') return JSON.stringify(v);
  return String(v);
}

/** 行数展示：大数缩写（万/亿），完整数字走 title 提示。 */
function formatCount(n: number): string {
  if (n >= 100_000_000) return `${(n / 100_000_000).toFixed(2)} 亿`;
  if (n >= 10_000) return `${(n / 10_000).toFixed(1)} 万`;
  return n.toLocaleString('en-US');
}

export function DataTableTabView({ tab }: { tab: DataTableTab }) {
  const updateDataTableTab = useAppStore((s) => s.updateDataTableTab);
  const saveDataTableTab = useAppStore((s) => s.saveDataTableTab);
  const nextDataTablePage = useAppStore((s) => s.nextDataTablePage);
  const prevDataTablePage = useAppStore((s) => s.prevDataTablePage);
  const firstDataTablePage = useAppStore((s) => s.firstDataTablePage);
  const countDataTableRows = useAppStore((s) => s.countDataTableRows);
  const conn = useAppStore((s) => s.connections.find((c) => c.id === tab.connectionId));
  const connKind = conn?.kind;
  const connIsMysql = connKind === 'mysql';
  const [copied, setCopied] = useState<string | null>(null);
  // 导出弹窗（C3：仅 MySQL 支持流式导出）
  const [exportOpen, setExportOpen] = useState(false);
  // 当前正在编辑的单元格（默认查看模式，双击进入编辑）。
  const [editingCell, setEditingCell] = useState<{ row: number; col: string } | null>(null);
  const [draft, setDraft] = useState('');
  const copyTimer = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (copyTimer.current) window.clearTimeout(copyTimer.current);
    },
    [],
  );

  const isDirtyRow = (i: number) => {
    const orig = tab.originalRows[i];
    if (!orig) return false;
    return tab.columns.some((c) => {
      if (tab.pkColumns.includes(c.name)) return false;
      return String(tab.rows[i][c.name] ?? '') !== String(orig[c.name] ?? '');
    });
  };

  const dirtyCount = useMemo(
    () => tab.rows.filter((_, i) => isDirtyRow(i)).length,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tab],
  );

  const startEdit = (i: number, col: string) => {
    if (!tab.editable || tab.pkColumns.includes(col)) return;
    setEditingCell({ row: i, col });
    setDraft(displayCell(tab.rows[i][col]));
  };

  const commitEdit = () => {
    if (!editingCell) return;
    const { row: i, col } = editingCell;
    setEditingCell(null);
    const next = draft === '' ? null : draft;
    const current = tab.rows[i][col];
    if (String(current ?? '') === String(next ?? '')) return; // 值未变化
    const rows = tab.rows.map((r, idx) => (idx === i ? { ...r, [col]: next } : r));
    updateDataTableTab(tab.id, { rows });
  };

  const cancelEdit = () => setEditingCell(null);

  const copyColumn = async (col: string) => {
    try {
      await navigator.clipboard.writeText(col);
    } catch {
      /* clipboard may be unavailable; ignore */
    }
    setCopied(col);
    if (copyTimer.current) window.clearTimeout(copyTimer.current);
    copyTimer.current = window.setTimeout(() => setCopied(null), 1200);
  };

  // ----- 列宽（task 3/9）：初始按容器宽度均分，全部列都有显式宽度（fixed 布局前提）；
  // 拖动时只加宽当前列并把增量同步到表格总宽，表格超出容器出现横向滚动条，
  // 其余列宽保持不变、不被压缩。拖动中直接操作 DOM，松手才写一次 state。 -----
  const bodyRef = useRef<HTMLDivElement>(null);
  const tableRef = useRef<HTMLTableElement>(null);
  const [colWidths, setColWidths] = useState<Record<string, number>>({});
  const initedForRef = useRef('');
  const resizeRef = useRef<{
    col: string;
    startX: number;
    startW: number;
    startTotal: number;
    th: HTMLElement;
  } | null>(null);

  useLayoutEffect(() => {
    if (!tab.loaded || tab.columns.length === 0) return;
    const key = `${tab.schema}.${tab.table}`;
    if (initedForRef.current === key) return;
    initedForRef.current = key;
    // 一次性读取容器宽度（批量读），随后只做写操作
    const w = bodyRef.current?.clientWidth ?? 0;
    const share = Math.max(96, Math.floor((w - 56) / tab.columns.length));
    const next: Record<string, number> = {};
    for (const c of tab.columns) next[c.name] = share;
    setColWidths(next);
  }, [tab.loaded, tab.columns, tab.schema, tab.table]);

  const hasWidths =
    tab.columns.length > 0 && tab.columns.every((c) => colWidths[c.name] != null);
  const totalColW = 56 + tab.columns.reduce((acc, c) => acc + (colWidths[c.name] ?? 0), 0);

  const startResize = (col: string) => (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const th = (e.currentTarget as HTMLElement).closest('th');
    if (!th) return;
    resizeRef.current = {
      col,
      startX: e.clientX,
      startW: th.offsetWidth,
      startTotal: tableRef.current?.offsetWidth ?? 0,
      th,
    };
    const move = (ev: MouseEvent) => {
      const r = resizeRef.current;
      if (!r) return;
      const w = Math.max(48, r.startW + ev.clientX - r.startX);
      r.th.style.width = `${w}px`;
      r.th.style.minWidth = `${w}px`;
      // 增量同步表格总宽：其他列不受影响，多出的宽度交给横向滚动条
      if (tableRef.current) {
        tableRef.current.style.width = `${r.startTotal + (w - r.startW)}px`;
      }
    };
    const up = () => {
      const r = resizeRef.current;
      if (r) {
        setColWidths((m) => ({ ...m, [r.col]: r.th.offsetWidth }));
        resizeRef.current = null;
      }
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
      document.body.classList.remove('col-resizing');
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
    document.body.classList.add('col-resizing');
  };

  // 键盘替代：拖拽手柄聚焦时 ←/→ 微调列宽
  const nudgeWidth = (col: string, delta: number) => {
    setColWidths((m) => ({ ...m, [col]: Math.max(48, (m[col] ?? 96) + delta) }));
  };

  // ----- 行详情（task 5）：双击行号打开，form/json 两种视图 -----
  const [detailRow, setDetailRow] = useState<number | null>(null);
  const [detailFormat, setDetailFormat] = useState<'form' | 'json'>('form');
  const [copiedField, setCopiedField] = useState<string | null>(null);

  const copyValue = async (label: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      /* clipboard may be unavailable; ignore */
    }
    setCopiedField(label);
    if (copyTimer.current) window.clearTimeout(copyTimer.current);
    copyTimer.current = window.setTimeout(() => setCopiedField(null), 1200);
  };

  const detailRowData = detailRow != null ? tab.rows[detailRow] : null;

  const colHeaderTitle = (c: ColumnInfo) => {
    const bits: string[] = [c.data_type];
    if (c.comment) bits.push(`说明: ${c.comment}`);
    if (c.is_primary_key) bits.push('主键');
    return bits.join(' · ');
  };

  return (
    <div className="datatable-tab">
      <div className="datatable-toolbar">
        <Group gap={8}>
          <IconDatabase size={15} style={{ color: 'var(--text-tertiary)' }} />
          <Text size="sm" c="dimmed">
            {tab.schema}.{tab.table}
          </Text>
          {tab.totalRows != null && (
            <Text size="xs" c="dimmed" title={`估算行数 ${tab.totalRows.toLocaleString('en-US')}`}>
              约 {formatCount(tab.totalRows)} 行
            </Text>
          )}
          {tab.exactCount != null ? (
            <Text size="xs" c="dimmed" title={`COUNT(*) = ${tab.exactCount.toLocaleString('en-US')}`}>
              共 {formatCount(tab.exactCount)} 行
            </Text>
          ) : (
            connIsMysql && (
              <Button
                size="compact-xs"
                variant="subtle"
                leftSection={<IconListNumbers size={12} />}
                loading={tab.counting}
                onClick={() => countDataTableRows(tab.id)}
                title="对大表执行 COUNT(*) 可能很慢"
              >
                精确计数
              </Button>
            )
          )}
        </Group>
        <span style={{ flex: 1 }} />
        {/* 翻页（keyset / OFFSET fallback，由 store 决定 nav 模式） */}
        <Group gap={4}>
          <ActionIcon
            size="sm"
            variant="default"
            disabled={tab.pageIndex === 0 || tab.loadingPage}
            onClick={() => firstDataTablePage(tab.id)}
            title="回到第一页"
            aria-label="回到第一页"
          >
            <IconChevronsLeft size={14} />
          </ActionIcon>
          <ActionIcon
            size="sm"
            variant="default"
            disabled={tab.pageIndex === 0 || tab.loadingPage}
            onClick={() => prevDataTablePage(tab.id)}
            title="上一页"
            aria-label="上一页"
          >
            <IconChevronLeft size={14} />
          </ActionIcon>
          <Text size="xs" c="dimmed" style={{ whiteSpace: 'nowrap' }}>
            第 {tab.pageIndex + 1} 页{tab.noPk ? ' · OFFSET' : ''}
          </Text>
          <ActionIcon
            size="sm"
            variant="default"
            loading={tab.loadingPage}
            disabled={!tab.hasMore && !tab.loadingPage}
            onClick={() => nextDataTablePage(tab.id)}
            title="下一页"
            aria-label="下一页"
          >
            <IconChevronRight size={14} />
          </ActionIcon>
        </Group>
        {copied && (
          <Text size="xs" c="teal">
            已复制列「{copied}」
          </Text>
        )}
        <span style={{ flex: 1 }} />
        {!tab.editable && (
          <Text size="xs" c="dimmed">
            {tab.pkColumns.length === 0 ? '（无主键，仅可查看）' : '只读'}
          </Text>
        )}
        {connIsMysql && (
          <Button
            size="xs"
            variant="light"
            leftSection={<IconDownload size={13} />}
            onClick={() => setExportOpen(true)}
            title="流式导出为 CSV / JSON Lines"
          >
            导出
          </Button>
        )}
        <Text size="xs" c={dirtyCount > 0 ? 'orange' : 'dimmed'}>
          {dirtyCount > 0 ? `${dirtyCount} 行已修改` : '无修改'}
        </Text>
        {tab.saving ? (
          <Button size="xs" leftSection={<IconCheck size={13} />} loading disabled>
            保存中…
          </Button>
        ) : (
          <Button
            size="xs"
            leftSection={<IconCheck size={13} />}
            disabled={dirtyCount === 0 || !tab.editable}
            onClick={() => saveDataTableTab(tab.id)}
          >
            保存
          </Button>
        )}
      </div>

      {tab.error && <div className="error-banner">{tab.error}</div>}

      {conn && (
        <ExportDialog
          opened={exportOpen}
          onClose={() => setExportOpen(false)}
          conn={conn}
          schema={tab.schema}
          table={tab.table}
        />
      )}

      <div className="datatable-body" ref={bodyRef}>
        {!tab.loaded ? (
          <div className="empty-state">
            <Text size="sm" c="dimmed">
              加载中…
            </Text>
          </div>
        ) : tab.rows.length === 0 ? (
          <div className="empty-state">
            <Text size="sm" c="dimmed">
              该表没有数据
            </Text>
          </div>
        ) : (
          <table
            ref={tableRef}
            className="datatable"
            style={
              hasWidths
                ? { tableLayout: 'fixed', width: totalColW, minWidth: '100%' }
                : { tableLayout: 'fixed' }
            }
          >
            <thead>
              <tr>
                <th className="row-num" style={{ width: 56, minWidth: 56 }} title="双击行号查看该行详情">
                  行
                </th>
                {tab.columns.map((c) => (
                  <th
                    key={c.name}
                    title={`${colHeaderTitle(c)} · 右键复制列名`}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      copyColumn(c.name);
                    }}
                    className={tab.pkColumns.includes(c.name) ? 'pk-col' : ''}
                    style={colWidths[c.name] ? { width: colWidths[c.name] } : undefined}
                  >
                    <div className="th-inner">
                      {c.name}
                      {c.is_primary_key && <span className="pk-badge">PK</span>}
                      {c.comment ? <IconInfoCircle size={11} style={{ marginLeft: 4, color: 'var(--text-tertiary)' }} /> : null}
                    </div>
                    <div
                      className="col-resizer"
                      role="separator"
                      aria-orientation="vertical"
                      aria-label={`调整列 ${c.name} 宽度`}
                      tabIndex={0}
                      onMouseDown={startResize(c.name)}
                      onKeyDown={(e) => {
                        if (e.key === 'ArrowLeft') nudgeWidth(c.name, -24);
                        else if (e.key === 'ArrowRight') nudgeWidth(c.name, 24);
                      }}
                      title="拖动调整列宽（聚焦后可用 ←/→）"
                    />
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {tab.rows.map((row, i) => {
                const dirty = isDirtyRow(i);
                return (
                  <tr key={i} className={dirty ? 'row-dirty' : ''}>
                    <td
                      className="cell-null row-num-cell"
                      title="双击查看行详情"
                      onDoubleClick={() => setDetailRow(i)}
                    >
                      {i + 1}
                    </td>
                    {tab.columns.map((c) => {
                      const isPk = tab.pkColumns.includes(c.name);
                      const value = row[c.name];
                      const orig = tab.originalRows[i]?.[c.name];
                      const changed =
                        !isPk && String(value ?? '') !== String(orig ?? '');
                      const editing =
                        editingCell?.row === i && editingCell?.col === c.name;
                      if (!tab.editable || isPk) {
                        return (
                          <td
                            key={c.name}
                            className={isPk ? 'cell-pk' : ''}
                            title={colHeaderTitle(c)}
                          >
                            {displayCell(value)}
                            {value === null || value === undefined ? <span className="cell-null">NULL</span> : null}
                          </td>
                        );
                      }
                      return (
                        <td
                          key={c.name}
                          className={`cell-editable ${changed ? 'cell-changed' : ''}`}
                          title={`${colHeaderTitle(c)} · 双击编辑`}
                          onDoubleClick={() => startEdit(i, c.name)}
                        >
                          {editing ? (
                            <input
                              className="cell-input"
                              value={draft}
                              autoFocus
                              onChange={(e) => setDraft(e.currentTarget.value)}
                              onBlur={commitEdit}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') commitEdit();
                                else if (e.key === 'Escape') cancelEdit();
                                e.stopPropagation();
                              }}
                              spellCheck={false}
                            />
                          ) : (
                            <>
                              {displayCell(value)}
                              {value === null || value === undefined ? <span className="cell-null">NULL</span> : null}
                            </>
                          )}
                        </td>
                      );
                    })}
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      {/* 行详情抽屉（task 5）：双击行号打开 */}
      <Drawer
        opened={detailRow !== null}
        onClose={() => setDetailRow(null)}
        position="right"
        size={460}
        title={<Text size="sm" fw={600}>行数据详情{detailRow != null ? ` · 第 ${detailRow + 1} 行` : ''}</Text>}
        overlayProps={{ opacity: 0.55, color: '#000' }}
      >
        {detailRowData && (
          <div className="row-detail">
            <Group justify="space-between" mb="sm">
              <SegmentedControl
                size="xs"
                value={detailFormat}
                onChange={(v) => setDetailFormat(v as 'form' | 'json')}
                data={[
                  { value: 'form', label: '字段' },
                  { value: 'json', label: 'JSON' },
                ]}
              />
              <Button
                size="xs"
                variant="light"
                onClick={() =>
                  copyValue(
                    '__all__',
                    detailFormat === 'json'
                      ? JSON.stringify(detailRowData, null, 2)
                      : tab.columns.map((c) => `${c.name}: ${displayCell(detailRowData[c.name]) ?? 'NULL'}`).join('\n'),
                  )
                }
              >
                {copiedField === '__all__' ? '已复制' : '复制全部'}
              </Button>
            </Group>

            {detailFormat === 'form' ? (
              <div className="row-detail-fields">
                {tab.columns.map((c) => {
                  const value = detailRowData[c.name];
                  const isNull = value === null || value === undefined;
                  const isPk = tab.pkColumns.includes(c.name);
                  return (
                    <div key={c.name} className="row-detail-field">
                      <div className="rdf-name">
                        {c.name}
                        {isPk && <span className="pk-badge">PK</span>}
                        <span className="rdf-type">{c.data_type}</span>
                      </div>
                      <div className={`rdf-value ${isNull ? 'is-null' : ''}`}>
                        <span className="rdf-text" title={isNull ? 'NULL' : displayCell(value)}>
                          {isNull ? 'NULL' : displayCell(value)}
                        </span>
                        <button
                          className="rdf-copy"
                          title="复制该字段值"
                          onClick={() => copyValue(c.name, isNull ? '' : displayCell(value))}
                        >
                          {copiedField === c.name ? '✓' : '复制'}
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            ) : (
              <pre className="row-detail-json">{JSON.stringify(detailRowData, null, 2)}</pre>
            )}
          </div>
        )}
      </Drawer>
    </div>
  );
}
