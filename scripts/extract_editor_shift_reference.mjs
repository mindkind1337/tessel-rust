// Synthetic ShiftCommand text edits, with language-aware indentation disabled.
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
const root = resolve(process.argv[2]);
const load = path => import(pathToFileURL(resolve(root, path)));
const { ShiftCommand } = await load('esm/vs/editor/common/commands/shiftCommand.js');
const { Selection } = await load('esm/vs/editor/common/core/selection.js');
const { IntervalNode, nodeAcceptEdit } = await load('esm/vs/editor/common/model/intervalTree.js');
const vectors = [];
for (const text of ['a\nb', '  a\n    b\nlast', '\t a\n\t\tb', '  é😀\r\n\r\n   z\r\nlast', '    x\n\n    y', '    ', '\t  \r\n  x']) {
  const lines = text.split(/\r\n|\n/);
  const sep = text.includes('\r\n') ? '\r\n' : '\n';
  const byteOffset = (line, column) => Buffer.byteLength(lines.slice(0, line-1).map(s => s + sep).join('') + lines[line-1].slice(0, column-1));
  const selections = [[1,1,lines.length,lines.at(-1).length+1], [1,1,2,1], [1,1,1,1], [1,2,1,2], [1,2,lines.length,lines.at(-1).length+1], [lines.length,lines.at(-1).length+1,1,2]];
  for (const selection of selections) for (const size of [2,4,8]) for (const spaces of [false,true]) for (const outdent of [false,true]) {
    if (selection[0] > lines.length || selection[2] > lines.length || selection[1] > lines[selection[0]-1].length+1 || selection[3] > lines[selection[2]-1].length+1) continue;
    const initial = new Selection(...selection);
    if (!outdent && (initial.isEmpty() || (initial.startLineNumber === initial.endLineNumber && (initial.startColumn !== 1 || initial.endColumn !== lines[initial.startLineNumber-1].length+1)))) continue; // TabOperation, not ShiftCommand
    const edits = [];
    const add = (range, replacement) => edits.push({ start: byteOffset(range.startLineNumber,range.startColumn), end: byteOffset(range.endLineNumber,range.endColumn), replacement });
    const model = { getLineContent: n => lines[n-1], getLineLength: n => lines[n-1].length, getLineMaxColumn: n => lines[n-1].length+1, tokenization: {isCheapToTokenize: () => false} };
    const marker = new IntervalNode('selection',byteOffset(initial.startLineNumber,initial.startColumn),byteOffset(initial.endLineNumber,initial.endColumn));
    // Same tracking stickiness chosen by cursor.js trackSelection.
    marker.setOptions({stickiness: initial.isEmpty() ? (initial.startColumn === model.getLineMaxColumn(initial.startLineNumber) ? 2 : 3) : 1, collapseOnReplaceEdit:false});
    const command = new ShiftCommand(initial, {tabSize:size, indentSize:size, insertSpaces:spaces, isUnshift:outdent, useTabStops:true, autoIndent:0}, null);
    command.getEditOperations(model, {addEditOperation:add, addTrackedEditOperation:add, trackSelection:()=> 'selection'});
    let result = Buffer.from(text);
    for (const edit of edits.sort((a,b)=>b.start-a.start)) {
      nodeAcceptEdit(marker,edit.start,edit.end,Buffer.byteLength(edit.replacement),false);
      result = Buffer.concat([result.subarray(0,edit.start),Buffer.from(edit.replacement),result.subarray(edit.end)]);
    }
    const position = offset => { const prefix = result.subarray(0,offset).toString().split('\n'); return [prefix.length,prefix.at(-1).length+1]; };
    const tracked = initial.getDirection() === 0 ? new Selection(...position(marker.start),...position(marker.end)) : new Selection(...position(marker.end),...position(marker.start));
    let delta = 0;
    const inverse = edits.slice().sort((a,b)=>a.start-b.start).map(edit=> {
      const start = edit.start + delta, end = start + Buffer.byteLength(edit.replacement);
      delta += Buffer.byteLength(edit.replacement) - (edit.end-edit.start);
      return {range:new Selection(...position(start),...position(end))};
    });
    const cursor = edits.length ? command.computeCursorState(model,{getTrackedSelection:()=>tracked,getInverseEditOperations:()=>inverse}) : initial;
    const resultLines = result.toString().split(sep);
    const resultOffset = (line,column) => Buffer.byteLength(resultLines.slice(0,line-1).map(s=>s+sep).join('')+resultLines[line-1].slice(0,column-1));
    vectors.push({text, start:byteOffset(initial.startLineNumber,initial.startColumn), end:byteOffset(initial.endLineNumber,initial.endColumn), size, spaces, outdent, expected:result.toString(), selection:[resultOffset(cursor.startLineNumber,cursor.startColumn),resultOffset(cursor.endLineNumber,cursor.endColumn)]});
  }
}
writeFileSync(new URL('../tests/fixtures/editor_shift_reference.json', import.meta.url), JSON.stringify({monacoVersion:JSON.parse(readFileSync(resolve(root,'package.json'),'utf8')).version,vectors},null,2)+'\n');
console.log(`${vectors.length} ShiftCommand vectors`);
