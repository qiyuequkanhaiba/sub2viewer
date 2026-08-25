export function formatUsd(value?: number | null, digits = 2): string {
  if (value === undefined || value === null || Number.isNaN(value)) return "—";
  if (value < 0) return "∞";
  // Compact for gauges
  if (value >= 1000) return `$${(value / 1000).toFixed(1)}k`;
  if (value >= 100) return `$${value.toFixed(0)}`;
  return `$${value.toFixed(digits)}`;
}

export function formatUsdFixed(value?: number | null, digits = 2): string {
  if (value === undefined || value === null || Number.isNaN(value)) return "—";
  if (value < 0) return "∞";
  return `$${value.toFixed(digits)}`;
}

export function formatTime(iso?: string | null): string {
  if (!iso) return "—";
  try {
    const d = new Date(iso);
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  } catch {
    return iso;
  }
}

export function pct(used: number, limit: number): number {
  if (!limit || limit <= 0) return 0;
  return Math.min(100, Math.max(0, (used / limit) * 100));
}

export function healthColor(available: number, total: number): string {
  if (total <= 0) return "var(--muted)";
  const r = available / total;
  if (r >= 0.8) return "var(--ok)";
  if (r >= 0.5) return "var(--warn)";
  return "var(--bad)";
}
