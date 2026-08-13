import { createApp } from "vue";
import App from "./App.vue";
import "./styles.css";

const settings = new URLSearchParams(location.search).get("view") === "settings";
if (!settings) {
  document.documentElement.classList.add("panel-mode");
  document.body.classList.add("panel-mode");
}

createApp(App).mount("#app");
