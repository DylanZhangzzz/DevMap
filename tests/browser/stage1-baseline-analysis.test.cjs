'use strict';
const {test}=require('node:test'),assert=require('node:assert/strict');
const {intervals,analyze}=require('./stage1-baseline-analysis.cjs');
test('identical paired blocks produce zero difference and a zero interval',()=>{
 const rows=Array.from({length:20},(_,i)=>1000+i*10);
 const [r]=intervals([rows],[rows],5,100);
 assert.equal(r.delta_p95_ms,0);assert.deepEqual(r.paired_block_95_interval_ms,[0,0]);assert.equal(r.noise_within_maximum_cap,true);
});
test('paired constant shifts retain sign and reject a shift beyond the cap',()=>{
 const rows=Array.from({length:20},(_,i)=>1000+i);
 const shifted=rows.map(v=>v+200);
 const result=intervals([rows,shifted],[shifted,rows],5,100);
 assert.deepEqual(result.map(r=>r.paired_block_95_interval_ms),[[200,200],[-200,-200]]);
 assert(result.every(r=>!r.noise_within_maximum_cap));
});
test('invalid populations and incomplete attempts cannot generate a result',()=>{
 assert.throws(()=>intervals([[1,2]],[[1]],1,100));
 assert.throws(()=>intervals([[1,NaN]],[[1,2]],1,100));
 assert.throws(()=>intervals([[1,-1]],[[1,2]],1,100));
 assert.throws(()=>intervals([[1,2,3]],[[1,2,3]],2,100));
 assert.throws(()=>analyze({schema:'devmap/stage1-baseline-calibration/1',completed:false}));
});
