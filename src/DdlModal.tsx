import { useEffect, useState } from 'react';
import { Button, Group, SegmentedControl, Text } from '@mantine/core';
import { IconPlayerPlay, IconRefresh } from '@tabler/icons-react';
import { useAppStore } from './store';
import { api } from './api';

/**
 * Modal for viewing / editing a schema object's DDL (SHOW CREATE ...).
 * "编辑结构" mode opens directly in edit state with an Execute button.
 */
export function DdlModal() {
  const target = useAppStore((s) => s.ddlTarget);
  const closeDdl = useAppStore((s) => s.closeDdl);
  const connections = useAppStore((s) => s.connections);

  const [mode, setMode] = useState<'view' | 'edit'>('view');
  const [ddl, setDdl] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [execResult, setExecResult] = useState<string | null>(null);

  const conn = target ? connections.find((c) => c.id === target.connectionId) : null;

  const loadDdl = async () => {
    if (!conn || !target) return;
    setLoading(true);
    setError(null);
    setExecResult(null);
    try {
      const info = await api.getObjectDdl(conn, target.schema, target.object, target.kind);
      setDdl(info.ddl);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  };

  // Reset state each time the target changes; jump to edit if requested.
  useEffect(() => {
    if (target) {
      setMode(target.edit ? 'edit' : 'view');
      loadDdl();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target?.connectionId, target?.schema, target?.object, target?.kind]);

  const onExecute = async () => {
    if (!conn || !target) return;
    setLoading(true);
    setExecResult(null);
    setError(null);
    try {
      const result = await api.executeQuery(conn, ddl, undefined);
      const msg = result.rows_affected != null
        ? `执行成功（影响 ${result.rows_affected} 行，耗时 ${result.execution_time_ms}ms）`
        : `执行成功（耗时 ${result.execution_time_ms}ms）`;
      setExecResult(msg);
      // DDL often changes the structure — refresh the object list
      await useAppStore.getState().loadObjects(conn, target.schema);
      // reload the DDL to show the new version
      await loadDdl();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  };

  if (!target) return null;

  const isTable = target.kind === 'table' || target.kind === 'view';

  return (
    <div className="ddl-modal-overlay" onClick={closeDdl}>
      <div className="ddl-modal" onClick={(e) => e.stopPropagation()}>
        <div className="ddl-modal-header">
          <Text fw={600}>{target.title}</Text>
          <Group>
            <SegmentedControl
              size="xs"
              value={mode}
              onChange={(v) => setMode(v as 'view' | 'edit')}
              data={[
                { value: 'view', label: '查看' },
                { value: 'edit', label: '编辑' },
              ]}
            />
            <Button
              size="xs"
              variant="light"
              leftSection={<IconRefresh size={13} />}
              onClick={loadDdl}
            >
              刷新
            </Button>
            <Button size="xs" variant="light" onClick={closeDdl}>
              关闭
            </Button>
          </Group>
        </div>

        {loading && <Text size="xs" c="dimmed" p="xs">加载中…</Text>}

        {!loading && mode === 'view' && (
          <pre className="ddl-readonly">{ddl || '(无法获取 DDL)'}</pre>
        )}

        {!loading && mode === 'edit' && (
          <>
            <textarea
              className="ddl-textarea"
              value={ddl}
              onChange={(e) => setDdl(e.target.value)}
              spellCheck={false}
            />
            <div className="ddl-modal-foot">
              <Text size="xs" c="dimmed" style={{ marginRight: 'auto' }}>
                执行将直接作用到数据库，请谨慎
              </Text>
              <Button
                size="xs"
                leftSection={<IconPlayerPlay size={13} />}
                onClick={onExecute}
              >
                执行 DDL
              </Button>
            </div>
          </>
        )}

        {error && <div className="ddl-result err" style={{ margin: 8 }}>{error}</div>}
        {execResult && <div className="ddl-result ok" style={{ margin: 8 }}>{execResult}</div>}

        {!isTable && mode === 'view' && (
          <Text size="xs" c="dimmed" style={{ padding: 8 }}>
            提示：该对象类型为 {target.kind}，可切到「编辑」修改并执行。
          </Text>
        )}
      </div>
    </div>
  );
}
