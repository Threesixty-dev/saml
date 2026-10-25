import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { LikeC4 } from 'likec4';

const workspace = process.argv[2] ?? fileURLToPath(new URL('.', import.meta.url));
const source = await LikeC4.fromWorkspace(workspace, { throwIfInvalid: true });
const model = await source.computedModel();
const relationships = [...model.relationships()];
const has = (kind, from, to) => relationships.some((relationship) =>
    relationship.kind === kind &&
    relationship.source.id === from &&
    relationship.target.id === to
);

for (const consumption of relationships.filter((relationship) => relationship.kind === 'consumes')) {
    const provided = relationships.some((relationship) =>
        relationship.kind === 'provides' && relationship.target.id === consumption.target.id
    );
    assert(provided, `${consumption.source.id} consumes ${consumption.target.id} without a provider`);
}

assert(has('precedes', 'evaluation', 'publication'), 'Publication lacks the evaluation predecessor');
assert(has('realizes', 'articleData', 'article'), 'Article data has no domain counterpart');
console.log('Custom article-publication checks pass for the modeled relationships.');
