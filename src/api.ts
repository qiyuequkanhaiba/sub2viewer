import { invoke } from "@tauri-apps/api/core";
import type {
  AppStateView,
  LoginResult,
  SitePublic,
  SiteUpsert,
} from "./types";

export function getState() {
  return invoke<AppStateView>("get_state");
}

export function refreshNow() {
  return invoke<AppStateView>("refresh_now");
}

export function upsertSite(input: SiteUpsert) {
  return invoke<SitePublic>("upsert_site", { input });
}

export function deleteSite(siteId: string) {
  return invoke<void>("delete_site", { siteId });
}

export function updateSettings(input: {
  refreshIntervalSecs?: number;
  lowBalanceThreshold?: number;
  warnBalanceUsd?: number;
  criticalBalanceUsd?: number;
  warnHealthPct?: number;
  criticalHealthPct?: number;
  warnAvailableCount?: number;
  criticalAvailableCount?: number;
  launchAtLogin?: boolean;
}) {
  return invoke<void>("update_settings", { input });
}

export function adminLogin(siteId: string, email: string, password: string) {
  return invoke<LoginResult>("admin_login", { siteId, email, password });
}

export function adminLogin2fa(
  siteId: string,
  tempToken: string,
  totpCode: string,
) {
  return invoke<LoginResult>("admin_login_2fa", {
    siteId,
    tempToken,
    totpCode,
  });
}

export function openSettingsWindow() {
  return invoke<void>("open_settings_window");
}
