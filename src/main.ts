import { createPinia } from 'pinia';
import { createApp } from 'vue';

import App from './App.vue';
import './assets/main.css';
import 'vue-sonner/style.css';
import { i18n } from './i18n';
import { useAppStore } from './stores/app-store';
import { useAiStore } from './stores/ai-store';
import { useAnalysisStore } from './stores/analysis-store';
import { useAnalysisExportPreferencesStore } from './stores/analysis-export-preferences-store';

const app = createApp(App);
const pinia = createPinia();
app.use(pinia);
app.use(i18n);

async function startApplication() {
  await Promise.all([
    useAppStore(pinia).loadSettings(),
    useAiStore(pinia).loadPreferences(),
    useAnalysisStore(pinia).initializeViewPreferences(),
    useAnalysisStore(pinia).initializeScanMode(),
    useAnalysisExportPreferencesStore(pinia).initialize(),
  ]);
  app.mount('#app');
}

void startApplication();
