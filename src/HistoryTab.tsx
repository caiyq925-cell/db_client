import { useEffect, useState } from 'react';
import { Button, Group, Text } from '@mantine/core';
import { IconTrash } from '@tabler/icons-react';
import { api } from './api';
import type { QueryHistoryEntry } from './types';

function formatTime(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleString('zh-CN', { hour12: false });
}

export function HistoryTabView() {
  const [entries, setEntries] = useState<QueryHistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await api.getQueryHistory(null, 200);
      setEntries(list.reverse()); // newest first
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const clearAll = async () => {
    if (!confirm('确定清空全部查询历史？')) return;
    try {
      await api.clearQueryHistory();
      setEntries([]);
    } catch (e) {
      setError((e as Error).message);
    }
  };

  return (
    <div className="history-tab">
      <Group justify="space-between" mb="md">
        <Text size="sm" fw={600}>查询历史（最近 200 条）</Text>
        <Button
          size="xs"
          variant="light"
          color="red"
          leftSection={<IconTrash size={13} />}
          onClick={clearAll}
          disabled={entries.length === 0}
        >
          清空历史
        </Button>
      </Group>

      {error && <div className="error-banner">{error}</div>}

      {loading && <Text size="sm" c="dimmed">加载中...</Text>}

      {!loading && entries.length === 0 && (
        <div className="empty-state">
          <Text size="sm" c="dimmed">暂无历史记录</Text>
        </div>
      )}

      {entries.map((entry) => (
        <div key={entry.id} className={`history-item ${entry.success ? '' : 'failed'}`}>
          <div className="query-preview" title={entry.query}>
            {entry.query.replace(/\s+/g, ' ').slice(0, 160)}
          </div>
          <div className="query-meta">
            <span>{formatTime(entry.timestamp)}</span>
            <span>{entry.execution_time_ms} ms</span>
            <span>{entry.success ? '✓ 成功' : '✗ 失败'}</span>
          </div>
          {entry.error && (
            <div className="query-meta" style={{ color: 'var(--danger)' }}>
              {entry.error.slice(0, 200)}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
