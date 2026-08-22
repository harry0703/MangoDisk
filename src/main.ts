import { setupBrowserDevMock } from './lib/utils/dev-mock';
setupBrowserDevMock();

import { createPinia } from 'pinia';
import { createApp } from 'vue';

import App from './App.vue';
import './assets/main.css';
import 'vue-sonner/style.css';
import { i18n } from './i18n';
import { ApplicationWindowService } from './lib/services/application-window-service';
import { useAppStore } from './stores/app-store';

document.documentElement.dataset.skin = 'mangodisk';

const app = createApp(App);
const pinia = createPinia();
app.use(pinia);
app.use(i18n);

async function startApplication() {
  await useAppStore(pinia).loadSettings();
  app.mount('#app');
  await ApplicationWindowService.showAfterMount();
}

void startApplication();
