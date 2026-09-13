'use strict';
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
function analyze(job,worker){
 assert.equal(job.root_exit_code,0);assert.equal(job.empty_confirmed,true);assert.equal(job.aborted,false);
 const warm=new Set(worker.cohorts.map(c=>c.pid)),cold=new Set(worker.cold.samples.map(c=>c.pid));
 assert.equal(warm.size,4);assert([...warm].every(Number.isInteger));
 const role=p=>p.pid===job.root_pid?'test_worker':warm.has(p.pid)?'warm_proxy':cold.has(p.pid)?'cold_proxy':
  path.win32.basename(p.image).toLowerCase()==='devmap.exe'?'other_devmap':'helper';
 const rows=job.resources.samples.map(s=>{
  const groups={};for(const p of s.processes){const key=role(p),g=groups[key]??={count:0,rss:0,private:0};g.count++;g.rss+=p.rss_bytes;g.private+=p.private_bytes;}
  return {time:s.elapsed_seconds,complete:s.complete_snapshot,groups,
   app_count:s.processes.filter(p=>role(p)!=='test_worker').length,
   app_rss:s.processes.filter(p=>role(p)!=='test_worker').reduce((n,p)=>n+p.rss_bytes,0),
   app_private:s.processes.filter(p=>role(p)!=='test_worker').reduce((n,p)=>n+p.private_bytes,0)};
 });
 const concurrent=rows.filter(r=>r.complete&&r.groups.warm_proxy?.count===4);assert(concurrent.length,'No complete four-proxy observation');
 return {samples:rows.length,incomplete_samples:rows.filter(r=>!r.complete).length,complete_four_proxy_samples:concurrent.length,
  max_four_proxy_count:Math.max(...concurrent.map(r=>r.app_count)),
  max_four_proxy_app_rss:Math.max(...concurrent.map(r=>r.app_rss)),
  max_four_proxy_app_private:Math.max(...concurrent.map(r=>r.app_private)),
  max_proxy_rss:Math.max(...concurrent.map(r=>r.groups.warm_proxy.rss)),
  other_devmap_counts:[...new Set(concurrent.map(r=>r.groups.other_devmap?.count||0))],
  helper_images:[...new Set(job.resources.samples.flatMap(s=>s.processes.filter(p=>role(p)==='helper').map(p=>p.image)))],
  rows,scope:'Small two-worktree fixture, sampled four-client public full-map workload. Not scale or performance acceptance. Other devmap is a role candidate, not independently authenticated owner identity. RSS sum double-counts shared pages; transient peaks can be missed.'};
}
if(require.main===module){const dir=path.resolve(process.argv[2]);const result={};for(const side of ['baseline','candidate'])result[side]=analyze(JSON.parse(fs.readFileSync(path.join(dir,side+'.job.json'))),JSON.parse(fs.readFileSync(path.join(dir,side+'.worker.json'))));fs.writeFileSync(path.join(dir,'job-resource-analysis.json'),JSON.stringify(result,null,2),{flag:'wx'});for(const [side,r]of Object.entries(result)){const {rows,...summary}=r;console.log(JSON.stringify({side,...summary}));}}
module.exports={analyze};
