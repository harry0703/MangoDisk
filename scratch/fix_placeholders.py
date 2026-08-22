import json

locales = {
    'src/locales/zh-CN.json': {
        "aiBaseUrlPlaceholder": "例如：https://api.openai.com/v1",
        "aiModelPlaceholder": ""
    },
    'src/locales/zh-TW.json': {
        "aiBaseUrlPlaceholder": "例如：https://api.openai.com/v1",
        "aiModelPlaceholder": ""
    },
    'src/locales/en-US.json': {
        "aiBaseUrlPlaceholder": "e.g., https://api.openai.com/v1",
        "aiModelPlaceholder": ""
    },
    'src/locales/ja-JP.json': {
        "aiBaseUrlPlaceholder": "例: https://api.openai.com/v1",
        "aiModelPlaceholder": ""
    }
}

for filepath, updates in locales.items():
    with open(filepath, 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    settings = data['settings']
    for k, v in updates.items():
        settings[k] = v
        
    with open(filepath, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
        f.write('\n')
