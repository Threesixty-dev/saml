import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { LikeC4 } from 'likec4';

const workspace = process.argv[2] ?? fileURLToPath(new URL('.', import.meta.url));
const source = await LikeC4.fromWorkspace(workspace, { throwIfInvalid: true });
const model = await source.computedModel();
const signals = [...model.relationships()].filter((r) => r.kind === 'signal');

// Walk every modeled signal predecessor, including any intermediate cloud hop.
const pending = ['alarm'];
const visited = new Set();
while (pending.length > 0) {
    const id = pending.pop();
    if (visited.has(id)) continue;
    visited.add(id);
    const element = model.element(id);
    assert.equal(element.metadata.site, 'edge', `R1: ${id} is outside the edge site`);
    for (const relation of signals.filter((r) => r.target.id === id)) {
        pending.push(relation.source.id);
    }
}
assert(visited.has('probe'), 'R1: no modeled signal path from probe to alarm');
assert(visited.has('evaluator'), 'R1: evaluator is absent from the alarm path');
console.log('R1 passes for the modeled signal path. R2 timing is not checked.');
