import { useEffect, useRef, useState } from 'react';
import { Text } from '@mantine/core';
import {
  IconCopy,
  IconPlus,
  IconPencil,
  IconPower,
  IconRefresh,
  IconFolder,
  IconTrash,
} from '@tabler/icons-react';
import { useAppStore } from './store';
import { buildConnectionUrl } from './connUrl';

/**
 * Task 1: right-click context menu for a *connection* (distinct from the
 * schema-object context menu). Rendered globally from App via store.connMenu.
 *
 * Actions: 关闭连接 / 编辑连接 / 新建查询 / 刷新 / 复制链接 / 移动至分组.
 */
export function ConnMenu() {
  const menu = useAppStore((s) => s.connMenu);
  const closeConnMenu = useAppStore((s) => s.closeConnMenu);
  const openConnEditor = useAppStore((s) => s.openConnEditor);
  const openMoveGroup = useAppStore((s) => s.openMoveGroup);
  const openQueryTab = useAppStore((s) => s.openQueryTab);
  const testConnection = useAppStore((s) => s.testConnection);
  const refreshDatabases = useAppStore((s) => s.refreshDatabases);
  const closeConnection = useAppStore((s) => s.closeConnection);
  const removeConnection = useAppStore((s) => s.removeConnection);
  const connections = useAppStore((s) => s.connections);
  const [copied, setCopied] = useState(false);
  // 删除连接需二次确认：第一次点击只切换菜单文案，再次点击才执行。
  const [confirmDelete, setConfirmDelete] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  // 菜单关闭/切换目标连接时重置确认状态。
  useEffect(() => {
    setConfirmDelete(false);
  }, [menu?.connId]);

  // Close on outside click / Escape — but NOT when the click lands inside
  // the menu itself, otherwise the item's onClick never fires (the menu would
  // be unmounted by the document-level mousedown before mouseup/click).
  useEffect(() => {
    if (!menu) return;
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && closeConnMenu();
    const onDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        closeConnMenu();
      }
    };
    const t = setTimeout(() => document.addEventListener('mousedown', onDown, true), 0);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
      document.removeEventListener('mousedown', onDown, true);
      clearTimeout(t);
    };
  }, [menu, closeConnMenu]);

  if (!menu) return null;
  const conn = connections.find((c) => c.id === menu.connId);
  if (!conn) return null;

  const style: React.CSSProperties = {
    left: Math.min(menu.x, window.innerWidth - 240),
    top: Math.min(menu.y, window.innerHeight - 300),
  };

  const doCopy = async () => {
    try {
      await navigator.clipboard.writeText(buildConnectionUrl(conn));
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      /* clipboard unavailable */
    }
  };

  const items: { label: string; icon: React.ReactNode; danger?: boolean; run: () => void }[] = [
    {
      label: '关闭连接',
      icon: <IconPower size={14} />,
      run: () => {
        closeConnection(conn.id);
        closeConnMenu();
      },
    },
    {
      label: '编辑连接',
      icon: <IconPencil size={14} />,
      run: () => {
        openConnEditor(conn);
      },
    },
    {
      label: '新建查询',
      icon: <IconPlus size={14} />,
      run: () => {
        openQueryTab(conn.id);
        closeConnMenu();
      },
    },
    {
      label: '刷新',
      icon: <IconRefresh size={14} />,
      run: () => {
        // 重新测试连接（更新状态点）并强制重拉数据库/对象树
        void testConnection(conn);
        void refreshDatabases(conn);
        closeConnMenu();
      },
    },
    {
      label: copied ? '已复制链接 ✓' : '复制链接',
      icon: <IconCopy size={14} />,
      run: () => void doCopy(),
    },
    {
      label: `移动至分组（${conn.group || 'Default'}）`,
      icon: <IconFolder size={14} />,
      run: () => {
        openMoveGroup(conn);
      },
    },
    {
      label: confirmDelete ? '再次点击确认删除' : '删除连接',
      icon: <IconTrash size={14} />,
      danger: true,
      run: () => {
        if (!confirmDelete) {
          setConfirmDelete(true);
          return;
        }
        void removeConnection(conn.id)
          .then(() => closeConnMenu())
          .catch(() => closeConnMenu());
      },
    },
  ];

  return (
    <div ref={menuRef} className="ctx-menu" style={style} onContextMenu={(e) => e.preventDefault()}>
      <div className="ctx-title">
        连接 · {conn.name || `${conn.host}:${conn.port}`}
      </div>
      {items.map((it) => (
        <button key={it.label} className={it.danger ? 'danger' : ''} onClick={it.run}>
          <span>{it.icon}</span>
          {it.label}
        </button>
      ))}
      {copied && <Text size="xs" c="teal" p="xs">已复制到剪贴板</Text>}
    </div>
  );
}
