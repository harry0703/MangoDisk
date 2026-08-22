import json

keys_zh_cn = {
  "aiSection": "AI 设置",
  "aiConfigTitle": "AI 建议配置",
  "aiConfigDescription": "配置自定义 API Key、Base URL 及模型",
  "aiConfigAction": "配置",
  "aiDialogTitle": "AI 建议配置",
  "aiDialogDescription": "设置自定义模型接口与凭证，留空则使用内置默认配置",
  "aiBaseUrlTitle": "API Base URL",
  "aiBaseUrlDescription": "自定义 AI 服务接口地址",
  "aiBaseUrlPlaceholder": "https://api.openai.com/v1",
  "aiApiKeyTitle": "API Key",
  "aiApiKeyDescription": "自定义 AI 服务密钥 (仅存储于本地，直连不经过中转)",
  "aiApiKeyPlaceholder": "sk-xxx",
  "aiModelTitle": "模型名称",
  "aiModelDescription": "自定义 AI 模型名称",
  "aiModelPlaceholder": "gpt-4o",
  "aiSaveAction": "完成"
}

keys_en_us = {
  "aiSection": "AI Settings",
  "aiConfigTitle": "AI Advisor Configuration",
  "aiConfigDescription": "Configure custom API Key, Base URL, and Model",
  "aiConfigAction": "Configure",
  "aiDialogTitle": "AI Advisor Configuration",
  "aiDialogDescription": "Configure custom model endpoints and credentials. Leave blank to use defaults",
  "aiBaseUrlTitle": "API Base URL",
  "aiBaseUrlDescription": "Custom AI service endpoint",
  "aiBaseUrlPlaceholder": "https://api.openai.com/v1",
  "aiApiKeyTitle": "API Key",
  "aiApiKeyDescription": "Custom API Key (stored locally, direct connection)",
  "aiApiKeyPlaceholder": "sk-xxx",
  "aiModelTitle": "Model Name",
  "aiModelDescription": "Custom AI model name",
  "aiModelPlaceholder": "gpt-4o",
  "aiSaveAction": "Done"
}

keys_zh_tw = {
  "aiSection": "AI 設定",
  "aiConfigTitle": "AI 建議設定",
  "aiConfigDescription": "設定自訂 API Key、Base URL 及模型",
  "aiConfigAction": "設定",
  "aiDialogTitle": "AI 建議設定",
  "aiDialogDescription": "設定自訂模型介面與金鑰，留空則使用內建預設設定",
  "aiBaseUrlTitle": "API Base URL",
  "aiBaseUrlDescription": "自訂 AI 服務介面位址",
  "aiBaseUrlPlaceholder": "https://api.openai.com/v1",
  "aiApiKeyTitle": "API Key",
  "aiApiKeyDescription": "自訂 AI 服務金鑰 (僅儲存於本地，直连不经过中轉)",
  "aiApiKeyPlaceholder": "sk-xxx",
  "aiModelTitle": "模型名稱",
  "aiModelDescription": "自訂 AI 模型名稱",
  "aiModelPlaceholder": "gpt-4o",
  "aiSaveAction": "完成"
}

keys_ja_jp = {
  "aiSection": "AI 設定",
  "aiConfigTitle": "AI アドバイザー設定",
  "aiConfigDescription": "カスタム API Key、Base URL、モデルの設定",
  "aiConfigAction": "設定",
  "aiDialogTitle": "AI アドバイザー設定",
  "aiDialogDescription": "カスタムモデルのエンドポイントとキーを設定します。空白の場合はデフォルト設定を使用します",
  "aiBaseUrlTitle": "API Base URL",
  "aiBaseUrlDescription": "カスタム AI サービスのエンドポイント",
  "aiBaseUrlPlaceholder": "https://api.openai.com/v1",
  "aiApiKeyTitle": "API Key",
  "aiApiKeyDescription": "カスタム API キー (ローカルにのみ保存され、直接接続)",
  "aiApiKeyPlaceholder": "sk-xxx",
  "aiModelTitle": "モデル名",
  "aiModelDescription": "カスタム AI モデル名",
  "aiModelPlaceholder": "gpt-4o",
  "aiSaveAction": "完了"
}

locales = {
    'src/locales/zh-CN.json': keys_zh_cn,
    'src/locales/en-US.json': keys_en_us,
    'src/locales/zh-TW.json': keys_zh_tw,
    'src/locales/ja-JP.json': keys_ja_jp
}

for filepath, new_keys in locales.items():
    with open(filepath, 'r', encoding='utf-8') as f:
        data = json.load(f)
    
    settings = data['settings']
    for k, v in new_keys.items():
        settings[k] = v
        
    with open(filepath, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
        f.write('\n')
