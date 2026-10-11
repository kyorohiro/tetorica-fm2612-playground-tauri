// Ephemeral SSH server: no user keys, agent, git CLI or ssh CLI.
const {Server,utils}=require('ssh2');const {createHash}=require('node:crypto');const {spawn}=require('node:child_process');
const keys=utils.generateKeyPairSync('ed25519');const hash=createHash('sha256').update(utils.parseKey(keys.private).getPublicSSH()).digest('base64').replace(/=+$/,'');
let accepted=0;
const server=new Server({hostKeys:[keys.private]},client=>{
 client.on('authentication',ctx=>{if(ctx.method==='publickey'&&ctx.key.algo==='ssh-ed25519'&&ctx.username==='git'){const key=utils.parseKey(ctx.key.algo+' '+ctx.key.data.toString('base64'));if(!ctx.signature||key.verify(ctx.blob,ctx.signature,ctx.hashAlgo)){accepted++;ctx.accept();}else ctx.reject(['publickey']);}else ctx.reject(['publickey']);});
 client.on('ready',()=>client.on('session',accept=>{const session=accept();session.on('exec',accept=>{const stream=accept();stream.end('0000');});}));
 client.on('error',()=>{});
});
server.listen(0,'127.0.0.1',()=>{
 const child=spawn('cargo',['test','--manifest-path','src-tauri/Cargo.toml','--locked','encrypted_app_key_authenticates_through_embedded_libssh2','--','--ignored','--nocapture'],{cwd:require('node:path').resolve(__dirname,'..'),env:{...process.env,TETORICA_TEST_SSH_PORT:String(server.address().port),TETORICA_TEST_SSH_FINGERPRINT:hash},stdio:'inherit'});
 child.on('exit',code=>{console.log(JSON.stringify({accepted,passed:code===0&&accepted>0}));server.close();process.exit(code||(!accepted?1:0));});
});
