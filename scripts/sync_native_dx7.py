#!/usr/bin/env python3
"""Synchronize/check the Apache-2.0 MSFA snapshot and allocation-free Native hooks."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = 'w/tetorica-msfa-dx7-wasm/'
FILES = ['vendor/msfa/' + name for name in '''aligned_buf.h controllers.h dx7note.cc dx7note.h env.cc env.h exp2.cc exp2.h fm_core.cc fm_core.h fm_op_kernel.cc fm_op_kernel.h freqlut.cc freqlut.h lfo.cc lfo.h module.h patch.cc patch.h pitchenv.cc pitchenv.h resofilter.cc resofilter.h ringbuffer.cc ringbuffer.h sin.cc sin.h synth.h synth_unit.cc synth_unit.h COPYING UPSTREAM.md'''.split()]

def native_bytes(name, data):
    text = data.decode()
    if name.endswith('/synth.h'):
        # Native MSVC uses C++17, where the upstream #warning is an error.
        # Supply an actual fence on platforms without the upstream barrier.
        old = '''#ifndef SynthMemoryBarrier
#warning Memory barrier is not enabled
#define SynthMemoryBarrier()
#endif'''
        new = '''#ifndef SynthMemoryBarrier
// Native host: portable C++ fence, including MSVC in C++17 mode.
#include <atomic>
#define SynthMemoryBarrier() std::atomic_thread_fence(std::memory_order_seq_cst)
#endif'''
        if text.count(old) != 1:
            raise ValueError('MSFA memory barrier fallback changed; review the Native patch')
        text = text.replace(old, new)
    if name.endswith('/synth_unit.h'):
        text = text.replace('  void AllNotesOff();', '''  void AllNotesOff();
  // Native host: no callback allocation; return the fixed preallocated slot.
  int NativeNoteOn(int midi_note, int velocity);
  void NativeReleaseSlot(int slot);
  void NativePanic();''')
    if name.endswith('/synth_unit.cc'):
        text = text.replace('#include <string.h>', '#include <string.h>\n#include <new>')
        text += '''
// Native host extension: all 16 Dx7Note objects were allocated at construction.
int SynthUnit::NativeNoteOn(int midi_note, int velocity) {
  int slot = AllocateNote();
  if (slot < 0) return -1;
  lfo_.keydown();
  ActiveNote &note = active_note_[slot];
  note.midi_note = midi_note;
  note.voice_id = voice_id_;
  note.keydown = true;
  note.sustained = false;
  note.live = true;
  note.dx7_note->init(unpacked_patch_, midi_note, velocity);
  return slot;
}
void SynthUnit::NativeReleaseSlot(int slot) {
  if (slot < 0 || slot >= max_active_notes) return;
  ActiveNote &note = active_note_[slot];
  if (note.keydown) note.dx7_note->keyup();
  note.keydown = note.sustained = false;
}
void SynthUnit::NativePanic() {
  for (int slot = 0; slot < max_active_notes; ++slot) {
    active_note_[slot].midi_note = -1;
    active_note_[slot].voice_id = 0;
    active_note_[slot].keydown = active_note_[slot].sustained = active_note_[slot].live = false;
  }
  current_note_ = 0;
  input_buffer_index_ = 0;
  extra_buf_size_ = 0;
  memset(extra_buf_, 0, sizeof(extra_buf_));
  sustain_ = false;
  controllers_ = Controllers();
  controllers_.values_[kControllerPitch] = 0x2000;
  filter_.~ResoFilter();
  new (&filter_) ResoFilter();
  filter_control_[0] = 258847126;
  filter_control_[1] = filter_control_[2] = 0;
  lfo_ = Lfo();
  lfo_.reset(unpacked_patch_ + 137);
}
'''
    return text.encode()

def digest(data):
    return hashlib.sha256(data).hexdigest()

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--check', action='store_true')
    p.add_argument('--source', type=Path)
    args = p.parse_args()
    dest = Path(__file__).resolve().parents[1] / 'src-tauri/native-dx7'
    manifest = dest / 'sources.json'
    if args.check:
        entries = json.loads(manifest.read_text())['files']
        for name, hashes in entries.items():
            if digest((dest / Path(name).name).read_bytes()) != hashes['native']:
                raise SystemExit('Native DX7 snapshot changed: ' + name)
            if args.source:
                data = (args.source / ROOT / name).read_bytes()
                if digest(data) != hashes['source'] or digest(native_bytes(name, data)) != hashes['native']:
                    raise SystemExit('Native DX7 snapshot differs from source: ' + name)
        print(f'Native DX7 snapshot verified ({len(entries)} files)')
        return
    if not args.source:
        p.error('synchronizing requires --source /path/to/hello_ymfm')
    dest.mkdir(parents=True, exist_ok=True)
    entries = {}
    for name in FILES:
        data = (args.source / ROOT / name).read_bytes()
        native = native_bytes(name, data)
        (dest / Path(name).name).write_bytes(native)
        entries[name] = {'source': digest(data), 'native': digest(native)}
    manifest.write_text(json.dumps({'source': ROOT, 'license': 'Apache-2.0', 'files': entries}, indent=2) + '\n')
    print(f'Native DX7 snapshot synchronized ({len(entries)} files)')

if __name__ == '__main__':
    main()
