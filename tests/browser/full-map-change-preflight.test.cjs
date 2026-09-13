'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),{qualifies}=require('./full-map-change-preflight.cjs');
test('change convergence rejects stale, unknown and wrong workspace evidence',()=>{
 const valid={generated_at:'2026-09-13T12:00:00Z',observation_revision:3,workspace_facts:[{worktree_id:'w',head_oid:'h',working_state:'dirty',git_observed_at:'2026-09-13T12:00:00Z'}]};
 assert(qualifies(valid,'w','h',true,2));
 for(const m of [{...valid,observation_revision:2},{...valid,observation_revision:3.5},{...valid,generated_at:null},{...valid,workspace_facts:[]},{...valid,workspace_facts:[{...valid.workspace_facts[0],git_observed_at:null}]},{...valid,workspace_facts:[{...valid.workspace_facts[0],working_state:'unknown'}]}])assert(!qualifies(m,'w','h',true,2));
 assert(!qualifies(valid,'other','h',true,2));assert(!qualifies(valid,'w','other',true,2));assert(!qualifies(valid,'w','h',false,2));
});
