import { useMemo, useState } from 'react';
import { Button, Group, Modal, Text, TextInput } from '@mantine/core';
import { IconFolder } from '@tabler/icons-react';
import { useAppStore } from './store';

/**
 * Store-driven "move to group" picker. Shared by the sidebar action row and
 * the connection right-click menu. Supports moving to an existing group or
 * creating a new one.
 */
export function MoveGroupModal() {
  const moveTarget = useAppStore((s) => s.moveTarget);
  const closeMoveGroup = useAppStore((s) => s.closeMoveGroup);
  const moveConnectionToGroup = useAppStore((s) => s.moveConnectionToGroup);
  const connections = useAppStore((s) => s.connections);

  const [newGroup, setNewGroup] = useState('');
  const [busy, setBusy] = useState<string | null>(null);

  const groups = useMemo(() => {
    const set = new Set<string>();
    for (const c of connections) set.add(c.group || 'Default');
    set.delete('');
    return [...set];
  }, [connections]);

  if (!moveTarget) return null;

  const pick = async (g: string) => {
    setBusy(g);
    try {
      await moveConnectionToGroup(moveTarget.id, g);
      closeMoveGroup();
    } finally {
      setBusy(null);
    }
  };

  const createNew = async () => {
    const g = newGroup.trim();
    if (!g) return;
    setBusy('__new__');
    try {
      await moveConnectionToGroup(moveTarget.id, g);
      setNewGroup('');
      closeMoveGroup();
    } finally {
      setBusy(null);
    }
  };

  return (
    <Modal opened={!!moveTarget} onClose={closeMoveGroup} title={`移动「${moveTarget.name}」到分组`} size={420} classNames={{ root: 'app-modal' }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <Text size="xs" c="dimmed">选择目标分组：</Text>
        {groups.map((g) => (
          <div key={g} className="group-option" onClick={() => pick(g)}>
            <IconFolder size={14} />
            <span style={{ flex: 1 }}>{g}</span>
            {g === moveTarget.group ? (
              <Text size="xs" c="dimmed">当前</Text>
            ) : (
              busy === g ? (
                <Text size="xs" c="teal">移动中…</Text>
              ) : null
            )}
          </div>
        ))}
        <div style={{ borderTop: '1px solid var(--border-subtle)', paddingTop: 12 }}>
          <Group grow>
            <TextInput
              placeholder="新建分组名称…"
              value={newGroup}
              onChange={(e) => setNewGroup(e.currentTarget.value)}
              size="xs"
            />
            <Button size="xs" variant="light" onClick={createNew} loading={busy === '__new__'}>
              新建并移入
            </Button>
          </Group>
        </div>
      </div>
    </Modal>
  );
}
