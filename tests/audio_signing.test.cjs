const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs/promises'),path=require('node:path'),os=require('node:os');
async function fixture(){const root=await fs.mkdtemp(path.join(os.tmpdir(),'tetorica-signing-'));await fs.mkdir(path.join(root,'lib'));
  for(const file of ['node','lib/audify.node','lib/opus.dylib'])await fs.writeFile(path.join(root,file),Buffer.from('cffaedfe00000000','hex'));
  await fs.writeFile(path.join(root,'readme.txt'),'not a binary');await fs.symlink('opus.dylib',path.join(root,'lib/alias.dylib'));return root;
}
test('nested native libraries are signed before Node with runtime/timestamp, and every signature is verified',async()=>{
  const {signAudioSidecar}=await import('../scripts/sign_audio_sidecar.mjs');const root=await fixture(),calls=[];
  try{await signAudioSidecar(root,{identity:'Developer ID test',arch:'arm64',run(command,args){calls.push({command,args});return 'arm64\n';}});
    const signed=calls.filter(call=>call.command==='codesign'&&call.args.includes('--sign'));
    assert.equal(signed.length,3);assert.equal(signed.at(-1).args.at(-1),path.join(root,'node'));
    for(const call of signed){assert.ok(call.args.includes('--timestamp'));assert.ok(call.args.includes('runtime'));assert.ok(!call.args.includes('--deep'));}
    assert.equal(signed.filter(call=>call.args.includes('--entitlements')).length,1);
    assert.equal(calls.filter(call=>call.command==='codesign'&&call.args.includes('--verify')).length,3);
    const plist=await fs.readFile(path.join(__dirname,'../src-tauri/AudioNode.entitlements.plist'),'utf8');assert.ok(plist.includes('allow-jit'));assert.ok(!plist.includes('get-task-allow'));assert.ok(!plist.includes('disable-library-validation'));
  }finally{await fs.rm(root,{recursive:true,force:true});}
});
test('architecture mismatch fails before signing; CI builds use consistent ad-hoc signatures',async()=>{
  const {signAudioSidecar}=await import('../scripts/sign_audio_sidecar.mjs');const root=await fixture(),calls=[];
  try{await assert.rejects(signAudioSidecar(root,{identity:'Developer ID test',arch:'x64',run(command,args){calls.push(command);return 'arm64\n';}}),/expected x86_64/);assert.ok(!calls.includes('codesign'));
    const unsigned=[];await signAudioSidecar(root,{identity:'',arch:'arm64',run(command,args){unsigned.push({command,args});return 'arm64\n';}});
    const signed=unsigned.filter(call=>call.command==='codesign'&&call.args.includes('--sign'));assert.equal(signed.length,3);
    for(const call of signed){assert.equal(call.args[call.args.indexOf('--sign')+1],'-');assert.ok(!call.args.includes('--timestamp'));assert.ok(!call.args.includes('runtime'));}
  }finally{await fs.rm(root,{recursive:true,force:true});}
});
