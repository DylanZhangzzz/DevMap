'use strict';
const {test}=require('node:test'),assert=require('node:assert/strict');
const {validateReadback}=require('./codex-host-reopen.cjs');
const route={route_id:'route',repository_id:'repository',worktree_id:'worktree',revision:1,goal:'retained goal',updated_at:'2026-09-13T10:00:00Z'};
const first={schema_version:'devmap/dock/4',repository_id:'repository',current_worktree_id:'worktree',route_plans:[route]};
test('readback accepts the same full persisted route',()=>{validateReadback(structuredClone(first),first,route);});
test('wrong repository, worktree, route fields or missing rows cannot pass',()=>{
 for(const change of [m=>m.repository_id='wrong',m=>m.current_worktree_id='wrong',m=>m.route_plans[0].goal='wrong',m=>m.route_plans[0].revision++,m=>m.route_plans[0].updated_at='changed',m=>m.route_plans=[],m=>m.route_plans.push(structuredClone(route))]){
  const model=structuredClone(first);change(model);assert.throws(()=>validateReadback(model,first,route));
 }
});
