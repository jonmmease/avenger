// Usage: node generate.mjs /path/to/runtime/with/pinned/node_modules
// Reference dependencies: vega-lite 6.4.3, vega 6.4.0. No runtime dependency in Rust tests.
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
const require = createRequire(path.resolve(process.argv[2], 'package.json'));
const vl = await import(pathToFileURL(require.resolve('vega-lite')));
const vega = await import(pathToFileURL(require.resolve('vega')));
const target = new URL('./basic-bars.json', import.meta.url);
const old = JSON.parse(fs.readFileSync(target));
const cases = [];
for (const {name,spec} of old.cases) {
  const compiled = vl.compile(spec).spec;
  const view = new vega.View(vega.parse(compiled), {renderer:'none'});
  await view.runAsync();
  const rects = [];
  function visit(node) {
    if (node.mark?.marktype === 'rect') rects.push({x:node.x,y:node.y,width:node.width,height:node.height});
    for (const child of node.items || []) visit(child);
  }
  visit(view.scenegraph().root);
  cases.push({name,spec,width:view.signal('width'),height:view.signal('height'),domains:Object.fromEntries(compiled.scales.map(s=>[s.name,view.scale(s.name).domain()])),rects});
  view.finalize();
}
fs.writeFileSync(target,JSON.stringify({vega_lite:vl.version,vega:vega.version,cases},null,2)+'\n');
