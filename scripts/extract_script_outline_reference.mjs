import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { writeFileSync } from 'node:fs';

const root = resolve(process.argv[2] || 'C:/Tessel/node_modules/monaco-editor/esm/vs/languages/features/typescript');
const { TypeScriptWorker } = await import(pathToFileURL(resolve(root, 'tsWorker.js')));
const kinds = new Map(Object.entries({ module: 1, class: 4, enum: 9, interface: 10,
  method: 5, property: 6, getter: 6, setter: 6, function: 11, 'local function': 11 }));
const inputs = [
  ["js", ""],
  ["js", "// function comment() {}\nconst literal = \"class Fake {}\";"],
  ["js", "const z=1, a = 2; let middle; var end = 3;"],
  ["js", "export function run(x) { const local = x; function nested() {} return local; }"],
  ["js", "export class Box { constructor(x) { this.x=x; } get size(){return 1} set size(v){} grow(){} }"],
  ["js", "const add = (a,b) => a+b; const settings = { label: \"A\", run() {}, nested: { level: 2 } };"],
  ["ts", "import def, {type A, B as Bee} from \"./a\"; import * as util from \"./b\";"],
  ["ts", "export interface Shape { area(): number; readonly label: string; }\nexport type Id = number;"],
  ["ts", "export enum Color { Red, Green = 3, Blue }"],
  ["ts", "/* 😀 */\r\nexport class Café { value = 1; protected act(): void { const inner = 2; } }"],
  ["ts", "namespace Outer { export function hello() {} }"],
  ["ts", "function parse(x: number): number; function parse(x: string): string; function parse(x: any) {return x}"],
  ["ts", "export default function () { const nested = 0; }"],
  ["js", "if (true) {const local = 1;} function outer(){if(true){let inner=2}}"],
  ["js", "const z=1,A=1,a=1,Z=1,_z=1,$b=1,b=1;"],
  ["ts", "type X = {a:number; b():void};"],
  ["js", "const c=class Named {act(){}};const f=function named(){ const n=1; };"],
  ["js", "const obj={x,\"x-y\":2,get count(){return 1}, f:()=>2};"],
  ["ts", "declare module \"alpha\" { export function run(): void; }"],
  ["ts", "const var1=1;\rconst var2=2;\r\n/*😀*/ const var3=3;"],
  ["ts","const {a:b=2,c,nested:{x},...rest}=obj; const [first=1,,...tail]=arr; class B{constructor(public readonly x:number,private y=1){const inside=1;}}"],
  ["js", "function edit() {\n const inside = 1;"],
  ["ts", "class InProgress { value = 1;"],
  ["js", "const obj = {name: 'ok',"],
  ["ts", "const completed = 1; const pending ="],
  ["ts", "function f(){class C{m(){ const n=1;  "],
  ["js", "function edit(){ const a=1; // trailing } 😀"],
  ["ts", "export function draft(){\r\n  /* 😀 */ const café = [1, 2,  "],
  ["js", "const rg = /[{}]/; function f(){ const v = '}' ; /* } */"],
  ["js", "const tpl = `a${foo()}!`; class C{"],
  ["ts", "function run(a: number"],
  ["js", "function run(a"],
  ["ts", "class Box { method("],
  ["ts", "const nested = {value: ["],
  ["ts", "function nested(){ const unfinished = // comment"],

  ["js", "function empty(){  // draft"],
  ["js", "export const fetchItems = async (id) => { await read(id);"],
  ["ts", "interface Shape { label: string; area(): number;"],
  ["ts", "enum Status { Idle, Done,"],
  ["ts", "namespace Tasks { export function run() { const job = {"],
  ["js", "const before = 1; const values = [1, 2, //😀"],
  ["ts", "const draft = { property: "],
  ["js", "function outer(){ const quoted = 'https://foo/{';\n  // 😀"],
  ["ts", "const complete = 3; function later()"],

];
const output = [];
for (const [extension, text] of inputs) {
  const path = '/fixture.' + extension;
  const uri = 'file://' + path;
  const model = {uri: {path, toString: () => uri}, version: 1, getValue: () => text};
  const worker = new TypeScriptWorker({getMirrorModels: () => [model]}, {compilerOptions: {allowJs: true}, extraLibs: {}});
  const tree = await worker.getNavigationTree(uri);
  function position(offset) {
    const lines = text.slice(0, offset).split(/\r\n|\r|\n/);
    return [lines.length, lines.at(-1).length + 1];
  }
  function symbol(item) {
    const {start, length} = item.spans[0];
    const [startLineNumber, startColumn] = position(start);
    const [endLineNumber, endColumn] = position(start + length);
    const range = {startLineNumber, startColumn, endLineNumber, endColumn};
    return {name: item.text, kind: kinds.get(item.kind) ?? 12, range, selectionRange: range,
      children: (item.childItems ?? []).map(symbol)};
  }
  output.push({extension, text, expected: (tree.childItems ?? []).map(symbol)});
}
writeFileSync(new URL('../tests/fixtures/script_outline.json', import.meta.url), JSON.stringify(output, null, 2) + '\n');
