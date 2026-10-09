const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs/promises'),path=require('node:path'),os=require('node:os');
test('Node LICENSE is found for Unix bin layouts and Windows adjacent-executable layouts',async()=>{
  const {findNodeLicense}=await import('../scripts/audio_node_license.mjs');const root=await fs.mkdtemp(path.join(os.tmpdir(),'tetorica-node-license-'));
  try{
    const unix=path.join(root,'unix');await fs.mkdir(path.join(unix,'bin'),{recursive:true});await fs.writeFile(path.join(unix,'LICENSE'),'Node license');
    assert.equal(await findNodeLicense(path.join(unix,'bin/node')),path.join(unix,'LICENSE'));
    const windows=path.join(root,'windows');await fs.mkdir(windows);await fs.writeFile(path.join(windows,'LICENSE'),'Node license');
    assert.equal(await findNodeLicense(path.join(windows,'node.exe')),path.join(windows,'LICENSE'));
    const custom=path.join(root,'custom.txt');await fs.writeFile(custom,'custom');assert.equal(await findNodeLicense(path.join(root,'other/node'),custom),custom);
    await assert.rejects(findNodeLicense(path.join(root,'missing/node')),/LICENSE was not found/);
    await assert.rejects(findNodeLicense(path.join(unix,'bin/node'),path.join(root,'invalid-override')),/LICENSE was not found/);
  }finally{await fs.rm(root,{recursive:true,force:true});}
});
