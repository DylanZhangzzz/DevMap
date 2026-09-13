'use strict';
const assert=require('node:assert/strict');
const {fingerprint}=require('./full-map-fingerprint.cjs');
// Exact expected delta for one tracked file in the frozen-old clean fixture.
// Never remove whole relationship, route, task, warning or topology fields.
function auditChangeModel(baseline,actual,{worktreeId,dirty,revision,observationRevision}){
 assert.equal(baseline.schema_version,'devmap/dock/4');
 assert(Number.isSafeInteger(revision)&&Number.isSafeInteger(observationRevision));
 assert.equal(actual.revision,revision);assert.equal(actual.observation_revision,observationRevision);
 const expected=structuredClone(baseline);
 expected.revision=revision;expected.observation_revision=observationRevision;
 const facts=expected.workspace_facts.filter(f=>f.worktree_id===worktreeId);assert.equal(facts.length,1);
 assert.equal(facts[0].working_state,'clean');facts[0].working_state=dirty?'dirty':'clean';
 for(const collection of [expected.lanes,expected.branch_groups.flatMap(g=>g.lanes)]){
  const matches=collection.filter(l=>l.worktree_id===worktreeId);assert.equal(matches.length,1);
  const relation=matches[0].relationship;assert.equal(relation.status_observed,true);assert.equal(relation.dirty,false);assert.equal(relation.changed_file_count,0);
  relation.dirty=dirty;relation.changed_file_count=dirty?1:0;
 }
 const expectedHash=fingerprint(expected,'fresh-git-observation'),actualHash=fingerprint(actual,'fresh-git-observation');
 assert.equal(actualHash,expectedHash,'Full map changed outside the tracked-file delta');return actualHash;
}
function population(trials){
 const rows=trials.filter(t=>t.phase==='measured');
 return {expected:rows.length,attempted:rows.filter(t=>t.status!=='not-executed').length,completed:rows.filter(t=>t.status==='complete').length,failed:rows.filter(t=>t.status==='failed').length,not_executed:rows.filter(t=>t.status==='not-executed').length,
 full_population:rows.length>0&&rows.every(t=>t.status==='complete'&&t.clients.length===4&&t.clients.every(c=>c.status==='converged'&&c.model_verified===true&&Number.isFinite(c.elapsed_ms)&&c.elapsed_ms>=0))};
}
module.exports={auditChangeModel,population};
