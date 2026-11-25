//! Fluid Speech Mini's encoder with integer maths for its 4-bit layers.
//!
//! Mini stores its encoder weights in 2 and 4 bits (`MatMulNBits`) and ONNX
//! Runtime multiplies them in 32-bit floats. Its integer path (MLAS
//! `CompInt8`, `accuracy_level` 4) multiplies 4-bit layers in 8-bit integers
//! instead, which recent CPUs do several times faster per instruction (Intel
//! AVX-VNNI from 12th gen, AVX-512 VNNI on Ice and Tiger Lake, Arm dot
//! product). That path needs each layer's scales as a constant per block and
//! whole-number zero points, but Mini keeps one scale and one fractional zero
//! point per row and expands them while it runs, so the level alone changes
//! nothing (measured: identical output).
//!
//! So each 4-bit layer `y = x · W`, with `W[n] = s[n] · (q - z[n])`, is
//! rewritten exactly as
//!
//! `y = MatMulNBits(x; q, scales s[n] per block, zero point 8) + sum(x) · c`
//!
//! with `c[n] = s[n] · (8 - z[n])`: the integer layer with the format's
//! default zero point, plus a per-row correction in floats. 2-bit layers
//! have no integer path and stay as they are. The copy holds the new
//! scales and corrections (about 12 MB); the weights stay in
//! `encoder.onnx.data`, shared with the usual encoder.
//!
//! Measured on Apple M5 Max (Python ONNX Runtime 1.32): encoder 390 → 182 ms
//! for 11 s of audio. Through the app's ONNX Runtime 1.28: 3-23 % faster
//! decodes, the same words on 818 words of speech. Emulated AVX2 without
//! VNNI: 5 %. It depends on the CPU, so the speed test times both and keeps
//! this only where it wins.
//!
//! The file is ONNX's protobuf. Only what the rewrite needs is parsed; every
//! other byte is copied as it was.

use crate::error::{CoreError, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The copy's file name, next to `encoder.onnx`. The version changes with
/// the rewrite, so an older copy is never used.
pub const FILE: &str = "encoder.int8math-v1.onnx";

/// `accuracy_level` 4: int8 compute (MLAS `CompInt8`).
const INT8_LEVEL: u64 = 4;
/// The 4-bit format's zero point when none is given.
const DEFAULT_ZERO_POINT: f32 = 8.0;

/// The integer-maths encoder in `dir`, written from `encoder.onnx` the first
/// time. Returns its path.
pub fn ensure(dir: &Path) -> Result<PathBuf> {
    let out = dir.join(FILE);
    if out.is_file() {
        return Ok(out);
    }
    let source = std::fs::read(dir.join("encoder.onnx"))?;
    let (rewritten, layers) = rewrite_model(&source, dir)?;
    if layers == 0 {
        return Err(CoreError::Transcription(
            "Express encoder has no 4-bit layers to switch to integer maths".into(),
        ));
    }
    let tmp = dir.join(format!("{FILE}.partial"));
    std::fs::write(&tmp, rewritten)?;
    std::fs::rename(&tmp, &out)?;
    tracing::info!("wrote the integer-maths encoder ({layers} layers)");
    Ok(out)
}

// MARK: - Protobuf

const VARINT: u8 = 0;
const I64: u8 = 1;
const LEN: u8 = 2;
const I32: u8 = 5;

/// One field: its number, wire type, the whole encoded field and its value
/// (the payload of a length-delimited field, the raw bytes otherwise).
struct Field<'a> {
    num: u64,
    wire: u8,
    raw: &'a [u8],
    value: &'a [u8],
}

fn fields(msg: &[u8]) -> Result<Vec<Field<'_>>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < msg.len() {
        let start = at;
        let (key, after_key) = read_varint(msg, at)?;
        let (num, wire) = (key >> 3, (key & 7) as u8);
        let (v_start, end) = match wire {
            VARINT => (after_key, read_varint(msg, after_key)?.1),
            I64 => (after_key, after_key + 8),
            I32 => (after_key, after_key + 4),
            LEN => {
                let (len, s) = read_varint(msg, after_key)?;
                (s, s.checked_add(len as usize).ok_or_else(bad)?)
            }
            _ => return Err(bad()),
        };
        if end > msg.len() || num == 0 {
            return Err(bad());
        }
        out.push(Field {
            num,
            wire,
            raw: &msg[start..end],
            value: &msg[v_start..end],
        });
        at = end;
    }
    Ok(out)
}

fn read_varint(msg: &[u8], mut at: usize) -> Result<(u64, usize)> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *msg.get(at).ok_or_else(bad)?;
        at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, at));
        }
    }
    Err(bad())
}

fn bad() -> CoreError {
    CoreError::Transcription("Express encoder file isn't a model ONNX can read".into())
}

fn text(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| bad())
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_varint_field(out: &mut Vec<u8>, field: u64, v: u64) {
    put_varint(out, field << 3 | u64::from(VARINT));
    put_varint(out, v);
}

fn put_bytes(out: &mut Vec<u8>, field: u64, bytes: &[u8]) {
    put_varint(out, field << 3 | u64::from(LEN));
    put_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

// MARK: - ONNX messages

/// NodeProto: input 1, output 2, name 3, op_type 4, attribute 5, domain 7.
struct Node<'a> {
    inputs: Vec<&'a str>,
    outputs: Vec<&'a str>,
    name: &'a str,
    op: &'a str,
    /// AttributeProto messages: name 1, i 3, type 20.
    attrs: Vec<&'a [u8]>,
}

impl<'a> Node<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut node = Node {
            inputs: Vec::new(),
            outputs: Vec::new(),
            name: "",
            op: "",
            attrs: Vec::new(),
        };
        for f in fields(bytes)? {
            match (f.num, f.wire) {
                (1, LEN) => node.inputs.push(text(f.value)?),
                (2, LEN) => node.outputs.push(text(f.value)?),
                (3, LEN) => node.name = text(f.value)?,
                (4, LEN) => node.op = text(f.value)?,
                (5, LEN) => node.attrs.push(f.value),
                _ => {}
            }
        }
        Ok(node)
    }

    fn int(&self, name: &str) -> Result<Option<u64>> {
        for attr in &self.attrs {
            let mut is = false;
            let mut value = None;
            for f in fields(attr)? {
                match (f.num, f.wire) {
                    (1, LEN) => is = f.value == name.as_bytes(),
                    (3, VARINT) => value = Some(read_varint(f.value, 0)?.0),
                    _ => {}
                }
            }
            if is {
                return Ok(value);
            }
        }
        Ok(None)
    }
}

fn int_attr(name: &str, v: u64) -> Vec<u8> {
    let mut a = Vec::new();
    put_bytes(&mut a, 1, name.as_bytes());
    put_varint_field(&mut a, 3, v);
    put_varint_field(&mut a, 20, 2); // AttributeType INT
    a
}

fn node_bytes(
    inputs: &[&str],
    outputs: &[&str],
    name: &str,
    op: &str,
    domain: &str,
    attrs: &[Vec<u8>],
) -> Vec<u8> {
    let mut n = Vec::new();
    for i in inputs {
        put_bytes(&mut n, 1, i.as_bytes());
    }
    for o in outputs {
        put_bytes(&mut n, 2, o.as_bytes());
    }
    put_bytes(&mut n, 3, name.as_bytes());
    put_bytes(&mut n, 4, op.as_bytes());
    for a in attrs {
        put_bytes(&mut n, 5, a);
    }
    if !domain.is_empty() {
        put_bytes(&mut n, 7, domain.as_bytes());
    }
    n
}

/// TensorProto: dims 1, data_type 2, float_data 4, name 8, raw_data 9,
/// external_data 13 (key 1, value 2).
fn tensor_name(bytes: &[u8]) -> Result<&str> {
    for f in fields(bytes)? {
        if f.num == 8 && f.wire == LEN {
            return text(f.value);
        }
    }
    Err(bad())
}

/// A float tensor's values, read from the model directory when stored
/// outside the graph file.
fn tensor_f32(bytes: &[u8], dir: &Path) -> Result<Vec<f32>> {
    let (mut data_type, mut raw, mut floats) = (0, None, Vec::new());
    let mut external: HashMap<&str, &str> = HashMap::new();
    for f in fields(bytes)? {
        match (f.num, f.wire) {
            (2, VARINT) => data_type = read_varint(f.value, 0)?.0,
            (9, LEN) => raw = Some(f.value.to_vec()),
            (4, LEN) => floats.extend_from_slice(f.value),
            (4, I32) => floats.extend_from_slice(f.value),
            (13, LEN) => {
                let (mut k, mut v) = ("", "");
                for e in fields(f.value)? {
                    match e.num {
                        1 => k = text(e.value)?,
                        2 => v = text(e.value)?,
                        _ => {}
                    }
                }
                external.insert(k, v);
            }
            _ => {}
        }
    }
    if data_type != 1 {
        return Err(bad());
    }
    let bytes = if let Some(raw) = raw {
        raw
    } else if let Some(location) = external.get("location") {
        let path = dir.join(location);
        // The data file must sit in the model's own folder.
        if path.parent() != Some(dir) {
            return Err(bad());
        }
        let offset: u64 = external
            .get("offset")
            .map_or(Ok(0), |o| o.parse())
            .map_err(|_| bad())?;
        let length: usize = external
            .get("length")
            .ok_or_else(bad)?
            .parse()
            .map_err(|_| bad())?;
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; length];
        file.read_exact(&mut buf)?;
        buf
    } else {
        floats
    };
    if bytes.len() % 4 != 0 {
        return Err(bad());
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

fn tensor_bytes(name: &str, data_type: u64, dims: &[u64], raw: &[u8]) -> Vec<u8> {
    let mut t = Vec::new();
    for &d in dims {
        put_varint_field(&mut t, 1, d);
    }
    put_varint_field(&mut t, 2, data_type);
    put_bytes(&mut t, 8, name.as_bytes());
    put_bytes(&mut t, 9, raw);
    t
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

// MARK: - The rewrite

/// A 4-bit layer's replacement: its nodes and new initializers.
struct Replacement {
    nodes: Vec<Vec<u8>>,
    initializers: Vec<Vec<u8>>,
    /// The nodes that expanded its scales and zero points, now unused.
    expands: [usize; 2],
}

/// `model` (an ONNX ModelProto in `dir`) with its 4-bit layers rewritten,
/// and how many were.
fn rewrite_model(model: &[u8], dir: &Path) -> Result<(Vec<u8>, usize)> {
    let model_fields = fields(model)?;
    let graph = model_fields
        .iter()
        .find(|f| f.num == 7 && f.wire == LEN)
        .ok_or_else(bad)?;
    let graph_fields = fields(graph.value)?;
    let nodes: Vec<Node> = graph_fields
        .iter()
        .filter(|f| f.num == 1 && f.wire == LEN)
        .map(|f| Node::parse(f.value))
        .collect::<Result<_>>()?;
    let mut initializers: HashMap<&str, &[u8]> = HashMap::new();
    for f in graph_fields.iter().filter(|f| f.num == 5 && f.wire == LEN) {
        initializers.insert(tensor_name(f.value)?, f.value);
    }
    let mut producer: HashMap<&str, usize> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        for o in &n.outputs {
            producer.insert(o, i);
        }
    }

    let mut replaced: HashMap<usize, Replacement> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        if let Some(r) = replace_layer(n, &nodes, &producer, &initializers, dir)? {
            replaced.insert(i, r);
        }
    }

    // Expand nodes only the rewritten layers read are dropped.
    let mut still_read: HashSet<&str> = HashSet::new();
    for (i, n) in nodes.iter().enumerate() {
        if !replaced.contains_key(&i) {
            still_read.extend(n.inputs.iter().copied());
        }
    }
    let dropped: HashSet<usize> = replaced
        .values()
        .flat_map(|r| r.expands)
        .filter(|&e| nodes[e].outputs.iter().all(|o| !still_read.contains(o)))
        .collect();

    let mut new_graph = Vec::with_capacity(graph.value.len() + (16 << 20));
    let mut node_index = 0;
    for f in &graph_fields {
        if f.num == 1 && f.wire == LEN {
            let i = node_index;
            node_index += 1;
            if let Some(r) = replaced.get(&i) {
                for n in &r.nodes {
                    put_bytes(&mut new_graph, 1, n);
                }
                continue;
            }
            if dropped.contains(&i) {
                continue;
            }
        }
        new_graph.extend_from_slice(f.raw);
    }
    let mut layers: Vec<&usize> = replaced.keys().collect();
    layers.sort();
    for i in layers {
        for t in &replaced[i].initializers {
            put_bytes(&mut new_graph, 5, t);
        }
    }

    let mut out = Vec::with_capacity(new_graph.len() + 1024);
    for f in &model_fields {
        if f.num == 7 && f.wire == LEN {
            put_bytes(&mut out, 7, &new_graph);
        } else {
            out.extend_from_slice(f.raw);
        }
    }
    Ok((out, replaced.len()))
}

/// The replacement for node `n` when it is a 4-bit layer whose scales and
/// zero points are per-row constants expanded to blocks; `None` otherwise.
fn replace_layer(
    n: &Node,
    nodes: &[Node],
    producer: &HashMap<&str, usize>,
    initializers: &HashMap<&str, &[u8]>,
    dir: &Path,
) -> Result<Option<Replacement>> {
    if n.op != "MatMulNBits" || n.int("bits")? != Some(4) || n.inputs.len() != 4 {
        return Ok(None);
    }
    let (Some(k), Some(rows), Some(block)) = (n.int("K")?, n.int("N")?, n.int("block_size")?)
    else {
        return Ok(None);
    };
    if block == 0 || k % block != 0 {
        return Ok(None);
    }
    let (rows, blocks) = (rows as usize, (k / block) as usize);
    // Each of scales and zero points: Expand(<a [N, 1] float constant>, shape).
    let row_of = |name: &str| -> Result<Option<(usize, Vec<f32>)>> {
        let Some(&p) = producer.get(name) else {
            return Ok(None);
        };
        let e = &nodes[p];
        if e.op != "Expand" || e.outputs.len() != 1 {
            return Ok(None);
        }
        let Some(&t) = e.inputs.first().and_then(|c| initializers.get(c)) else {
            return Ok(None);
        };
        let values = tensor_f32(t, dir)?;
        Ok((values.len() == rows).then_some((p, values)))
    };
    let (Some((se, scale)), Some((ze, zero))) = (row_of(n.inputs[2])?, row_of(n.inputs[3])?) else {
        return Ok(None);
    };

    let base = if n.name.is_empty() {
        n.outputs[0]
    } else {
        n.name
    };
    let id = |suffix: &str| format!("{base}__int8_{suffix}");
    let (scales, correction, axes, y0, sum, corr) = (
        id("scales"),
        id("corr"),
        id("axes"),
        id("y0"),
        id("sum"),
        id("c"),
    );
    let block_scales: Vec<f32> = scale
        .iter()
        .flat_map(|&s| std::iter::repeat_n(s, blocks))
        .collect();
    let corrections: Vec<f32> = scale
        .iter()
        .zip(&zero)
        .map(|(&s, &z)| s * (DEFAULT_ZERO_POINT - z))
        .collect();
    let initializers = vec![
        tensor_bytes(
            &scales,
            1,
            &[(rows * blocks) as u64],
            &f32_bytes(&block_scales),
        ),
        tensor_bytes(&correction, 1, &[1, rows as u64], &f32_bytes(&corrections)),
        tensor_bytes(&axes, 7, &[1], &(-1i64).to_le_bytes()),
    ];
    let mut attrs: Vec<Vec<u8>> = Vec::new();
    for a in &n.attrs {
        let name = fields(a)?
            .into_iter()
            .find(|f| f.num == 1)
            .map(|f| f.value)
            .unwrap_or_default();
        if name != b"accuracy_level" {
            attrs.push(a.to_vec());
        }
    }
    attrs.push(int_attr("accuracy_level", INT8_LEVEL));
    let x = n.inputs[0];
    let nodes = vec![
        node_bytes(
            &[x, n.inputs[1], &scales],
            &[&y0],
            base,
            "MatMulNBits",
            "com.microsoft",
            &attrs,
        ),
        node_bytes(
            &[x, &axes],
            &[&sum],
            &id("sum_node"),
            "ReduceSum",
            "",
            &[int_attr("keepdims", 1)],
        ),
        node_bytes(
            &[&sum, &correction],
            &[&corr],
            &id("mul_node"),
            "Mul",
            "",
            &[],
        ),
        node_bytes(
            &[&y0, &corr],
            &[n.outputs[0]],
            &id("add_node"),
            "Add",
            "",
            &[],
        ),
    ];
    Ok(Some(Replacement {
        nodes,
        initializers,
        expands: [se, ze],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn float_tensor(name: &str, dims: &[u64], values: &[f32]) -> Vec<u8> {
        tensor_bytes(name, 1, dims, &f32_bytes(values))
    }

    /// A one-layer model in Mini's form: a 4-bit layer whose scales and
    /// zero points expand from per-row constants.
    fn mini_like(rows: usize, k: u64, block: u64, scale: &[f32], zero: &[f32]) -> Vec<u8> {
        let mut g = Vec::new();
        let expand = |input: &str, output: &str| {
            node_bytes(
                &[input, "shape"],
                &[output],
                &format!("{output}_expand"),
                "Expand",
                "",
                &[],
            )
        };
        put_bytes(&mut g, 1, &expand("s_row", "s"));
        put_bytes(&mut g, 1, &expand("z_row", "z"));
        put_bytes(
            &mut g,
            1,
            &node_bytes(
                &["x", "q", "s", "z"],
                &["y"],
                "layer",
                "MatMulNBits",
                "com.microsoft",
                &[
                    int_attr("K", k),
                    int_attr("N", rows as u64),
                    int_attr("bits", 4),
                    int_attr("block_size", block),
                ],
            ),
        );
        put_bytes(&mut g, 2, b"g");
        put_bytes(&mut g, 5, &float_tensor("s_row", &[rows as u64, 1], scale));
        put_bytes(&mut g, 5, &float_tensor("z_row", &[rows as u64, 1], zero));
        let mut m = Vec::new();
        put_varint_field(&mut m, 1, 8);
        put_bytes(&mut m, 7, &g);
        m
    }

    #[test]
    fn a_layer_becomes_integer_maths_plus_a_correction() {
        let m = mini_like(2, 512, 256, &[0.5, 0.25], &[7.5, 8.25]);
        let (out, n) = rewrite_model(&m, Path::new(".")).unwrap();
        assert_eq!(n, 1);
        let graph = fields(&out)
            .unwrap()
            .into_iter()
            .find(|f| f.num == 7)
            .unwrap();
        let gf = fields(graph.value).unwrap();
        let nodes: Vec<Node> = gf
            .iter()
            .filter(|f| f.num == 1)
            .map(|f| Node::parse(f.value).unwrap())
            .collect();
        let ops: Vec<&str> = nodes.iter().map(|n| n.op).collect();
        assert_eq!(
            ops,
            ["MatMulNBits", "ReduceSum", "Mul", "Add"],
            "expands dropped"
        );
        let layer = &nodes[0];
        assert_eq!(
            layer.inputs,
            ["x", "q", "layer__int8_scales"],
            "no zero point: 8"
        );
        assert_eq!(layer.int("accuracy_level").unwrap(), Some(4));
        assert_eq!(layer.int("block_size").unwrap(), Some(256));
        assert_eq!(nodes[3].outputs, ["y"], "the layer's output is kept");
        let tensors: HashMap<&str, &[u8]> = gf
            .iter()
            .filter(|f| f.num == 5)
            .map(|f| (tensor_name(f.value).unwrap(), f.value))
            .collect();
        let scales = tensor_f32(tensors["layer__int8_scales"], Path::new(".")).unwrap();
        assert_eq!(
            scales,
            [0.5, 0.5, 0.25, 0.25],
            "a scale per block, row by row"
        );
        let corr = tensor_f32(tensors["layer__int8_corr"], Path::new(".")).unwrap();
        // s * (8 - z): 0.5 * 0.5 and 0.25 * -0.25.
        assert_eq!(corr, [0.25, -0.0625]);
    }

    #[test]
    fn other_layers_are_left_alone() {
        let mut g = Vec::new();
        put_bytes(
            &mut g,
            1,
            &node_bytes(
                &["x", "q", "s"],
                &["y"],
                "two_bit",
                "MatMulNBits",
                "com.microsoft",
                &[
                    int_attr("bits", 2),
                    int_attr("K", 256),
                    int_attr("N", 1),
                    int_attr("block_size", 256),
                ],
            ),
        );
        let mut m = Vec::new();
        put_bytes(&mut m, 7, &g);
        let (out, n) = rewrite_model(&m, Path::new(".")).unwrap();
        assert_eq!(n, 0);
        assert_eq!(out, m);
    }

    #[test]
    fn broken_files_are_errors_not_panics() {
        for bytes in [
            &[0x3a, 0xff][..],
            &[0x3a, 0x05, 0x00][..],
            &[0xff; 3][..],
            &[0x07][..],
        ] {
            assert!(rewrite_model(bytes, Path::new(".")).is_err(), "{bytes:?}");
        }
    }

    /// Rewrites the real encoder: `ORTTAAI_FLUID_DIR=<fluid-speech-mini-onnx>`.
    #[test]
    #[ignore]
    fn rewrites_the_real_encoder() {
        let dir = std::env::var("ORTTAAI_FLUID_DIR").expect("ORTTAAI_FLUID_DIR");
        let dir = Path::new(&dir);
        let bytes = std::fs::read(dir.join("encoder.onnx")).unwrap();
        let (_, n) = rewrite_model(&bytes, dir).unwrap();
        assert_eq!(n, 146, "Mini has 146 4-bit layers");
    }

    /// Integer maths against the usual encoder on real speech: word
    /// differences and times only, never the words.
    /// `ORTTAAI_FLUID_DIR=<a copy of fluid-speech-mini-onnx>
    /// ORTTAAI_SEGMENT_WAVS=a.wav,b.wav cargo test --release ... -- --ignored
    /// int8_math_keeps_the_words --nocapture` (writes the copy's file).
    #[test]
    #[ignore]
    fn int8_math_keeps_the_words() {
        use crate::transcription::fluid::FluidModel;
        let dir = std::env::var("ORTTAAI_FLUID_DIR").expect("ORTTAAI_FLUID_DIR");
        let wavs = std::env::var("ORTTAAI_SEGMENT_WAVS").expect("ORTTAAI_SEGMENT_WAVS");
        let usual = FluidModel::load_with(Path::new(&dir), 4, false).unwrap();
        let int8 = FluidModel::load_with(Path::new(&dir), 4, true).unwrap();
        let words = |t: &str| -> Vec<String> {
            t.split_whitespace()
                .map(|w| {
                    w.to_lowercase()
                        .chars()
                        .filter(|c| c.is_alphanumeric())
                        .collect()
                })
                .collect()
        };
        let (mut total, mut differ) = (0, 0);
        for wav in wavs.split(',') {
            let mut reader = hound::WavReader::open(wav).unwrap();
            let audio: Vec<f32> = match reader.spec().sample_format {
                hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
                hound::SampleFormat::Int => reader
                    .samples::<i16>()
                    .map(|s| s.unwrap() as f32 / 32768.0)
                    .collect(),
            };
            let time = |m: &FluidModel| {
                m.transcribe(&audio).unwrap();
                let started = std::time::Instant::now();
                let out = m.transcribe(&audio).unwrap();
                (out.text, started.elapsed())
            };
            let (a, ta) = time(&usual);
            let (b, tb) = time(&int8);
            let (a, b) = (words(&a), words(&b));
            let d = a.iter().zip(&b).filter(|(x, y)| x != y).count() + a.len().abs_diff(b.len());
            eprintln!(
                "{}: {} words, {d} differ; usual {ta:?}, integer maths {tb:?}",
                Path::new(wav).file_name().unwrap().to_string_lossy(),
                a.len()
            );
            total += a.len();
            differ += d;
        }
        eprintln!("all: {differ} of {total} words differ");
        assert!(differ * 100 <= total, "{differ} of {total}");
    }
}
