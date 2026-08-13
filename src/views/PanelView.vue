<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import { LogicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import type { AppStateView } from "../types";
import { invoke } from "@tauri-apps/api/core";
import { getState } from "../api";
import { mergeMetrics, statusLabel, thresholdsFromSettings } from "../metrics";
import { bindWindowDrag } from "../useWindowDrag";

const state = ref<AppStateView | null>(null);
const pillRef = ref<HTMLElement | null>(null);
const pinned = ref(false);
let unlistenState: (() => void) | undefined;
let unbindDrag: (() => void) | undefined;
let ro: ResizeObserver | undefined;

const m = computed(() =>
  mergeMetrics(
    state.value?.snapshots || [],
    thresholdsFromSettings(state.value?.settings),
  ),
);

const fill = computed(() => {
  if (m.value.tone === "bad") return "#ff3b30";
  if (m.value.tone === "warn") return "#ff9f0a";
  return "#34c759";
});

const statusText = computed(() => statusLabel(m.value.tone));

const showRows = computed(() => m.value.rows.length > 1);

const open = computed(() => pinned.value);

async function syncChrome() {
  const el = pillRef.value;
  if (!el) return;
  const r = el.getBoundingClientRect();
  const radius = open.value || el.matches(":hover") ? 22 : 19;
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

watch([pinned, () => m.value.rows.length], async () => {
  await nextTick();
  await syncChrome();
});

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
    :class="[m.tone, { open: pinned, rows: showRows }]"
    @mouseenter="void syncChrome()"
    @mouseleave="void syncChrome()"
    @dblclick.stop="pinned = !pinned"
  >
    <div class="head">
      <div class="lead">
        <span class="dot" />
        <span class="hero">{{ m.hero }}</span>
      </div>
      <span class="status-chip">{{ statusText }}</span>
    </div>

    <div class="body">
      <p class="detail">{{ m.detail }}</p>

      <div class="bar">
        <i :style="{ width: m.remainPct + '%', background: fill }" />
      </div>

      <div v-if="showRows" class="sites">
        <div v-for="row in m.rows" :key="row.name" class="site" :class="row.tone">
          <span class="site-name">{{ row.name }}</span>
          <span class="site-val">{{ row.value }}</span>
        </div>
      </div>

      <div class="foot">
        <span>{{ m.footerLeft }}</span>
        <span>{{ m.footerRight }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.pill {
  box-sizing: border-box;
  width: 156px;
  height: 38px;
  padding: 0 14px;
  margin: 0;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.42), rgba(255, 255, 255, 0.2)),
    rgba(255, 255, 255, 0.22);
  backdrop-filter: blur(22px) saturate(170%);
  -webkit-backdrop-filter: blur(22px) saturate(170%);
  border: 1px solid rgba(255, 255, 255, 0.5);
  border-radius: 19px;
  box-shadow: none;
  color: #1d1d1f;
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
    width 0.42s cubic-bezier(0.16, 1, 0.3, 1),
    height 0.42s cubic-bezier(0.16, 1, 0.3, 1),
    border-radius 0.42s cubic-bezier(0.16, 1, 0.3, 1),
    padding 0.42s cubic-bezier(0.16, 1, 0.3, 1),
    background 0.42s cubic-bezier(0.16, 1, 0.3, 1);
}

.pill:active {
  cursor: grabbing;
}

.pill.warn {
  background:
    linear-gradient(180deg, rgba(255, 214, 10, 0.28), rgba(255, 255, 255, 0.18)),
    rgba(255, 255, 255, 0.22);
  border-color: rgba(255, 159, 10, 0.45);
}

.pill.bad {
  background:
    linear-gradient(180deg, rgba(255, 59, 48, 0.26), rgba(255, 255, 255, 0.16)),
    rgba(255, 255, 255, 0.22);
  border-color: rgba(255, 59, 48, 0.42);
}

.pill:hover,
.pill.open {
  width: 268px;
  height: 132px;
  border-radius: 22px;
  padding: 6px 18px 14px;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.5), rgba(255, 255, 255, 0.3)),
    rgba(255, 255, 255, 0.28);
}

.pill.warn:hover,
.pill.warn.open {
  background:
    linear-gradient(180deg, rgba(255, 214, 10, 0.38), rgba(255, 255, 255, 0.26)),
    rgba(255, 255, 255, 0.28);
}

.pill.bad:hover,
.pill.bad.open {
  background:
    linear-gradient(180deg, rgba(255, 59, 48, 0.34), rgba(255, 255, 255, 0.22)),
    rgba(255, 255, 255, 0.26);
}

.pill.rows:hover,
.pill.rows.open {
  height: 168px;
}

.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 38px;
  gap: 10px;
}

.lead {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #34c759;
  box-shadow: 0 0 7px rgba(52, 199, 89, 0.55);
  flex-shrink: 0;
}

.pill.warn .dot {
  background: #ff9f0a;
  box-shadow: 0 0 7px rgba(255, 159, 10, 0.5);
}

.pill.bad .dot {
  background: #ff3b30;
  box-shadow: 0 0 7px rgba(255, 59, 48, 0.5);
}

.pill.muted .dot {
  background: #8e8e93;
  box-shadow: none;
}

.hero {
  font-size: 15px;
  font-weight: 590;
  letter-spacing: -0.35px;
  font-variant-numeric: tabular-nums;
  font-feature-settings: "tnum" 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.status-chip {
  font-size: 11px;
  font-weight: 620;
  letter-spacing: 0.02em;
  color: #248a3d;
  flex-shrink: 0;
  white-space: nowrap;
}

.pill.warn .status-chip,
.pill.warn .hero {
  color: #c93400;
}

.pill.bad .status-chip,
.pill.bad .hero {
  color: #d70015;
}

.pill.muted .status-chip {
  color: rgba(29, 29, 31, 0.42);
}

.body {
  opacity: 0;
  transform: translateY(8px);
  transition:
    opacity 0.28s ease 0.06s,
    transform 0.28s ease 0.06s;
  pointer-events: none;
}

.pill:hover .body,
.pill.open .body {
  opacity: 1;
  transform: translateY(0);
}

.detail {
  margin: 2px 0 0;
  font-size: 12px;
  font-weight: 500;
  letter-spacing: -0.15px;
  line-height: 1.35;
  color: rgba(29, 29, 31, 0.68);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.bar {
  height: 5px;
  margin: 10px 0 8px;
  border-radius: 99px;
  background: rgba(0, 0, 0, 0.07);
  overflow: hidden;
}

.bar > i {
  display: block;
  height: 100%;
  border-radius: 99px;
  transition: width 0.5s cubic-bezier(0.16, 1, 0.3, 1);
}

.sites {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-bottom: 8px;
}

.site {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  font-size: 12px;
  letter-spacing: -0.1px;
  padding: 4px 8px;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.28);
}

.site-name {
  color: rgba(29, 29, 31, 0.48);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.site-val {
  font-variant-numeric: tabular-nums;
  font-weight: 560;
  color: rgba(29, 29, 31, 0.78);
  flex-shrink: 0;
}

.site.warn .site-val {
  color: #c93400;
}

.site.bad .site-val {
  color: #d70015;
}

.foot {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  font-size: 11px;
  font-weight: 400;
  letter-spacing: -0.08px;
  color: rgba(29, 29, 31, 0.46);
}

.foot span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
