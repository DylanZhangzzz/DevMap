'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {auditChangeModel,population}=require('./full-map-change-model.cjs');
function fixture(){
 const lane={worktree_id:'w',head:'head',relationship:{status_observed:true,dirty:false,changed_file_count:0},chats:[{session_id:'session'}]};
 return {schema_version:'devmap/dock/4',generated_at:'time-1',revision:1,observation_revision:2,lanes:[lane],branch_groups:[{lanes:[structuredClone(lane)]}],workspace_facts:[{worktree_id:'w',working_state:'clean',git_observed_at:'time-1',head_oid:'head'}],route_plans:[{id:'route'}],warnings:[],topology:{complete:true}};
}
function dirtyModel(base){const m=structuredClone(base);m.generated_at='time-2';m.revision=2;m.observation_revision=3;m.workspace_facts[0].git_observed_at='time-2';m.workspace_facts[0].working_state='dirty';for(const l of [m.lanes[0],m.branch_groups[0].lanes[0]]){l.relationship.dirty=true;l.relationship.changed_file_count=1;}return m;}
const options={worktreeId:'w',dirty:true,revision:2,observationRevision:3};
test('full-model audit accepts only the declared tracked-file delta',()=>{const base=fixture(),model=dirtyModel(base);assert.match(auditChangeModel(base,model,options),/^[a-f0-9]{64}$/);assert.equal(base.workspace_facts[0].working_state,'clean');});
test('loss or drift outside the Git probe cannot hide behind convergence',()=>{
 const base=fixture();for(const mutate of [m=>m.route_plans=[],m=>m.lanes[0].chats=[],m=>m.warnings.push('new warning'),m=>m.topology.complete=false,m=>m.lanes.push(structuredClone(m.lanes[0])),m=>m.branch_groups[0].lanes[0].relationship.changed_file_count=2,m=>m.workspace_facts[0].head_oid='wrong',m=>m.revision=3,m=>m.observation_revision=2,m=>m.workspace_facts[0].git_observed_at='time-1']){
  const m=dirtyModel(base);mutate(m);assert.throws(()=>auditChangeModel(base,m,options));
 }
});
test('failure population preserves failed and not-executed trials',()=>{
 const complete=()=>({phase:'measured',status:'complete',clients:Array.from({length:4},()=>({status:'converged',elapsed_ms:1,model_verified:true}))});
 const rows=[complete(),{phase:'measured',status:'failed',clients:[]},...Array.from({length:2},()=>({phase:'measured',status:'not-executed',clients:[]}))];
 assert.deepEqual(population(rows),{expected:4,attempted:2,completed:1,failed:1,not_executed:2,full_population:false});
 assert(population(Array.from({length:4},complete)).full_population);const bad=complete();bad.clients[2].model_verified=false;assert(!population([bad]).full_population);assert(!population([]).full_population);
});
