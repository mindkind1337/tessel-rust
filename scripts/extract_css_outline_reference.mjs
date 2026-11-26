import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
import { writeFileSync } from 'node:fs';
const base = pathToFileURL(resolve(process.argv[2] || 'C:/Tessel/node_modules/monaco-editor/esm/external/vscode-css-languageservice/lib/esm') + '/');
const { getCSSLanguageService } = await import(new URL('cssLanguageService.js', base));
const { TextDocument } = await import(new URL('cssLanguageTypes.js', base));
const service = getCSSLanguageService();
const range = ({start,end}) => ({startLineNumber:start.line+1,startColumn:start.character+1,endLineNumber:end.line+1,endColumn:end.character+1});
const symbol = s => ({name:s.name, kind:s.kind-1, range:range(s.range), selectionRange:range(s.selectionRange), children:(s.children||[]).map(symbol)});
const cases = [
  '@unknown example { .ignored {} } @container card (width > 2px) {main > aside {}}',
  '@MEDIA screen, print { .a, .b {} }', '@layer base, theme; @layer theme {a:hover {color:red}}',
  '@font-face {font-family:"Mock";} @supports (width:1px) {@media print {a{}}}',
  'p { background: url("a{b}"); content: "a;b}"; }', '.é😀 { color: red; }\r.next {}',
  '@keyframes move {0%, 50% {opacity:.5} to {opacity:1}}', 'a; b {}',
  '', '/* empty */', '.button { color: blue; }', 'p, h1, main > a:hover {text-decoration:none}',
  '@charset "UTF-8"; @import url("theme.css"); body {padding:0}',
  '@media screen and (min-width: 20em) {body { margin:0 } @media print { .note {color:red} }}',
  '@keyframes fade { from {opacity:0} 50%, to {opacity:1} }',
  '@-webkit-keyframes pulse { to {opacity:.8} } @font-face {font-family: Demo;src:url(test.woff2)}',
  '@supports (display:grid) { .a { display:grid; } } @layer components { .b { color:blue; } }',
  '.a, .b { color: red; & > span, &:hover { color: blue } }',
  '/* é😀 */\r\n.a:is(.b,.c), [data-x="a,b"] {content:"{};/*";} ',
  '.x /* keep */ > .y /* tail */ , .z { }', ':root {--color:red;--data:{x:y};}',
  'a { color: red', '.only', '@media print {\n.foo {', '@font-face {}',
  '@page {margin:0} @namespace svg url("x"); svg|path {}', '.a { } .a { }'
].map(text => {
  const doc = TextDocument.create('file:///fixture.css','css',1,text);
  return {text,expected:service.findDocumentSymbols2(doc,service.parseStylesheet(doc)).map(symbol)};
});
writeFileSync(new URL('../tests/fixtures/css_outline.json',import.meta.url),JSON.stringify(cases,null,2)+'\n');
