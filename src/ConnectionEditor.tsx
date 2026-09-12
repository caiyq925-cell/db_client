import { useEffect, useState } from 'react';
import {
  Button,
  Group,
  Input,
  Modal,
  NumberInput,
  PasswordInput,
  Paper,
  SegmentedControl,
  Stack,
  Switch,
  Text,
  TextInput,
  Textarea,
} from '@mantine/core';
import { useAppStore } from './store';
import { api, errMessage } from './api';
import { DEFAULT_PORTS, newConnection, type DatabaseConnection, type DbType } from './types';

const DB_LABELS: Record<DbType, string> = {
  mysql: 'MySQL',
  mongo: 'MongoDB',
  redis: 'Redis',
};

/** 名称输入框的占位名随数据库类型联动（bug 1：切类型仍显示"本地 MySQL"）。 */
const DB_NAME_PLACEHOLDER: Record<DbType, string> = {
  mysql: '本地 MySQL',
  mongo: '本地 MongoDB',
  redis: '本地 Redis',
};

/** 小节标题，用于分组表单区域。 */
function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <Text size="xs" fw={700} mt={4} className="conn-section-title">
      {children}
    </Text>
  );
}

/**
 * Store-driven connection editor (new + edit).
 * Shared by the sidebar's "新建/编辑" buttons and the connection right-click menu.
 */
export function ConnectionEditor() {
  const showConnEditor = useAppStore((s) => s.showConnEditor);
  const editingConn = useAppStore((s) => s.editingConn);
  const closeConnEditor = useAppStore((s) => s.closeConnEditor);
  const upsertConnection = useAppStore((s) => s.upsertConnection);

  const [conn, setConn] = useState<DatabaseConnection | null>(null);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const isEdit = editingConn != null;

  useEffect(() => {
    if (showConnEditor) {
      setConn(editingConn ?? newConnection('mysql'));
      setTestResult(null);
    }
  }, [showConnEditor, editingConn]);

  const patch = (p: Partial<DatabaseConnection>) =>
    setConn((c) => (c ? { ...c, ...p } : c));

  const changeKind = (kind: DbType) => {
    if (!conn) return;
    setConn({
      ...conn,
      kind,
      port: DEFAULT_PORTS[kind],
      auth:
        kind === 'redis'
          ? { kind: 'password', username: '', password: '' }
          : { kind: 'password', username: kind === 'mysql' ? 'root' : '', password: '' },
    });
    setTestResult(null);
  };

  const auth = conn?.auth;

  const onTest = async () => {
    if (!conn) return;
    setTesting(true);
    setTestResult(null);
    try {
      const msg = await api.testConnection(conn);
      setTestResult(`✓ ${msg}`);
    } catch (e) {
      // Tauri 拒绝时抛出的是字符串（Err(String)），必须用 errMessage 归一化。
      setTestResult(`✗ ${errMessage(e)}`);
    } finally {
      setTesting(false);
    }
  };

  const onSave = async () => {
    if (!conn) return;
    setSaving(true);
    try {
      await upsertConnection(conn);
      closeConnEditor();
    } catch (e) {
      setTestResult(`✗ 保存失败: ${errMessage(e)}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      opened={showConnEditor}
      onClose={closeConnEditor}
      title={isEdit && editingConn?.name ? `编辑连接 - ${editingConn.name}` : '新建连接'}
      size={560}
      classNames={{ root: 'app-modal' }}
    >
      {conn && (
        <Stack gap="sm">
          <SectionTitle>基本信息</SectionTitle>
          <Group grow>
            <TextInput
              label="名称"
              placeholder={DB_NAME_PLACEHOLDER[conn.kind]}
              value={conn.name}
              onChange={(e) => patch({ name: e.currentTarget.value })}
            />
            <TextInput
              label="分组"
              value={conn.group}
              onChange={(e) => patch({ group: e.currentTarget.value })}
              placeholder="Default"
            />
          </Group>

          {/* 编辑已有连接时不允许修改数据库类型（会导致端口/认证结构失效）。 */}
          <Input.Wrapper label="数据库类型" description={isEdit ? '已创建的连接不可更改类型' : undefined}>
            <SegmentedControl
              mt={6}
              fullWidth
              data={(Object.keys(DB_LABELS) as DbType[]).map((k) => ({
                value: k,
                label: DB_LABELS[k],
              }))}
              value={conn.kind}
              disabled={isEdit}
              onChange={(v) => changeKind(v as DbType)}
            />
          </Input.Wrapper>

          <Group grow>
            <TextInput
              label="主机"
              value={conn.host}
              onChange={(e) => patch({ host: e.currentTarget.value })}
            />
            <NumberInput
              label="端口"
              value={conn.port}
              onChange={(v) => patch({ port: Number(v) || DEFAULT_PORTS[conn.kind] })}
              min={1}
              max={65535}
            />
          </Group>

          <TextInput
            label={conn.kind === 'mysql' ? '数据库' : conn.kind === 'mongo' ? '默认数据库' : '命名空间（可选）'}
            value={conn.database}
            onChange={(e) => patch({ database: e.currentTarget.value })}
            placeholder={conn.kind === 'mysql' ? 'mydb' : conn.kind === 'mongo' ? 'test' : ''}
          />

          <SectionTitle>认证</SectionTitle>
          {auth?.kind === 'password' && (
            <Group grow>
              <TextInput
                label="用户名"
                value={auth.username}
                onChange={(e) => patch({ auth: { ...auth, username: e.currentTarget.value } })}
              />
              <PasswordInput
                label="密码"
                value={auth.password}
                onChange={(e) => patch({ auth: { ...auth, password: e.currentTarget.value } })}
              />
            </Group>
          )}

          {auth?.kind === 'connection_string' && (
            <Textarea
              label="连接字符串"
              value={auth.url}
              onChange={(e) => patch({ auth: { ...auth, url: e.currentTarget.value } })}
              placeholder="mongodb://user:pass@host:port/db"
            />
          )}

          <Group justify="space-between" align="center">
            <Switch label="SSL" checked={conn.ssl} onChange={(e) => patch({ ssl: e.currentTarget.checked })} />
            <NumberInput
              label="超时 (秒)"
              value={conn.connection_timeout_secs}
              onChange={(v) => patch({ connection_timeout_secs: Number(v) || 30 })}
              min={1}
              max={300}
              w={140}
            />
          </Group>

          <Paper
            withBorder
            p="sm"
            radius="md"
            className={`ssh-section${conn.ssh_tunnel?.enabled ? ' open' : ''}`}
          >
            <Switch
              label="SSH 隧道"
              checked={conn.ssh_tunnel?.enabled ?? false}
              onChange={(e) =>
                patch({
                  ssh_tunnel: {
                    enabled: e.currentTarget.checked,
                    host: conn.ssh_tunnel?.host ?? '127.0.0.1',
                    port: conn.ssh_tunnel?.port ?? 22,
                    username: conn.ssh_tunnel?.username ?? 'root',
                    key_path: conn.ssh_tunnel?.key_path ?? '',
                    password: conn.ssh_tunnel?.password ?? '',
                  },
                })
              }
            />
            {conn.ssh_tunnel?.enabled && (
              <Stack gap="xs" mt="xs">
                <Group grow>
                  <TextInput
                    label="SSH 主机"
                    value={conn.ssh_tunnel.host}
                    onChange={(e) =>
                      patch({ ssh_tunnel: { ...conn.ssh_tunnel!, host: e.currentTarget.value } })
                    }
                  />
                  <NumberInput
                    label="SSH 端口"
                    value={conn.ssh_tunnel.port}
                    onChange={(v) =>
                      patch({ ssh_tunnel: { ...conn.ssh_tunnel!, port: Number(v) || 22 } })
                    }
                    min={1}
                    max={65535}
                  />
                </Group>
                <TextInput
                  label="SSH 用户名"
                  value={conn.ssh_tunnel.username}
                  onChange={(e) =>
                    patch({ ssh_tunnel: { ...conn.ssh_tunnel!, username: e.currentTarget.value } })
                  }
                />
                <Group grow>
                  <TextInput
                    label="密钥路径（可选）"
                    value={conn.ssh_tunnel.key_path}
                    onChange={(e) =>
                      patch({ ssh_tunnel: { ...conn.ssh_tunnel!, key_path: e.currentTarget.value } })
                    }
                    placeholder="~/.ssh/id_rsa"
                  />
                  <PasswordInput
                    label="密码（可选）"
                    value={conn.ssh_tunnel.password}
                    onChange={(e) =>
                      patch({ ssh_tunnel: { ...conn.ssh_tunnel!, password: e.currentTarget.value } })
                    }
                  />
                </Group>
              </Stack>
            )}
          </Paper>

          {testResult && (
            <div className={`conn-test-result ${testResult.startsWith('✓') ? 'ok' : 'err'}`}>
              {testResult}
            </div>
          )}

          <Group justify="flex-end" mt="sm">
            <Button variant="subtle" className="btn-cancel" onClick={closeConnEditor}>
              取消
            </Button>
            <Button variant="default" className="btn-ghost" onClick={onTest} loading={testing} disabled={!conn.name}>
              测试连接
            </Button>
            <Button className="btn-save" onClick={onSave} loading={saving} disabled={!conn.name}>
              保存
            </Button>
          </Group>
        </Stack>
      )}
    </Modal>
  );
}
