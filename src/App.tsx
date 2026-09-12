import { MantineProvider, Modal, NumberInput, Button, Group, Text, createTheme } from '@mantine/core';
import { Notifications } from '@mantine/notifications';
import { IconDatabase, IconHistory, IconPlus, IconSettings, IconTable } from '@tabler/icons-react';
import { useEffect, useState } from 'react';
import { Sidebar } from './Sidebar';
import { QueryTabView } from './QueryTab';
import { SchemaTabView } from './SchemaTab';
import { HistoryTabView } from './HistoryTab';
import { DataTableTabView } from './DataTableTab';
import { ContextMenu } from './ContextMenu';
import { ConnMenu } from './ConnMenu';
import { ConnectionEditor } from './ConnectionEditor';
import { MoveGroupModal } from './MoveGroupModal';
import { DdlModal } from './DdlModal';
import { useAppStore, type AppTab } from './store';

// Mantine 主色对齐设计令牌 --primary #4C8DFF（brand[5]），
// 让 Select 聚焦边、Checkbox、焦点环等与自绘样式同源。
const theme = createTheme({
  primaryColor: 'brand',
  primaryShade: 5,
  colors: {
    brand: [
      '#e7f0ff',
      '#cfe0ff',
      '#aec9ff',
      '#8fb4ff',
      '#6ba0ff',
      '#4c8dff',
      '#3a73e0',
      '#2f5fbe',
      '#25499a',
      '#1c3a7d',
    ],
  },
});

function TabIcon({ kind }: { kind: AppTab['kind'] }) {
  const size = 12;
  if (kind === 'query') return null;
  if (kind === 'schema') return <IconTable size={size} />;
  if (kind === 'datatable') return <IconTable size={size} />;
  return <IconHistory size={size} />;
}

// ----- 设置弹窗（task 11）：连接超时时间 -----
function SettingsModal({ opened, onClose }: { opened: boolean; onClose: () => void }) {
  const connectTimeoutSecs = useAppStore((s) => s.settings.connectTimeoutSecs);
  const setConnectTimeoutSecs = useAppStore((s) => s.setConnectTimeoutSecs);
  const [draft, setDraft] = useState(connectTimeoutSecs);

  // 每次打开弹窗时同步当前设置到草稿
  useEffect(() => {
    if (opened) setDraft(connectTimeoutSecs);
  }, [opened, connectTimeoutSecs]);

  const save = () => {
    setConnectTimeoutSecs(draft);
    onClose();
  };

  return (
    <Modal opened={opened} onClose={onClose} title="设置" size={420} classNames={{ root: 'app-modal' }}>
      <Text size="sm" fw={600} mb={6}>
        连接超时时间（秒）
      </Text>
      <Text size="xs" c="dimmed" mb="md">
        新建/打开连接时，若在设定时间内未连通则停止并提示超时。取值 1-300 秒。
      </Text>
      <NumberInput
        value={draft}
        onChange={(v) => setDraft(Number(v) || 15)}
        min={1}
        max={300}
        clampBehavior="strict"
      />
      <Group justify="flex-end" mt="lg">
        <Button variant="subtle" className="btn-cancel" onClick={onClose}>
          取消
        </Button>
        <Button className="btn-save" onClick={save}>保存</Button>
      </Group>
    </Modal>
  );
}

function TabBar() {
  const { tabs, activeTabId, closeTab, setActiveTab, activeConnectionId, openQueryTab, openHistoryTab, connections } =
    useAppStore();
  const [settingsOpen, setSettingsOpen] = useState(false);

  const newQuery = () => {
    const connId = activeConnectionId ?? connections[0]?.id;
    if (connId) openQueryTab(connId);
  };

  return (
    <>
      <div className="tab-bar">
        {tabs.map((tab) => (
        <div
          key={tab.id}
          className={`tab-item ${tab.id === activeTabId ? 'active' : ''}`}
          onClick={() => setActiveTab(tab.id)}
        >
          <TabIcon kind={tab.kind} />
          {tab.title}
          <span
            className="tab-close"
            role="button"
            tabIndex={0}
            aria-label={`关闭标签页 ${tab.title}`}
            onClick={(e) => {
              e.stopPropagation();
              closeTab(tab.id);
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                e.stopPropagation();
                closeTab(tab.id);
              }
            }}
          >
            ✕
          </span>
        </div>
      ))}
      <div className="tab-item" onClick={newQuery} title="新建查询">
        <IconPlus size={12} />
      </div>
      <div className="tab-item" onClick={openHistoryTab} title="查询历史">
        <IconHistory size={12} />
      </div>
      <span style={{ flex: 1 }} />
      <div className="tab-item" onClick={() => setSettingsOpen(true)} title="设置">
        <IconSettings size={13} />
      </div>
      </div>
      <SettingsModal opened={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </>
  );
}

function TabContent() {
  const { tabs, activeTabId } = useAppStore();
  const active = tabs.find((t) => t.id === activeTabId);

  if (!active) {
    return (
      <div className="empty-state">
        <IconDatabase size={96} stroke={1} className="empty-icon" aria-hidden="true" />
        <div style={{ textAlign: 'center' }}>
          左侧选择连接，双击新建查询
          <br />
          或点击标签栏 + 号新建标签页
        </div>
      </div>
    );
  }

  if (active.kind === 'query') return <QueryTabView tab={active} />;
  if (active.kind === 'schema') return <SchemaTabView connectionId={active.connectionId} />;
  if (active.kind === 'datatable') return <DataTableTabView tab={active} />;
  return <HistoryTabView />;
}

// 全局轻提示（保存结果等）：右下角浮出，自动消失
function ToastHost() {
  const toast = useAppStore((s) => s.toast);
  if (!toast) return null;
  return (
    <div className={`app-toast app-toast-${toast.kind}`} role="status" aria-live="polite">
      {toast.text}
    </div>
  );
}

// 危险操作二次确认弹窗（删除存储过程等），确认后才执行 onConfirm
function ConfirmHost() {
  const confirmState = useAppStore((s) => s.confirmState);
  const closeConfirm = useAppStore((s) => s.closeConfirm);
  if (!confirmState) return null;
  return (
    <Modal opened onClose={closeConfirm} title={confirmState.title} size={400} centered classNames={{ root: 'app-modal' }}>
      <Text size="sm" style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>
        {confirmState.message}
      </Text>
      <Group justify="flex-end" mt="lg">
        <Button variant="default" onClick={closeConfirm}>
          取消
        </Button>
        <Button
          color="red"
          onClick={() => {
            const run = confirmState.onConfirm;
            closeConfirm();
            run();
          }}
        >
          确认
        </Button>
      </Group>
    </Modal>
  );
}

export default function App() {
  return (
    <MantineProvider theme={theme} defaultColorScheme="dark">
      <Notifications position="top-right" autoClose={6000} />
      <div className="app-shell">
        <div className="app-body">
          <Sidebar />
          <div className="main-area">
            <TabBar />
            <div className="tab-content">
              <TabContent />
            </div>
          </div>
        </div>
      </div>
      <ContextMenu />
      <ConnMenu />
      <ConnectionEditor />
      <MoveGroupModal />
      <DdlModal />
      <ConfirmHost />
      <ToastHost />
    </MantineProvider>
  );
}
