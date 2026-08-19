import re

with open('src/pages/settings/index.vue', 'r', encoding='utf-8') as f:
    content = f.read()

# Add Input import
if "import { Input }" not in content:
    content = content.replace(
        "import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';",
        "import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';\nimport { Input } from '@/components/ui/input';"
    )

new_section = """
    <section class="settings-section">
      <h2>{{ t('settings.aiSection') }}</h2>
      <Card class="settings-list">
        <div class="setting-row grid-cols-[40px_minmax(0,1fr)] @2xl/settings:grid-cols-[42px_minmax(0,1fr)_auto]">
          <span class="section-icon"><MdIcon :name="ICON_NAMES.globe" /></span>
          <span class="setting-copy">
            <strong>{{ t('settings.aiBaseUrlTitle') }}</strong>
            <small class="whitespace-normal @2xl/settings:whitespace-nowrap">{{ t('settings.aiBaseUrlDescription') }}</small>
          </span>
          <Input 
            class="setting-input col-start-2 w-full @2xl/settings:col-auto @2xl/settings:w-64"
            v-model="form.aiApiBaseUrl"
            @blur="save"
            :placeholder="t('settings.aiBaseUrlPlaceholder')"
          />
        </div>
        <div class="setting-row grid-cols-[40px_minmax(0,1fr)] @2xl/settings:grid-cols-[42px_minmax(0,1fr)_auto]">
          <span class="section-icon"><MdIcon :name="ICON_NAMES.code" /></span>
          <span class="setting-copy">
            <strong>{{ t('settings.aiApiKeyTitle') }}</strong>
            <small class="whitespace-normal @2xl/settings:whitespace-nowrap">{{ t('settings.aiApiKeyDescription') }}</small>
          </span>
          <Input 
            class="setting-input col-start-2 w-full @2xl/settings:col-auto @2xl/settings:w-64"
            type="password"
            v-model="form.aiApiKey"
            @blur="save"
            :placeholder="t('settings.aiApiKeyPlaceholder')"
          />
        </div>
        <div class="setting-row grid-cols-[40px_minmax(0,1fr)] @2xl/settings:grid-cols-[42px_minmax(0,1fr)_auto]">
          <span class="section-icon"><MdIcon :name="ICON_NAMES.aiModel" /></span>
          <span class="setting-copy">
            <strong>{{ t('settings.aiModelTitle') }}</strong>
            <small class="whitespace-normal @2xl/settings:whitespace-nowrap">{{ t('settings.aiModelDescription') }}</small>
          </span>
          <Input 
            class="setting-input col-start-2 w-full @2xl/settings:col-auto @2xl/settings:w-64"
            v-model="form.aiModel"
            @blur="save"
            :placeholder="t('settings.aiModelPlaceholder')"
          />
        </div>
      </Card>
    </section>
"""

# Insert before Support Section
content = content.replace('    <section class="settings-section">\n      <h2>{{ t(\'settings.supportSection\') }}</h2>', new_section + '\n    <section class="settings-section">\n      <h2>{{ t(\'settings.supportSection\') }}</h2>')

with open('src/pages/settings/index.vue', 'w', encoding='utf-8') as f:
    f.write(content)
