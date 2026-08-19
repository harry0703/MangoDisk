with open('src/pages/settings/index.vue', 'r', encoding='utf-8') as f:
    content = f.read()

# 1. Update imports
old_import = "import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';\nimport { Input } from '@/components/ui/input';"
new_import = """import MdDialogContent from '@/components/custom/md-dialog-content.vue';
import { Button } from '@/components/ui/button';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';"""

if old_import in content:
    content = content.replace(old_import, new_import)
else:
    # fallback in case exact string differs
    content = content.replace("import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';", new_import)

# 2. Add aiDialogOpen state
if "const aiDialogOpen = ref(false);" not in content:
    content = content.replace("const form = reactive<AppSettings>({ ...props.settings });", "const form = reactive<AppSettings>({ ...props.settings });\nconst aiDialogOpen = ref(false);")

# 3. Replace AI section
old_section = """    <section class="settings-section">
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
    </section>"""

new_section = """    <section class="settings-section">
      <h2>{{ t('settings.aiSection') }}</h2>
      <Card class="settings-list">
        <button
          class="setting-row action-row grid-cols-[40px_minmax(0,1fr)] @2xl/settings:grid-cols-[42px_minmax(0,1fr)_auto]"
          type="button"
          @click="aiDialogOpen = true"
        >
          <span class="section-icon"><MdIcon :name="ICON_NAMES.aiTools" /></span>
          <span class="setting-copy">
            <strong>{{ t('settings.aiConfigTitle') }}</strong>
            <small class="whitespace-normal @2xl/settings:whitespace-nowrap">{{
              t('settings.aiConfigDescription')
            }}</small>
          </span>
          <span class="row-action col-start-2 @2xl/settings:col-auto">
            {{ t('settings.aiConfigAction') }}
            <MdIcon :name="ICON_NAMES.chevronRight" :size="16" />
          </span>
        </button>
      </Card>
    </section>"""

content = content.replace(old_section, new_section)

# 4. Add Dialog before closing </MdPageShell>
dialog_template = """
    <Dialog v-model:open="aiDialogOpen">
      <MdDialogContent class="max-w-md">
        <DialogHeader>
          <DialogTitle>{{ t('settings.aiDialogTitle') }}</DialogTitle>
          <DialogDescription>{{ t('settings.aiDialogDescription') }}</DialogDescription>
        </DialogHeader>

        <div class="space-y-4 py-3">
          <div class="space-y-1.5">
            <label class="text-sm font-medium text-foreground block">
              {{ t('settings.aiBaseUrlTitle') }}
            </label>
            <p class="text-xs text-muted-foreground">{{ t('settings.aiBaseUrlDescription') }}</p>
            <Input
              v-model="form.aiApiBaseUrl"
              :placeholder="t('settings.aiBaseUrlPlaceholder')"
              @blur="save"
            />
          </div>

          <div class="space-y-1.5">
            <label class="text-sm font-medium text-foreground block">
              {{ t('settings.aiApiKeyTitle') }}
            </label>
            <p class="text-xs text-muted-foreground">{{ t('settings.aiApiKeyDescription') }}</p>
            <Input
              type="password"
              v-model="form.aiApiKey"
              :placeholder="t('settings.aiApiKeyPlaceholder')"
              @blur="save"
            />
          </div>

          <div class="space-y-1.5">
            <label class="text-sm font-medium text-foreground block">
              {{ t('settings.aiModelTitle') }}
            </label>
            <p class="text-xs text-muted-foreground">{{ t('settings.aiModelDescription') }}</p>
            <Input
              v-model="form.aiModel"
              :placeholder="t('settings.aiModelPlaceholder')"
              @blur="save"
            />
          </div>
        </div>

        <DialogFooter>
          <Button @click="aiDialogOpen = false; save()">{{ t('settings.aiSaveAction') }}</Button>
        </DialogFooter>
      </MdDialogContent>
    </Dialog>
  </MdPageShell>"""

content = content.replace("  </MdPageShell>", dialog_template)

with open('src/pages/settings/index.vue', 'w', encoding='utf-8') as f:
    f.write(content)
