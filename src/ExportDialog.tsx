import { useEffect, useRef, useState } from 'react';
import {
  Alert,
  Button,
  Group,
  Modal,
  NumberInput,
  Progress,
  SegmentedControl,
  Stack,
  Text,
  TextInput,
} from '@mantine/core';
import { save } from '@tauri-apps/plugin-dialog';
import {
  IconAlertCircle,
  IconCheck,
  IconFileExport,
} from '@tabler/icons-react';
import { api, errMessage, type ExportFormat, type ExportSummary } from './api';
import type { DatabaseConnection } from './types';

/** Default export row cap (mirrors Rust EXPORT_MAX_ROWS_DEFAULT). */
const MAX_ROWS_DEFAULT = 50_000_000;

type Phase = 'idle' | 'running' | 'done' | 'error';

/** 1234567 → "1.2 MB" style display. */
function formatBytes(n: number): string {
  if (n >= 1 << 30) return `${(n / (1 << 30)).toFixed(2)} GB`;
  if (n >= 1 << 20) return `${(n / (1 << 20)).toFixed(1)} MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${n} B`;
}

interface ExportDialogProps {
  opened: boolean;
  onClose: () => void;
  conn: DatabaseConnection;
  schema: string;
  table: string;
}

/**
 * Streaming-export dialog (Feature C3): format + row cap + destination file,
 * then live progress with cancel, then a summary. State machine:
 * idle → running → done | error (cancel lands on `done` with a note).
 */
export function ExportDialog({ opened, onClose, conn, schema, table }: ExportDialogProps) {
  const [phase, setPhase] = useState<Phase>('idle');
  const [format, setFormat] = useState<ExportFormat>('csv');
  const [maxRows, setMaxRows] = useState<number>(MAX_ROWS_DEFAULT);
  const [path, setPath] = useState<string | null>(null);
  const [rows, setRows] = useState(0);
  const [summary, setSummary] = useState<ExportSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const unlistenRef = useRef<(() => void) | null>(null);

  const running = phase === 'running';
  const ext = format === 'csv' ? 'csv' : 'jsonl';

  // Reset transient state whenever the dialog opens for a (new) table.
  useEffect(() => {
    if (opened) {
      setPhase('idle');
      setRows(0);
      setSummary(null);
      setError(null);
      setPath(null);
      setFormat('csv');
      setMaxRows(MAX_ROWS_DEFAULT);
    }
  }, [opened, schema, table]);

  useEffect(
    () => () => {
      unlistenRef.current?.();
    },
    [],
  );

  const pickPath = async () => {
    const picked = await save({
      defaultPath: `${schema}.${table}.${ext}`,
      filters: [
        { name: ext.toUpperCase(), extensions: [ext] },
        { name: '所有文件', extensions: ['*'] },
      ],
    });
    if (picked) setPath(picked);
  };

  const startExport = async () => {
    if (!path) return;
    setPhase('running');
    setRows(0);
    setSummary(null);
    setError(null);
    try {
      unlistenRef.current = await api.onExportProgress((n) => setRows(n));
    } catch {
      // Progress events are best-effort; the export still runs.
      unlistenRef.current = null;
    }
    try {
      const s = await api.exportStream(
        conn,
        { source: 'table', schema, table },
        format,
        path,
        maxRows,
      );
      setSummary(s);
      setPhase('done');
    } catch (e) {
      setError(errMessage(e));
      setPhase('error');
    } finally {
      unlistenRef.current?.();
      unlistenRef.current = null;
    }
  };

  const cancelExport = async () => {
    try {
      await api.exportCancel();
    } catch (e) {
      setError(errMessage(e));
    }
  };

  return (
    <Modal
      opened={opened}
      onClose={running ? () => {} : onClose}
      title={
        <Group gap={6}>
          <IconFileExport size={15} style={{ color: 'var(--text-tertiary)' }} />
          <Text size="sm" fw={600}>
            导出 {schema}.{table}
          </Text>
        </Group>
      }
      overlayProps={{ opacity: 0.55, color: '#000' }}
      size={460}
      classNames={{ root: 'app-modal' }}
    >
      <Stack gap="sm">
        {phase === 'idle' && (
          <>
            <SegmentedControl
              size="xs"
              value={format}
              onChange={(v) => {
                setFormat(v as ExportFormat);
                setPath(null); // extension changes → pick the file again
              }}
              data={[
                { value: 'csv', label: 'CSV（Excel 可开）' },
                { value: 'jsonlines', label: 'JSON Lines' },
              ]}
            />
            <NumberInput
              size="xs"
              label="行数上限"
              description="达到上限即停止导出，防止超大表失控"
              value={maxRows}
              onChange={(v) => setMaxRows(typeof v === 'number' && v >= 1 ? Math.floor(v) : MAX_ROWS_DEFAULT)}
              min={1}
              step={1000000}
              thousandSeparator
            />
            <TextInput
              size="xs"
              label="保存位置"
              placeholder="选择导出文件路径"
              value={path ?? ''}
              readOnly
              rightSection={
                <Button size="compact-xs" variant="light" onClick={pickPath}>
                  浏览
                </Button>
              }
            />
            <Group justify="flex-end" mt="xs">
              <Button size="xs" variant="default" onClick={onClose}>
                取消
              </Button>
              <Button size="xs" disabled={!path} onClick={startExport}>
                开始导出
              </Button>
            </Group>
          </>
        )}

        {phase === 'running' && (
          <>
            <Progress value={0} animated />
            <Text size="xs" c="dimmed">
              已导出 {rows.toLocaleString('en-US')} 行，写入中…
            </Text>
            <Group justify="flex-end" mt="xs">
              <Button size="xs" color="red" variant="light" onClick={cancelExport}>
                取消导出
              </Button>
            </Group>
          </>
        )}

        {phase === 'done' && summary && (
          <>
            <Alert
              icon={<IconCheck size={15} />}
              color={summary.cancelled ? 'yellow' : 'teal'}
              variant="light"
            >
              {summary.cancelled ? '导出已取消（部分文件保留）' : '导出完成'}
              {summary.truncated && ' · 已达行数上限，数据未导完'}
            </Alert>
            <Stack gap={4}>
              <Text size="xs" c="dimmed">
                行数：{summary.rows.toLocaleString('en-US')}
              </Text>
              <Text size="xs" c="dimmed">
                耗时：{(summary.elapsed_ms / 1000).toFixed(1)} 秒 · 大小：{formatBytes(summary.bytes)}
              </Text>
              <Text size="xs" c="dimmed" style={{ wordBreak: 'break-all' }}>
                文件：{summary.bytes > 0 || summary.rows > 0 ? path : '（未写入内容）'}
              </Text>
            </Stack>
            <Group justify="flex-end" mt="xs">
              <Button size="xs" onClick={onClose}>
                关闭
              </Button>
            </Group>
          </>
        )}

        {phase === 'error' && (
          <>
            <Alert icon={<IconAlertCircle size={15} />} color="red" variant="light">
              导出失败
            </Alert>
            <Text size="xs" c="dimmed" style={{ wordBreak: 'break-all' }}>
              {error}
            </Text>
            <Group justify="flex-end" mt="xs">
              <Button size="xs" variant="default" onClick={() => setPhase('idle')}>
                返回
              </Button>
              <Button size="xs" onClick={onClose}>
                关闭
              </Button>
            </Group>
          </>
        )}
      </Stack>
    </Modal>
  );
}
