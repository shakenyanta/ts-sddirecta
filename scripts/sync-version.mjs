import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

function validateVersion(version) {
  const match = typeof version === 'string' && version.match(
    /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/,
  );
  if (!match || match.slice(1, 4).some(value => BigInt(value) > 18446744073709551615n)
    || match[4]?.split('.').some(value => /^\d+$/.test(value) && value.length > 1 && value.startsWith('0'))) {
    throw new Error('package.json の version は 0.1.1 のような有効なSemVerで指定してください（先頭のvは不要です）。');
  }
}

export async function syncVersion(root = projectRoot) {
  const loadJson = async name => {
    const source = await readFile(join(root, name), 'utf8');
    return { source, value: JSON.parse(source.replace(/^\uFEFF/, '')) };
  };
  const { value: pkg } = await loadJson('package.json');
  const version = pkg.version;
  validateVersion(version);
  const updates = [];
  const updateJson = (name, document, change) => {
    if (!change(document.value)) return;
    const eol = document.source.includes('\r\n') ? '\r\n' : '\n';
    const bom = document.source.startsWith('\uFEFF') ? '\uFEFF' : '';
    const end = document.source.endsWith('\n') ? eol : '';
    updates.push([name, bom + JSON.stringify(document.value, null, 2).replace(/\n/g, eol) + end]);
  };
  updateJson('src-tauri/tauri.conf.json', await loadJson('src-tauri/tauri.conf.json'), config => {
    if (config.version === version) return false;
    config.version = version;
    return true;
  });
  const cargo = await readFile(join(root, 'src-tauri/Cargo.toml'), 'utf8');
  let found = false;
  const syncedCargo = cargo.replace(
    /(^\[package\][^\r\n]*\r?\n)([\s\S]*?)(?=^\[|$(?![\s\S]))/m,
    (_, header, section) => header + section.replace(
      /^([ \t]*version[ \t]*=[ \t]*)(["'])([^"'\r\n]*)\2/m,
      (_, prefix, quote) => { found = true; return `${prefix}${quote}${version}${quote}`; },
    ),
  );
  if (!found) throw new Error('Cargo.toml の [package] に version がありません。同期を中止しました。');
  if (syncedCargo !== cargo) updates.push(['src-tauri/Cargo.toml', syncedCargo]);
  let lock;
  try { lock = await loadJson('package-lock.json'); }
  catch (error) { if (error.code !== 'ENOENT') throw error; }
  if (lock) updateJson('package-lock.json', lock, value => {
    const changed = value.version !== version || (value.packages?.[''] && value.packages[''].version !== version);
    value.version = version;
    if (value.packages?.['']) value.packages[''].version = version;
    return changed;
  });
  // Parse and validate every input before modifying any file.
  for (const [name, contents] of updates) await writeFile(join(root, name), contents, 'utf8');
  return { version, updatedFiles: updates.map(([name]) => name) };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const result = await syncVersion();
    console.log(`バージョン同期: ${result.version}（${result.updatedFiles.length ? result.updatedFiles.join(', ') : '変更なし'}）`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
