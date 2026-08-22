import re

with open('src/pages/settings/index.vue', 'r', encoding='utf-8') as f:
    content = f.read()

# Replace the old dialog template with the new polished one
old_dialog_pattern = r'<Dialog v-model:open="aiDialogOpen">.*?</Dialog>'
new_dialog = """<Dialog v-model:open="aiDialogOpen">
      <MdDialogContent class="max-w-[480px] gap-0 p-0 overflow-hidden">
        <DialogHeader class="px-6 pt-6 pb-4 pr-12 bg-muted/30 border-b border-border/40">
          <DialogTitle class="text-base">{{ t('settings.aiDialogTitle') }}</DialogTitle>
          <DialogDescription class="text-sm mt-1">{{ t('settings.aiDialogDescription') }}</DialogDescription>
        </DialogHeader>

        <div class="px-6 py-6 space-y-5">
          <div class="space-y-1.5">
            <label class="text-[13px] font-semibold text-foreground tracking-tight block">
              {{ t('settings.aiBaseUrlTitle') }}
            </label>
            <p class="text-[12px] text-muted-foreground leading-snug">{{ t('settings.aiBaseUrlDescription') }}</p>
            <Input
              v-model="form.aiApiBaseUrl"
              :placeholder="t('settings.aiBaseUrlPlaceholder')"
              @blur="save"
              class="mt-2"
            />
          </div>

          <div class="space-y-1.5">
            <label class="text-[13px] font-semibold text-foreground tracking-tight block">
              {{ t('settings.aiApiKeyTitle') }}
            </label>
            <p class="text-[12px] text-muted-foreground leading-snug">{{ t('settings.aiApiKeyDescription') }}</p>
            <Input
              type="password"
              v-model="form.aiApiKey"
              :placeholder="t('settings.aiApiKeyPlaceholder')"
              @blur="save"
              class="mt-2"
            />
          </div>

          <div class="space-y-1.5">
            <label class="text-[13px] font-semibold text-foreground tracking-tight block">
              {{ t('settings.aiModelTitle') }}
            </label>
            <p class="text-[12px] text-muted-foreground leading-snug">{{ t('settings.aiModelDescription') }}</p>
            <Input
              v-model="form.aiModel"
              :placeholder="t('settings.aiModelPlaceholder')"
              @blur="save"
              class="mt-2"
            />
          </div>
        </div>

        <DialogFooter class="px-6 py-4 bg-muted/30 border-t border-border/40 sm:justify-end">
          <Button @click="aiDialogOpen = false; save()">{{ t('settings.aiSaveAction') }}</Button>
        </DialogFooter>
      </MdDialogContent>
    </Dialog>"""

content = re.sub(old_dialog_pattern, new_dialog, content, flags=re.DOTALL)

with open('src/pages/settings/index.vue', 'w', encoding='utf-8') as f:
    f.write(content)
