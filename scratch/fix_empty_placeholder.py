import json
import re

# 1. Update locales: remove aiModelPlaceholder
locales = [
    'src/locales/zh-CN.json',
    'src/locales/zh-TW.json',
    'src/locales/en-US.json',
    'src/locales/ja-JP.json'
]

for filepath in locales:
    with open(filepath, 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    settings = data['settings']
    if 'aiModelPlaceholder' in settings:
        del settings['aiModelPlaceholder']
        
    with open(filepath, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
        f.write('\n')

# 2. Update settings UI: remove the placeholder attribute from aiModel input
with open('src/pages/settings/index.vue', 'r', encoding='utf-8') as f:
    content = f.read()

content = content.replace(
    'v-model="form.aiModel"\n              :placeholder="t(\'settings.aiModelPlaceholder\')"',
    'v-model="form.aiModel"'
)

with open('src/pages/settings/index.vue', 'w', encoding='utf-8') as f:
    f.write(content)
