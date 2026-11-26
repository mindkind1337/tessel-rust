import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
import { writeFileSync } from 'node:fs';
const base = pathToFileURL(resolve(process.argv[2] || 'C:/Tessel/node_modules/monaco-editor/esm/external/vscode-html-languageservice/lib/esm') + '/');
const { getLanguageService } = await import(new URL('htmlLanguageService.js', base));
const { TextDocument } = await import(new URL('htmlLanguageTypes.js', base));
const service = getLanguageService();
const range = ({start,end}) => ({startLineNumber:start.line+1,startColumn:start.character+1,endLineNumber:end.line+1,endColumn:end.character+1});
const symbol = s => ({name:s.name, kind:s.kind-1, range:range(s.range), selectionRange:range(s.selectionRange), children:(s.children||[]).map(symbol)});
const cases = [
  '<div <p>x', '<', '<  div>', '<😀>', '<script type="text/html"><x></script>',
  '<script><!-- <script>bad</script><b> --></script><p>',
  '', 'plain 😀 text', '<!DOCTYPE html><html><head><title>T</title></head><body><h1>Hello</h1></body></html>',
  '<DIV ID="UP" CLASS="ignored"><x-box id="box" class=" a  b " data-val=">">one</x-box></DIV>',
  "<main>\r\n😀<img src='p'><br/><p id=x class='a b'>é</p>\r</main>",
  '<!-- <fake> --><script>let tag = "<div>";</script><style>.a:after{content:"<p>"}</style><hr>',
  '<ul><li>a<li>b</ul><footer>open', '<section><div', '<div id="" class=""><input disabled><span/></div>',
  '<div id=a id=b class=one></DIV>', '<article><b>bold</article></orphan><aside>tail',
  '<svg><linearGradient id="g"/><path d="M1 1"/></svg>', '<script type="text/x-template"><b>template</b></script>',
  '<p title="a>b">x</p><div data-q="<fake>"></div>', '<area><base><br><col><embed><hr><img><input><link><meta><param><source><track><wbr>'
].map(text => {
  const doc = TextDocument.create('file:///fixture.html','html',1,text);
  return {text,expected:service.findDocumentSymbols2(doc,service.parseHTMLDocument(doc)).map(symbol)};
});
writeFileSync(new URL('../tests/fixtures/html_outline.json',import.meta.url),JSON.stringify(cases,null,2)+'\n');
