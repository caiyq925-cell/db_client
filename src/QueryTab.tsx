import { useMemo, useState } from 'react';
import { ActionIcon, Button, Select, Text, Tooltip } from '@mantine/core';
import { IconDatabase, IconPlayerPlay, IconWand } from '@tabler/icons-react';
import { EditorView, keymap } from '@codemirror/view';
import { EditorState } from '@codemirror/state';
import { basicSetup } from 'codemirror';
import { sql } from '@codemirror/lang-sql';
import {
  HighlightStyle,
  StreamLanguage,
  StreamParser,
  syntaxHighlighting,
} from '@codemirror/language';
import { tags } from '@lezer/highlight';
import { javascript } from '@codemirror/legacy-modes/mode/javascript';
import { useEffect, useRef } from 'react';
import { api } from './api';
import { useAppStore, type QueryTab as QueryTabData } from './store';
import { downloadFile, rowsToCsv, rowsToJson, stamp } from './export';
import type { DatabaseConnection } from './types';

const DEFAULT_LIMITS = [100, 500, 1000, 5000, 0]; // 0 = no limit

const DB_PLACEHOLDERS: Record<string, string> = {
  mysql: 'SELECT * FROM users LIMIT 10',
  mongo: '{ "name": { "$regex": "^A" } }',
  redis: 'KEYS *',
};
// Redis has no official CM mode; a permissive tokenizer is enough for display.
const redisMode: StreamParser<unknown> = {
  token: (stream) => {
    stream.skipToEnd();
    return null;
  },
};

// 编辑器暗色主题：默认文字、光标、选区、活动行、行号沟槽的配色
// 与应用墨阶令牌保持一致（编辑区 = --bg-surface #1C1C21）。
// 行号沟槽化：沟槽底 #16161A（与侧边栏同阶）与编辑区形成视觉纵深，
// 当前行 rgba(76,141,255,0.08) 全宽底色 + 左侧 2px 主色指示条。
const queryTheme = EditorView.theme(
  {
    '&': { height: '100%', backgroundColor: '#1c1c21', color: '#e8e8ed' },
    '.cm-content': {
      caretColor: '#4c8dff',
      // 右侧安全边距：代码不贴边；80/120 字符参考线（1px 虚线 --border-subtle），
      // 以 ch 定位随内容横向滚动（background-attachment: local）
      paddingRight: '32px',
      backgroundImage:
        'repeating-linear-gradient(to bottom, #2a2a32 0 3px, transparent 3px 6px), repeating-linear-gradient(to bottom, #2a2a32 0 3px, transparent 3px 6px)',
      backgroundSize: '1px 100%, 1px 100%',
      backgroundPosition: '80ch 0, 120ch 0',
      backgroundRepeat: 'no-repeat, no-repeat',
      backgroundAttachment: 'local, local',
    },
    '.cm-cursor, .cm-dropCursor': { borderLeftColor: '#4c8dff' },
    // 光标闪烁：1000ms ease-in-out 柔和淡入淡出（覆盖默认 steps 硬切）。
    // 聚焦态需与 CM 基础主题同构选择器对抗特异性（&.cm-focused > .cm-scroller > .cm-cursorLayer），
    // 否则聚焦时会被 1.2s steps 硬切压回；CM 在光标移动时会内联改写 animation-name
    // （cm-blink/cm-blink2），但其 keyframes 无 steps，落入本规则的 1s ease-in-out 时序后仍是柔和渐隐。
    '&.cm-focused > .cm-scroller > .cm-cursorLayer': {
      animation: 'cm-cursor-fade 1s ease-in-out infinite',
    },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground': {
      backgroundColor: 'rgba(76, 141, 255, 0.22)',
    },
    '.cm-selectionMatch': { backgroundColor: 'rgba(76, 141, 255, 0.16)' },
    '.cm-gutters': {
      backgroundColor: '#16161a',
      borderRight: '1px solid #2a2a32',
      color: '#6b6b76',
    },
    '.cm-activeLine': { backgroundColor: 'rgba(76, 141, 255, 0.08)' },
    '.cm-activeLineGutter': {
      backgroundColor: 'rgba(76, 141, 255, 0.08)',
      boxShadow: 'inset 2px 0 0 #4c8dff',
      color: '#a0a0ab',
    },
    '.cm-lineNumbers .cm-gutterElement': { color: '#6b6b76' },
    '.cm-foldPlaceholder': {
      backgroundColor: 'transparent',
      border: 'none',
      color: '#6b6b76',
    },
    '.cm-matchingBracket': { backgroundColor: 'rgba(76, 141, 255, 0.28)', outline: 'none' },
  },
  { dark: true },
);

// 暗色语法高亮：Nordic Dark 校准，全部色相收在 蓝-青-绿-赭 扇区（杜绝紫色等高刺激色相），
// 饱和度 40-55%、明度 65-75% 对齐，暗底下连续阅读不眩光。
const queryHighlight = HighlightStyle.define([
  { tag: tags.keyword, color: '#81a1c1' },
  { tag: [tags.controlKeyword, tags.moduleKeyword], color: '#81a1c1' },
  { tag: [tags.string, tags.special(tags.string)], color: '#a3be8c' },
  { tag: [tags.number, tags.bool, tags.null], color: '#d08770' },
  { tag: [tags.comment, tags.meta], color: '#4c566a', fontStyle: 'italic' },
  { tag: [tags.operator, tags.derefOperator], color: '#88c0d0' },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName)], color: '#88c0d0' },
  { tag: [tags.variableName, tags.propertyName], color: '#c6c6ce' },
  { tag: [tags.typeName, tags.className], color: '#88c0d0' },
  // 括号提亮至近白，强化结构识别；其余标点保持次级灰
  { tag: tags.bracket, color: '#eceff4' },
  { tag: [tags.punctuation, tags.separator], color: '#a0a0ab' },
  { tag: [tags.definition(tags.variableName)], color: '#e8e8ed' },
  { tag: tags.invalid, color: '#f87171' },
]);

// 轻量 SQL 格式化：字符串字面量与注释先掩码保护，关键字统一大写后还原，
// 压缩 3 连以上空行——不做结构重排，保证不破坏原语义。
const SQL_KEYWORDS = [
  'SELECT', 'FROM', 'WHERE', 'AND', 'OR', 'NOT', 'NULL', 'AS', 'JOIN', 'LEFT', 'RIGHT',
  'INNER', 'OUTER', 'FULL', 'CROSS', 'ON', 'GROUP', 'BY', 'ORDER', 'HAVING', 'LIMIT',
  'OFFSET', 'INSERT', 'INTO', 'VALUES', 'UPDATE', 'SET', 'DELETE', 'CREATE', 'TABLE',
  'DROP', 'ALTER', 'VIEW', 'INDEX', 'DISTINCT', 'UNION', 'ALL', 'CASE', 'WHEN', 'THEN',
  'ELSE', 'END', 'BETWEEN', 'IN', 'EXISTS', 'ASC', 'DESC', 'LIKE', 'IS',
];

function formatSql(text: string): string {
  const masked: string[] = [];
  const shielded = text.replace(
    /'(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*"|--[^\n]*|\/\*[\s\S]*?\*\//g,
    (m) => {
      masked.push(m);
      return `\u0000${masked.length - 1}\u0000`;
    },
  );
  const keywordRe = new RegExp(`\\b(${SQL_KEYWORDS.join('|')})\\b`, 'gi');
  return shielded
    .replace(keywordRe, (w) => w.toUpperCase())
    .replace(/\n{3,}/g, '\n\n')
    .replace(/\u0000(\d+)\u0000/g, (_, i) => masked[Number(i)]);
}

function makeExtensions(
  dbKind: string,
  handlers: {
    onRun: () => void;
    onFormat?: () => void;
    onDocChange?: (text: string) => void;
    onCursor?: (line: number, col: number) => void;
  },
) {
  const lang =
    dbKind === 'mysql'
      ? sql()
      : dbKind === 'mongo'
        ? StreamLanguage.define(javascript)
        : StreamLanguage.define(redisMode);

  return [
    basicSetup,
    lang,
    // 追加在 basicSetup 之后以覆盖其默认（白底）高亮样式
    syntaxHighlighting(queryHighlight),
    keymap.of([
      {
        key: 'Mod-Enter',
        preventDefault: true,
        run: () => {
          handlers.onRun();
          return true;
        },
      },
      {
        key: 'Mod-Shift-f',
        preventDefault: true,
        run: () => {
          handlers.onFormat?.();
          return true;
        },
      },
    ]),
    queryTheme,
    // 文档/光标变化回调：驱动执行按钮「SQL 就绪」状态与状态栏 行:列
    EditorView.updateListener.of((u) => {
      if (u.docChanged) handlers.onDocChange?.(u.state.doc.toString());
      if (u.selectionSet) {
        const pos = u.state.selection.main.head;
        const line = u.state.doc.lineAt(pos);
        handlers.onCursor?.(line.number, pos - line.from + 1);
      }
    }),
  ];
}

export function QueryTabView({ tab }: { tab: QueryTabData }) {
  const connections = useAppStore((s) => s.connections);
  const updateQueryTab = useAppStore((s) => s.updateQueryTab);
  const activeDb = useAppStore((s) => s.activeDb);
  const setActiveDb = useAppStore((s) => s.setActiveDb);
  const dbList = useAppStore((s) => s.dbList);
  const loadDatabases = useAppStore((s) => s.loadDatabases);
  const objectsByDb = useAppStore((s) => s.objectsByDb);
  const loadObjects = useAppStore((s) => s.loadObjects);
  const editorRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const [limit, setLimit] = useState<string>('500');
  // mongo 查询目标集合（find 在该集合上执行；与库下拉联动重置）
  const [collection, setCollection] = useState<string>('');
  // 「SQL 就绪可执行」状态：编辑器有内容时执行按钮实填主色
  const [hasSql, setHasSql] = useState(() => !!(tab.queryText || DB_PLACEHOLDERS['mysql']));
  // 状态栏光标 行:列（selectionSet 时增量更新，开销可忽略）
  const [cursorPos, setCursorPos] = useState({ line: 1, col: 1 });

  const conn: DatabaseConnection | undefined = connections.find((c) => c.id === tab.connectionId);

  // 当前数据库：优先侧栏/下拉已选，回落到连接配置里的默认库。
  const currentDb = (conn && (activeDb[conn.id] || conn.database)) || '';
  const dbOptions = (conn && dbList[conn.id]) || null;

  // mongo：当前库的集合列表（复用侧栏 loadObjects 缓存）
  const collectionOptions =
    conn?.kind === 'mongo' ? objectsByDb[`${conn.id}::${currentDb}`]?.tables.map((t) => t.name) ?? null : null;

  // 库列表未加载时静默拉取，供下拉选择（与侧栏共用 dbList 缓存）
  useEffect(() => {
    if (conn && conn.kind !== 'redis' && !dbList[conn.id]) void loadDatabases(conn);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn?.id]);

  // mongo 切库时：拉取该库集合列表并重置集合选择
  useEffect(() => {
    if (conn && conn.kind === 'mongo' && currentDb) {
      void loadObjects(conn, currentDb);
      setCollection('');
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn?.id, currentDb]);

  const run = async () => {
    if (!conn) return;
    if (conn.kind === 'mongo' && !collection) {
      updateQueryTab(tab.id, { running: false, error: null, result: null });
      useAppStore.getState().showToast('请先在工具栏选择目标集合', 'error');
      return;
    }
    const queryText = viewRef.current?.state.doc.toString() ?? '';
    updateQueryTab(tab.id, { running: true, error: null, result: null, queryText });

    const start = performance.now();
    try {
      const result = await api.executeQuery(
        conn,
        queryText,
        limit === '0' ? undefined : Number(limit),
        currentDb || undefined,
        collection || undefined,
      );
      updateQueryTab(tab.id, { result, running: false });
      await api.addQueryHistory(conn.id, queryText, true, null, result.execution_time_ms);
    } catch (e) {
      const msg = (e as Error).message;
      updateQueryTab(tab.id, { running: false, error: msg });
      await api.addQueryHistory(conn.id, queryText, false, msg, Math.round(performance.now() - start)).catch(() => {});
    }
  };

  // 轻量格式化：仅 MySQL；掩码保护字符串/注释，关键字大写
  const formatDoc = () => {
    const view = viewRef.current;
    if (!view || conn?.kind !== 'mysql') return;
    const text = view.state.doc.toString();
    const formatted = formatSql(text);
    if (formatted === text) return;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: formatted } });
  };

  // Initialize CodeMirror once
  useEffect(() => {
    if (!editorRef.current) return;
    const runRef = { current: run };
    const formatRef = { current: formatDoc };

    const state = EditorState.create({
      doc: tab.queryText || DB_PLACEHOLDERS[conn?.kind ?? 'mysql'] || '',
      extensions: makeExtensions(conn?.kind ?? 'mysql', {
        onRun: () => runRef.current(),
        onFormat: () => formatRef.current(),
        onDocChange: (text) => setHasSql(text.trim().length > 0),
        onCursor: (line, col) => setCursorPos({ line, col }),
      }),
    });

    const view = new EditorView({ state, parent: editorRef.current });
    viewRef.current = view;

    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.id]);

  return (
    <div className="query-tab">
      <div className="editor-toolbar">
        <Button
          size="xs"
          className={`btn-run ${hasSql ? 'btn-run-ready' : ''}`}
          leftSection={<IconPlayerPlay size={13} />}
          onClick={run}
          loading={tab.running}
          disabled={!conn || tab.running}
        >
          执行 (Ctrl+Enter)
        </Button>
        <Select
          size="xs"
          w={110}
          aria-label="结果行数限制"
          value={limit}
          onChange={(v) => setLimit(v ?? '500')}
          data={DEFAULT_LIMITS.map((l) => ({ value: String(l), label: l === 0 ? '不限' : `限 ${l} 行` }))}
        />
        {conn && conn.kind !== 'redis' && (
          <Select
            size="xs"
            w={180}
            aria-label="当前数据库"
            placeholder={dbOptions ? '选择数据库' : '加载库列表…'}
            value={currentDb || null}
            onChange={(v) => v && conn && setActiveDb(conn.id, v)}
            data={(dbOptions ?? []).map((db) => ({ value: db, label: db }))}
            title="SQL 将在该库下执行；与侧栏点击选中的库联动"
            allowDeselect={false}
          />
        )}
        {conn && conn.kind === 'mongo' && (
          <Select
            size="xs"
            w={200}
            aria-label="目标集合"
            placeholder={collectionOptions ? '选择集合' : '加载集合…'}
            value={collection || null}
            onChange={(v) => v && setCollection(v)}
            data={(collectionOptions ?? []).map((c) => ({ value: c, label: c }))}
            title="查询 filter 将在该集合上执行 find"
            allowDeselect={false}
          />
        )}
        {conn && conn.kind === 'redis' && currentDb && (
          <Text size="xs" c="dimmed">
            db: {currentDb}
          </Text>
        )}
        <Text size="xs" c="dimmed">
          {conn ? `${conn.name} (${conn.kind})` : '未选择连接'}
        </Text>
        <span style={{ flex: 1 }} />
        {tab.result && tab.result.rows.length > 0 && (
          <>
            <Button
              size="xs"
              className="btn-ghost"
              onClick={() => downloadFile(rowsToCsv(tab.result!.columns, tab.result!.rows), `query-${stamp()}.csv`, 'text/csv')}
            >
              导出 CSV
            </Button>
            <Button
              size="xs"
              className="btn-ghost"
              onClick={() => downloadFile(rowsToJson(tab.result!.rows), `query-${stamp()}.json`, 'application/json')}
            >
              导出 JSON
            </Button>
          </>
        )}
        {conn?.kind === 'mysql' && (
          <Tooltip label="Ctrl+Shift+F" position="bottom" withArrow>
            <ActionIcon
              size={28}
              className="btn-format"
              variant="default"
              aria-label="格式化 SQL（Ctrl+Shift+F）"
              onClick={formatDoc}
            >
              <IconWand size={14} aria-hidden="true" />
            </ActionIcon>
          </Tooltip>
        )}
      </div>

      <div className="editor-pane">
        <div className="editor-container" ref={editorRef} />
        {/* 底部状态栏：信息降噪（11px / tertiary），错误区仅在出错时出现 */}
        <div className="query-statusbar">
          {tab.error && (
            <button
              type="button"
              className="qs-error"
              title="点击聚焦编辑器"
              onClick={() => viewRef.current?.focus()}
            >
              ⚠ 执行出错
            </button>
          )}
          <span className="qs-right">
            <span className="qs-info">{cursorPos.line}:{cursorPos.col}</span>
            <span className="qs-info">UTF-8</span>
            <span className="qs-info">
              {conn?.kind === 'mysql' ? 'MySQL' : conn?.kind === 'mongo' ? 'MongoDB' : conn?.kind === 'redis' ? 'Redis' : '未连接'}
            </span>
          </span>
        </div>
      </div>

      <div className="results-pane">
        {tab.running && <div className="run-progress" aria-hidden="true" />}
        {tab.error && <div className="error-banner">{tab.error}</div>}

        {tab.result && (
          <>
            <div className="results-toolbar">
              <span>{tab.result.rows_affected != null ? `${tab.result.rows_affected} rows affected` : `${tab.result.rows.length} rows`}</span>
              <span>{tab.result.execution_time_ms} ms</span>
              {tab.result.truncated && <Text size="xs" c="orange">结果已截断</Text>}
              {tab.result.message && <Text size="xs" c="dimmed">{tab.result.message}</Text>}
            </div>
            {tab.result.rows.length > 0 && <ResultsTable columns={tab.result.columns} rows={tab.result.rows} />}
            {tab.result.rows.length === 0 && tab.result.rows_affected == null && (
              <div className="empty-state">
                <Text size="sm" c="dimmed">查询结果为空</Text>
              </div>
            )}
          </>
        )}

        {!tab.result && !tab.error && !tab.running && (
          <div className="empty-state">
            <IconDatabase size={72} stroke={1} className="empty-icon" aria-hidden="true" />
            <Text size="sm" c="dimmed">
              {conn?.kind === 'mysql' && '输入 SQL 后按 Ctrl+Enter 执行'}
              {conn?.kind === 'mongo' && '输入 JSON filter 后按 Ctrl+Enter 执行'}
              {conn?.kind === 'redis' && '输入 Redis 命令后按 Ctrl+Enter 执行'}
              {!conn && '请先在左侧选择一个连接'}
            </Text>
          </div>
        )}
      </div>
    </div>
  );
}

export function ResultsTable({
  columns,
  rows,
}: {
  columns: string[];
  rows: Record<string, unknown>[];
}) {
  const displayCols = useMemo(
    () => (columns.length > 0 ? columns : Object.keys(rows[0] ?? {})),
    [columns, rows],
  );

  const renderCell = (v: unknown) => {
    if (v === null || v === undefined) return <span className="cell-null">NULL</span>;
    if (typeof v === 'object') return JSON.stringify(v);
    return String(v);
  };

  return (
    <table className="results-table">
      <thead>
        <tr>
          <th>#</th>
          {displayCols.map((col) => (
            <th key={col}>{col}</th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row, i) => (
          <tr key={i}>
            <td className="cell-null">{i + 1}</td>
            {displayCols.map((col) => (
              <td key={col} title={typeof row[col] === 'object' ? JSON.stringify(row[col]) : String(row[col] ?? '')}>
                {renderCell(row[col])}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}
