<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { AppStateView, SitePublic, SiteRole } from "../types";
import {
  adminLogin,
  adminLogin2fa,
  deleteSite,
  getState,
  updateSettings,
  upsertSite,
} from "../api";

const state = ref<AppStateView | null>(null);
const message = ref("");
const error = ref("");
const saving = ref(false);
const ready = ref(false);
let unlisten: (() => void) | undefined;
let persistTimer: number | undefined;

const form = reactive({
  id: "" as string,
  name: "",
  baseUrl: "",
  role: "user" as SiteRole,
  apiKey: "",
  apiKeyLabel: "",
  email: "",
  password: "",
  enabled: true,
});

const settingsForm = reactive({
  refreshIntervalSecs: 60,
  warnBalanceUsd: 5,
  criticalBalanceUsd: 1,
  warnAvailableCount: 5,
  criticalAvailableCount: 2,
});

const editing = computed(() => !!form.id);

const twoFa = reactive({
  siteId: "",
  tempToken: "",
  code: "",
  emailMasked: "",
  active: false,
});

async function load() {
  state.value = await getState();
  settingsForm.refreshIntervalSecs = state.value.settings.refreshIntervalSecs;
  settingsForm.warnBalanceUsd = state.value.settings.warnBalanceUsd ?? 5;
  settingsForm.criticalBalanceUsd =
    state.value.settings.criticalBalanceUsd ??
    state.value.settings.lowBalanceThreshold ??
    1;
  settingsForm.warnAvailableCount = state.value.settings.warnAvailableCount ?? 5;
  settingsForm.criticalAvailableCount =
    state.value.settings.criticalAvailableCount ?? 2;
}

function resetForm() {
  form.id = "";
  form.name = "";
  form.baseUrl = "";
  form.role = "user";
  form.apiKey = "";
  form.apiKeyLabel = "";
  form.email = "";
  form.password = "";
  form.enabled = true;
}

function editSite(site: SitePublic) {
  form.id = site.id;
  form.name = site.name;
  form.baseUrl = site.baseUrl;
  form.role = site.role;
  form.apiKey = "";
  form.apiKeyLabel = site.apiKeyLabel || "";
  form.email = site.email || "";
  form.password = "";
  form.enabled = site.enabled;
  message.value = `正在编辑：${site.name}`;
}

async function saveSite() {
  saving.value = true;
  error.value = "";
  message.value = "";
  try {
    await upsertSite({
      id: form.id || null,
      name: form.name,
      baseUrl: form.baseUrl,
      role: form.role,
      apiKey: form.apiKey || null,
      apiKeyLabel: form.apiKeyLabel || null,
      email: form.email || null,
      password: form.password || null,
      enabled: form.enabled,
    });
    message.value = "站点已保存，正在刷新数据…";
    resetForm();
    await load();
  } catch (e) {
    error.value = String(e);
  } finally {
    saving.value = false;
  }
}

async function removeSite(site: SitePublic) {
  if (!confirm(`确认删除站点「${site.name}」？`)) return;
  try {
    await deleteSite(site.id);
    if (form.id === site.id) resetForm();
    await load();
    message.value = "已删除";
  } catch (e) {
    error.value = String(e);
  }
}

async function persistSettings() {
  const refresh = Number(settingsForm.refreshIntervalSecs);
  const warnBal = Number(settingsForm.warnBalanceUsd);
  const critBal = Number(settingsForm.criticalBalanceUsd);
  const warnAvail = Number(settingsForm.warnAvailableCount);
  const critAvail = Number(settingsForm.criticalAvailableCount);
  if ([refresh, warnBal, critBal, warnAvail, critAvail].some((n) => Number.isNaN(n))) {
    return;
  }
  try {
    await updateSettings({
      refreshIntervalSecs: refresh,
      warnBalanceUsd: warnBal,
      criticalBalanceUsd: critBal,
      warnAvailableCount: warnAvail,
      criticalAvailableCount: critAvail,
    });
    error.value = "";
  } catch (e) {
    error.value = String(e);
  }
}

function schedulePersist() {
  if (!ready.value) return;
  if (persistTimer) window.clearTimeout(persistTimer);
  persistTimer = window.setTimeout(() => {
    void persistSettings();
  }, 220);
}

watch(settingsForm, schedulePersist, { deep: true });

async function doAdminLogin(site: SitePublic) {
  const email = prompt("管理员邮箱", site.email || "") || "";
  const password = prompt("管理员密码") || "";
  if (!email || !password) return;
  try {
    const res = await adminLogin(site.id, email, password);
    if (res.requires2fa && res.tempToken) {
      twoFa.active = true;
      twoFa.siteId = site.id;
      twoFa.tempToken = res.tempToken;
      twoFa.emailMasked = res.userEmailMasked || email;
      twoFa.code = "";
      message.value = "需要 2FA 验证码";
    } else {
      message.value = res.message || "登录成功";
    }
    await load();
  } catch (e) {
    error.value = String(e);
  }
}

async function submit2fa() {
  try {
    const res = await adminLogin2fa(twoFa.siteId, twoFa.tempToken, twoFa.code);
    message.value = res.message;
    twoFa.active = false;
    await load();
  } catch (e) {
    error.value = String(e);
  }
}

function credText(site: SitePublic) {
  if (site.role === "user") {
    return site.hasApiKey
      ? site.apiKeyLabel
        ? `Key 已配置 · ${site.apiKeyLabel}`
        : "Key 已配置"
      : "缺少 API Key";
  }
  const bits = [
    site.email || "未填邮箱",
    site.hasPassword ? "密码已存" : "无密码",
    site.hasToken ? "已登录" : "未登录",
  ];
  return bits.join(" · ");
}

onMounted(async () => {
  await load();
  ready.value = true;
  unlisten = await listen<AppStateView>("state-updated", (e) => {
    state.value = e.payload;
  });
});

onUnmounted(() => {
  unlisten?.();
  if (persistTimer) window.clearTimeout(persistTimer);
});
</script>

<template>
  <div class="settings">
    <!-- Header -->
    <header class="head">
      <div class="title-block">
        <h1>设置与站点管理</h1>
        <p>密钥仅保存在本机 · 金额统一 USD</p>
      </div>
      <div v-if="message" class="toast ok">{{ message }}</div>
      <div v-if="error" class="toast bad">{{ error }}</div>
    </header>

    <!-- General Threshold Panel -->
    <section class="panel general">
      <div class="panel-head">
        <h2>监测告警阈值</h2>
        <span class="live">即时同步生效</span>
      </div>

      <div class="threshold-grid">
        <label class="metric-cell span2">
          <span class="metric-label">自动刷新间隔</span>
          <span class="metric-field">
            <input
              v-model.number="settingsForm.refreshIntervalSecs"
              type="number"
              min="15"
              max="3600"
            />
            <em>秒</em>
          </span>
        </label>

        <div class="metric-group">
          <div class="group-title">普通用户余额 (USD)</div>
          <label class="metric-cell">
            <span class="metric-label"><i class="swatch warn" />预警阈值</span>
            <span class="metric-field">
              <input
                v-model.number="settingsForm.warnBalanceUsd"
                type="number"
                min="0"
                step="0.1"
              />
              <em>USD</em>
            </span>
          </label>
          <label class="metric-cell">
            <span class="metric-label"><i class="swatch bad" />严重告警</span>
            <span class="metric-field">
              <input
                v-model.number="settingsForm.criticalBalanceUsd"
                type="number"
                min="0"
                step="0.1"
              />
              <em>USD</em>
            </span>
          </label>
        </div>

        <div class="metric-group">
          <div class="group-title">管理员可用账号数</div>
          <label class="metric-cell">
            <span class="metric-label"><i class="swatch warn" />预警阈值</span>
            <span class="metric-field">
              <input
                v-model.number="settingsForm.warnAvailableCount"
                type="number"
                min="0"
                step="1"
              />
              <em>个</em>
            </span>
          </label>
          <label class="metric-cell">
            <span class="metric-label"><i class="swatch bad" />严重告警</span>
            <span class="metric-field">
              <input
                v-model.number="settingsForm.criticalAvailableCount"
                type="number"
                min="0"
                step="1"
              />
              <em>个</em>
            </span>
          </label>
        </div>
      </div>
    </section>

    <!-- Split View for Sites & Editor -->
    <div class="split">
      <!-- Sites List -->
      <section class="panel sites">
        <h2>已配置站点 ({{ state?.settings.sites.length || 0 }})</h2>
        <div v-if="!state?.settings.sites.length" class="blank">
          暂无已配置站点，请在右侧表单添加
        </div>
        <div v-else class="cards">
          <article
            v-for="site in state.settings.sites"
            :key="site.id"
            class="site-card"
            :class="{ active: form.id === site.id, off: !site.enabled }"
          >
            <div class="avatar" :class="site.role">
              {{ (site.name || "S").slice(0, 1).toUpperCase() }}
            </div>
            <div class="info">
              <div class="line1">
                <strong :title="site.name">{{ site.name }}</strong>
                <i class="tag" :class="site.role">{{ site.role === "admin" ? "管理员" : "普通用户" }}</i>
                <i v-if="!site.enabled" class="tag mute">已停用</i>
              </div>
              <div class="line2" :title="site.baseUrl">{{ site.baseUrl }}</div>
              <div class="line3" :title="credText(site)">{{ credText(site) }}</div>
            </div>
            <div class="acts">
              <button
                v-if="site.role === 'admin'"
                class="btn"
                type="button"
                @click="doAdminLogin(site)"
              >
                登录
              </button>
              <button class="btn" type="button" @click="editSite(site)">编辑</button>
              <button class="btn danger" type="button" @click="removeSite(site)">删除</button>
            </div>
          </article>
        </div>
      </section>

      <!-- Site Editor Form -->
      <section class="panel editor">
        <h2>{{ editing ? "编辑站点" : "添加新站点" }}</h2>

        <div v-if="twoFa.active" class="twofa">
          <strong>2FA 二次验证 — {{ twoFa.emailMasked }}</strong>
          <label class="inline">
            <span>TOTP 验证码</span>
            <input v-model="twoFa.code" maxlength="6" placeholder="6 位验证码" />
            <button class="btn primary" type="button" @click="submit2fa">提交验证</button>
            <button class="btn" type="button" @click="twoFa.active = false">取消</button>
          </label>
        </div>

        <div class="grid">
          <label class="field">
            <span>站点名称</span>
            <input v-model="form.name" placeholder="例如 Claude 主站 / GPT 聚合" />
          </label>
          <label class="field">
            <span>站点角色</span>
            <select v-model="form.role">
              <option value="user">普通用户 · API Key 模式</option>
              <option value="admin">管理员 · 账号密码模式</option>
            </select>
          </label>
          <label class="field wide">
            <span>Base URL (站点根地址)</span>
            <input v-model="form.baseUrl" placeholder="https://sub2.example.com (不带 /v1)" />
          </label>

          <template v-if="form.role === 'user'">
            <label class="field">
              <span>API Key{{ editing ? "（留空保留原密钥）" : "" }}</span>
              <input v-model="form.apiKey" type="password" placeholder="sk-..." />
            </label>
            <label class="field">
              <span>Key 备注</span>
              <input v-model="form.apiKeyLabel" placeholder="可选，如个人主 Key" />
            </label>
          </template>

          <template v-else>
            <label class="field">
              <span>管理员邮箱</span>
              <input v-model="form.email" type="email" placeholder="admin@example.com" />
            </label>
            <label class="field">
              <span>管理员密码{{ editing ? "（留空保留原密码）" : "" }}</span>
              <input v-model="form.password" type="password" placeholder="密码" />
            </label>
          </template>
        </div>

        <div class="editor-foot">
          <label class="check">
            <input v-model="form.enabled" type="checkbox" />
            <span>启用此站点监控</span>
          </label>
          <div class="acts">
            <button class="btn primary" type="button" :disabled="saving" @click="saveSite">
              {{ editing ? "更新站点" : "添加站点" }}
            </button>
            <button v-if="editing" class="btn" type="button" @click="resetForm">取消编辑</button>
          </div>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.settings {
  height: 100vh;
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px 20px;
  box-sizing: border-box;
  background:
    radial-gradient(1000px 300px at 10% -10%, rgba(10, 132, 255, 0.16), transparent 55%),
    radial-gradient(900px 300px at 90% 10%, rgba(94, 92, 230, 0.1), transparent 55%),
    var(--bg);
  font-family:
    "SF Pro Display",
    "SF Pro Text",
    -apple-system,
    BlinkMacSystemFont,
    "Segoe UI",
    sans-serif;
  color: var(--text);
}

.head {
  display: flex;
  align-items: center;
  gap: 16px;
  flex-shrink: 0;
  padding-bottom: 2px;
}

.title-block h1 {
  margin: 0;
  font-size: 18px;
  font-weight: 700;
  letter-spacing: -0.3px;
  white-space: nowrap;
}

.title-block p {
  margin: 2px 0 0;
  font-size: 11px;
  color: var(--muted);
  white-space: nowrap;
}

.toast {
  margin-left: auto;
  padding: 5px 12px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 45%;
}

.toast.ok {
  color: #30d158;
  background: rgba(48, 209, 88, 0.12);
  border: 1px solid rgba(48, 209, 88, 0.3);
}

.toast.bad {
  color: #ff453a;
  background: rgba(255, 69, 58, 0.12);
  border: 1px solid rgba(255, 69, 58, 0.3);
}

.panel {
  background: rgba(25, 29, 40, 0.7);
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-top: 1px solid rgba(255, 255, 255, 0.18);
  border-radius: 14px;
  padding: 14px 16px;
  box-shadow:
    0 12px 30px rgba(0, 0, 0, 0.25),
    inset 0 1px 0 rgba(255, 255, 255, 0.06);
  backdrop-filter: blur(24px);
  -webkit-backdrop-filter: blur(24px);
}

.panel h2 {
  margin: 0 0 10px;
  font-size: 11px;
  font-weight: 650;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--muted);
  white-space: nowrap;
}

.general {
  flex-shrink: 0;
}

.panel-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
}

.panel-head h2 {
  margin: 0;
}

.live {
  font-size: 10px;
  font-weight: 600;
  color: #30d158;
  white-space: nowrap;
}

.threshold-grid {
  display: grid;
  grid-template-columns: 160px minmax(0, 1fr) minmax(0, 1fr);
  gap: 10px;
  align-items: stretch;
}

@media (max-width: 880px) {
  .threshold-grid {
    grid-template-columns: 1fr 1fr;
  }
  .metric-cell.span2 {
    grid-column: 1 / -1;
  }
}

.metric-group {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
  padding: 10px;
  border-radius: 10px;
  background: rgba(0, 0, 0, 0.22);
  border: 1px solid rgba(255, 255, 255, 0.08);
}

.group-title {
  grid-column: 1 / -1;
  font-size: 11px;
  font-weight: 650;
  letter-spacing: -0.1px;
  white-space: nowrap;
  color: #d1d1d6;
}

.metric-cell {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.metric-cell.span2 {
  justify-content: center;
  padding: 10px;
  border-radius: 10px;
  background: rgba(0, 0, 0, 0.22);
  border: 1px solid rgba(255, 255, 255, 0.08);
}

.metric-label {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  color: var(--muted);
  white-space: nowrap;
}

.metric-field {
  display: flex;
  align-items: center;
  gap: 6px;
}

.metric-field input {
  width: 76px;
  min-width: 0;
  flex: 1;
  border: 1px solid rgba(255, 255, 255, 0.14);
  background: rgba(0, 0, 0, 0.35);
  border-radius: 7px;
  padding: 6px 8px;
  color: #fff;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
  outline: none;
  transition: all 0.15s ease;
}

.metric-field input:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px rgba(10, 132, 255, 0.25);
}

.metric-field em {
  font-style: normal;
  font-size: 11px;
  color: var(--muted);
  white-space: nowrap;
}

.swatch {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex-shrink: 0;
}

.swatch.warn {
  background: #ff9f0a;
}

.swatch.bad {
  background: #ff453a;
}

.split {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(460px, 1.25fr) minmax(320px, 1fr);
  gap: 12px;
}

.sites {
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.blank {
  color: var(--muted);
  font-size: 12px;
  padding: 36px 8px;
  text-align: center;
  white-space: nowrap;
}

.cards {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: 1;
  overflow-y: auto;
  padding-right: 4px;
}

.site-card {
  display: grid;
  grid-template-columns: 36px minmax(0, 1fr) auto;
  gap: 10px;
  align-items: center;
  padding: 10px 12px;
  border-radius: 11px;
  background: rgba(0, 0, 0, 0.2);
  border: 1px solid rgba(255, 255, 255, 0.08);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
  transition: all 0.16s var(--ease-apple);
}

.site-card:hover {
  background: rgba(255, 255, 255, 0.06);
  border-color: rgba(255, 255, 255, 0.18);
}

.site-card.active {
  border-color: var(--accent);
  background: rgba(10, 132, 255, 0.12);
}

.site-card.off {
  opacity: 0.65;
}

.avatar {
  width: 36px;
  height: 36px;
  border-radius: 10px;
  display: grid;
  place-items: center;
  font-size: 14px;
  font-weight: 700;
  color: #fff;
  flex-shrink: 0;
}

.avatar.user {
  background: linear-gradient(135deg, #0a84ff, #0071e3);
}

.avatar.admin {
  background: linear-gradient(135deg, #ff9f0a, #d97706);
}

.info {
  min-width: 0;
}

.line1 {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.line1 strong {
  font-size: 13px;
  font-weight: 650;
  letter-spacing: -0.2px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.line2,
.line3 {
  margin-top: 2px;
  font-size: 11px;
  color: var(--muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.tag {
  display: inline-flex;
  align-items: center;
  height: 18px;
  padding: 0 6px;
  border-radius: 999px;
  font-size: 10px;
  font-style: normal;
  font-weight: 600;
  white-space: nowrap;
}

.tag.user {
  color: #64d2ff;
  background: rgba(10, 132, 255, 0.16);
  border: 1px solid rgba(10, 132, 255, 0.35);
}

.tag.admin {
  color: #ff9f0a;
  background: rgba(255, 159, 10, 0.16);
  border: 1px solid rgba(255, 159, 10, 0.35);
}

.tag.mute {
  color: var(--muted);
  background: rgba(255, 255, 255, 0.08);
}

.acts {
  display: flex;
  gap: 6px;
  flex-wrap: nowrap;
  flex-shrink: 0;
}

.btn {
  flex: 0 0 auto;
  border: 1px solid rgba(255, 255, 255, 0.14);
  background: rgba(255, 255, 255, 0.07);
  border-radius: 7px;
  padding: 5px 10px;
  font-size: 11px;
  font-weight: 500;
  color: #fff;
  white-space: nowrap;
  transition: all 0.15s ease;
}

.btn:hover {
  border-color: rgba(255, 255, 255, 0.28);
  background: rgba(255, 255, 255, 0.14);
}

.btn.primary {
  background: linear-gradient(135deg, #0a84ff, #5e5ce6);
  border-color: transparent;
  color: #fff;
  font-weight: 600;
  box-shadow: 0 4px 12px rgba(10, 132, 255, 0.3);
}

.btn.danger {
  color: var(--bad);
  background: rgba(255, 69, 58, 0.1);
  border-color: rgba(255, 69, 58, 0.3);
}

.btn.danger:hover {
  background: rgba(255, 69, 58, 0.22);
}

.btn:disabled {
  opacity: 0.45;
}

.editor {
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.twofa {
  margin-bottom: 10px;
  padding: 8px 10px;
  border-radius: 8px;
  background: rgba(255, 159, 10, 0.1);
  border: 1px solid rgba(255, 159, 10, 0.35);
}

.twofa strong {
  display: block;
  margin-bottom: 6px;
  font-size: 11px;
  color: #ff9f0a;
}

.inline {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
}

.inline span {
  font-size: 11px;
  color: var(--text);
}

.inline input {
  width: 80px;
  border: 1px solid rgba(255, 255, 255, 0.15);
  background: rgba(0, 0, 0, 0.4);
  border-radius: 6px;
  padding: 5px 8px;
  color: #fff;
  font-size: 11px;
  outline: none;
}

.grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px 12px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.field.wide {
  grid-column: 1 / -1;
}

.field span {
  font-size: 11px;
  font-weight: 500;
  color: var(--muted);
  white-space: nowrap;
}

.field input,
.field select {
  width: 100%;
  box-sizing: border-box;
  border: 1px solid rgba(255, 255, 255, 0.12);
  background: rgba(0, 0, 0, 0.3);
  border-radius: 8px;
  padding: 7px 9px;
  color: #fff;
  font-size: 12px;
  outline: none;
  transition: all 0.15s ease;
}

.field input:focus,
.field select:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px rgba(10, 132, 255, 0.25);
}

.editor-foot {
  margin-top: auto;
  padding-top: 14px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.check {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
  font-size: 12px;
  color: rgba(255, 255, 255, 0.85);
  cursor: pointer;
}

.check input {
  width: 15px;
  height: 15px;
  accent-color: #0a84ff;
  cursor: pointer;
}
</style>
