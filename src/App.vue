<script setup lang="ts">
import { computed } from "vue";
import PanelView from "./views/PanelView.vue";
import SettingsView from "./views/SettingsView.vue";

const view = computed(() => {
  const q = new URLSearchParams(window.location.search);
  return q.get("view") === "settings" ? "settings" : "panel";
});
</script>

<template>
  <SettingsView v-if="view === 'settings'" />
  <PanelView v-else />
</template>
