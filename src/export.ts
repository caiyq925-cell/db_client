// CSV / JSON export helpers for query results

export function rowsToCsv(columns: string[], rows: Record<string, unknown>[]): string {
  const esc = (v: unknown): string => {
    if (v === null || v === undefined) return '';
    const s = typeof v === 'object' ? JSON.stringify(v) : String(v);
    if (/[",\n\r]/.test(s)) return `"${s.replace(/"/g, '""')}"`;
    return s;
  };
  const header = columns.map(esc).join(',');
  const body = rows.map((r) => columns.map((c) => esc(r[c])).join(',')).join('\n');
  return `${header}\n${body}`;
}

export function rowsToJson(rows: Record<string, unknown>[]): string {
  return JSON.stringify(rows, null, 2);
}

export function downloadFile(content: string, filename: string, mime: string) {
  const blob = new Blob([content], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

export function stamp(): string {
  return new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
}
