import re

filepath = 'src/lib/services/ai-advisor-service.ts'
with open(filepath, 'r', encoding='utf-8') as f:
    content = f.read()

# Pattern to remove the mock blocks
mock_pattern = re.compile(
    r'\s*// TODO\(demo\):.*?if\s*\(baseUrl\s*===\s*\'https://api\.mangodisk\.app/v1\'\s*&&\s*!apiKey\)\s*\{[^\}]+\}\s*',
    re.DOTALL
)

# Wait, in the other methods, it might not have the "// TODO(demo)" comment!
# Let's just match the if block.
mock_pattern_generic = re.compile(
    r'\s*(?:// TODO\(demo\)[^\n]*\n)*\s*// If using the default[^\n]*\n\s*if\s*\(baseUrl\s*===\s*\'https://api\.mangodisk\.app/v1\'\s*&&\s*!apiKey\)\s*\{.*?\n\s*\}\s*\n',
    re.DOTALL
)

# Actually, let's just write a more robust replacement using string replace
# Method 1 mock
content = re.sub(r'(\s*// TODO\(demo\):.*?if \(baseUrl === \'https://api\.mangodisk\.app/v1\' && !apiKey\) \{.*?\n      \}\n)', '\n', content, flags=re.DOTALL)

# In case the comment is missing:
content = re.sub(r'(\s*if \(baseUrl === \'https://api\.mangodisk\.app/v1\' && !apiKey\) \{.*?\n    \}\n)', '\n', content, flags=re.DOTALL)

with open(filepath, 'w', encoding='utf-8') as f:
    f.write(content)

print("Mocks removed")
