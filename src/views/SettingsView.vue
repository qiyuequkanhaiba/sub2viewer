<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
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
let unlisten: (() => void) | undefined;

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
  lowBalanceThreshold: 1,
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
  settingsForm.lowBalanceThreshold = state.value.settings.lowBalanceThreshold;
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

async function saveSettings() {
  try {
    await updateSettings({
      refreshIntervalSecs: Number(settingsForm.refreshIntervalSecs),
      lowBalanceThreshold: Number(settingsForm.lowBalanceThreshold),
    });
    message.value = "通用设置已保存";
    await load();
  } catch (e) {
    error.value = String(e);
  }
}

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
  unlisten = await listen<AppStateView>("state-updated", (e) => {
    state.value = e.payload;
  });
});

onUnmounted(() => unlisten?.());
</script>

<template>
  <div class="settings">
    <header class="head">
      <div class="title-block">
        <h1>设置</h1>
        <p>密钥仅保存在本机 · 余额单位 USD</p>
      </div>
      <div v-if="message" class="toast ok">{{ message }}</div>
      <div v-if="error" class="toast bad">{{ error }}</div>
    </header>

    <section class="panel general">
      <h2>通用</h2>
      <div class="inline-form">
        <label class="inline">
          <span>刷新间隔</span>
          <input
            v-model.number="settingsForm.refreshIntervalSecs"
            type="number"
            min="15"
            max="3600"
          />
          <em>秒</em>
        </label>
        <label class="inline">
          <span>低余额阈值</span>
          <input
            v-model.number="settingsForm.lowBalanceThreshold"
            type="number"
            min="0"
            step="0.1"
          />
          <em>USD</em>
        </label>
        <button class="btn primary" type="button" @click="saveSettings">保存通用设置</button>
      </div>
    </section>

    <div class="split">
      <section class="panel sites">
        <h2>已配置站点</h2>
        <div v-if="!state?.settings.sites.length" class="blank">还没有站点，请在右侧添加</div>
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
                <i class="tag" :class="site.role">{{ site.role === "admin" ? "管理员" : "用户" }}</i>
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

      <section class="panel editor">
        <h2>{{ editing ? "编辑站点" : "添加站点" }}</h2>

        <div v-if="twoFa.active" class="twofa">
          <strong>2FA 验证 — {{ twoFa.emailMasked }}</strong>
          <label class="inline">
            <span>TOTP</span>
            <input v-model="twoFa.code" maxlength="6" placeholder="6 位数字" />
            <button class="btn primary" type="button" @click="submit2fa">提交</button>
            <button class="btn" type="button" @click="twoFa.active = false">取消</button>
          </label>
        </div>

        <div class="grid">
          <label class="field">
            <span>名称</span>
            <input v-model="form.name" placeholder="例如 My Sub2" />
          </label>
          <label class="field">
            <span>角色</span>
            <select v-model="form.role">
              <option value="user">普通用户 · API Key</option>
              <option value="admin">管理员 · 账号密码</option>
            </select>
          </label>
          <label class="field wide">
            <span>Base URL</span>
            <input v-model="form.baseUrl" placeholder="https://sub2.example.com" />
          </label>

          <template v-if="form.role === 'user'">
            <label class="field">
              <span>API Key{{ editing ? "（留空保留）" : "" }}</span>
              <input v-model="form.apiKey" type="password" placeholder="sk-..." />
            </label>
            <label class="field">
              <span>Key 备注</span>
              <input v-model="form.apiKeyLabel" placeholder="可选，如 Claude 组" />
            </label>
          </template>

          <template v-else>
            <label class="field">
              <span>管理员邮箱</span>
              <input v-model="form.email" type="email" placeholder="admin@example.com" />
            </label>
            <label class="field">
              <span>密码{{ editing ? "（留空保留）" : "" }}</span>
              <input v-model="form.password" type="password" />
            </label>
          </template>
        </div>

        <div class="editor-foot">
          <label class="check">
            <input v-model="form.enabled" type="checkbox" />
            <span>启用监控</span>
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
  gap: 14px;
  padding: 18px 22px 20px;
  box-sizing: border-box;
  background:
    radial-gradient(900px 280px at 0% -10%, rgba(91, 140, 255, 0.14), transparent 55%),
    var(--bg);
  font-family:
    "SF Pro Text",
    -apple-system,
    BlinkMacSystemFont,
    "Segoe UI",
    sans-serif;
}

.head {
  display: flex;
  align-items: center;
  gap: 16px;
  flex-shrink: 0;
}

.title-block h1 {
  margin: 0;
  font-size: 18px;
  font-weight: 650;
  letter-spacing: -0.3px;
  white-space: nowrap;
}

.title-block p {
  margin: 3px 0 0;
  font-size: 12px;
  color: var(--muted);
  white-space: nowrap;
}

.toast {
  margin-left: auto;
  padding: 6px 12px;
  border-radius: 999px;
  font-size: 12px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 46%;
}

.toast.ok {
  color: var(--ok);
  background: color-mix(in srgb, var(--ok) 12%, transparent);
}

.toast.bad {
  color: var(--bad);
  background: color-mix(in srgb, var(--bad) 12%, transparent);
}

.panel {
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.035), transparent 42%),
    var(--bg-card);
  border: 1px solid color-mix(in srgb, var(--border) 80%, transparent);
  border-radius: 16px;
  padding: 16px 18px;
  box-shadow:
    0 10px 28px rgba(0, 0, 0, 0.18),
    inset 0 1px 0 rgba(255, 255, 255, 0.05);
}

.panel h2 {
  margin: 0 0 12px;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--muted);
  white-space: nowrap;
}

.general {
  flex-shrink: 0;
}

.inline-form {
  display: flex;
  align-items: center;
  gap: 18px;
  flex-wrap: nowrap;
}

.inline {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  white-space: nowrap;
  flex-shrink: 0;
}

.inline span {
  font-size: 13px;
  color: var(--text);
  white-space: nowrap;
}

.inline em {
  font-style: normal;
  font-size: 12px;
  color: var(--muted);
}

.inline input {
  width: 88px;
  border: 1px solid var(--border);
  background: var(--bg);
  border-radius: 8px;
  padding: 7px 10px;
  outline: none;
}

.inline input:focus {
  border-color: var(--accent);
}

.split {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(520px, 1.35fr) minmax(300px, 0.85fr);
  gap: 14px;
}

.sites {
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: auto;
}

.blank {
  color: var(--muted);
  font-size: 13px;
  padding: 36px 8px;
  text-align: center;
  white-space: nowrap;
}

.cards {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-height: 0;
  overflow: auto;
  padding-right: 2px;
}

.site-card {
  display: grid;
  grid-template-columns: 40px minmax(0, 1fr) auto;
  gap: 12px;
  align-items: center;
  padding: 12px 14px;
  border-radius: 14px;
  background: color-mix(in srgb, var(--bg) 55%, transparent);
  border: 1px solid color-mix(in srgb, var(--border) 75%, transparent);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
  transition:
    border-color 0.18s ease,
    background 0.18s ease,
    transform 0.18s ease;
}

.site-card:hover {
  background: color-mix(in srgb, var(--bg) 30%, var(--bg-card));
  border-color: color-mix(in srgb, var(--accent) 28%, var(--border));
}

.site-card.active {
  border-color: color-mix(in srgb, var(--accent) 55%, var(--border));
  background: color-mix(in srgb, var(--accent) 8%, var(--bg-card));
}

.site-card.off {
  opacity: 0.72;
}

.avatar {
  width: 40px;
  height: 40px;
  border-radius: 12px;
  display: grid;
  place-items: center;
  font-size: 15px;
  font-weight: 700;
  letter-spacing: 0;
  color: #fff;
  flex-shrink: 0;
}

.avatar.user {
  background: linear-gradient(145deg, #34c759, #1f9d4a);
}

.avatar.admin {
  background: linear-gradient(145deg, #ff9f0a, #d97706);
}

.info {
  min-width: 0;
}

.line1 {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.line1 strong {
  font-size: 14px;
  font-weight: 650;
  letter-spacing: -0.2px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.line2,
.line3 {
  margin-top: 3px;
  font-size: 12px;
  color: var(--muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.site-card .acts {
  align-self: center;
}

.tag {
  display: inline-flex;
  align-items: center;
  height: 22px;
  padding: 0 8px;
  border-radius: 999px;
  font-size: 11px;
  font-style: normal;
  font-weight: 600;
  white-space: nowrap;
}

.tag.user {
  color: var(--ok);
  background: color-mix(in srgb, var(--ok) 12%, transparent);
}

.tag.admin {
  color: var(--warn);
  background: color-mix(in srgb, var(--warn) 12%, transparent);
}

.tag.mute {
  color: var(--muted);
  background: color-mix(in srgb, var(--muted) 14%, transparent);
}

.acts {
  display: flex;
  gap: 6px;
  flex-wrap: nowrap;
  flex-shrink: 0;
}

.btn {
  flex: 0 0 auto;
  border: 1px solid color-mix(in srgb, var(--border) 85%, transparent);
  background: color-mix(in srgb, var(--bg) 70%, transparent);
  border-radius: 9px;
  padding: 6px 11px;
  font-size: 12px;
  line-height: 1.2;
  white-space: nowrap;
  transition: 0.15s ease;
}

.btn:hover {
  border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  background: color-mix(in srgb, var(--accent) 10%, var(--bg));
}

.btn.primary {
  background: linear-gradient(135deg, #5b8cff, #7c5cff);
  border-color: transparent;
  color: #fff;
  font-weight: 600;
  box-shadow: 0 6px 16px rgba(91, 140, 255, 0.28);
}

.btn.primary:hover {
  filter: brightness(1.06);
}

.btn.danger {
  color: var(--bad);
  border-color: color-mix(in srgb, var(--bad) 35%, var(--border));
}

.btn.danger:hover {
  background: color-mix(in srgb, var(--bad) 12%, transparent);
}

.btn:disabled {
  opacity: 0.5;
}

.editor {
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.twofa {
  margin-bottom: 12px;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid color-mix(in srgb, var(--warn) 40%, var(--border));
}

.twofa strong {
  display: block;
  margin-bottom: 8px;
  font-size: 12px;
  white-space: nowrap;
}

.grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px 14px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.field.wide {
  grid-column: 1 / -1;
}

.field span {
  font-size: 12px;
  color: var(--muted);
  white-space: nowrap;
}

.field input,
.field select {
  width: 100%;
  box-sizing: border-box;
  border: 1px solid color-mix(in srgb, var(--border) 90%, transparent);
  background: color-mix(in srgb, var(--bg) 80%, transparent);
  border-radius: 10px;
  padding: 9px 11px;
  outline: none;
  white-space: nowrap;
}

.field input:focus,
.field select:focus {
  border-color: var(--accent);
}

.editor-foot {
  margin-top: auto;
  padding-top: 16px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.check {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  white-space: nowrap;
  font-size: 13px;
}

.check input {
  width: 14px;
  height: 14px;
}
</style>
