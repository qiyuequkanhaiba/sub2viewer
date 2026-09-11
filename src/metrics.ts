import type { SiteSnapshot, Thresholds } from "./types";
import { formatUsd } from "./utils";

export type Tone = "ok" | "warn" | "bad" | "info" | "muted";

export const DEFAULT_THRESHOLDS: Thresholds = {
  warnBalanceUsd: 5,
  criticalBalanceUsd: 1,
  warnAvailableCount: 5,
  criticalAvailableCount: 2,
};

export function thresholdsFromSettings(
  settings?: {
    warnBalanceUsd?: number;
    criticalBalanceUsd?: number;
    warnAvailableCount?: number;
    criticalAvailableCount?: number;
    lowBalanceThreshold?: number;
  } | null,
): Thresholds {
  const critical = settings?.criticalBalanceUsd ?? settings?.lowBalanceThreshold ?? 1;
  return {
    warnBalanceUsd: settings?.warnBalanceUsd ?? Math.max(5, critical),
    criticalBalanceUsd: critical,
    warnAvailableCount: settings?.warnAvailableCount ?? 5,
    criticalAvailableCount: settings?.criticalAvailableCount ?? 2,
  };
}

export interface RingSpec {
  id: string;
  value: number;
  color: string;
  track: string;
}

export interface TickSpec {
  id: string;
  label: string;
  value: number;
  tone: Tone;
}

export interface SiteMetricView {
  title: string;
  subtitle: string;
  hero: string;
  unit: string;
  tone: Tone;
  rings: RingSpec[];
  ticks: TickSpec[];
  error?: string;
  brand: string;
  detail: string;
  remainPct: number;
  footerLeft: string;
  footerRight: string;
}

const C = {
  ok: "#30D158",
  cyan: "#64D2FF",
  blue: "#0A84FF",
  warn: "#FF9F0A",
  bad: "#FF3B30",
  track: "rgba(255,255,255,0.10)",
};

function toneFromBalance(remaining: number, t: Thresholds): Tone {
  if (remaining < 0) return "ok"; // unlimited
  if (remaining <= t.criticalBalanceUsd) return "bad";
  if (remaining <= t.warnBalanceUsd) return "warn";
  return "ok";
}

function toneFromAvailable(available: number, t: Thresholds): Tone {
  if (available <= t.criticalAvailableCount) return "bad";
  if (available <= t.warnAvailableCount) return "warn";
  return "ok";
}

function worstTone(a: Tone, b: Tone): Tone {
  const rank = { muted: 0, info: 1, ok: 2, warn: 3, bad: 4 };
  return rank[a] >= rank[b] ? a : b;
}

function clamp01(n: number) {
  return Math.min(1, Math.max(0, n));
}

export function statusLabel(tone: Tone): string {
  if (tone === "bad") return "告警";
  if (tone === "warn") return "预警";
  if (tone === "muted") return "未配置";
  return "正常";
}

export interface SiteRow {
  name: string;
  value: string;
  pct: number;
  tone: Tone;
  todayCost: number;
  monthCost: number;
  kind: "user" | "admin";
}

export interface MergedMetricView {
  tone: Tone;
  hero: string;
  heroBalance?: string;
  heroAccounts?: string;
  brand: string;
  detail: string;
  remainPct: number;
  todayCost: number;
  monthCost: number;
  usageScope: "user" | "admin";
  hasAdmin: boolean;
  okAccounts: number;
  errorAccounts: number;
  rateLimitedAccounts: number;
  unschedulableAccounts: number;
  totalAccounts: number;
  footerLeft: string;
  footerRight: string;
  error?: string;
  rows: SiteRow[];
}

function siteSpend(snap: SiteSnapshot): { today: number; month: number } {
  if (snap.user) {
    return {
      today: snap.user.todayCost ?? snap.user.today?.actualCost ?? 0,
      month: snap.user.monthCost ?? 0,
    };
  }
  if (snap.admin) {
    return {
      today: snap.admin.todayCost ?? 0,
      month: snap.admin.monthCost ?? 0,
    };
  }
  return { today: 0, month: 0 };
}

export function mergeMetrics(
  snapshots: SiteSnapshot[],
  thresholds: Thresholds,
): MergedMetricView {
  if (!snapshots.length) {
    return {
      tone: "muted",
      hero: "未配置",
      brand: "Sub2",
      detail: "右键菜单栏图标添加站点",
      remainPct: 0,
      todayCost: 0,
      monthCost: 0,
      usageScope: "user",
      hasAdmin: false,
      okAccounts: 0,
      errorAccounts: 0,
      rateLimitedAccounts: 0,
      unschedulableAccounts: 0,
      totalAccounts: 0,
      footerLeft: "",
      footerRight: "",
      rows: [],
    };
  }

  const parts = snapshots.map((s) => siteMetric(s, thresholds));
  let worst: Tone = "ok";
  for (const p of parts) worst = worstTone(worst, p.tone);

  let balance = 0;
  let hasBalance = false;
  let unlimited = false;
  let userToday = 0;
  let userMonth = 0;
  let adminToday = 0;
  let adminMonth = 0;
  let hasUser = false;
  let avail = 0;
  let errAcc = 0;
  let rateAcc = 0;
  let unschedAcc = 0;
  let totalAcc = 0;
  let hasAdmin = false;
  const errors: string[] = [];

  for (const snap of snapshots) {
    if (snap.user) {
      hasUser = true;
      const rem = snap.user.remaining ?? snap.user.balance;
      if (rem != null && rem < 0) unlimited = true;
      else if (rem != null) {
        balance += rem;
        hasBalance = true;
      }
      userToday += snap.user.todayCost ?? snap.user.today?.actualCost ?? 0;
      userMonth += snap.user.monthCost ?? 0;
      // Only a hard failure (no usable remaining) counts as an error for color.
      if (snap.user.error && snap.user.remaining == null && snap.user.balance == null) {
        errors.push(snap.user.error);
      }
    }
    if (snap.admin) {
      hasAdmin = true;
      avail += snap.admin.availableAccounts;
      errAcc += snap.admin.errorAccounts;
      rateAcc += snap.admin.rateLimitedAccounts;
      unschedAcc += snap.admin.unschedulableAccounts ?? 0;
      totalAcc += snap.admin.totalAccounts;
      adminToday += snap.admin.todayCost ?? 0;
      adminMonth += snap.admin.monthCost ?? 0;
      if (snap.admin.error && snap.admin.totalAccounts <= 0) {
        errors.push(snap.admin.error);
      }
    }
  }

  // Never mix personal Key spend with another site's platform revenue.
  const usageScope: "user" | "admin" = hasUser ? "user" : "admin";
  const today = hasUser ? userToday : adminToday;
  const month = hasUser ? userMonth : adminMonth;

  if (hasBalance && !unlimited) {
    worst = worstTone(worst, toneFromBalance(balance, thresholds));
  }
  if (hasAdmin) {
    worst = worstTone(worst, toneFromAvailable(avail, thresholds));
  }

  const heroBalance =
    unlimited && !hasBalance ? "∞" : hasBalance ? formatUsd(balance) : undefined;
  const heroAccounts = hasAdmin
    ? totalAcc
      ? `${avail}/${errAcc}/${totalAcc}`
      : "0"
    : undefined;
  const hero = [heroBalance, heroAccounts].filter(Boolean).join("  ") || "—";

  const remainPct = Math.round(
    parts.reduce((a, p) => a + p.remainPct, 0) / parts.length,
  );

  const detailBits: string[] = [];
  if (hasBalance) detailBits.push(`余额 ${formatUsd(balance)}`);
  if (hasAdmin && totalAcc) {
    detailBits.push(`正常 ${avail} · 错误 ${errAcc} / ${totalAcc}`);
  }
  if (today > 0) detailBits.push(`今日 ${formatUsd(today)}`);
  const detail =
    errors[0] ||
    (detailBits.length ? detailBits.join("  ·  ") : `${parts.length} 个站点`);

  return {
    tone: worst,
    hero,
    heroBalance,
    heroAccounts,
    brand: "Sub2",
    detail,
    remainPct,
    todayCost: today,
    monthCost: month,
    usageScope,
    hasAdmin,
    okAccounts: avail,
    errorAccounts: errAcc,
    rateLimitedAccounts: rateAcc,
    unschedulableAccounts: unschedAcc,
    totalAccounts: totalAcc,
    footerLeft: `${parts.length} 个站点`,
    footerRight: errors.length
      ? `${errors.length} 项异常`
      : statusLabel(worst),
    error: errors[0],
    rows: snapshots.map((snap, i) => {
      const p = parts[i];
      const spend = siteSpend(snap);
      return {
        name: p.brand || p.title,
        value: p.hero,
        pct: p.remainPct,
        tone: p.tone,
        todayCost: spend.today,
        monthCost: spend.month,
        kind: snap.admin ? "admin" : "user",
      };
    }),
  };
}

export function siteMetric(
  snapshot: SiteSnapshot,
  thresholds: Thresholds,
): SiteMetricView {
  if (snapshot.site.role === "admin") {
    return adminMetric(snapshot, thresholds);
  }
  return userMetric(snapshot, thresholds);
}

function userMetric(
  snapshot: SiteSnapshot,
  thresholds: Thresholds,
): SiteMetricView {
  const u = snapshot.user;
  const title = snapshot.site.name;
  const subtitle = snapshot.site.apiKeyLabel || "余额";

  if (!u || u.error) {
    return {
      title,
      subtitle,
      hero: "—",
      unit: "USD",
      tone: "bad",
      rings: [
        { id: "bal", value: 0, color: C.bad, track: C.track },
        { id: "day", value: 0, color: C.blue, track: C.track },
      ],
      ticks: [],
      error: u?.error || "无数据",
      brand: title,
      detail: u?.error || "无法读取用量",
      remainPct: 0,
      footerLeft: "需重新配置",
      footerRight: "",
    };
  }

  const remainingRaw = u.remaining ?? u.balance;
  const remaining = remainingRaw ?? 0;
  const totalUsed = u.total?.cost ?? u.total?.actualCost ?? 0;

  let remainRatio = 0;
  if (remaining < 0) remainRatio = 1;
  else if (totalUsed + remaining > 0) remainRatio = remaining / (remaining + totalUsed);
  else remainRatio = clamp01(remaining / Math.max(thresholds.warnBalanceUsd, 1));

  // Color follows only the user-configured balance threshold.
  // Missing remaining (and a valid key) is treated as OK, not $0.
  const tone: Tone =
    remainingRaw == null
      ? "ok"
      : toneFromBalance(remainingRaw, thresholds);
  const hero = remainingRaw == null ? "—" : remaining < 0 ? "∞" : formatUsd(remaining);

  let detail = "";
  if (u.subscription?.dailyLimitUsd && u.subscription.dailyLimitUsd > 0) {
    detail = `日额度 ${formatUsd(u.subscription.dailyUsageUsd)} / ${formatUsd(u.subscription.dailyLimitUsd)}`;
  } else if (totalUsed + Math.max(remaining, 0) > 0 && remaining >= 0) {
    detail = `剩余 ${formatUsd(remaining)} · 累计 ${formatUsd(totalUsed)}`;
  } else {
    detail = u.planName || subtitle;
  }

  let footerLeft = u.planName || "USD";
  if (u.subscription?.expiresAt) {
    const days = Math.ceil(
      (new Date(u.subscription.expiresAt).getTime() - Date.now()) / 86400000,
    );
    footerLeft = days > 0 ? `${days}天后到期` : "已到期";
  }
  const footerRight = statusLabel(tone);

  return {
    title,
    subtitle,
    hero,
    unit: "USD",
    tone,
    rings: [],
    ticks: [],
    error: undefined,
    brand: title,
    detail,
    remainPct: Math.round(remainRatio * 100),
    footerLeft,
    footerRight,
  };
}

function adminMetric(
  snapshot: SiteSnapshot,
  thresholds: Thresholds,
): SiteMetricView {
  const a = snapshot.admin;
  const title = snapshot.site.name;
  const subtitle = "健康";

  if (!a || (a.error && a.totalAccounts <= 0)) {
    return {
      title,
      subtitle,
      hero: "—",
      unit: "可用",
      tone: "bad",
      rings: [],
      ticks: [],
      error: a?.error || "无数据",
      brand: title,
      detail: a?.error || "无法读取健康度",
      remainPct: 0,
      footerLeft: "需重新登录",
      footerRight: "告警",
    };
  }

  const total = a.totalAccounts || 0;
  const available = a.availableAccounts;
  const errors = a.errorAccounts;
  const tone = toneFromAvailable(available, thresholds);

  return {
    title,
    subtitle,
    hero: total ? `${available}/${errors}/${total}` : "0",
    unit: "账号",
    tone,
    rings: [],
    ticks: [],
    error: undefined,
    brand: title,
    detail: total
      ? `正常 ${available} · 错误 ${errors} / ${total}`
      : "暂无账号",
    remainPct: total > 0 ? Math.round((available / total) * 100) : 0,
    footerLeft: a.errorAccounts ? `异常 ${a.errorAccounts}` : "运行正常",
    footerRight: statusLabel(tone),
  };
}
