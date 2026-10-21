//! Built-in FX: shared C DSP, prepared on control threads, callback-owned state.
//! The control registry retains every queued/active graph. It only frees graphs
//! after their callback references disappear, and enforces a fixed memory bound.
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

const MAX_GRAPHS: usize = 8;
const PARAMS: [&[(&str, f64, f64, f64)]; 13] = [
    &[("gain", 1., 0., 10.)],
    &[
        ("bass", 0., -12., 12.),
        ("mid", 0., -12., 12.),
        ("treble", 0., -12., 12.),
    ],
    &[
        ("threshold", 0.04, 0.0001, 1.),
        ("hysteresis", 6., 0., 24.),
        ("attack", 0.005, 0.0001, 0.2),
        ("hold", 0.05, 0., 1.),
        ("release", 0.1, 0.01, 2.),
    ],
    &[
        ("threshold", -24., -60., 0.),
        ("ratio", 4., 1., 20.),
        ("attack", 0.01, 0.0001, 0.2),
        ("release", 0.25, 0.01, 2.),
        ("makeup", 0., -12., 24.),
    ],
    &[
        ("mix", 0.2, 0., 1.),
        ("room", 0.7, 0., 0.98),
        ("damping", 0.4, 0., 1.),
        ("tone", 7200., 200., 20000.),
    ],
    &[
        ("cutoff", 1200., 20., 20000.),
        ("q", 0.707, 0.2, 12.),
        ("mode", 0., 0., 2.),
        ("mix", 1., 0., 1.),
    ],
    &[
        ("time", 0.25, 0.001, 2.),
        ("feedback", 0.35, 0., 0.9),
        ("mix", 0.3, 0., 1.),
    ],
    &[("drive", 4., 1., 20.), ("mix", 0.5, 0., 1.)],
    &[
        ("bitDepth", 8., 2., 16.),
        ("holdFrames", 6., 1., 480.),
        ("mix", 0.5, 0., 1.),
    ],
    &[
        ("cutoff", 800., 20., 10000.),
        ("depth", 1100., 0., 12000.),
        ("rate", 0.5, 0.03125, 16.),
        ("resonance", 0.707, 0.2, 8.),
        ("mix", 1., 0., 1.),
    ],
    &[
        ("time", 0.003, 0.001, 0.015),
        ("depth", 0.002, 0., 0.01),
        ("rate", 1., 0.03125, 16.),
        ("feedback", 0.3, 0., 0.9),
        ("mix", 0.5, 0., 1.),
    ],
    &[
        ("phase", 0.25, 0.03125, 16.),
        ("duty", 0.5, 0.05, 0.95),
        ("floor", 0., 0., 1.),
        ("mix", 1., 0., 1.),
    ],
    &[
        ("time", 0.02, 0.01, 0.04),
        ("depth", 0.005, 0., 0.01),
        ("rate", 1., 0.03125, 16.),
        ("mix", 0.5, 0., 1.),
    ],
];
fn kind(name: &str) -> Result<u8, String> {
    [
        "gain",
        "eq",
        "gate",
        "compressor",
        "reverb",
        "filter",
        "delay",
        "distortion",
        "bitcrusher",
        "wobble",
        "flanger",
        "slicer",
        "chorus",
    ]
    .iter()
    .position(|candidate| *candidate == name)
    .map(|index| index as u8 + 2)
    .ok_or_else(|| format!("Unsupported Native Genesis FX: {name}"))
}
fn beat(value: f64) -> Result<f64, String> {
    if value.is_finite() && value > 0. && value <= 3600. {
        Ok(value)
    } else {
        Err("Invalid native FX beat duration".into())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct FxUnit {
    #[serde(rename = "type")]
    kind: String,
    slot: u8,
    values: BTreeMap<String, f64>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct FxNode {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    slot: u8,
    #[serde(default)]
    children: Vec<FxNode>,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum FxMessage {
    Create {
        unit: FxUnit,
        #[serde(rename = "beatSeconds")]
        beat_seconds: f64,
    },
    Chain {
        children: Vec<FxNode>,
        units: Vec<FxUnit>,
        #[serde(rename = "beatSeconds")]
        beat_seconds: f64,
    },
    Parameter {
        unit: FxUnit,
        key: String,
        value: f64,
        seconds: f64,
        #[serde(rename = "beatSeconds")]
        beat_seconds: f64,
    },
    Tempo {
        #[serde(rename = "beatSeconds")]
        beat_seconds: f64,
    },
    Clear,
    Reset,
}
#[repr(C)]
struct CNode {
    kind: i32,
    slot: i32,
    count: i32,
    children: [i32; 32],
}
#[repr(C)]
struct CUnit {
    kind: i32,
    slot: i32,
    values: [f64; 8],
}
extern "C" {
    fn native_genesis_fx_create(
        rate: u32,
        nodes: *const CNode,
        node_count: u32,
        units: *const CUnit,
        unit_count: u32,
        beat: f64,
    ) -> *mut c_void;
    fn native_genesis_fx_destroy(ptr: *mut c_void);
    fn native_genesis_fx_sample(ptr: *mut c_void, stereo: *mut f32);
    fn native_genesis_fx_parameter(
        ptr: *mut c_void,
        kind: i32,
        slot: i32,
        key: i32,
        value: f64,
        frames: u32,
    );
    fn native_genesis_fx_tempo(ptr: *mut c_void, beat: f64);
}
pub struct FxGraph {
    pointer: *mut c_void,
    claimed: AtomicBool,
}
// An atomic lease permits one renderer to mutate DSP. Other Arc references are
// lifetime guards. Even a duplicated internal Install cannot share mutable DSP.
unsafe impl Send for FxGraph {}
unsafe impl Sync for FxGraph {}
impl std::fmt::Debug for FxGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FxGraph")
    }
}
impl Drop for FxGraph {
    fn drop(&mut self) {
        unsafe {
            native_genesis_fx_destroy(self.pointer);
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct FxParameter {
    kind: u8,
    slot: u8,
    key: u8,
    value: f64,
    frames: u32,
}
#[derive(Clone, Debug)]
pub enum FxAction {
    Install(Option<Arc<FxGraph>>),
    Parameter(FxParameter),
    Tempo(f64),
}

#[derive(Clone, Default)]
struct Model {
    units: BTreeMap<(u8, u8), [f64; 8]>,
    children: Vec<FxNode>,
    beat: f64,
}
#[derive(Default)]
pub struct FxController {
    model: Model,
    registry: Vec<Arc<FxGraph>>,
}
impl FxController {
    /// Called only from control/teardown threads, never a device callback.
    pub fn collect(&mut self) {
        self.registry.retain(|graph| Arc::strong_count(graph) > 1);
    }
    pub fn command(&mut self, message: FxMessage, rate: u32) -> Result<Option<FxAction>, String> {
        self.collect();
        let mut model = self.model.clone();
        let action = match message {
            FxMessage::Create { unit, beat_seconds } => {
                model.beat = beat(beat_seconds)?;
                let (id, values) = validate_unit(&unit)?;
                model.units.insert(id, values);
                None
            }
            FxMessage::Chain {
                children,
                units,
                beat_seconds,
            } => {
                model.beat = beat(beat_seconds)?;
                if units.len() > 104 {
                    return Err("Native FX unit limit exceeded".into());
                }
                let mut next = BTreeMap::new();
                for unit in units {
                    let (id, values) = validate_unit(&unit)?;
                    if next.insert(id, values).is_some() {
                        return Err("Duplicate native FX unit".into());
                    }
                }
                model.units = next;
                model.children = children;
                Some(FxAction::Install(self.prepare(&model, rate)?))
            }
            FxMessage::Parameter {
                unit,
                key,
                value,
                seconds,
                beat_seconds,
            } => {
                model.beat = beat(beat_seconds)?;
                let (id, _) = validate_unit(&unit)?;
                let specs = PARAMS[(id.0 - 2) as usize];
                let index = specs
                    .iter()
                    .position(|spec| spec.0 == key)
                    .ok_or("Unknown native FX parameter")?;
                let (_, _, low, high) = specs[index];
                if !value.is_finite()
                    || value < low
                    || value > high
                    || !seconds.is_finite()
                    || seconds < 0.
                    || seconds * rate as f64 > u32::MAX as f64
                {
                    return Err("Invalid native FX parameter value or ramp".into());
                }
                let values = model
                    .units
                    .get_mut(&id)
                    .ok_or("Native FX has been disposed")?;
                values[index] = value;
                if id.0 == 6 && index == 3 {
                    values[2] = 1. - (value - 200.) / 19800.;
                }
                Some(FxAction::Parameter(FxParameter {
                    kind: id.0,
                    slot: id.1,
                    key: index as u8,
                    value,
                    frames: (seconds * rate as f64).round() as u32,
                }))
            }
            FxMessage::Tempo { beat_seconds } => {
                model.beat = beat(beat_seconds)?;
                Some(FxAction::Tempo(model.beat))
            }
            FxMessage::Clear => Some(FxAction::Install(self.prepare(&model, rate)?)),
            FxMessage::Reset => {
                model = Model::default();
                Some(FxAction::Install(None))
            }
        };
        self.model = model;
        Ok(action)
    }
    fn prepare(&mut self, model: &Model, rate: u32) -> Result<Option<Arc<FxGraph>>, String> {
        if model.children.is_empty() {
            return Ok(None);
        }
        if self.registry.len() >= MAX_GRAPHS {
            return Err(
                "Native FX graph changes are arriving too quickly (maximum 8 pending graphs)"
                    .into(),
            );
        }
        let mut nodes = Vec::with_capacity(32);
        let mut used = [[false; 8]; 15];
        for child in &model.children {
            add_node(child, &mut nodes, &mut used, model, 0)?;
        }
        let mut root = CNode {
            kind: 0,
            slot: 0,
            count: model.children.len() as i32,
            children: [0; 32],
        };
        // Each subtree was appended postorder; recover roots from bounded sizes.
        let mut offset = 0;
        for (index, child) in model.children.iter().enumerate() {
            offset += tree_size(child);
            root.children[index] = (offset - 1) as i32;
        }
        if nodes.len() >= 32 {
            return Err("Native FX maximum is 32 graph nodes".into());
        }
        nodes.push(root);
        let units: Vec<_> = model
            .units
            .iter()
            .map(|(&(kind, slot), values)| CUnit {
                kind: kind as i32,
                slot: slot as i32,
                values: *values,
            })
            .collect();
        let ptr = unsafe {
            native_genesis_fx_create(
                rate,
                nodes.as_ptr(),
                nodes.len() as u32,
                units.as_ptr(),
                units.len() as u32,
                model.beat,
            )
        };
        if ptr.is_null() {
            return Err("Could not prepare native FX graph".into());
        }
        let graph = Arc::new(FxGraph {
            pointer: ptr,
            claimed: AtomicBool::new(false),
        });
        self.registry.push(graph.clone());
        Ok(Some(graph))
    }
}
fn tree_size(node: &FxNode) -> usize {
    1 + node.children.iter().map(tree_size).sum::<usize>()
}
fn add_node(
    node: &FxNode,
    nodes: &mut Vec<CNode>,
    used: &mut [[bool; 8]; 15],
    model: &Model,
    depth: usize,
) -> Result<i32, String> {
    if depth >= 32 || nodes.len() >= 31 || node.children.len() > 31 || node.slot >= 8 {
        return Err("Invalid native FX graph capacity or slot".into());
    }
    let kind = match node.kind.as_str() {
        "chain" => 0,
        "parallel" => 1,
        name => kind(name)?,
    };
    if kind == 1 && node.children.is_empty() {
        return Err("Empty native FX parallel".into());
    }
    if kind > 1
        && (!node.children.is_empty()
            || used[kind as usize][node.slot as usize]
            || !model.units.contains_key(&(kind, node.slot)))
    {
        return Err("Missing or reused native FX unit".into());
    }
    if kind > 1 {
        used[kind as usize][node.slot as usize] = true;
    }
    let mut value = CNode {
        kind: kind as i32,
        slot: node.slot as i32,
        count: node.children.len() as i32,
        children: [0; 32],
    };
    for (index, child) in node.children.iter().enumerate() {
        value.children[index] = add_node(child, nodes, used, model, depth + 1)?;
    }
    if nodes.len() >= 31 {
        return Err("Native FX maximum is 32 graph nodes".into());
    }
    let id = nodes.len() as i32;
    nodes.push(value);
    Ok(id)
}
fn validate_unit(unit: &FxUnit) -> Result<((u8, u8), [f64; 8]), String> {
    let kind = kind(&unit.kind)?;
    if unit.slot >= 8 {
        return Err("Invalid native FX slot".into());
    }
    let specs = PARAMS[(kind - 2) as usize];
    let mut values = [0.; 8];
    for (index, (_, default, _, _)) in specs.iter().enumerate() {
        values[index] = *default;
    }
    for (key, value) in &unit.values {
        let index = specs
            .iter()
            .position(|spec| spec.0 == key)
            .ok_or("Unsupported native FX unit option")?;
        let (_, _, low, high) = specs[index];
        if !value.is_finite()
            || *value < low
            || *value > high
            || kind == 7 && index == 2 && value.fract() != 0.
        {
            return Err("Invalid native FX unit value".into());
        }
        values[index] = *value;
    }
    Ok(((kind, unit.slot), values))
}

pub struct FxRenderer {
    current: Option<Arc<FxGraph>>,
    previous: Option<Arc<FxGraph>>,
    remaining: u32,
    fade: u32,
}
impl FxRenderer {
    pub fn new(rate: u32) -> Self {
        Self {
            current: None,
            previous: None,
            remaining: 0,
            fade: (rate / 200).max(1),
        }
    }
    pub fn apply(&mut self, action: FxAction) {
        match action {
            FxAction::Install(graph) => {
                if graph.as_ref().is_some_and(|graph| {
                    graph
                        .claimed
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                }) {
                    return;
                }
                if self.remaining == self.fade {
                    // setChain([]) + clear(), or several graph edits at one
                    // frame, have not rendered the intermediate target. Keep
                    // the actual previous output as the fade source.
                    Self::release(&mut self.current);
                } else {
                    Self::release(&mut self.previous);
                    self.previous = self.current.take();
                }
                self.current = graph;
                self.remaining = self.fade;
            }
            FxAction::Parameter(p) => {
                for graph in self.current.iter().chain(self.previous.iter()) {
                    unsafe {
                        native_genesis_fx_parameter(
                            graph.pointer,
                            p.kind as i32,
                            p.slot as i32,
                            p.key as i32,
                            p.value,
                            p.frames,
                        );
                    }
                }
            }
            FxAction::Tempo(beat) => {
                for graph in self.current.iter().chain(self.previous.iter()) {
                    unsafe {
                        native_genesis_fx_tempo(graph.pointer, beat);
                    }
                }
            }
        }
    }
    pub fn process(&mut self, stereo: &mut [f32; 2]) {
        let mut old = *stereo;
        if let Some(graph) = &self.current {
            unsafe {
                native_genesis_fx_sample(graph.pointer, stereo.as_mut_ptr());
            }
        }
        if self.remaining > 0 {
            if let Some(graph) = &self.previous {
                unsafe {
                    native_genesis_fx_sample(graph.pointer, old.as_mut_ptr());
                }
            }
            let mix = 1. - self.remaining as f32 / self.fade as f32;
            for c in 0..2 {
                stereo[c] = old[c] * (1. - mix) + stereo[c] * mix;
            }
            self.remaining -= 1;
            if self.remaining == 0 {
                Self::release(&mut self.previous);
            }
        }
    }
    /// Clears references only. Graph memory is reclaimed later by collect().
    pub fn stop(&mut self) {
        Self::release(&mut self.current);
        Self::release(&mut self.previous);
        self.remaining = 0;
    }
    fn release(slot: &mut Option<Arc<FxGraph>>) {
        if let Some(graph) = slot.take() {
            graph.claimed.store(false, Ordering::Release);
        }
    }
}
impl Drop for FxRenderer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(value: serde_json::Value) -> FxMessage {
        serde_json::from_value(value).unwrap()
    }
    fn gain(value: f64) -> serde_json::Value {
        serde_json::json!({"type":"gain","slot":0,"values":{"gain":value}})
    }
    fn chain(value: f64) -> FxMessage {
        message(
            serde_json::json!({"op":"chain","children":[{"type":"gain","slot":0}],"units":[gain(value)],"beatSeconds":0.5}),
        )
    }
    #[test]
    fn gain_stereo_ramp_and_control_side_reclamation() {
        let mut control = FxController::default();
        let mut render = FxRenderer::new(48000);
        render.apply(control.command(chain(0.25), 48000).unwrap().unwrap());
        for _ in 0..1000 {
            let mut pcm = [1., -0.5];
            render.process(&mut pcm);
        }
        let mut pcm = [1., -0.5];
        render.process(&mut pcm);
        assert!((pcm[0] - 0.25).abs() < 1e-6);
        assert!((pcm[1] + 0.125).abs() < 1e-6);
        assert_eq!(control.registry.len(), 1);
        render.stop();
        assert_eq!(control.registry.len(), 1);
        control.collect();
        assert_eq!(control.registry.len(), 0);
    }
    #[test]
    fn graph_changes_are_bounded_without_callback_memory_destruction() {
        let mut control = FxController::default();
        let mut queued = Vec::new();
        for _ in 0..8 {
            queued.push(control.command(chain(0.5), 48000).unwrap().unwrap());
        }
        assert!(control
            .command(chain(0.5), 48000)
            .unwrap_err()
            .contains("maximum 8"));
        queued.clear();
        control.collect();
        assert!(control.registry.is_empty());
        assert!(control.command(chain(0.5), 48000).is_ok());
    }
    #[test]
    fn graph_cannot_be_mutated_by_two_renderers() {
        let mut control = FxController::default();
        let action = control.command(chain(0.25), 48000).unwrap().unwrap();
        let mut first = FxRenderer::new(48000);
        let mut second = FxRenderer::new(48000);
        first.apply(action.clone());
        second.apply(action.clone());
        for _ in 0..1000 {
            first.process(&mut [1., 1.]);
        }
        let mut pcm = [1., 1.];
        second.process(&mut pcm);
        assert_eq!(pcm, [1., 1.]);
        first.stop();
        second.apply(action);
        for _ in 0..1000 {
            second.process(&mut [1., 1.]);
        }
        let mut pcm = [1., 1.];
        second.process(&mut pcm);
        assert!((pcm[0] - 0.25).abs() < 1e-6);
    }
    #[test]
    fn same_frame_clear_commands_keep_the_previous_output_fade() {
        let mut control = FxController::default();
        let mut render = FxRenderer::new(48000);
        render.apply(control.command(chain(0.25), 48000).unwrap().unwrap());
        for _ in 0..1000 {
            render.process(&mut [1., 1.]);
        }
        render.apply(FxAction::Install(None));
        render.apply(FxAction::Install(None));
        let mut first = [1., 1.];
        render.process(&mut first);
        assert!((first[0] - 0.25).abs() < 1e-6);
        for _ in 0..300 {
            render.process(&mut [1., 1.]);
        }
        let mut last = [1., 1.];
        render.process(&mut last);
        assert_eq!(last, [1., 1.]);
    }
    #[test]
    fn invalid_graph_and_parameter_leave_previous_model_intact() {
        let mut control = FxController::default();
        let _active = control.command(chain(0.5), 48000).unwrap();
        let invalid = message(
            serde_json::json!({"op":"chain","children":[{"type":"gain"},{"type":"gain"}],"units":[gain(0.5)],"beatSeconds":0.5}),
        );
        assert!(control.command(invalid, 48000).is_err());
        assert_eq!(control.model.children.len(), 1);
        let invalid = message(
            serde_json::json!({"op":"parameter","unit":gain(0.5),"key":"gain","value":11,"seconds":0,"beatSeconds":0.5}),
        );
        assert!(control.command(invalid, 48000).is_err());
        assert_eq!(control.model.units[&(2, 0)][0], 0.5);
    }
    #[test]
    fn all_fx_build_and_remain_finite_in_parallel() {
        let names = [
            "gain",
            "eq",
            "gate",
            "compressor",
            "reverb",
            "filter",
            "delay",
            "distortion",
            "bitcrusher",
            "wobble",
            "flanger",
            "slicer",
            "chorus",
        ];
        for name in names {
            let mut control = FxController::default();
            let mut render = FxRenderer::new(48000);
            let command = message(
                serde_json::json!({"op":"chain","children":[{"type":"parallel","children":[{"type":"chain"},{"type":name,"slot":0}]}],"units":[{"type":name,"slot":0,"values":{}}],"beatSeconds":0.5}),
            );
            render.apply(control.command(command, 48000).unwrap().unwrap());
            for frame in 0..2000 {
                let mut pcm = [if frame == 0 { 0.5 } else { 0. }, 0.];
                render.process(&mut pcm);
                assert!(pcm.into_iter().all(f32::is_finite));
            }
            render.stop();
            control.collect();
            assert!(control.registry.is_empty());
        }
    }
}
