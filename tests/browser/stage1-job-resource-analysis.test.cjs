const {test}=require('node:test'),assert=require('node:assert/strict');
const {analyze}=require('./stage1-job-resource-analysis.cjs');
const worker={cohorts:[1,2,3,4].map(pid=>({pid})),cold:{samples:[{pid:5}]}};
const fixture=()=>({root_exit_code:0,empty_confirmed:true,aborted:false,root_pid:9,resources:{samples:[{elapsed_seconds:1,complete_snapshot:true,processes:[1,2,3,4,9].map(pid=>({pid,image:pid===9?'node.exe':'devmap.exe',rss_bytes:10,private_bytes:5}))}]}});
test('aggregate excludes the known test worker and requires all four proxies',()=>{const result=analyze(fixture(),worker);assert.equal(result.max_four_proxy_app_rss,40);assert.equal(result.max_four_proxy_app_private,20);assert.equal(result.max_four_proxy_count,4);});
test('incomplete membership or missing proxy cannot establish a complete four-client sample',()=>{const a=fixture();a.resources.samples[0].complete_snapshot=false;assert.throws(()=>analyze(a,worker));const b=fixture();b.resources.samples[0].processes.shift();assert.throws(()=>analyze(b,worker));});
