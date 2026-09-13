'use strict';
// Read-only OS counters. No process enumeration, priority or power-plan changes.
const os=require('node:os');
function sample(previous){
 const current={at:new Date().toISOString(),monotonic_ms:performance.now(),free_memory_bytes:os.freemem(),total_memory_bytes:os.totalmem(),cpus:os.cpus().map(c=>({...c.times}))};
 if(previous){
  current.cpu_busy_percent=current.cpus.map((cpu,i)=>{
   const old=previous.cpus[i];if(!old)return null;
   const total=Object.values(cpu).reduce((a,b)=>a+b,0)-Object.values(old).reduce((a,b)=>a+b,0),idle=cpu.idle-old.idle;
   return total>0&&idle>=0&&idle<=total?100*(total-idle)/total:null;
  });
 }
 return current;
}
module.exports={sample};
