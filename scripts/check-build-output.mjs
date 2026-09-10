import { readdir, stat } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const assetDirectory = fileURLToPath(new URL('../dist/assets/', import.meta.url));
const expectedLocaleIds = new Set(['en-us', 'ja-jp', 'ko-kr', 'zh-cn', 'zh-tw']);
const maximumApplicationChunkBytes = 300 * 1024;
const maximumLocaleChunkBytes = 280 * 1024;

function fail(message) {
  console.error(`[build-output] ${message}`);
  process.exitCode = 1;
}

let assetNames;
try {
  assetNames = await readdir(assetDirectory);
} catch (error) {
  fail(`cannot read ${assetDirectory}: ${error instanceof Error ? error.message : String(error)}`);
  process.exit();
}

const javaScriptAssets = assetNames.filter(name => name.endsWith('.js'));
const localeAssets = javaScriptAssets.filter(name => name.startsWith('locale-'));

for (const localeId of expectedLocaleIds) {
  const matchingAssets = localeAssets.filter(name => name.startsWith(`locale-${localeId}-`));
  if (matchingAssets.length !== 1) {
    fail(`expected one ${localeId} chunk, found ${matchingAssets.length}`);
  }
}

const unexpectedLocaleAssets = localeAssets.filter(
  name => ![...expectedLocaleIds].some(localeId => name.startsWith(`locale-${localeId}-`))
);
if (unexpectedLocaleAssets.length > 0) {
  fail(`unexpected locale chunks: ${unexpectedLocaleAssets.join(', ')}`);
}

for (const assetName of javaScriptAssets) {
  const assetSize = (await stat(`${assetDirectory}/${assetName}`)).size;
  const isLocaleAsset = assetName.startsWith('locale-');
  const maximumBytes = isLocaleAsset ? maximumLocaleChunkBytes : maximumApplicationChunkBytes;
  if (assetSize > maximumBytes) {
    fail(
      `${assetName} is ${assetSize} bytes, exceeding the ${maximumBytes}-byte ` +
        `${isLocaleAsset ? 'locale' : 'application'} chunk limit`
    );
  }
}

if (process.exitCode) {
  process.exit();
}

console.log(
  `[build-output] verified ${javaScriptAssets.length} JavaScript chunks and ` +
    `${localeAssets.length} project-owned locale chunks`
);
