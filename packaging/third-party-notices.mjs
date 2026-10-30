// Writes THIRD_PARTY_NOTICES.md from the crates compiled into the Windows and Linux builds.
// Run from the repository root after changing dependencies: node packaging/third-party-notices.mjs
import { execSync } from 'node:child_process';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const run = cmd => execSync(cmd, { encoding: 'utf8', maxBuffer: 1 << 28 });

const compiled = new Set();
for (const target of ['x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu']) {
  const tree = run(`cargo tree --locked --target ${target} --edges normal --prefix none --format "{p}"`);
  for (const line of tree.split('\n')) {
    const [name, version] = line.trim().split(' ');
    if (name && name !== 'featherlaunch') compiled.add(`${name} ${version.replace(/^v/, '')}`);
  }
}

const packages = JSON.parse(run('cargo metadata --locked --format-version 1')).packages;
const crates = packages
  .filter(p => compiled.has(`${p.name} ${p.version}`))
  .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));

const texts = new Map();
const unlicensedFiles = [];
const rows = [];
for (const p of crates) {
  const dir = p.manifest_path.replace(/[\\/]Cargo\.toml$/, '');
  const files = readdirSync(dir).filter(f => /^(licen[sc]e|copying|notice|copyright)/i.test(f) && !/\.(rs|toml)$/i.test(f));
  // The embedded fonts carry their licenses beside them.
  if (p.name === 'epaint_default_fonts') {
    files.push(...readdirSync(join(dir, 'fonts')).filter(f => f.endsWith('.txt')).map(f => join('fonts', f)));
  }
  if (!files.length) unlicensedFiles.push(p);
  rows.push(`| ${p.name} | ${p.version} | ${(p.license ?? 'see crate').replace(/\|/g, '/')} |`);
  for (const file of files) {
    let text;
    try { text = readFileSync(join(dir, file), 'utf8').replace(/\r\n/g, '\n').trim(); } catch { continue; }
    if (!texts.has(text)) texts.set(text, new Set());
    texts.get(text).add(p.name);
  }
}

const sections = [...texts].map(([text, names]) => `### ${[...names].join(', ')}\n\n\`\`\`text\n${text}\n\`\`\``);
writeFileSync('THIRD_PARTY_NOTICES.md', `# Third-party notices

FeatherLaunch is built with the open source components below. They keep their own licenses, and
FeatherLaunch's own license doesn't apply to them. Where a component offers a choice of licenses,
FeatherLaunch uses it under one of the licenses listed.

| Component | Version | License |
|---|---|---|
${rows.join('\n')}

## License texts

${sections.join('\n\n')}

### Components published without a license file

These are under the licenses in the table above. The egui crates are
Copyright (c) 2018-2021 Emil Ernerfeldt. Their sources:

${unlicensedFiles.map(p => `- ${p.name}: ${p.repository}`).join('\n')}
`);
console.log(`${crates.length} crates, ${texts.size} distinct texts, ${unlicensedFiles.length} without a file`);
