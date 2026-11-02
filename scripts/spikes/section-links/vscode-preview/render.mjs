// A VS Code-style Markdown preview of a folder, for the section-links spike.
//
// What is VS Code's own and what is emulated, stated so the report can say it:
// - Link resolution is VS Code's own code: `vscode-markdown-languageservice` 0.5.0, the exact
//   version VS Code's `markdown-language-features` extension pins, asked `resolveLinkTarget`
//   for each link with the workspace folder given on the command line.
// - What the preview then does is ported from `extensions/markdown-language-features`:
//   `preview.ts#onDidClickPreviewLink` opens a resolved Markdown file in the preview with the
//   resolved `fragment` (set only when the language service found a heading or HTML id for
//   it), and otherwise falls back to opening the link in a text editor; the preview script's
//   `getLineElementForFragment` scrolls to the `.code-line` element whose `id` is that
//   fragment, and to the top when none is.
// - Rendering is markdown-it with `html: true`, as the preview has it, with each heading's
//   `id` from the same slug builder the preview's `markdownEngine.ts` uses and `code-line`
//   and `data-line` on every block, as the preview's renderer adds.
//
// Each link's href is rewritten to the page the preview would open (`<target>.html?fragment=`),
// or to `not-opened.html` naming why the preview would open nothing; `resolutions.json` records
// every resolution.
import fs from 'node:fs';
import path from 'node:path';
import MarkdownIt from 'markdown-it';
import { TextDocument } from 'vscode-languageserver-textdocument';
import { createLanguageService, githubSlugifier } from 'vscode-markdown-languageservice';
import { URI } from 'vscode-uri';

const args = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, arg, i, all) => (arg.startsWith('--') ? [...pairs, [arg.slice(2), all[i + 1]]] : pairs), []),
);
const workspace = path.resolve(args.workspace);
const dir = path.resolve(args.dir);
const out = path.resolve(args.out);
fs.mkdirSync(out, { recursive: true });

const md = new MarkdownIt({ html: true });
const docs = new Map();
function open(uri) {
  const file = uri.fsPath;
  if (!file.endsWith('.md') || !fs.existsSync(file)) return undefined;
  if (!docs.has(file)) docs.set(file, TextDocument.create(uri.toString(), 'markdown', 1, fs.readFileSync(file, 'utf8')));
  return docs.get(file);
}
const none = () => ({ dispose() {} });
const ls = createLanguageService({
  workspace: {
    get workspaceFolders() { return [URI.file(workspace)]; },
    onDidChangeMarkdownDocument: none,
    onDidCreateMarkdownDocument: none,
    onDidDeleteMarkdownDocument: none,
    async getAllMarkdownDocuments() { return fs.readdirSync(dir).filter((f) => f.endsWith('.md')).map((f) => open(URI.file(path.join(dir, f)))); },
    hasMarkdownDocument(uri) { return open(uri) !== undefined; },
    async openMarkdownDocument(uri) { return open(uri); },
    async stat(uri) { try { return { isDirectory: fs.statSync(uri.fsPath).isDirectory() }; } catch { return undefined; } },
    async readDirectory(uri) { return fs.readdirSync(uri.fsPath, { withFileTypes: true }).map((e) => [e.name, { isDirectory: e.isDirectory() }]); },
  },
  parser: { slugifier: githubSlugifier, async tokenize(doc) { return md.parse(doc.getText(), {}); } },
  logger: { level: 0, log() {} },
});
const token = { isCancellationRequested: false, onCancellationRequested: none };

const page = (title, body) => `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title>
<style>body{font-family:sans-serif;max-width:820px;margin:0 auto;padding:16px;line-height:1.5}</style></head>
<body class="vscode-body">${body}
<script>
// Ported from VS Code's preview-src: index.ts encodes the fragment, scroll-sync.ts's
// getLineElementForFragment finds the .code-line element whose id is it, and the preview
// reveals that line at the top. No fragment, or none matching, leaves the page at the top.
window.addEventListener('load', () => {
  const raw = new URLSearchParams(location.search).get('fragment');
  if (!raw) return;
  let fragment; try { fragment = encodeURIComponent(raw); } catch { fragment = raw; }
  const el = [...document.querySelectorAll('.code-line')].find((e) => e.id === fragment);
  if (el) window.scrollTo(0, el.getBoundingClientRect().top + window.scrollY);
});
</script></body></html>`;

function renderHtml(text) {
  const env = { slugifier: githubSlugifier.createBuilder() };
  const tokens = md.parse(text, env);
  for (const t of tokens) {
    if (t.map && t.nesting >= 0 && t.type !== 'inline') {
      t.attrJoin('class', 'code-line');
      t.attrSet('data-line', String(t.map[0]));
    }
  }
  for (let i = 0; i < tokens.length; i++) {
    if (tokens[i].type === 'heading_open') {
      const title = tokens[i + 1].children.reduce((acc, t) => acc + t.content, '');
      tokens[i].attrSet('id', env.slugifier.add(title).value);
    }
  }
  return { tokens, env };
}

const resolutions = {};
fs.writeFileSync(path.join(out, 'not-opened.html'), page('not opened', '<h1 id="not-opened">VS Code would not open a Markdown preview for this link</h1><p id="reason"></p><script>document.getElementById("reason").textContent=new URLSearchParams(location.search).get("reason")||""</script>'));
for (const name of fs.readdirSync(dir).filter((f) => f.endsWith('.md'))) {
  const file = path.join(dir, name);
  const uri = URI.file(file);
  const { tokens, env } = renderHtml(fs.readFileSync(file, 'utf8'));
  for (const t of tokens) {
    for (const child of t.children ?? []) {
      if (child.type !== 'link_open') continue;
      const href = child.attrGet('href');
      if (/^[a-z-]+:/i.test(href)) continue;
      const text = t.children[t.children.indexOf(child) + 1]?.content ?? '';
      const code = text.split(' ')[0];
      const resolved = await ls.resolveLinkTarget(href, uri, token);
      let action;
      let newHref;
      if (resolved?.kind === 'file' && resolved.uri.fsPath.endsWith('.md') && fs.existsSync(resolved.uri.fsPath)) {
        const fragment = resolved.fragment ? decodeURIComponent(resolved.fragment) : undefined;
        action = fragment ? `preview of ${path.basename(resolved.uri.fsPath)}, revealing #${fragment}` : `preview of ${path.basename(resolved.uri.fsPath)}, at the top (no heading or HTML id matched the fragment)`;
        newHref = path.basename(resolved.uri.fsPath).replace(/\.md$/, '.html') + (fragment ? `?fragment=${encodeURIComponent(fragment)}` : '');
      } else {
        const where = resolved?.uri?.fsPath;
        action = resolved
          ? `no preview: resolved to ${where}, which does not exist, so VS Code falls back to opening it as a text editor and reports the file missing`
          : 'no preview: the language service resolved nothing, so the click does nothing';
        newHref = `not-opened.html?reason=${encodeURIComponent(action)}`;
      }
      child.attrSet('href', newHref);
      if (/^[A-Z]{2}-[A-Z]-[A-Z]\d$/.test(code)) {
        resolutions[code] = {
          href,
          resolved: resolved ? { kind: resolved.kind, path: resolved.uri?.fsPath, fragment: resolved.fragment ?? null, position: resolved.positionOrRange ?? null } : null,
          preview: action,
        };
      }
    }
  }
  fs.writeFileSync(path.join(out, name.replace(/\.md$/, '.html')), page(name, md.renderer.render(tokens, md.options, env)));
}
fs.writeFileSync(path.join(out, 'resolutions.json'), JSON.stringify(resolutions, null, 2) + '\n');
