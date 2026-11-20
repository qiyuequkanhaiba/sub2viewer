<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { AppStateView, NotchGeometry } from "../types";
import { getNotchGeometry, getState, setPillRect } from "../api";
import {
  changeLine,
  keyGroupPanel,
  mergeMetrics,
  poolPanel,
  statusLabel,
  thresholdsFromSettings,
} from "../metrics";
import { formatUsdFixed } from "../utils";
import { bindWindowDrag } from "../useWindowDrag";

const state = ref<AppStateView | null>(null);
const pillRef = ref<HTMLElement | null>(null);
const leadRef = ref<HTMLElement | null>(null);
const chipRef = ref<HTMLElement | null>(null);
const pinned = ref(false);
const hoverIntent = ref(false);
const peeking = ref(false);
const geometry = ref<NotchGeometry>({
  hasNotch: false,
  width: 185,
  height: 32,
});
let unlistenState: (() => void) | undefined;
let unlistenPeek: (() => void) | undefined;
let unlistenCursorOut: (() => void) | undefined;
let unbindDrag: (() => void) | undefined;
let hoverTimer: ReturnType<typeof setTimeout> | undefined;
let leaveTimer: ReturnType<typeof setTimeout> | undefined;
let peekTimer: ReturnType<typeof setTimeout> | undefined;

const m = computed(() =>
  mergeMetrics(
    state.value?.snapshots || [],
    thresholdsFromSettings(state.value?.settings),
  ),
);

const statusText = computed(() => statusLabel(m.value.tone));
const showRows = computed(() => m.value.rows.length > 0);
const isExpanded = computed(
  () => pinned.value || hoverIntent.value || peeking.value,
);
const keys = computed(() => keyGroupPanel(state.value?.snapshots || []));
const pools = computed(() => poolPanel(state.value?.snapshots || []));
const deltaText = computed(() => changeLine(state.value?.snapshots || []));
const updatedText = computed(() => {
  const iso = state.value?.lastRefreshAt;
  if (!iso) return "";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const clock = date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return `更新于 ${clock}`;
});

// --- Island sizing -------------------------------------------------------
// The window is fixed-size; the capsule morphs inside it, driven by a
// JS spring integrator using boring.notch's parameters converted to
// stiffness/damping: open k=224 c=23.9 (slight overshoot), close
// k=195 c=27.9 (critically damped), content resize k=273 c=26.4.
interface Spring {
  k: number;
  c: number;
}
const SPRING_OPEN: Spring = { k: 224, c: 23.9 };
const SPRING_CLOSE: Spring = { k: 195, c: 27.9 };
const SPRING_INTERACTIVE: Spring = { k: 273, c: 26.4 };
const EXPANDED_WIDTH = 290;

let curW = 0;
let curH = 0;
let anim: {
  raf: number;
  last: number;
  tw: number;
  th: number;
  vw: number;
  vh: number;
  spring: Spring;
} | null = null;
let lastRect = { x: -1, w: -1, h: -1 };

function collapsedSize() {
  const h = geometry.value.hasNotch ? geometry.value.height : 36;
  let w = 168;
  if (leadRef.value && chipRef.value) {
    // lead content + status chip + head gap + pill horizontal padding/border
    w = leadRef.value.scrollWidth + chipRef.value.offsetWidth + 8 + 22 + 2;
  }
  if (geometry.value.hasNotch) {
    w = Math.max(w, geometry.value.width + 4);
  }
  return { w: Math.ceil(w), h };
}

function expandedSize() {
  const el = pillRef.value;
  if (!el) return { w: EXPANDED_WIDTH, h: 300 };
  // scrollHeight is the full content height even while the pill is
  // clipped mid-animation (overflow: hidden).
  return { w: EXPANDED_WIDTH, h: Math.ceil(el.scrollHeight) };
}

function applySize() {
  const el = pillRef.value;
  if (!el) return;
  el.style.width = `${curW}px`;
  el.style.height = `${curH}px`;
  // Keep the Rust click-through guard in sync with the visible capsule.
  const x = Math.round((window.innerWidth - curW) / 2);
  const w = Math.round(curW);
  const h = Math.round(curH);
  if (
    Math.abs(x - lastRect.x) >= 1 ||
    Math.abs(w - lastRect.w) >= 1 ||
    Math.abs(h - lastRect.h) >= 1
  ) {
    lastRect = { x, w, h };
    void setPillRect(x, 0, w, h).catch(() => {});
  }
}

function animateTo(tw: number, th: number, spring: Spring) {
  if (!pillRef.value) return;
  if (anim) {
    anim.tw = tw;
    anim.th = th;
    anim.spring = spring;
    return;
  }
  const run = {
    raf: 0,
    last: performance.now(),
    tw,
    th,
    vw: 0,
    vh: 0,
    spring,
  };
  anim = run;
  const tick = (now: number) => {
    if (anim !== run) return;
    const dt = Math.min((now - run.last) / 1000, 0.05);
    run.last = now;
    const steps = 4;
    const step = dt / steps;
    for (let i = 0; i < steps; i++) {
      run.vw += (-run.spring.k * (curW - run.tw) - run.spring.c * run.vw) * step;
      curW += run.vw * step;
      run.vh += (-run.spring.k * (curH - run.th) - run.spring.c * run.vh) * step;
      curH += run.vh * step;
    }
    applySize();
    const settled =
      Math.abs(curW - run.tw) < 0.5 &&
      Math.abs(run.vw) < 5 &&
      Math.abs(curH - run.th) < 0.5 &&
      Math.abs(run.vh) < 5;
    if (settled) {
      curW = run.tw;
      curH = run.th;
      applySize();
      anim = null;
      return;
    }
    run.raf = requestAnimationFrame(tick);
  };
  run.raf = requestAnimationFrame(tick);
}

function retarget(mode: "toggle" | "content") {
  if (!pillRef.value) return;
  if (isExpanded.value) {
    const t = expandedSize();
    animateTo(t.w, t.h, mode === "toggle" ? SPRING_OPEN : SPRING_INTERACTIVE);
  } else {
    const t = collapsedSize();
    animateTo(t.w, t.h, mode === "toggle" ? SPRING_CLOSE : SPRING_INTERACTIVE);
  }
}

async function load() {
  try {
    state.value = await getState();
  } catch {
    // keep last
  }
}

// Hover follows boring.notch's timing: only expand after the cursor has
// rested 300ms (intent), collapse 100ms after it leaves (debounce).
function handleMouseEnter() {
  if (leaveTimer) {
    clearTimeout(leaveTimer);
    leaveTimer = undefined;
  }
  if (hoverTimer) clearTimeout(hoverTimer);
  hoverTimer = setTimeout(() => {
    hoverIntent.value = true;
  }, 300);
}

// Collapse intent, shared by the DOM mouseleave and the Rust
// click-through guard's pill-cursor-out event: the DOM event alone
// is not reliable here (see hud.rs — it can be swallowed when the
// window flips to ignoring cursor events), so both paths schedule
// the same 100ms-debounced collapse.
function scheduleCollapse() {
  if (hoverTimer) {
    clearTimeout(hoverTimer);
    hoverTimer = undefined;
  }
  if (leaveTimer) clearTimeout(leaveTimer);
  leaveTimer = setTimeout(() => {
    hoverIntent.value = false;
  }, 100);
}

function handleMouseLeave() {
  scheduleCollapse();
}

onMounted(async () => {
  document.documentElement.classList.add("panel-mode");
  document.body.classList.add("panel-mode");
  try {
    geometry.value = await getNotchGeometry();
  } catch {
    // keep the fallback geometry
  }
  await load();
  unlistenState = await listen<AppStateView>("state-updated", (e) => {
    state.value = e.payload;
  });
  unlistenPeek = await listen<string>("alert-peek", () => {
    peeking.value = true;
    if (peekTimer) clearTimeout(peekTimer);
    peekTimer = setTimeout(() => {
      peeking.value = false;
    }, 4000);
  });
  unlistenCursorOut = await listen("pill-cursor-out", () => {
    scheduleCollapse();
  });
  await nextTick();
  if (pillRef.value) {
    unbindDrag = bindWindowDrag(pillRef.value);
    pillRef.value.style.setProperty(
      "--headh",
      `${geometry.value.hasNotch ? geometry.value.height : 36}px`,
    );
    const t = collapsedSize();
    curW = t.w;
    curH = t.h;
    applySize();
  }
});

watch(isExpanded, async () => {
  await nextTick();
  retarget("toggle");
});

watch(
  [
    () => m.value.rows.length,
    () => m.value.hero,
    () => m.value.heroBalance,
    () => m.value.heroAccounts,
    () => m.value.okAccounts,
    () => m.value.totalAccounts,
    () => keys.value.rows.length,
    () => keys.value.notes.length,
    () => keys.value.more,
    () => keys.value.rows.map((row) => `${row.group}:${row.error ?? ""}`).join("|"),
    () => pools.value.rows.map((row) => `${row.available}/${row.rateLimited}/${row.error}`).join("|"),
    deltaText,
    () => m.value.rows.map((row) => row.meta ?? "").join("|"),
    updatedText,
  ],
  async () => {
    await nextTick();
    retarget("content");
  },
);

onUnmounted(() => {
  document.documentElement.classList.remove("panel-mode");
  document.body.classList.remove("panel-mode");
  unlistenState?.();
  unlistenPeek?.();
  unlistenCursorOut?.();
  unbindDrag?.();
  if (hoverTimer) clearTimeout(hoverTimer);
  if (leaveTimer) clearTimeout(leaveTimer);
  if (peekTimer) clearTimeout(peekTimer);
  if (anim) cancelAnimationFrame(anim.raf);
  anim = null;
});
</script>

<template>
  <div class="island-root">
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
        <div ref="leadRef" class="lead">
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
        <span ref="chipRef" class="status-chip">{{ statusText }}</span>
      </div>

      <!-- Expanded Body Content -->
      <div v-if="isExpanded" class="body">
        <p class="detail" :title="m.detail">{{ m.detail }}</p>
        <p v-if="deltaText" class="delta" :title="deltaText">{{ deltaText }}</p>

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

        <div v-if="pools.visible" class="keys">
          <div class="keys-title">号池 · 正常/限流/异常</div>
          <template v-for="row in pools.rows" :key="row.id">
            <div v-if="row.site" class="key-site">{{ row.site }}</div>
            <div class="key-row">
              <span class="key-name" :title="row.name">{{ row.name }}</span>
              <span class="pool-nums" title="正常 / 限流 / 异常">
                <em class="ok">{{ row.available }}</em>
                <em class="sep">/</em>
                <em class="warn">{{ row.rateLimited }}</em>
                <em class="sep">/</em>
                <em class="bad">{{ row.error }}</em>
              </span>
            </div>
          </template>
        </div>

        <div v-if="keys.visible" class="keys">
          <div class="keys-title">密钥分组</div>
          <p v-for="note in keys.notes" :key="note" class="key-note">{{ note }}</p>
          <template v-for="row in keys.rows" :key="row.id">
            <div v-if="row.site" class="key-site">{{ row.site }}</div>
            <div class="key-row">
              <span class="key-name" :title="row.name">{{ row.name }}</span>
              <span class="key-group" :title="row.group">{{ row.group }}</span>
            </div>
            <p v-if="row.error" class="key-err">{{ row.error }}</p>
          </template>
          <p v-if="keys.more" class="key-note">其余 {{ keys.more }} 个</p>
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
              <span v-if="row.meta" class="site-meta" :title="row.meta">{{ row.meta }}</span>
            </div>
            <span class="site-val">{{ row.value }}</span>
          </div>
        </div>

        <div class="foot">
          <span>{{ updatedText || m.footerLeft }}</span>
          <span>{{ pinned ? '已固定 · ' : '' }}{{ m.footerRight }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.island-root {
  position: fixed;
  inset: 0;
  display: flex;
  justify-content: center;
  align-items: flex-start;
}

.pill {
  box-sizing: border-box;
  padding: 0 11px;
  margin: 0;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.22), rgba(255, 255, 255, 0.12)),
    rgba(20, 24, 35, 0.45);
  backdrop-filter: blur(28px) saturate(200%);
  -webkit-backdrop-filter: blur(28px) saturate(200%);
  border: 1px solid rgba(255, 255, 255, 0.35);
  border-top: none;
  /* Top-attached island corners (boring.notch): small on top, large below. */
  border-radius: 6px 6px 14px 14px;
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
    border-radius 0.32s cubic-bezier(0.16, 1, 0.3, 1),
    background 0.28s cubic-bezier(0.16, 1, 0.3, 1),
    box-shadow 0.28s cubic-bezier(0.16, 1, 0.3, 1);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
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
}

.pill.bad {
  background:
    linear-gradient(180deg, rgba(255, 69, 58, 0.28), rgba(255, 255, 255, 0.12)),
    rgba(35, 15, 18, 0.5);
  border-color: rgba(255, 69, 58, 0.45);
}

.pill.muted {
  background:
    linear-gradient(180deg, rgba(142, 142, 147, 0.2), rgba(255, 255, 255, 0.08)),
    rgba(25, 25, 30, 0.4);
}

/* Expanded State: size itself is driven by the JS spring. */
.pill.expanded {
  border-radius: 19px 19px 24px 24px;
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
  height: var(--headh, 36px);
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
  /* Never shrink: collapsedSize() reads this element's scrollWidth as
     the capsule's target width. If the lead (or its text spans) could
     shrink and ellipsize, the measurement would return the clipped
     width and the capsule would stay too narrow forever. */
  flex-shrink: 0;
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
  flex-shrink: 0;
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
  /* No entrance animation: the body used to fade in via a CSS
     animation starting at opacity 0 (0.12s delay, fill both). In
     this never-activated background panel, WebKit can leave CSS
     animations paused at their first frame, which left the expanded
     body permanently invisible even though it was laid out and the
     capsule sized for it. The JS spring already provides the motion;
     the body is simply visible. */
  margin-top: 7px;
  display: flex;
  flex-direction: column;
  gap: 7px;
  max-height: 520px;
  overflow-y: auto;
  /* Never shrink: on newer WebKit (seen on macOS 27.2.1) a flex item
     whose overflow is not visible gets an automatic minimum size of
     0, so this body collapsed to height 0 inside the pill and
     expandedSize() (pill.scrollHeight) under-measured the expanded
     height (54px instead of head + full body). Keeping the body at
     its content height makes the measurement truthful on every
     WebKit version; max-height + overflow-y still cap and scroll
     very long content. Same pattern as .lead/.hero above. */
  flex-shrink: 0;
}

.detail,
.delta {
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

.delta {
  color: #ffd60a;
  font-variant-numeric: tabular-nums;
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

.keys {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.keys-title,
.key-site,
.key-note {
  font-size: 10px;
  font-weight: 550;
  letter-spacing: 0.02em;
  color: rgba(255, 255, 255, 0.55);
}

.key-note,
.key-err {
  margin: 0;
  line-height: 1.3;
}

.key-err {
  font-size: 10px;
  color: #ff453a;
}

.key-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  font-size: 11px;
  padding: 4px 8px;
  border-radius: 7px;
  background: rgba(0, 0, 0, 0.16);
  border: 1px solid rgba(255, 255, 255, 0.06);
}

.key-name,
.key-group {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.key-name {
  color: rgba(255, 255, 255, 0.88);
  font-weight: 500;
  min-width: 0;
}

.key-group {
  color: #64d2ff;
  font-weight: 650;
  flex-shrink: 0;
  max-width: 58%;
}

.pool-nums,
.pool-nums em {
  flex-shrink: 0;
  font-size: 11px;
  font-weight: 650;
  font-style: normal;
  font-variant-numeric: tabular-nums;
}

.pool-nums .bad {
  color: #ff453a;
}

.pool-nums .warn {
  color: #ff9f0a;
}

.pool-nums .ok {
  color: #30d158;
}

.pool-nums .sep {
  color: rgba(255, 255, 255, 0.35);
  margin: 0 1px;
}

.sites {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 168px;
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

.site-usage,
.site-meta {
  font-size: 10px;
  font-weight: 500;
  color: rgba(255, 255, 255, 0.5);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.site-meta {
  color: rgba(100, 210, 255, 0.88);
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
