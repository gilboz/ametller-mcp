import fs from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { createHash } from 'node:crypto';
import * as prettier from '../sources/tooling/prettier/standalone.mjs';
import * as babel from '../sources/tooling/prettier/plugins/babel.mjs';
import * as estree from '../sources/tooling/prettier/plugins/estree.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const raw = path.join(root, 'sources/web/raw');
const pretty = path.join(root, 'sources/web/pretty');
await fs.mkdir(pretty, { recursive: true });
const manifest = JSON.parse(await fs.readFile(path.join(root, 'sources/web/manifest.json'), 'utf8'));
const files = manifest.assets.filter(a => a.status === 200 && a.path?.endsWith('.js'));
const output = [];
for (const asset of files) {
  const name = path.basename(asset.path);
  const code = await fs.readFile(path.join(raw, name), 'utf8');
  const formatted = await prettier.format(code, { parser: 'babel', plugins: [babel, estree], embeddedLanguageFormatting: 'off' });
  await fs.writeFile(path.join(pretty, name), formatted);
  output.push({ path: 'pretty/' + name, original_sha256: asset.sha256, sha256: createHash('sha256').update(formatted).digest('hex'), lines: formatted.split('\n').length - 1 });
}
await fs.writeFile(path.join(root, 'sources/web/pretty-manifest.json'), JSON.stringify({ formatter: prettier.version, files: output }, null, 2) + '\n');
console.log(`Beautified ${output.length} JavaScript assets; storefront code was parsed, never executed.`);
