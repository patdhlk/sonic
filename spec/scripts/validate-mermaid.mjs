#!/usr/bin/env node
/**
 * Validate every `.. mermaid::` block in spec/**\/*.rst against the mermaid
 * parser. Catches the failure class we hit when sphinx silently double-escapes
 * `&lt;` to `&amp;lt;` (or other RST/HTML/mermaid-syntax mismatches) before
 * the diagram ever reaches the browser.
 *
 * Runs in pure Node + jsdom (no Chromium / Puppeteer), so it adds ~5s to CI
 * instead of the ~60s that `mmdc` would.
 *
 * Usage:
 *   node scripts/validate-mermaid.mjs [root]
 *
 * Default root is the directory the script is invoked from. Walks all `.rst`
 * files (excluding `_build`, `_static`, and dotfiles), extracts each
 * `.. mermaid::` block by indentation, and pipes it through `mermaid.parse()`.
 * Exits 1 if any block fails to parse.
 */
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { JSDOM } from 'jsdom';

// mermaid pulls window/document at module-init time. Stub a DOM before import.
const dom = new JSDOM('<!doctype html><html><head></head><body></body></html>', {
  pretendToBeVisual: true,
});
globalThis.window = dom.window;
globalThis.document = dom.window.document;
globalThis.HTMLElement = dom.window.HTMLElement;
globalThis.SVGElement = dom.window.SVGElement;
globalThis.Element = dom.window.Element;
globalThis.Node = dom.window.Node;
// `navigator` is read-only on modern Node; only set it if assignable.
try { globalThis.navigator = dom.window.navigator; } catch { /* Node ≥ 21 */ }

const { default: mermaid } = await import('mermaid');
mermaid.initialize({ startOnLoad: false, securityLevel: 'loose', suppressErrorRendering: true });

const ROOT = process.argv[2] || '.';

function* walkRst(dir) {
  for (const entry of readdirSync(dir)) {
    if (entry.startsWith('_') || entry.startsWith('.')) continue;
    const full = join(dir, entry);
    const stat = statSync(full);
    if (stat.isDirectory()) yield* walkRst(full);
    else if (full.endsWith('.rst')) yield full;
  }
}

function* extractMermaidBlocks(rstText, file) {
  const lines = rstText.split('\n');
  let i = 0;
  while (i < lines.length) {
    const m = lines[i].match(/^(\s*)\.\. mermaid::\s*$/);
    if (!m) { i++; continue; }
    const directiveIndent = m[1].length;
    const startLine = i + 1; // 1-indexed RST line
    i++;
    while (i < lines.length && lines[i].trim() === '') i++;
    const body = [];
    let bodyIndent = null;
    while (i < lines.length) {
      if (lines[i].trim() === '') { body.push(''); i++; continue; }
      const lineIndent = lines[i].match(/^\s*/)[0].length;
      if (lineIndent <= directiveIndent) break;
      if (bodyIndent === null) bodyIndent = lineIndent;
      body.push(lines[i].slice(bodyIndent));
      i++;
    }
    yield { source: body.join('\n').trim(), file, line: startLine };
  }
}

const errors = [];
let blockCount = 0;

for (const rst of walkRst(ROOT)) {
  const text = readFileSync(rst, 'utf8');
  for (const block of extractMermaidBlocks(text, rst)) {
    blockCount++;
    try {
      await mermaid.parse(block.source);
    } catch (e) {
      errors.push({ ...block, error: (e && e.message) ? e.message : String(e) });
    }
  }
}

if (errors.length) {
  for (const err of errors) {
    console.error(`\nFAIL ${relative(ROOT, err.file)}:${err.line}`);
    const msg = err.error.split('\n').slice(0, 6).join('\n  ');
    console.error(`  ${msg}`);
  }
  console.error(`\n${errors.length} of ${blockCount} mermaid block(s) failed to parse`);
  process.exit(1);
}

console.log(`OK: ${blockCount} mermaid block(s) parsed successfully`);
