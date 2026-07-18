use super::*;

// ── Host capability operations ─────────────────────────────────────────────────

#[unsafe(export_name = "zutai.host.io_print")]
pub extern "C" fn host_io_print(v: i64) -> i64 {
    print_text(v);
    let newline = text_from_bytes(b"\n");
    print_text(newline);
    v
}

#[unsafe(export_name = "zutai.host.fs_read")]
pub extern "C" fn host_fs_read(path: i64) -> i64 {
    let path = unsafe {
        str::from_utf8(text_parts(path))
            .unwrap_or_else(|_| runtime_error("fs.read path is not UTF-8"))
    };
    match std::fs::read_to_string(path) {
        Ok(contents) => text_from_string(contents),
        Err(err) => runtime_error(&format!("fs.read failed for {path:?}: {err}")),
    }
}

#[unsafe(export_name = "zutai.host.load_zti")]
pub extern "C" fn host_load_zti(path: i64) -> i64 {
    let path = unsafe {
        str::from_utf8(text_parts(path))
            .unwrap_or_else(|_| runtime_error("load.zti path is not UTF-8"))
    };
    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|err| runtime_error(&format!("load.zti failed for {path:?}: {err}")));
    let block = zutai_im::parse(&source)
        .unwrap_or_else(|err| runtime_error(&format!("load.zti parse failed for {path:?}: {err}")));
    data_from_zti_block(&block)
}

#[unsafe(export_name = "zutai.host.load_zt")]
pub extern "C" fn host_load_zt(path: i64) -> i64 {
    let path = unsafe {
        str::from_utf8(text_parts(path))
            .unwrap_or_else(|_| runtime_error("load.zt path is not UTF-8"))
    };
    match zutai_eval::eval_tlc_path(std::path::Path::new(path))
        .and_then(|value| data_from_eval_value(&value))
    {
        Ok(value) => value,
        Err(err) => runtime_error(&format!("load.zt failed for {path:?}: {err}")),
    }
}

pub(crate) fn data_from_zti_block(block: &zutai_im::Block) -> i64 {
    let mut fields = list_nil();
    for pair in block.iter().rev() {
        let field = data_field(&pair.field_name, data_from_zti_value(&pair.value));
        fields = list_cons(field, fields);
    }
    data_variant(6, data_record(&[("fields", fields)]))
}

pub(crate) fn data_from_zti_value(value: &zutai_im::Value) -> i64 {
    match value {
        zutai_im::Value::True => data_variant(0, data_record(&[("value", 1)])),
        zutai_im::Value::False => data_variant(0, data_record(&[("value", 0)])),
        zutai_im::Value::Atom(atom) => data_variant(
            4,
            data_record(&[("value", text_from_bytes(atom.as_bytes()))]),
        ),
        zutai_im::Value::String(text) => data_variant(
            3,
            data_record(&[("value", text_from_bytes(text.as_bytes()))]),
        ),
        zutai_im::Value::Float(value) => {
            data_variant(2, data_record(&[("value", value.to_bits() as i64)]))
        }
        zutai_im::Value::Integer(value) => data_variant(1, data_record(&[("value", *value)])),
        zutai_im::Value::Array(items) => {
            let mut out = list_nil();
            for item in items.iter().rev() {
                out = list_cons(data_from_zti_value(item), out);
            }
            data_variant(5, data_record(&[("items", out)]))
        }
        zutai_im::Value::Block(block) => data_from_zti_block(block),
    }
}

pub(crate) fn data_from_eval_value(value: &EvalValue) -> Result<i64, EvalError> {
    match value {
        EvalValue::Bool(value) => Ok(data_variant(
            0,
            data_record(&[("value", i64::from(*value))]),
        )),
        EvalValue::Int(value) => Ok(data_variant(1, data_record(&[("value", *value)]))),
        EvalValue::Float(value) => Ok(data_variant(
            2,
            data_record(&[("value", value.to_bits() as i64)]),
        )),
        EvalValue::Posit(literal) => Ok(data_variant(
            2,
            data_record(&[("value", (literal.bits as f64).to_bits() as i64)]),
        )),
        EvalValue::Text(value) => Ok(data_variant(
            3,
            data_record(&[("value", text_from_bytes(value.as_bytes()))]),
        )),
        EvalValue::Atom(value) => Ok(data_variant(
            4,
            data_record(&[("value", text_from_bytes(value.as_bytes()))]),
        )),
        EvalValue::List(items) => {
            let mut out = list_nil();
            for item in items.iter().rev() {
                let Some(value) = item.peek() else {
                    return Err(EvalError::Internal(
                        "load.zt result contains an unforced list item",
                    ));
                };
                out = list_cons(data_from_eval_value(&value)?, out);
            }
            Ok(data_variant(5, data_record(&[("items", out)])))
        }
        EvalValue::Tuple(items) => {
            let mut fields = list_nil();
            for (index, item) in items.iter().enumerate().rev() {
                let Some(value) = item.value.peek() else {
                    return Err(EvalError::Internal(
                        "load.zt result contains an unforced tuple item",
                    ));
                };
                let name = item
                    .name
                    .as_ref()
                    .map(|name| name.to_string())
                    .unwrap_or_else(|| index.to_string());
                fields = list_cons(data_field(&name, data_from_eval_value(&value)?), fields);
            }
            Ok(data_variant(6, data_record(&[("fields", fields)])))
        }
        EvalValue::Record(source_fields) => {
            let mut fields = list_nil();
            for (name, thunk) in source_fields.iter().rev() {
                let Some(value) = thunk.peek() else {
                    return Err(EvalError::Internal(
                        "load.zt result contains an unforced record field",
                    ));
                };
                fields = list_cons(data_field(name, data_from_eval_value(&value)?), fields);
            }
            Ok(data_variant(6, data_record(&[("fields", fields)])))
        }
        EvalValue::TaggedValue { tag, payload } => {
            let payload = data_from_eval_value(&EvalValue::Record(payload.clone()))?;
            Ok(data_variant(
                7,
                data_record(&[
                    ("payload", payload),
                    ("tag", text_from_bytes(tag.as_bytes())),
                ]),
            ))
        }
        EvalValue::Nothing => Ok(data_variant(
            4,
            data_record(&[("value", text_from_bytes(b"absent"))]),
        )),
        EvalValue::Closure(_)
        | EvalValue::TypeValue(_)
        | EvalValue::WitnessDict(_)
        | EvalValue::TlcClosure(_)
        | EvalValue::HostHandle(_)
        | EvalValue::Builtin(_)
        | EvalValue::BuiltinPartial { .. } => Err(EvalError::EffectfulNotExecutable(
            "load.zt final value is not first-order serializable data".to_string(),
        )),
    }
}

pub(crate) fn data_field(name: &str, value: i64) -> i64 {
    data_record(&[("name", text_from_bytes(name.as_bytes())), ("value", value)])
}

pub(crate) fn data_record(fields: &[(&str, i64)]) -> i64 {
    let record = record_new(fields.len() as i64);
    for (index, (_, value)) in fields.iter().enumerate() {
        record_set(record, index as i64, *value);
    }
    record
}

pub(crate) fn data_variant(tag_index: i64, payload: i64) -> i64 {
    variant_new(tag_index, payload)
}

#[unsafe(export_name = "zutai.host.fs_write")]
pub extern "C" fn host_fs_write(request: i64) -> i64 {
    unsafe {
        // Dataflow/Core and SSA sort record fields by name, so the standard
        // `{ contents : Text; path : Path; }` payload stores contents at slot 0
        // and path at slot 1.
        let contents = str::from_utf8(text_parts(word(request, 1)))
            .unwrap_or_else(|_| runtime_error("fs.write contents are not UTF-8"));
        let path = str::from_utf8(text_parts(word(request, 2)))
            .unwrap_or_else(|_| runtime_error("fs.write path is not UTF-8"));
        if let Err(err) = std::fs::write(path, contents) {
            runtime_error(&format!("fs.write failed for {path:?}: {err}"));
        }
    }
    tuple_new(0)
}

// ── Scoped filesystem text handles ─────────────────────────────────────────────

pub(crate) static FS_READERS: Mutex<Vec<Option<BufReader<File>>>> = Mutex::new(Vec::new());
pub(crate) static FS_WRITERS: Mutex<Vec<Option<BufWriter<File>>>> = Mutex::new(Vec::new());
pub(crate) static FS_NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn fs_alloc_id() -> u64 {
    FS_NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[unsafe(export_name = "zutai.host.fs_open_read")]
pub extern "C" fn host_fs_open_read(path: i64) -> i64 {
    let path = unsafe {
        str::from_utf8(text_parts(path))
            .unwrap_or_else(|_| runtime_error("fs.openRead path is not UTF-8"))
    };
    let file = File::open(path).unwrap_or_else(|err| runtime_error(&format!("fs.openRead: {err}")));
    let id = fs_alloc_id();
    let mut readers = FS_READERS.lock().unwrap();
    while readers.len() <= id as usize {
        readers.push(None);
    }
    readers[id as usize] = Some(BufReader::new(file));
    id as i64
}

#[unsafe(export_name = "zutai.host.fs_read_line")]
pub extern "C" fn host_fs_read_line(reader_id: i64) -> i64 {
    let mut readers = FS_READERS.lock().unwrap();
    let reader = readers
        .get_mut(reader_id as usize)
        .and_then(|slot| slot.as_mut())
        .unwrap_or_else(|| runtime_error(&format!("fs.readLine: reader {reader_id} not found")));
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .unwrap_or_else(|err| runtime_error(&format!("fs.readLine: {err}")));
    if bytes == 0 {
        optional_text(None)
    } else {
        optional_text(Some(strip_read_line_ending(&line).to_string()))
    }
}

pub(crate) fn strip_read_line_ending(line: &str) -> &str {
    let Some(stripped) = line.strip_suffix('\n') else {
        return line;
    };
    stripped.strip_suffix('\r').unwrap_or(stripped)
}

#[unsafe(export_name = "zutai.host.fs_close_read")]
pub extern "C" fn host_fs_close_read(reader_id: i64) -> i64 {
    let mut readers = FS_READERS.lock().unwrap();
    let slot = readers
        .get_mut(reader_id as usize)
        .unwrap_or_else(|| runtime_error(&format!("fs.closeRead: reader {reader_id} not found")));
    *slot = None;
    tuple_new(0)
}

#[unsafe(export_name = "zutai.host.fs_open_write")]
pub extern "C" fn host_fs_open_write(path: i64) -> i64 {
    let path = unsafe {
        str::from_utf8(text_parts(path))
            .unwrap_or_else(|_| runtime_error("fs.openWrite path is not UTF-8"))
    };
    let file =
        File::create(path).unwrap_or_else(|err| runtime_error(&format!("fs.openWrite: {err}")));
    let id = fs_alloc_id();
    let mut writers = FS_WRITERS.lock().unwrap();
    while writers.len() <= id as usize {
        writers.push(None);
    }
    writers[id as usize] = Some(BufWriter::new(file));
    id as i64
}

#[unsafe(export_name = "zutai.host.fs_write_text")]
pub extern "C" fn host_fs_write_text(request: i64) -> i64 {
    unsafe {
        // `{ contents : Text; writer : Writer; }` is slot-sorted by name.
        let contents = str::from_utf8(text_parts(word(request, 1)))
            .unwrap_or_else(|_| runtime_error("fs.writeText contents are not UTF-8"));
        let writer_id = word(request, 2);
        let mut writers = FS_WRITERS.lock().unwrap();
        let writer = writers
            .get_mut(writer_id as usize)
            .and_then(|slot| slot.as_mut())
            .unwrap_or_else(|| {
                runtime_error(&format!("fs.writeText: writer {writer_id} not found"))
            });
        writer
            .write_all(contents.as_bytes())
            .unwrap_or_else(|err| runtime_error(&format!("fs.writeText: {err}")));
    }
    tuple_new(0)
}

#[unsafe(export_name = "zutai.host.fs_flush")]
pub extern "C" fn host_fs_flush(writer_id: i64) -> i64 {
    let mut writers = FS_WRITERS.lock().unwrap();
    let writer = writers
        .get_mut(writer_id as usize)
        .and_then(|slot| slot.as_mut())
        .unwrap_or_else(|| runtime_error(&format!("fs.flush: writer {writer_id} not found")));
    writer
        .flush()
        .unwrap_or_else(|err| runtime_error(&format!("fs.flush: {err}")));
    tuple_new(0)
}

#[unsafe(export_name = "zutai.host.fs_close_write")]
pub extern "C" fn host_fs_close_write(writer_id: i64) -> i64 {
    let mut writers = FS_WRITERS.lock().unwrap();
    let slot = writers
        .get_mut(writer_id as usize)
        .unwrap_or_else(|| runtime_error(&format!("fs.closeWrite: writer {writer_id} not found")));
    if let Some(mut writer) = slot.take() {
        writer
            .flush()
            .unwrap_or_else(|err| runtime_error(&format!("fs.closeWrite: {err}")));
    }
    tuple_new(0)
}

#[unsafe(export_name = "zutai.host.env_get")]
pub extern "C" fn host_env_get(name: i64) -> i64 {
    let name = unsafe {
        str::from_utf8(text_parts(name))
            .unwrap_or_else(|_| runtime_error("env.get name is not UTF-8"))
    };
    optional_text(std::env::var(name).ok())
}

#[unsafe(export_name = "zutai.host.clock_now")]
pub extern "C" fn host_clock_now(_unit: i64) -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    text_from_string(millis.to_string())
}

pub(crate) static RNG_STATE: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);

#[unsafe(export_name = "zutai.host.rng_next")]
pub extern "C" fn host_rng_next(_unit: i64) -> i64 {
    let mut state = RNG_STATE.load(Ordering::Relaxed);
    loop {
        let next = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        match RNG_STATE.compare_exchange_weak(state, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return (next >> 1) as i64,
            Err(found) => state = found,
        }
    }
}

// ── Network capability operations ────────────────────────────────────────────────

pub(crate) static NET_LISTENERS: Mutex<Vec<Option<TcpListener>>> = Mutex::new(Vec::new());
pub(crate) static NET_CONNECTIONS: Mutex<Vec<Option<TcpStream>>> = Mutex::new(Vec::new());
pub(crate) static NET_CURRENT_CONN: AtomicU64 = AtomicU64::new(0);
pub(crate) static NET_NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn net_alloc_id(next: &AtomicU64) -> u64 {
    next.fetch_add(1, Ordering::Relaxed)
}

#[unsafe(export_name = "zutai.host.net_listen")]
pub extern "C" fn host_net_listen(port: i64) -> i64 {
    let addr = format!("127.0.0.1:{port}");
    let listener =
        TcpListener::bind(&addr).unwrap_or_else(|err| runtime_error(&format!("net.listen: {err}")));
    let id = net_alloc_id(&NET_NEXT_ID);
    let mut listeners = NET_LISTENERS.lock().unwrap();
    while listeners.len() <= id as usize {
        listeners.push(None);
    }
    listeners[id as usize] = Some(listener);
    id as i64
}

#[unsafe(export_name = "zutai.host.net_accept")]
pub extern "C" fn host_net_accept(listener_id: i64) -> i64 {
    let mut listeners = NET_LISTENERS.lock().unwrap();
    let listener = listeners
        .get_mut(listener_id as usize)
        .and_then(|opt| opt.as_mut())
        .unwrap_or_else(|| runtime_error(&format!("net.accept: listener {listener_id} not found")));
    let (stream, _addr) = listener
        .accept()
        .unwrap_or_else(|err| runtime_error(&format!("net.accept: {err}")));
    let conn_id = net_alloc_id(&NET_NEXT_ID);
    let mut conns = NET_CONNECTIONS.lock().unwrap();
    while conns.len() <= conn_id as usize {
        conns.push(None);
    }
    conns[conn_id as usize] = Some(stream);
    NET_CURRENT_CONN.store(conn_id, Ordering::Relaxed);
    conn_id as i64
}

#[unsafe(export_name = "zutai.host.net_read")]
pub extern "C" fn host_net_read(conn_id: i64) -> i64 {
    let mut conns = NET_CONNECTIONS.lock().unwrap();
    let stream = conns
        .get_mut(conn_id as usize)
        .and_then(|opt| opt.as_mut())
        .unwrap_or_else(|| runtime_error(&format!("net.read: connection {conn_id} not found")));
    let mut reader = BufReader::new(&mut *stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .unwrap_or_else(|err| runtime_error(&format!("net.read: {err}")));
    let trimmed = line.trim_end_matches(['\r', '\n']);
    text_from_string(trimmed.to_string())
}

#[unsafe(export_name = "zutai.host.net_write")]
pub extern "C" fn host_net_write(text: i64) -> i64 {
    let conn_id = NET_CURRENT_CONN.load(Ordering::Relaxed);
    if conn_id == 0 {
        runtime_error("net.write: no current connection");
    }
    let mut conns = NET_CONNECTIONS.lock().unwrap();
    let stream = conns
        .get_mut(conn_id as usize)
        .and_then(|opt| opt.as_mut())
        .unwrap_or_else(|| runtime_error(&format!("net.write: connection {conn_id} not found")));
    let data = unsafe { text_parts(text) };
    use std::io::Write;
    stream
        .write_all(data)
        .and_then(|_| stream.flush())
        .unwrap_or_else(|err| runtime_error(&format!("net.write: {err}")));
    tuple_new(0)
}

#[unsafe(export_name = "zutai.host.net_close")]
pub extern "C" fn host_net_close(conn_id: i64) -> i64 {
    let mut conns = NET_CONNECTIONS.lock().unwrap();
    if (conn_id as usize) < conns.len() {
        conns[conn_id as usize] = None;
    }
    if NET_CURRENT_CONN.load(Ordering::Relaxed) == conn_id as u64 {
        NET_CURRENT_CONN.store(0, Ordering::Relaxed);
    }
    tuple_new(0)
}

// ── Output ──────────────────────────────────────────────────────────────────────

pub(crate) fn out_bytes(bytes: &[u8]) {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(bytes);
    let _ = lock.flush();
}

#[unsafe(export_name = "zutai.print_i64")]
pub extern "C" fn print_i64(v: i64) {
    out_bytes(v.to_string().as_bytes());
}

#[unsafe(export_name = "zutai.print_bool")]
pub extern "C" fn print_bool(v: i64) {
    out_bytes(if v != 0 { b"true" } else { b"false" });
}

#[unsafe(export_name = "zutai.print_float")]
pub extern "C" fn print_float(v: i64) {
    out_bytes(fmt_float(f64::from_bits(v as u64)).as_bytes());
}

/// Raw text output — the `io.print` handler. Writes the UTF-8 bytes verbatim
/// (no quotes); contrast with `show`, which quotes and escapes `Text` values.
#[unsafe(export_name = "zutai.print_text")]
pub extern "C" fn print_text(v: i64) {
    out_bytes(unsafe { text_parts(v) });
}

#[unsafe(export_name = "zutai.print_posit")]
pub extern "C" fn print_posit(v: i64, nbits: i64, es: i64) {
    out_bytes(fmt_posit(v, nbits, es).as_bytes());
}
