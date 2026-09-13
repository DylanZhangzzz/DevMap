'use strict';
const {test}=require('node:test'),assert=require('node:assert/strict');
const {fingerprint}=require('./full-map-fingerprint.cjs');
const base=()=>({schema_version:'devmap/dock/4',generated_at:'2026-09-13T10:00:00Z',repository_id:'repo',revision:1,workspace_facts:[{git_observed_at:'2026-09-13T10:00:00Z',task_observed_at:null,worktree_id:'wt',working_state:'clean'}],warnings:[]});
test('strict fingerprints keep nested observation times',()=>{
 const a=base(),b=base();b.generated_at='2026-09-13T10:01:00Z';
 assert.equal(fingerprint(a),fingerprint(b));
 b.workspace_facts[0].git_observed_at=b.generated_at;
 assert.notEqual(fingerprint(a),fingerprint(b));
});
test('fresh observation mode validates equality instead of ignoring stale time',()=>{
 const a=base(),b=base();b.generated_at='2026-09-13T10:01:00Z';
 assert.throws(()=>fingerprint(b,'fresh-git-observation'),/cached\/older/);
 b.workspace_facts[0].git_observed_at=b.generated_at;
 assert.equal(fingerprint(a,'fresh-git-observation'),fingerprint(b,'fresh-git-observation'));
 b.workspace_facts[0].git_observed_at=null;
 assert.notEqual(fingerprint(a,'fresh-git-observation'),fingerprint(b,'fresh-git-observation'));
});
test('business, identity, revision, task time and array changes remain visible',()=>{
 const a=base(),mode='fresh-git-observation';
 for(const mutate of [b=>b.repository_id='wrong',b=>b.revision++,b=>b.workspace_facts[0].working_state='dirty',b=>b.workspace_facts[0].worktree_id='wrong',b=>b.workspace_facts[0].task_observed_at=b.generated_at,b=>b.warnings.push({message:'unexpected'})]){
  const b=structuredClone(a);mutate(b);assert.notEqual(fingerprint(a,mode),fingerprint(b,mode));
 }
 assert.deepEqual(a,base(),'fingerprinting must not mutate the source response');
 const b={...a,warnings:[1,2]},c={...a,warnings:[2,1]};assert.notEqual(fingerprint(b),fingerprint(c));
 assert.throws(()=>fingerprint(a,'ignore-all-times'));
});
