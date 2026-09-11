<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import { LogicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import type { AppStateView } from "../types";
import { invoke } from "@tauri-apps/api/core";
import { getState } from "../api";
import { mergeMetrics, statusLabel, thresholdsFromSettings } from "../metrics";
import { formatUsdFixed } from "../utils";
import { bindWindowDrag } from "../useWindowDrag";

const state = ref<AppStateView | null>(null);
const pillRef = ref<HTMLElement | null>(null);
const pinned = ref(false);
const isHovered = ref(false);
let unlistenState: (() => void) | undefined;
let unbindDrag: (() => void) | undefined;
let ro: ResizeObserver | undefined;

const m = computed(() =>
  mergeMetrics(
    state.value?.snapshots || [],
    thresholdsFromSettings(state.value?.settings),
  ),
);

const statusText = computed(() => statusLabel(m.value.tone));
const showRows = computed(() => m.value.rows.length > 0);
const isExpanded = computed(() => pinned.value || isHovered.value);

async function syncChrome() {
  const el = pillRef.value;
  if (!el) return;
  const r = el.getBoundingClientRect();
  const radius = isExpanded.value ? 18 : 18;
  try {
    await getCurrentWindow().setSize(
      new LogicalSize(Math.ceil(r.width), Math.ceil(r.height)),
    );
    await invoke("set_hud_radius", { radius });
  } catch {
    // ignore
  }
}

async function load() {
  try {
    state.value = await getState();
  } catch {
    // keep last
  }
}

function handleMouseEnter() {
  isHovered.value = true;
}

function handleMouseLeave() {
  isHovered.value = false;
}

onMounted(async () => {
  document.documentElement.classList.add("panel-mode");
  document.body.classList.add("panel-mode");
  await load();
  unlistenState = await listen<AppStateView>("state-updated", (e) => {
    state.value = e.payload;
  });
  await nextTick();
  if (pillRef.value) {
    unbindDrag = bindWindowDrag(pillRef.value);
    ro = new ResizeObserver(() => void syncChrome());
    ro.observe(pillRef.value);
    await syncChrome();
  }
});

watch(
  [
    isExpanded,
    () => m.value.rows.length,
    () => m.value.hero,
    () => m.value.okAccounts,
    () => m.value.totalAccounts,
  ],
  async () => {
    await nextTick();
    await syncChrome();
  },
);

onUnmounted(() => {
  document.documentElement.classList.remove("panel-mode");
  document.body.classList.remove("panel-mode");
  unlistenState?.();
  unbindDrag?.();
  ro?.disconnect();
});
</script>

<template>
  <div
    ref="pillRef"
    class="pill"
    :class="[m.tone, { expanded: isExpanded }]"
    @mouseenter="handleMouseEnter"
    @mouseleave="handleMouseLeave"
    @dblclick.stop="pinned = !pinned"
  >
    <!-- Top Capsule Bar -->
    <div class="head">
      <div class="lead">
        <div class="dot" :class="{ pulse: m.tone === 'ok' }" />
        <span v-if="m.heroBalance" class="hero money">{{ m.heroBalance }}</span>
        <span v-if="m.heroBalance && m.heroAccounts" class="sep">·</span>
        <span
          v-if="m.heroAccounts"
          class="hero accounts"
          title="正常 / 错误 / 总量"
        >{{ m.heroAccounts }}</span>
        <span v-if="!m.heroBalance && !m.heroAccounts" class="hero">{{ m.hero }}</span>
      </div>
      <span class="status-chip">{{ statusText }}</span>
    </div>

    <!-- Expanded Body Content (Auto height hugging) -->
    <div v-if="isExpanded" class="body">
      <p class="detail" :title="m.detail">{{ m.detail }}</p>

      <div class="usage">
        <div class="stat">
          <span class="k">{{ m.usageScope === "user" && m.hasAdmin ? "个人今日" : "今日消耗" }}</span>
          <span class="v">{{ formatUsdFixed(m.todayCost) }}</span>
        </div>
        <div class="stat">
          <span class="k">{{ m.usageScope === "user" && m.hasAdmin ? "个人本月" : "本月累计" }}</span>
          <span class="v">{{ formatUsdFixed(m.monthCost) }}</span>
        </div>
      </div>

      <div v-if="m.hasAdmin && m.totalAccounts" class="health">
        <div class="stat ok">
          <span class="k">正常</span>
          <span class="v">{{ m.okAccounts }}</span>
        </div>
        <div class="stat bad">
          <span class="k">错误</span>
          <span class="v">{{ m.errorAccounts }}</span>
        </div>
        <div class="stat warn">
          <span class="k">限流</span>
          <span class="v">{{ m.rateLimitedAccounts }}</span>
        </div>
        <div class="stat dim">
          <span class="k">不可调度</span>
          <span class="v">{{ m.unschedulableAccounts }}</span>
        </div>
      </div>

      <div v-if="showRows" class="sites">
        <div
          v-for="row in m.rows"
          :key="row.name"
          class="site"
          :class="row.tone"
        >
          <div class="site-info">
            <span class="site-name" :title="row.name">{{ row.name }}</span>
            <span class="site-usage">
              {{ row.kind === "admin" ? "平台" : "今日" }}
              {{ formatUsdFixed(row.todayCost) }}
              · 本月 {{ formatUsdFixed(row.monthCost) }}
            </span>
          </div>
          <span class="site-val">{{ row.value }}</span>
        </div>
      </div>

      <div class="foot">
        <span>{{ m.footerLeft }}</span>
        <span>{{ pinned ? '已固定 · ' : '' }}{{ m.footerRight }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.pill {
  box-sizing: border-box;
  width: max-content;
  max-width: 320px;
  height: 36px;
  padding: 0 11px;
  margin: 0;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.22), rgba(255, 255, 255, 0.12)),
    rgba(20, 24, 35, 0.45);
  backdrop-filter: blur(28px) saturate(200%);
  -webkit-backdrop-filter: blur(28px) saturate(200%);
  border: 1px solid rgba(255, 255, 255, 0.35);
  border-top: 1px solid rgba(255, 255, 255, 0.7);
  border-radius: 18px;
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.25), inset 0 1px 0 rgba(255, 255, 255, 0.4);
  color: #ffffff;
  overflow: hidden;
  cursor: grab;
  user-select: none;
  -webkit-user-select: none;
  font-family:
    "SF Pro Display",
    "SF Pro Text",
    -apple-system,
    BlinkMacSystemFont,
    "Segoe UI",
    sans-serif;
  -webkit-font-smoothing: antialiased;
  transition:
    width 0.28s cubic-bezier(0.16, 1, 0.3, 1),
    border-radius 0.28s cubic-bezier(0.16, 1, 0.3, 1),
    background 0.28s cubic-bezier(0.16, 1, 0.3, 1),
    box-shadow 0.28s cubic-bezier(0.16, 1, 0.3, 1);
  display: inline-flex;
  flex-direction: column;
}

.pill:active {
  cursor: grabbing;
}

/* Tone Variants */
.pill.warn {
  background:
    linear-gradient(180deg, rgba(255, 159, 10, 0.28), rgba(255, 255, 255, 0.12)),
    rgba(35, 24, 15, 0.5);
  border-color: rgba(255, 159, 10, 0.45);
  border-top-color: rgba(255, 214, 10, 0.75);
}

.pill.bad {
  background:
    linear-gradient(180deg, rgba(255, 69, 58, 0.28), rgba(255, 255, 255, 0.12)),
    rgba(35, 15, 18, 0.5);
  border-color: rgba(255, 69, 58, 0.45);
  border-top-color: rgba(255, 105, 97, 0.75);
}

.pill.muted {
  background:
    linear-gradient(180deg, rgba(142, 142, 147, 0.2), rgba(255, 255, 255, 0.08)),
    rgba(25, 25, 30, 0.4);
}

/* Expanded State: Hugs content automatically without empty space */
.pill.expanded {
  width: 290px;
  height: auto;
  min-height: 36px;
  border-radius: 18px;
  padding: 8px 13px 11px;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.26), rgba(255, 255, 255, 0.14)),
    rgba(16, 20, 30, 0.72);
  box-shadow: 0 20px 48px rgba(0, 0, 0, 0.45), inset 0 1px 0 rgba(255, 255, 255, 0.6);
}

.head {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 36px;
  flex-shrink: 0;
}

.pill.expanded .head {
  justify-content: space-between;
  width: 100%;
  height: 28px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.12);
  padding-bottom: 4px;
}

.lead {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #30d158;
  box-shadow: 0 0 8px rgba(48, 209, 88, 0.75);
  flex-shrink: 0;
  position: relative;
}

.dot.pulse::after {
  content: "";
  position: absolute;
  inset: -2px;
  border-radius: 50%;
  border: 1px solid #30d158;
  animation: ripple 2s infinite ease-out;
}

@keyframes ripple {
  0% { transform: scale(1); opacity: 0.8; }
  100% { transform: scale(2.2); opacity: 0; }
}

.pill.warn .dot {
  background: #ff9f0a;
  box-shadow: 0 0 8px rgba(255, 159, 10, 0.75);
}

.pill.bad .dot {
  background: #ff453a;
  box-shadow: 0 0 8px rgba(255, 69, 58, 0.75);
}

.pill.muted .dot {
  background: #8e8e93;
  box-shadow: none;
}

.hero {
  font-size: 13px;
  font-weight: 700;
  letter-spacing: -0.25px;
  font-variant-numeric: tabular-nums;
  font-feature-settings: "tnum" 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hero.accounts {
  color: rgba(255, 255, 255, 0.9);
  font-size: 12px;
  font-weight: 600;
}

.sep {
  font-size: 11px;
  color: rgba(255, 255, 255, 0.35);
  flex-shrink: 0;
}

.status-chip {
  font-size: 10px;
  font-weight: 650;
  letter-spacing: 0.02em;
  color: #30d158;
  background: rgba(48, 209, 88, 0.16);
  border: 1px solid rgba(48, 209, 88, 0.35);
  padding: 2px 7px;
  border-radius: 6px;
  flex-shrink: 0;
  white-space: nowrap;
}

.pill.warn .status-chip {
  color: #ff9f0a;
  background: rgba(255, 159, 10, 0.16);
  border-color: rgba(255, 159, 10, 0.35);
}

.pill.bad .status-chip {
  color: #ff453a;
  background: rgba(255, 69, 58, 0.16);
  border-color: rgba(255, 69, 58, 0.35);
}

.pill.muted .status-chip {
  color: rgba(255, 255, 255, 0.5);
  background: rgba(255, 255, 255, 0.08);
  border-color: rgba(255, 255, 255, 0.15);
}

.body {
  animation: fadeInBody 0.2s cubic-bezier(0.16, 1, 0.3, 1);
  margin-top: 7px;
  display: flex;
  flex-direction: column;
  gap: 7px;
}

@keyframes fadeInBody {
  from {
    opacity: 0;
    transform: translateY(4px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

.detail {
  margin: 0;
  font-size: 11px;
  font-weight: 500;
  letter-spacing: -0.15px;
  line-height: 1.3;
  color: rgba(255, 255, 255, 0.75);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.usage,
.health {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 7px;
}

.stat {
  padding: 5px 8px;
  border-radius: 8px;
  background: rgba(0, 0, 0, 0.18);
  border: 1px solid rgba(255, 255, 255, 0.09);
}

.stat .k {
  display: block;
  font-size: 10px;
  font-weight: 500;
  letter-spacing: 0.02em;
  color: rgba(255, 255, 255, 0.55);
}

.stat .v {
  display: block;
  margin-top: 1px;
  font-size: 13px;
  font-weight: 700;
  letter-spacing: -0.25px;
  font-variant-numeric: tabular-nums;
  color: #ffffff;
}

.stat.ok .v {
  color: #30d158;
}

.stat.bad .v {
  color: #ff453a;
}

.stat.warn .v {
  color: #ff9f0a;
}

.stat.dim .v {
  color: rgba(255, 255, 255, 0.72);
}

.sites {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 110px;
  overflow-y: auto;
}

.site {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  font-size: 11px;
  padding: 5px 8px;
  border-radius: 7px;
  background: rgba(0, 0, 0, 0.16);
  border: 1px solid rgba(255, 255, 255, 0.06);
}

.site-info {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.site-usage {
  font-size: 10px;
  font-weight: 500;
  color: rgba(255, 255, 255, 0.5);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.site-name {
  color: rgba(255, 255, 255, 0.88);
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.site-val {
  font-variant-numeric: tabular-nums;
  font-weight: 650;
  color: #64d2ff;
  flex-shrink: 0;
}

.site.warn .site-val {
  color: #ff9f0a;
}

.site.bad .site-val {
  color: #ff453a;
}

.foot {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  font-size: 10px;
  font-weight: 450;
  letter-spacing: -0.05px;
  color: rgba(255, 255, 255, 0.5);
  padding-top: 4px;
  border-top: 1px solid rgba(255, 255, 255, 0.08);
}

.foot span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
