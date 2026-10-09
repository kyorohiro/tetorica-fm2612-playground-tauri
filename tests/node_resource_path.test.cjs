const test=require('node:test'),assert=require('node:assert/strict');
test('Node launch paths strip drive and UNC namespace prefixes and preserve spaces and Unicode',async()=>{
  const {nodeResourcePath}=await import('../scripts/node_resource_path.mjs');
  assert.equal(nodeResourcePath('\\\\?\\C:\\Program Files\\Tetorica 音楽'),'C:\\Program Files\\Tetorica 音楽');
  assert.equal(nodeResourcePath('\\\\?\\UNC\\server\\share\\音楽'),'\\\\server\\share\\音楽');
  for(const ordinary of ['C:\\Tetorica','\\\\server\\share','/tmp/Tetorica 音楽'])assert.equal(nodeResourcePath(ordinary),ordinary);
});
