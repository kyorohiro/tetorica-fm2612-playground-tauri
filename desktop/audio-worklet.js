class DesktopAudioOutput extends AudioWorkletProcessor {
  constructor(options){
    super();this.frames=options.processorOptions.frames;this.offset=0;this.credits=0;
    this.samples=new Float32Array(this.frames*2);
    this.step=sampleRate/(options.processorOptions.outputSampleRate??sampleRate);
    this.inputFrame=0;this.nextOutput=0;this.previous=[0,0];
    this.port.onmessage=event=>{
      if(event.data.type==='credit')this.credits=Math.min(4,this.credits+event.data.count);
      if(event.data.type==='stop'){this.credits=0;this.offset=0;}
    };
  }
  process(inputs,outputs){
    for(const channel of outputs[0]??[])channel.fill(0);
    const channels=inputs[0];if(!channels?.[0])return true;
    // No credits: discard current audio, never retain an ever-growing delayed stream.
    if(!this.credits)this.offset=0;
    for(let i=0;i<channels[0].length;i++){
      const left=channels[0][i],right=(channels[1]??channels[0])[i];
      while(this.nextOutput<=this.inputFrame){
        if(this.credits){
          const fraction=this.nextOutput-(this.inputFrame-1);
          this.samples[this.offset*2]=this.previous[0]+(left-this.previous[0])*fraction;
          this.samples[this.offset*2+1]=this.previous[1]+(right-this.previous[1])*fraction;this.offset++;
          if(this.offset===this.frames){
            this.credits--;this.port.postMessage(this.samples.buffer,[this.samples.buffer]);
            this.samples=new Float32Array(this.frames*2);this.offset=0;
          }
        }
        this.nextOutput+=this.step;
      }
      this.previous[0]=left;this.previous[1]=right;this.inputFrame++;
    }
    return true;
  }
}
registerProcessor('tetorica-desktop-output',DesktopAudioOutput);
