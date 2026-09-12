import { useEffect, useRef } from 'react';
import { useAppStore } from './store';
import { api } from './api';
import type { ObjectKind } from './types';

/**
 * 菜单项按钮：动作在 onMouseDown 阶段执行（双保险）；
 * 容器级 outside-mousedown 保护见 MenuShell / ContextMenu 注释。
 */
function MenuItem({
  label,
  icon,
  danger,
  run,
}: {
  label: string;
  icon: string;
  danger?: boolean;
  run: () => void;
}) {
  return (
    <button
      className={danger ? 'danger' : ''}
      onMouseDown={(e) => {
        e.stopPropagation();
        run();
      }}
      onClick={(e) => e.stopPropagation()}
    >
      <span>{icon}</span>
      {label}
    </button>
  );
}

/**
 * Global right-click context menu for schema objects / database nodes /
 * object-group nodes. Position comes from store.contextMenu; renders fixed
 * at that point.
 *
 * 关闭保护与 ConnMenu 一致：outside-mousedown 才关闭，菜单内部的 mousedown
 * 不关闭（否则菜单先被卸载，菜单项的 click 永远不会到来）。
 */
/** 菜单容器：绑 ref 供 outside-click 判断；三个分支共用。 */
function MenuShell({
  menuRef,
  style,
  title,
  children,
}: {
  menuRef: React.RefObject<HTMLDivElement | null>;
  style: React.CSSProperties;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div ref={menuRef} className="ctx-menu" style={style} onContextMenu={(e) => e.preventDefault()}>
      <div className="ctx-title">{title}</div>
      {children}
    </div>
  );
}

export function ContextMenu() {
  const menu = useAppStore((s) => s.contextMenu);
  const close = useAppStore((s) => s.closeContextMenu);
  const openDdl = useAppStore((s) => s.openDdl);
  const openQueryTab = useAppStore((s) => s.openQueryTab);
  const openDataTableTab = useAppStore((s) => s.openDataTableTab);
  const openConfirm = useAppStore((s) => s.openConfirm);
  const showToast = useAppStore((s) => s.showToast);
  const connections = useAppStore((s) => s.connections);
  const menuRef = useRef<HTMLDivElement>(null);

  // Close on outside click / Escape
  useEffect(() => {
    if (!menu) return;
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && close();
    const onDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        close();
      }
    };
    window.addEventListener('keydown', onKey);
    // small delay so the click that opened the menu doesn't immediately close it
    const t = setTimeout(() => document.addEventListener('mousedown', onDown, true), 0);
    return () => {
      window.removeEventListener('keydown', onKey);
      document.removeEventListener('mousedown', onDown, true);
      clearTimeout(t);
    };
  }, [menu, close]);

  if (!menu) return null;
  const conn = connections.find((c) => c.id === menu.connectionId);
  if (!conn) return null;

  // Keep the menu on-screen
  const style: React.CSSProperties = {
    left: Math.min(menu.x, window.innerWidth - 220),
    top: Math.min(menu.y, window.innerHeight - 260),
  };

  // ===== 数据库节点菜单：新建查询（自动选中该库） =====
  if (menu.kind === 'database') {
    const db = menu.object;
    return (
      <MenuShell menuRef={menuRef} style={style} title={`数据库 · ${db}`}>
        <MenuItem
          label="新建查询"
          icon="🚀"
          run={() => {
            close();
            openQueryTab(menu.connectionId, { database: db });
          }}
        />
      </MenuShell>
    );
  }

  // ===== 对象分组节点菜单（存储过程组：新建存储过程模板） =====
  if (menu.kind === 'group') {
    const db = menu.schema;
    const items: { label: string; icon: string; run: () => void }[] = [];
    if (menu.object === 'procedure') {
      items.push({
        label: '新建存储过程',
        icon: '✨',
        run: () => {
          close();
          // 基础骨架：无 DELIMITER（协议执行不支持），执行走 text 协议整句发送
          const sql = [
            `CREATE PROCEDURE \`${db}\`.\`new_procedure\`(`,
            '    IN p_id BIGINT',
            ')',
            'BEGIN',
            '    -- 过程体：在此编写 SQL 逻辑',
            '    SELECT p_id;',
            'END',
          ].join('\n');
          openQueryTab(menu.connectionId, { database: db, sql });
        },
      });
    }
    if (items.length === 0) return null;
    return (
      <MenuShell menuRef={menuRef} style={style} title={`${menu.object} · ${db}`}>
        {items.map((it) => (
          <MenuItem key={it.label} label={it.label} icon={it.icon} run={it.run} />
        ))}
      </MenuShell>
    );
  }

  // ===== schema 对象节点菜单 =====
  const isTable = menu.kind === 'table';
  const isProcedure = menu.kind === 'procedure';
  // database/group 已在上面 early-return；闭包内 TS 不保留判别收窄，这里显式断言
  const objKind = menu.kind as ObjectKind;

  const items: { label: string; icon: string; danger?: boolean; run: () => void }[] = [];

  // 表类型：第一项为「打开表」，在右侧区域打开数据表格。
  if (isTable) {
    items.push({
      label: '打开表',
      icon: '📂',
      run: () => {
        close();
        void openDataTableTab(conn, menu.schema, menu.object);
      },
    });
  }

  // 新建查询：打开查询页并自动选中该库
  items.push({
    label: '新建查询',
    icon: '🚀',
    run: () => {
      close();
      openQueryTab(menu.connectionId, { database: menu.schema });
    },
  });

  items.push({
    label: '查看 DDL',
    icon: '📄',
    run: () => {
      close();
      openDdl({
        connectionId: menu.connectionId,
        schema: menu.schema,
        object: menu.object,
        kind: objKind,
        title: `DDL · ${menu.object}`,
      });
    },
  });

  if (!isProcedure) {
    items.push({
      label: '编辑表结构',
      icon: '🔧',
      run: () => {
        close();
        openDdl({
          connectionId: menu.connectionId,
          schema: menu.schema,
          object: menu.object,
          kind: objKind,
          title: `编辑 · ${menu.object}`,
          edit: true,
        });
      },
    });
  }

  if (isTable) {
    items.push({
      label: '数据预览 (LIMIT 100)',
      icon: '👀',
      run: () => {
        close();
        const q = `SELECT * FROM \`${menu.schema}\`.\`${menu.object}\` LIMIT 100;`;
        openQueryTab(menu.connectionId, { database: menu.schema, sql: q });
      },
    });
  }

  // 存储过程：修改（CREATE -> ALTER 打开查询页）/ 删除（二次确认，演示版不实际执行）
  if (isProcedure) {
    items.push({
      label: '修改存储过程',
      icon: '✏️',
      run: async () => {
        close();
        try {
          const info = await api.getObjectDdl(conn, menu.schema, menu.object, 'procedure');
          const alter = (info.ddl || '').replace(/^CREATE\b/i, 'ALTER');
          openQueryTab(menu.connectionId, {
            database: menu.schema,
            sql: alter || `-- 未能读取到存储过程 ${menu.object} 的定义`,
          });
        } catch (e) {
          showToast(`读取存储过程定义失败：${(e as Error).message}`, 'error');
        }
      },
    });
    items.push({
      label: '删除存储过程',
      icon: '🗑️',
      danger: true,
      run: () => {
        openConfirm({
          title: '删除存储过程',
          message: `确定删除存储过程 \`${menu.schema}\`.\`${menu.object}\` 吗？该操作不可恢复。`,
          onConfirm: () => {
            // 演示版：仅走确认流程，不实际执行 DROP
            showToast('已通过二次确认（演示版未实际执行删除）', 'info');
          },
        });
      },
    });
  }

  items.push({
    label: '复制 DDL 到剪贴板',
    icon: '📋',
    run: async () => {
      close();
      try {
        const ddl = await api.getObjectDdl(conn, menu.schema, menu.object, objKind);
        await navigator.clipboard.writeText(ddl.ddl || '(empty)');
      } catch (e) {
        console.error('copy ddl failed', e);
      }
    },
  });

  return (
    <MenuShell
      menuRef={menuRef}
      style={style}
      title={`${menu.kind} · ${menu.schema}.${menu.object}`}
    >
      {items.map((it) => (
        <MenuItem key={it.label} label={it.label} icon={it.icon} danger={it.danger} run={it.run} />
      ))}
    </MenuShell>
  );
}
