const {test}=require('node:test'),assert=require('node:assert/strict');
const {validateRecords}=require('./hook-cold-end.cjs');
const records=[{sequence:1,sha256:'first',event:{event_type:'session_started',host:{name:'codex'},context:{session_id:'session',repository:'repo',worktree:'worktree'},actor:{agent_id:'actor'}}},{sequence:2,previous_sha256:'first',event:{event_type:'session_stopped',host:{name:'codex'},context:{session_id:'session',repository:'repo',worktree:'worktree'},actor:{agent_id:'actor'}}}];
test('durable ordered start/stop pair is accepted',()=>validateRecords(records,'session'));
test('wrong identity, missing stop, sequence or hash association cannot pass',()=>{
 for(const change of [r=>r.pop(),r=>r.reverse(),r=>r[1].event.context.session_id='other',r=>r[1].event.context.repository='other',r=>r[1].event.actor.agent_id='other',r=>r[1].previous_sha256='wrong',r=>r[1].sequence=3]){const input=structuredClone(records);change(input);assert.throws(()=>validateRecords(input,'session'));}
});
