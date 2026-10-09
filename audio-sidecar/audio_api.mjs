/** Explicit Windows API avoids probing third-party ASIO drivers while listing speakers. */
export function outputApi(audify,platform=process.platform){
  if(platform!=='win32')return undefined;
  const api=audify.RtAudioApi?.WINDOWS_WASAPI;
  if(!Number.isInteger(api))throw Error('This Audify build does not expose Windows WASAPI');
  return api;
}
