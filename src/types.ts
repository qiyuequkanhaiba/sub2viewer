export type SiteRole = "user" | "admin";

export interface SitePublic {
  id: string;
  name: string;
  baseUrl: string;
  role: SiteRole;
  apiKeyLabel?: string | null;
  email?: string | null;
  enabled: boolean;
  hasApiKey: boolean;
  hasPassword: boolean;
  hasToken: boolean;
}

export interface UsageSummary {
  requests: number;
  totalTokens: number;
  cost: number;
  actualCost: number;
}

export interface RateLimitWindow {
  window: string;
  limit: number;
  used: number;
  remaining: number;
  resetAt?: string | null;
}

export interface SubscriptionUsage {
  dailyUsageUsd: number;
  weeklyUsageUsd: number;
  monthlyUsageUsd: number;
  dailyLimitUsd?: number | null;
  weeklyLimitUsd?: number | null;
  monthlyLimitUsd?: number | null;
  expiresAt?: string | null;
}

export interface UserSnapshot {
  siteId: string;
  mode: string;
  unit: string;
  balance?: number | null;
  remaining?: number | null;
  planName?: string | null;
  isValid: boolean;
  status?: string | null;
  today?: UsageSummary | null;
  total?: UsageSummary | null;
  rateLimits: RateLimitWindow[];
  subscription?: SubscriptionUsage | null;
  rpm?: number | null;
  todayCost?: number | null;
  monthCost?: number | null;
  updatedAt: string;
  error?: string | null;
}

export interface GroupHealth {
  groupId: number;
  groupName: string;
  platform?: string | null;
  total: number;
  available: number;
  rateLimited: number;
  error: number;
}

export interface ApiKeyBinding {
  id: number;
  name: string;
  groupId?: number | null;
  groupName?: string | null;
  status: string;
  switchError?: string | null;
}

export interface BindableGroup {
  id: number;
  name: string;
  platform?: string | null;
}

export interface AdminSnapshot {
  siteId: string;
  monitoringEnabled: boolean;
  statusBreakdown: Record<string, number>;
  groups: GroupHealth[];
  totalAccounts: number;
  availableAccounts: number;
  errorAccounts: number;
  rateLimitedAccounts: number;
  unschedulableAccounts?: number;
  apiKeys?: ApiKeyBinding[];
  bindableGroups?: BindableGroup[];
  keySwitchSupported?: boolean;
  keysTruncated?: boolean;
  keyListError?: string | null;
  todayCost?: number | null;
  monthCost?: number | null;
  updatedAt: string;
  error?: string | null;
}

export interface SiteSnapshot {
  site: SitePublic;
  user?: UserSnapshot | null;
  admin?: AdminSnapshot | null;
}

export interface Thresholds {
  warnBalanceUsd: number;
  criticalBalanceUsd: number;
  warnAvailableCount: number;
  criticalAvailableCount: number;
}

export interface AppSettingsPublic {
  refreshIntervalSecs: number;
  lowBalanceThreshold: number;
  warnBalanceUsd: number;
  criticalBalanceUsd: number;
  warnHealthPct: number;
  criticalHealthPct: number;
  warnAvailableCount: number;
  criticalAvailableCount: number;
  sites: SitePublic[];
}

export interface AppStateView {
  settings: AppSettingsPublic;
  snapshots: SiteSnapshot[];
  lastRefreshAt?: string | null;
  refreshing: boolean;
}

export interface SiteUpsert {
  id?: string | null;
  name: string;
  baseUrl: string;
  role: SiteRole;
  apiKey?: string | null;
  apiKeyLabel?: string | null;
  email?: string | null;
  password?: string | null;
  enabled?: boolean | null;
}

export interface LoginResult {
  requires2fa: boolean;
  tempToken?: string | null;
  userEmailMasked?: string | null;
  role?: string | null;
  message: string;
}
