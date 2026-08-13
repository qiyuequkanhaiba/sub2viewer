import type { SiteSnapshot } from "./types";
import { formatUsd } from "./utils";

export type Tone = "ok" | "warn" | "bad" | "info" | "muted";

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
  purple: "#BF5AF2",
  warn: "#FFD60A",
  bad: "#FF453A",
  track: "rgba(255,255,255,0.10)",
};

function toneOf(value: number, invert = false): Tone {
  const v = invert ? 1 - value : value;
  if (v < 0.2) return "bad";
  if (v < 0.45) return "warn";
  return "ok";
}

function clamp01(n: number) {
  return Math.min(1, Math.max(0, n));
}

export interface SiteRow {
  name: string;
  value: string;
  pct: number;
  tone: Tone;
}

export interface MergedMetricView {
  tone: Tone;
  hero: string;
  brand: string;
  detail: string;
  remainPct: number;
  footerLeft: string;
  footerRight: string;
  error?: string;
  rows: SiteRow[];
}

export function mergeMetrics(
  snapshots: SiteSnapshot[],
  lowBalanceThreshold: number,
): MergedMetricView {
  if (!snapshots.length) {
    return {
      tone: "muted",
      hero: "未配置",
      brand: "Sub2",
      detail: "右键菜单栏图标添加站点",
      remainPct: 0,
      footerLeft: "",
      footerRight: "",
      rows: [],
    };
  }

  const parts = snapshots.map((s) => siteMetric(s, lowBalanceThreshold));
  const worst: Tone = parts.some((p) => p.tone === "bad")
    ? "bad"
    : parts.some((p) => p.tone === "warn")
      ? "warn"
      : "ok";

  let balance = 0;
  let hasBalance = false;
  let unlimited = false;
  let today = 0;
  let avail = 0;
  let totalAcc = 0;
  let hasAdmin = false;
  const errors: string[] = [];

  for (const snap of snapshots) {
    if (snap.user) {
      const rem = snap.user.remaining ?? snap.user.balance;
      if (rem != null && rem < 0) unlimited = true;
      else if (rem != null) {
        balance += rem;
        hasBalance = true;
      }
      today += snap.user.today?.cost ?? snap.user.today?.actualCost ?? 0;
      if (snap.user.error) errors.push(snap.user.error);
    }
    if (snap.admin) {
      hasAdmin = true;
      avail += snap.admin.availableAccounts;
      totalAcc += snap.admin.totalAccounts;
      if (snap.admin.error) errors.push(snap.admin.error);
    }
  }

  let hero = "—";
  if (unlimited && !hasBalance) hero = "∞";
  else if (hasBalance) hero = formatUsd(balance);
  else if (hasAdmin) hero = totalAcc ? `${avail}/${totalAcc}` : "0";

  const remainPct = Math.round(
    parts.reduce((a, p) => a + p.remainPct, 0) / parts.length,
  );

  const detailBits: string[] = [];
  if (hasBalance) detailBits.push(`余额 ${formatUsd(balance)}`);
  if (hasAdmin && totalAcc) detailBits.push(`可用 ${avail}/${totalAcc}`);
  if (today > 0) detailBits.push(`今日 ${formatUsd(today)}`);
  const detail =
    errors[0] ||
    (detailBits.length ? detailBits.join("  ·  ") : `${parts.length} 个站点`);

  return {
    tone: errors.length ? "bad" : worst,
    hero,
    brand: "Sub2",
    detail,
    remainPct,
    footerLeft: `${parts.length} 个站点`,
    footerRight: errors.length
      ? `${errors.length} 项异常`
      : worst === "ok"
        ? "运行正常"
        : "需关注",
    error: errors[0],
    rows: parts.map((p) => ({
      name: p.brand || p.title,
      value: p.hero,
      pct: p.remainPct,
      tone: p.tone,
    })),
  };
}

export function siteMetric(
  snapshot: SiteSnapshot,
  lowBalanceThreshold: number,
): SiteMetricView {
  if (snapshot.site.role === "admin") {
    return adminMetric(snapshot, lowBalanceThreshold);
  }
  return userMetric(snapshot, lowBalanceThreshold);
}

function userMetric(
  snapshot: SiteSnapshot,
  lowBalanceThreshold: number,
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

  const remaining = u.remaining ?? u.balance ?? 0;
  const totalUsed = u.total?.cost ?? u.total?.actualCost ?? 0;
  const today = u.today?.cost ?? u.today?.actualCost ?? 0;

  let remainRatio = 0;
  if (remaining < 0) remainRatio = 1;
  else if (totalUsed + remaining > 0) remainRatio = remaining / (remaining + totalUsed);
  else remainRatio = clamp01(remaining / Math.max(lowBalanceThreshold * 10, 5));

  let dayRemain = 1;
  const ticks: TickSpec[] = [];

  if (u.subscription?.dailyLimitUsd && u.subscription.dailyLimitUsd > 0) {
    const used = clamp01(u.subscription.dailyUsageUsd / u.subscription.dailyLimitUsd);
    dayRemain = 1 - used;
    ticks.push({ id: "d", label: "日", value: used, tone: toneOf(used, true) });
    if (u.subscription.weeklyLimitUsd && u.subscription.weeklyLimitUsd > 0) {
      const w = clamp01(u.subscription.weeklyUsageUsd / u.subscription.weeklyLimitUsd);
      ticks.push({ id: "w", label: "周", value: w, tone: toneOf(w, true) });
    }
    if (u.subscription.monthlyLimitUsd && u.subscription.monthlyLimitUsd > 0) {
      const m = clamp01(u.subscription.monthlyUsageUsd / u.subscription.monthlyLimitUsd);
      ticks.push({ id: "m", label: "月", value: m, tone: toneOf(m, true) });
    }
  } else if (u.rateLimits?.length) {
    const first = u.rateLimits[0];
    const used = first.limit > 0 ? clamp01(first.used / first.limit) : 0;
    dayRemain = 1 - used;
    for (const rl of u.rateLimits.slice(0, 3)) {
      const v = rl.limit > 0 ? clamp01(rl.used / rl.limit) : 0;
      ticks.push({
        id: rl.window,
        label: rl.window,
        value: v,
        tone: toneOf(v, true),
      });
    }
  } else {
    const denom = remaining + today;
    const used = denom > 0 ? clamp01(today / denom) : 0;
    dayRemain = 1 - used;
    ticks.push({ id: "today", label: "今日", value: used, tone: toneOf(used, true) });
  }

  const low = remaining >= 0 && remaining < lowBalanceThreshold;
  const tone: Tone = !u.isValid || low ? "bad" : toneOf(remainRatio);
  const hero = remaining < 0 ? "∞" : formatUsd(remaining);

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
  const footerRight =
    today > 0 ? `今日 ${formatUsd(today)}` : u.rpm != null ? `RPM ${u.rpm.toFixed(1)}` : "实时";

  return {
    title,
    subtitle,
    hero,
    unit: "USD",
    tone,
    rings: [
      {
        id: "bal",
        value: remainRatio,
        color: tone === "bad" ? C.bad : tone === "warn" ? C.warn : C.ok,
        track: C.track,
      },
      { id: "day", value: dayRemain, color: C.cyan, track: C.track },
    ],
    ticks: ticks.slice(0, 3),
    error: u.isValid ? undefined : u.status || "异常",
    brand: title,
    detail,
    remainPct: Math.round(remainRatio * 100),
    footerLeft,
    footerRight,
  };
}

function adminMetric(snapshot: SiteSnapshot, _low: number): SiteMetricView {
  const a = snapshot.admin;
  const title = snapshot.site.name;
  const subtitle = "健康";

  if (!a || a.error) {
    return {
      title,
      subtitle,
      hero: "—",
      unit: "可用",
      tone: "bad",
      rings: [
        { id: "ok", value: 0, color: C.ok, track: C.track },
        { id: "rl", value: 0, color: C.warn, track: C.track },
      ],
      ticks: [],
      error: a?.error || "无数据",
      brand: title,
      detail: a?.error || "无法读取健康度",
      remainPct: 0,
      footerLeft: "需重新登录",
      footerRight: "",
    };
  }

  const total = a.totalAccounts || 0;
  const avail = total > 0 ? a.availableAccounts / total : 0;
  const rl = total > 0 ? a.rateLimitedAccounts / total : 0;
  const err = total > 0 ? a.errorAccounts / total : 0;
  const tone: Tone = a.errorAccounts > 0 || avail < 0.5 ? "bad" : avail < 0.8 ? "warn" : "ok";

  const ticks: TickSpec[] = a.groups.slice(0, 4).map((g) => {
    const r = g.total > 0 ? g.available / g.total : 0;
    return {
      id: String(g.groupId),
      label: g.groupName,
      value: 1 - r,
      tone: g.error > 0 ? "bad" : r < 0.6 ? "warn" : "ok",
    };
  });

  if (!ticks.length) {
    ticks.push(
      { id: "err", label: "异常", value: err, tone: err > 0.1 ? "bad" : err > 0 ? "warn" : "ok" },
      { id: "rl", label: "限速", value: rl, tone: rl > 0.2 ? "warn" : "info" },
    );
  }

  return {
    title,
    subtitle,
    hero: total ? `${a.availableAccounts}/${total}` : "0",
    unit: "可用",
    tone,
    rings: [
      { id: "ok", value: avail, color: C.ok, track: C.track },
      { id: "rl", value: 1 - rl, color: C.warn, track: C.track },
      { id: "er", value: 1 - err, color: C.bad, track: C.track },
    ],
    ticks: ticks.slice(0, 3),
    error: undefined,
    brand: title,
    detail: total ? `可用账号 ${a.availableAccounts} / ${total}` : "暂无账号",
    remainPct: Math.round(avail * 100),
    footerLeft: a.errorAccounts ? `异常 ${a.errorAccounts}` : "运行正常",
    footerRight: a.rateLimitedAccounts ? `限速 ${a.rateLimitedAccounts}` : `${a.groups.length} 组`,
  };
}
