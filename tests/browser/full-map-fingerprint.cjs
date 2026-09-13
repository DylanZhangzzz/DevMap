'use strict';
const assert = require('node:assert/strict'), crypto = require('node:crypto');
function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === 'object') return Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])]));
  return value;
}
function fingerprint(model, mode = 'strict') {
  assert(['strict', 'fresh-git-observation'].includes(mode));
  assert.equal(model.schema_version, 'devmap/dock/4');
  // Only the top-level response generation time is volatile by definition.
  // Preserve identities, revisions, freshness, nested timestamps and array order.
  const {generated_at, ...content} = model;
  assert.equal(typeof generated_at, 'string');
  if (mode === 'fresh-git-observation') {
    content.workspace_facts = model.workspace_facts.map(facts => {
      if (facts.git_observed_at === null) return facts;
      assert.equal(facts.git_observed_at, generated_at, 'Expected Git observation from this generation; cached/older facts need separate audit');
      return {...facts, git_observed_at: '@same-as-generated-at'};
    });
  }
  return crypto.createHash('sha256').update(JSON.stringify(canonical(content))).digest('hex');
}
module.exports = {fingerprint};
