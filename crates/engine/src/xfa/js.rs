//! XFA JavaScript on the boa engine (pure Rust, MIT/Unlicense).
//!
//! Script DOM objects are JavaScript `Proxy` objects whose traps call
//! [`Dom::get`], [`Dom::set`] and [`Dom::call`]. Each script runs as
//!
//! ```text
//! with (__wf_scope) { <source, with `this` rewritten to __wf_this> }
//! ```
//!
//! where `__wf_scope` is a proxy resolving unqualified names by XFA
//! scoping from the script's node (PDFium `CFXJSE_Engine` global property
//! handlers); the completion value is the script's result (calculate).
//!
//! Sandbox: the context has no host APIs (no network, file system,
//! timers or modules), `eval` is replaced by a `Function`-based
//! equivalent, size-amplifying string builtins are capped, and scripts run
//! through boa's budgeted evaluation: execution yields every few thousand
//! instructions and is abandoned once the instruction budget or the
//! deadline is exhausted (after which no further JavaScript runs for the
//! document). Code reached only through native callbacks
//! (`Function.prototype.call`, accessors, `String.prototype.replace`
//! callbacks) is bounded by the per-frame loop-iteration and recursion
//! limits rather than the instruction budget.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context as TaskContext, Poll, Waker};

use boa_engine::object::builtins::JsProxy;
use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;
use boa_engine::{
    js_string, Context, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue,
    NativeFunction, Script, Source,
};

use super::som::{is_name_char, Class, Dom, Obj, Seg, SomIndex, SomName, Val};

/// Instructions ("clock cycles") between budget checks.
const SLICE: u32 = 4_096;
/// Total instruction budget per document.
const MAX_CYCLES: u64 = 400_000_000;
const MAX_HANDLES: usize = 1_000_000;

pub(crate) enum JsFailure {
    /// The script threw or failed to parse.
    Script(String),
    /// The budget or deadline ran out mid-script.
    Aborted(String),
}

struct ScopeState {
    this: usize,
    /// Names the script declares with `var`/`function` (never SOM).
    declared: HashSet<String>,
    cache: HashMap<String, bool>,
}

struct Host {
    dom: Rc<RefCell<Dom>>,
    handles: RefCell<Vec<Obj>>,
    node_proxies: RefCell<HashMap<usize, JsObject>>,
    script_objects: RefCell<HashMap<usize, JsValue>>,
    scope: RefCell<ScopeState>,
}

pub(crate) struct JsRuntime {
    context: Context,
    host: Rc<Host>,
    cycles: u64,
}

const HANDLE: &str = "__wfh";
const SCOPE_TAG: &str = "__wfscope";

/// SOM methods exposed as functions, by object kind.
const NODE_METHODS: &[&str] = &[
    "resolveNode",
    "resolveNodes",
    "execCalculate",
    "execInitialize",
    "execEvent",
    "execValidate",
    "addItem",
    "clearItems",
    "deleteItem",
    "setItems",
    "getDisplayItem",
    "getSaveItem",
    "boundItem",
    "getItemState",
    "setItemState",
    "isPropertySpecified",
    "getElement",
    "setElement",
    "getAttribute",
    "setAttribute",
    "applyXSL",
    "assignNode",
    "clone",
    "saveXML",
    "loadXML",
    "relayout",
    "isRecordGroup",
    "item",
    "createNode",
    "clearErrorList",
];
const LIST_METHODS: &[&str] = &["item", "namedItem", "append", "insert", "remove"];
const MODEL_METHODS: &[&str] = &[
    "resolveNode",
    "resolveNodes",
    "recalculate",
    "execCalculate",
    "execInitialize",
    "execValidate",
    "remerge",
    "execEvent",
    "createNode",
    "isCompatibleNS",
];
const HOST_METHODS: &[&str] = &[
    "messageBox",
    "setFocus",
    "beep",
    "resetData",
    "print",
    "exportData",
    "importData",
    "gotoURL",
    "openList",
    "response",
    "getFocus",
];
const LAYOUT_METHODS: &[&str] = &[
    "page",
    "pageCount",
    "absPage",
    "absPageCount",
    "absPageInBatch",
    "absPageCountInBatch",
    "sheet",
    "sheetCount",
    "sheetInBatch",
    "sheetCountInBatch",
    "pageSpan",
    "h",
    "w",
    "x",
    "y",
    "relayout",
    "relayoutPageArea",
    "pageContent",
];
const MANAGER_METHODS: &[&str] = &[
    "setInstances",
    "addInstance",
    "insertInstance",
    "removeInstance",
    "moveInstance",
];

fn methods_of(obj: &Obj) -> &'static [&'static str] {
    match obj {
        Obj::Node(_) | Obj::Nodes(_) => NODE_METHODS,
        Obj::Children(_) => LIST_METHODS,
        Obj::Xfa | Obj::FormModel => MODEL_METHODS,
        Obj::Host => HOST_METHODS,
        Obj::Layout => LAYOUT_METHODS,
        Obj::Manager { .. } => MANAGER_METHODS,
        _ => &[],
    }
}

const PRELUDE: &str = r#"
(function (g) {
  var nativeRepeat = String.prototype.repeat;
  var limit = 1048576;
  String.prototype.repeat = function (n) {
    if (String(this).length * Number(n) > limit) throw new RangeError("XFA script string limit");
    return nativeRepeat.call(this, n);
  };
  var nativePadStart = String.prototype.padStart;
  String.prototype.padStart = function (n, s) {
    if (Number(n) > limit) throw new RangeError("XFA script string limit");
    return nativePadStart.call(this, n, s);
  };
  var nativePadEnd = String.prototype.padEnd;
  String.prototype.padEnd = function (n, s) {
    if (Number(n) > limit) throw new RangeError("XFA script string limit");
    return nativePadEnd.call(this, n, s);
  };
  // Array callbacks run from script code (inside the instruction budget).
  var AP = Array.prototype;
  AP.forEach = function (f, t) { for (var i = 0; i < this.length; i++) if (i in this) { if (t === undefined) f(this[i], i, this); else f.call(t, this[i], i, this); } };
  AP.map = function (f, t) { var r = new Array(this.length); for (var i = 0; i < this.length; i++) if (i in this) r[i] = t === undefined ? f(this[i], i, this) : f.call(t, this[i], i, this); return r; };
  AP.filter = function (f, t) { var r = []; for (var i = 0; i < this.length; i++) if (i in this) { var v = this[i]; if (t === undefined ? f(v, i, this) : f.call(t, v, i, this)) r.push(v); } return r; };
  AP.some = function (f, t) { for (var i = 0; i < this.length; i++) if (i in this && (t === undefined ? f(this[i], i, this) : f.call(t, this[i], i, this))) return true; return false; };
  AP.every = function (f, t) { for (var i = 0; i < this.length; i++) if (i in this && !(t === undefined ? f(this[i], i, this) : f.call(t, this[i], i, this))) return false; return true; };
  AP.find = function (f, t) { for (var i = 0; i < this.length; i++) { var v = this[i]; if (t === undefined ? f(v, i, this) : f.call(t, v, i, this)) return v; } return undefined; };
  AP.findIndex = function (f, t) { for (var i = 0; i < this.length; i++) if (t === undefined ? f(this[i], i, this) : f.call(t, this[i], i, this)) return i; return -1; };
  AP.reduce = function (f, a) { var i = 0; if (arguments.length < 2) { while (i < this.length && !(i in this)) i++; if (i >= this.length) throw new TypeError("Reduce of empty array with no initial value"); a = this[i++]; } for (; i < this.length; i++) if (i in this) a = f(a, this[i], i, this); return a; };
  var nativeSort = AP.sort;
  AP.sort = function (cmp) {
    if (cmp === undefined) return nativeSort.call(this);
    var a = this, n = a.length, tmp = new Array(n);
    for (var w = 1; w < n; w *= 2) {
      for (var lo = 0; lo < n - w; lo += 2 * w) {
        var mid = lo + w, hi = Math.min(lo + 2 * w, n), i = lo, j = mid, k = lo;
        while (i < mid && j < hi) tmp[k++] = cmp(a[j], a[i]) < 0 ? a[j++] : a[i++];
        while (i < mid) tmp[k++] = a[i++];
        while (j < hi) tmp[k++] = a[j++];
        for (k = lo; k < hi; k++) a[k] = tmp[k];
      }
    }
    return a;
  };
  // eval: indirect, compiled through Function so the code runs inside
  // the instruction budget.
  g.eval = function (s) {
    if (typeof s !== "string") return s;
    var f;
    try { f = new Function("with (__wf_scope) { return (" + s + "\n); }"); } catch (e) { f = new Function("with (__wf_scope) {\n" + s + "\n}"); }
    return f();
  };
  g.app = {
    alert: function (m) { __wf_alert(String(m)); return 1; },
    beep: function () {},
    response: function () { return null; },
    setTimeOut: function () { return null; },
    setInterval: function () { return null; },
    clearTimeOut: function () {},
    clearInterval: function () {},
    viewerType: "Exchange-Pro",
    viewerVariation: "Full",
    viewerVersion: 11,
    platform: "WIN",
    language: "ENU"
  };
  g.console = { println: function (m) { __wf_alert(String(m)); }, show: function () {}, clear: function () {}, hide: function () {} };
  g.util = {
    printd: function (f, d) { return String(d); },
    printf: function (f) { return String(f); },
    scand: function (f, s) { return new Date(s); }
  };
})(this);
"#;

impl JsRuntime {
    pub(crate) fn new(dom: Rc<RefCell<Dom>>) -> Result<Self, String> {
        let mut context = Context::default();
        {
            let limits = context.runtime_limits_mut();
            limits.set_loop_iteration_limit(1_000_000);
            limits.set_recursion_limit(256);
        }
        let host = Rc::new(Host {
            dom,
            handles: RefCell::new(Vec::new()),
            node_proxies: RefCell::new(HashMap::new()),
            script_objects: RefCell::new(HashMap::new()),
            scope: RefCell::new(ScopeState {
                this: 0,
                declared: HashSet::new(),
                cache: HashMap::new(),
            }),
        });
        context.insert_data(Rc::clone(&host));
        let alert = NativeFunction::from_fn_ptr(native_alert).to_js_function(context.realm());
        context
            .register_global_property(js_string!("__wf_alert"), alert, Attribute::all())
            .map_err(|e| e.to_string())?;
        let xfa = make_proxy(&mut context, Obj::Xfa).map_err(|e| e.to_string())?;
        context
            .register_global_property(js_string!("xfa"), xfa, Attribute::all())
            .map_err(|e| e.to_string())?;
        let event = make_proxy(&mut context, Obj::Event).map_err(|e| e.to_string())?;
        context
            .register_global_property(js_string!("event"), event, Attribute::all())
            .map_err(|e| e.to_string())?;
        context
            .register_global_property(js_string!("__wf_this"), JsValue::null(), Attribute::all())
            .map_err(|e| e.to_string())?;
        context
            .register_global_property(js_string!("__wf_scope"), JsValue::null(), Attribute::all())
            .map_err(|e| e.to_string())?;
        context
            .eval(Source::from_bytes(PRELUDE))
            .map_err(|e| format!("XFA JavaScript prelude failed: {e}"))?;
        Ok(Self {
            context,
            host,
            cycles: 0,
        })
    }

    /// Runs `source` with `this` = `node`; returns its completion value
    /// (`None` for `undefined`).
    pub(crate) fn run(&mut self, node: usize, source: &str) -> Result<Option<Val>, JsFailure> {
        {
            let dom = self.host.dom.borrow();
            if source.len() > dom.limits.max_source_bytes {
                return Err(JsFailure::Script(
                    "XFA JavaScript source exceeds the size limit".into(),
                ));
            }
            if !dom.time_left() {
                return Err(JsFailure::Aborted(
                    "XFA script time budget exhausted".into(),
                ));
            }
        }
        {
            let mut scope = self.host.scope.borrow_mut();
            scope.this = node;
            scope.declared = declared_names(source);
            scope.cache.clear();
        }
        let this =
            node_proxy(&mut self.context, node).map_err(|e| JsFailure::Script(e.to_string()))?;
        let scope = make_scope(&mut self.context).map_err(|e| JsFailure::Script(e.to_string()))?;
        let global = self.context.global_object();
        global
            .set(js_string!("__wf_this"), this, false, &mut self.context)
            .map_err(|e| JsFailure::Script(e.to_string()))?;
        global
            .set(js_string!("__wf_scope"), scope, false, &mut self.context)
            .map_err(|e| JsFailure::Script(e.to_string()))?;
        let code = format!("with (__wf_scope) {{\n{}\n}}", rewrite_this(source));
        let script = Script::parse(Source::from_bytes(code.as_bytes()), None, &mut self.context)
            .map_err(|e| JsFailure::Script(format!("JavaScript syntax error: {e}")))?;
        let host = Rc::clone(&self.host);
        let mut cycles = self.cycles;
        let outcome = {
            let future = script.evaluate_async_with_budget(&mut self.context, SLICE);
            block_on_budget(future, || {
                cycles += u64::from(SLICE);
                cycles <= MAX_CYCLES && host.dom.borrow().time_left()
            })
        };
        self.cycles = cycles;
        let value = match outcome {
            None => {
                host.dom.borrow_mut().exhausted = true;
                return Err(JsFailure::Aborted(
                    "XFA JavaScript instruction or time budget exhausted".into(),
                ));
            }
            Some(Err(error)) => {
                return Err(JsFailure::Script(error_text(&error, &mut self.context)))
            }
            Some(Ok(value)) => value,
        };
        if value.is_undefined() {
            return Ok(None);
        }
        Ok(Some(to_val(&value, &mut self.context)))
    }
}

fn error_text(error: &JsError, context: &mut Context) -> String {
    match error.try_native(context) {
        Ok(native) => native.to_string(),
        Err(_) => error.to_string(),
    }
}

/// Polls `future` to completion with a no-op waker; `keep_going` is asked
/// at every yield and abandons the evaluation when it returns false.
fn block_on_budget<F: Future>(
    future: F,
    mut keep_going: impl FnMut() -> bool,
) -> Option<F::Output> {
    let mut future = pin!(future);
    let mut cx = TaskContext::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => return Some(output),
            Poll::Pending => {
                if !keep_going() {
                    return None;
                }
            }
        }
    }
}

fn host_of(context: &Context) -> JsResult<Rc<Host>> {
    context.get_data::<Rc<Host>>().cloned().ok_or_else(|| {
        JsNativeError::error()
            .with_message("XFA host missing")
            .into()
    })
}

fn script_error(error: super::som::ScriptError) -> JsError {
    JsNativeError::error().with_message(error.0).into()
}

fn native_alert(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let host = host_of(context)?;
    let text = match args.first() {
        Some(value) => value.to_string(context)?.to_std_string_escaped(),
        None => String::new(),
    };
    host.dom.borrow_mut().log.push(text);
    Ok(JsValue::null())
}

fn new_handle(host: &Host, obj: Obj) -> JsResult<usize> {
    let mut handles = host.handles.borrow_mut();
    if handles.len() >= MAX_HANDLES {
        return Err(JsNativeError::range()
            .with_message("XFA script object limit exceeded")
            .into());
    }
    handles.push(obj);
    Ok(handles.len() - 1)
}

fn make_proxy(context: &mut Context, obj: Obj) -> JsResult<JsValue> {
    let host = host_of(context)?;
    let handle = new_handle(&host, obj)?;
    let target = ObjectInitializer::new(context)
        .property(js_string!(HANDLE), handle as f64, Attribute::all())
        .build();
    let proxy = JsProxy::builder(target)
        .get(trap_get)
        .set(trap_set)
        .has(trap_has)
        .build(context)?;
    Ok(JsValue::from(JsObject::from(proxy)))
}

fn node_proxy(context: &mut Context, id: usize) -> JsResult<JsValue> {
    let host = host_of(context)?;
    if let Some(existing) = host.node_proxies.borrow().get(&id) {
        return Ok(JsValue::from(existing.clone()));
    }
    let proxy = make_proxy(context, Obj::Node(id))?;
    if let Some(object) = proxy.as_object() {
        host.node_proxies.borrow_mut().insert(id, object);
    }
    Ok(proxy)
}

fn make_scope(context: &mut Context) -> JsResult<JsValue> {
    let target = ObjectInitializer::new(context)
        .property(js_string!(SCOPE_TAG), true, Attribute::all())
        .build();
    let proxy = JsProxy::builder(target)
        .get(scope_get)
        .set(scope_set)
        .has(scope_has)
        .build(context)?;
    Ok(JsValue::from(JsObject::from(proxy)))
}

fn handle_of(target: &JsValue, context: &mut Context) -> JsResult<Option<Obj>> {
    let Some(object) = target.as_object() else {
        return Ok(None);
    };
    let value = object.get(js_string!(HANDLE), context)?;
    let Some(number) = value.as_number() else {
        return Ok(None);
    };
    let host = host_of(context)?;
    let handles = host.handles.borrow();
    Ok(handles.get(number as usize).cloned())
}

fn key_name(key: &JsValue) -> Option<String> {
    if let Some(text) = key.as_string() {
        return Some(text.to_std_string_escaped());
    }
    key.as_number().map(super::som::format_number)
}

fn from_val(value: Val, context: &mut Context) -> JsResult<JsValue> {
    Ok(match value {
        Val::Null => JsValue::null(),
        Val::Num(n) => JsValue::from(n),
        Val::Str(s) => JsValue::from(JsString::from(s.as_str())),
        Val::Bool(b) => JsValue::from(b),
        Val::Obj(Obj::Node(id)) => node_proxy(context, id)?,
        Val::Obj(obj) => make_proxy(context, obj)?,
    })
}

fn to_val(value: &JsValue, context: &mut Context) -> Val {
    if value.is_null_or_undefined() {
        return Val::Null;
    }
    if let Some(b) = value.as_boolean() {
        return Val::Bool(b);
    }
    if let Some(n) = value.as_number() {
        return Val::Num(n);
    }
    if let Some(s) = value.as_string() {
        return Val::Str(s.to_std_string_escaped());
    }
    if value.is_object() {
        if let Ok(Some(obj)) = handle_of(value, context) {
            return Val::Obj(obj);
        }
    }
    match value.to_string(context) {
        Ok(text) => Val::Str(text.to_std_string_escaped()),
        Err(_) => Val::Null,
    }
}

fn method_function(handle: usize, method: &'static str, context: &mut Context) -> JsValue {
    let function = NativeFunction::from_copy_closure(move |_this, args, context| {
        let host = host_of(context)?;
        let obj = host.handles.borrow().get(handle).cloned().ok_or_else(|| {
            JsError::from(JsNativeError::error().with_message("stale XFA object"))
        })?;
        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            values.push(to_val(arg, context));
        }
        let this = host.scope.borrow().this;
        let result = host
            .dom
            .borrow_mut()
            .call(&obj, Some(this), method, &values)
            .map_err(script_error)?;
        from_val(result, context)
    });
    JsValue::from(function.to_js_function(context.realm()))
}

fn default_value_function(handle: usize, as_string: bool, context: &mut Context) -> JsValue {
    let function = NativeFunction::from_copy_closure(move |_this, _args, context| {
        let host = host_of(context)?;
        let obj = host.handles.borrow().get(handle).cloned();
        let Some(obj) = obj else {
            return Ok(JsValue::null());
        };
        let value = host.dom.borrow_mut().value_of(&obj).map_err(script_error)?;
        if as_string {
            let text = match &value {
                Val::Null => match &obj {
                    Obj::Node(_) | Obj::Prop { .. } => String::new(),
                    _ => "[object XFAObject]".to_string(),
                },
                other => other.to_text(),
            };
            return Ok(JsValue::from(JsString::from(text.as_str())));
        }
        from_val(value, context)
    });
    JsValue::from(function.to_js_function(context.realm()))
}

fn trap_get(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let target = args.first().cloned().unwrap_or_default();
    let Some(name) = args.get(1).and_then(key_name) else {
        return Ok(JsValue::undefined());
    };
    if name == HANDLE {
        return match target.as_object() {
            Some(object) => object.get(js_string!(HANDLE), context),
            None => Ok(JsValue::undefined()),
        };
    }
    let handle = match target.as_object() {
        Some(object) => object
            .get(js_string!(HANDLE), context)?
            .as_number()
            .map(|n| n as usize),
        None => None,
    };
    let Some(handle) = handle else {
        return Ok(JsValue::undefined());
    };
    let host = host_of(context)?;
    let Some(obj) = host.handles.borrow().get(handle).cloned() else {
        return Ok(JsValue::undefined());
    };
    match name.as_str() {
        "toString" => return Ok(default_value_function(handle, true, context)),
        "valueOf" => return Ok(default_value_function(handle, false, context)),
        "then" | "constructor" | "prototype" | "__proto__" => return Ok(JsValue::undefined()),
        _ => {}
    }
    if let Some(method) = methods_of(&obj).iter().find(|m| **m == name) {
        // A node's child named like a method wins.
        let child = match &obj {
            Obj::Node(id) => !host.dom.borrow().children_named(*id, &name).is_empty(),
            _ => false,
        };
        if !child {
            return Ok(method_function(handle, method, context));
        }
    }
    let value = host
        .dom
        .borrow_mut()
        .get(&obj, &name)
        .map_err(script_error)?;
    from_val(value, context)
}

fn trap_set(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let target = args.first().cloned().unwrap_or_default();
    let Some(name) = args.get(1).and_then(key_name) else {
        return Ok(JsValue::from(true));
    };
    let value = args.get(2).cloned().unwrap_or_default();
    let Some(obj) = handle_of(&target, context)? else {
        return Ok(JsValue::from(true));
    };
    let value = to_val(&value, context);
    let host = host_of(context)?;
    host.dom
        .borrow_mut()
        .set(&obj, &name, &value)
        .map_err(script_error)?;
    Ok(JsValue::from(true))
}

fn trap_has(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let target = args.first().cloned().unwrap_or_default();
    let Some(name) = args.get(1).and_then(key_name) else {
        return Ok(JsValue::from(false));
    };
    let Some(obj) = handle_of(&target, context)? else {
        return Ok(JsValue::from(false));
    };
    let host = host_of(context)?;
    let value = host
        .dom
        .borrow_mut()
        .get(&obj, &name)
        .map_err(script_error)?;
    Ok(JsValue::from(value != Val::Null))
}

/// `$`-prefixed SOM shortcuts usable as JavaScript identifiers (PDFium's
/// global property handler resolves them like SOM expressions).
fn dollar_name(name: &str) -> Option<&str> {
    let rest = name.strip_prefix('$')?;
    matches!(
        rest,
        "" | "form" | "data" | "record" | "host" | "layout" | "event" | "template" | "xfa"
    )
    .then_some(rest)
}

/// Whether an unqualified name resolves through XFA scoping.
fn scope_resolves(host: &Host, name: &str) -> bool {
    if dollar_name(name).is_some() {
        return !host.scope.borrow().declared.contains(name);
    }
    if name.starts_with("__wf") || name.is_empty() || !name.chars().all(is_name_char) {
        return false;
    }
    let mut scope = host.scope.borrow_mut();
    if scope.declared.contains(name) {
        return false;
    }
    if let Some(&cached) = scope.cache.get(name) {
        return cached;
    }
    let this = scope.this;
    let dom = host.dom.borrow();
    let found = !dom.scope_find(this, name).is_empty()
        || name
            .strip_prefix('_')
            .is_some_and(|rest| !rest.is_empty() && !dom.scope_find(this, rest).is_empty())
        || name
            .strip_prefix('_')
            .is_some_and(|rest| manager_exists(&dom, this, rest))
        || dom.nodes[dom.root].name == name;
    scope.cache.insert(name.to_string(), found);
    found
}

fn manager_exists(dom: &Dom, from: usize, name: &str) -> bool {
    let mut current = Some(from);
    while let Some(id) = current {
        if dom.nodes[id].managed.iter().any(|m| m.0 == name) {
            return true;
        }
        current = dom.nodes[id].parent;
    }
    false
}

fn scope_has(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(name) = args.get(1).and_then(key_name) else {
        return Ok(JsValue::from(false));
    };
    let host = host_of(context)?;
    Ok(JsValue::from(scope_resolves(&host, &name)))
}

fn scope_get(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(name) = args.get(1).and_then(key_name) else {
        return Ok(JsValue::undefined());
    };
    if name == SCOPE_TAG {
        return Ok(JsValue::from(true));
    }
    let host = host_of(context)?;
    if !scope_resolves(&host, &name) {
        return Ok(JsValue::undefined());
    }
    let this = host.scope.borrow().this;
    let seg = Seg {
        desc: false,
        name: match dollar_name(&name) {
            Some(rest) => SomName::Dollar(rest.to_string()),
            None => SomName::Name(name.clone()),
        },
        index: SomIndex::Default,
    };
    let objs = host
        .dom
        .borrow_mut()
        .start(Some(this), &seg)
        .map_err(script_error)?;
    let Some(first) = objs.into_iter().next() else {
        return Ok(JsValue::undefined());
    };
    if let Obj::Node(id) = first {
        let is_script_object = host.dom.borrow().nodes[id].class == Class::ScriptObject;
        if is_script_object {
            return script_object(context, &host, id);
        }
    }
    from_val(Val::Obj(first), context)
}

fn scope_set(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(name) = args.get(1).and_then(key_name) else {
        return Ok(JsValue::from(true));
    };
    let value = args.get(2).cloned().unwrap_or_default();
    let value = to_val(&value, context);
    let host = host_of(context)?;
    let this = host.scope.borrow().this;
    let seg = Seg {
        desc: false,
        name: match dollar_name(&name) {
            Some(rest) => SomName::Dollar(rest.to_string()),
            None => SomName::Name(name),
        },
        index: SomIndex::Default,
    };
    let mut dom = host.dom.borrow_mut();
    let objs = dom.start(Some(this), &seg).map_err(script_error)?;
    if let Some(Obj::Node(id)) = objs.first() {
        // `Field1 = x` sets the field's value (PDFium global setter).
        dom.set_value(*id, &value).map_err(script_error)?;
    }
    Ok(JsValue::from(true))
}

/// A `<variables><script>` object: its functions and variables as an
/// object, evaluated once per document.
fn script_object(context: &mut Context, host: &Host, id: usize) -> JsResult<JsValue> {
    if let Some(existing) = host.script_objects.borrow().get(&id) {
        return Ok(existing.clone());
    }
    let source = {
        let dom = host.dom.borrow();
        let node = &dom.nodes[id];
        if node.scripts.first().map(|s| s.lang) != Some(super::som::Lang::JavaScript) {
            return Ok(JsValue::undefined());
        }
        node.scripts[0].source.clone()
    };
    let names = declared_names(&source);
    let mut members = String::new();
    for name in &names {
        members.push_str(&format!(
            "{name}: (typeof {name} !== 'undefined' ? {name} : undefined),"
        ));
    }
    let code = format!("(function () {{\n{source}\nreturn {{ {members} }};\n}})()");
    let value = context.eval(Source::from_bytes(code.as_bytes()))?;
    host.script_objects.borrow_mut().insert(id, value.clone());
    Ok(value)
}

/// Names declared at any level with `var`, `let`, `const` or `function`
/// (a conservative over-approximation of the script's own bindings).
fn declared_names(source: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let tokens = js_tokens(source);
    for window in tokens.windows(2) {
        if let (Token::Word(keyword), Token::Word(name)) = (&window[0], &window[1]) {
            if matches!(keyword.as_str(), "var" | "let" | "const" | "function") {
                out.insert(name.clone());
            }
        }
    }
    // `var a = 1, b = 2`: names after commas at the same statement.
    let mut in_decl = false;
    let mut depth = 0i32;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Word(word) if matches!(word.as_str(), "var" | "let" | "const") => {
                in_decl = true;
                depth = 0;
            }
            Token::Punct(';') => in_decl = false,
            Token::Punct('(' | '[' | '{') => depth += 1,
            Token::Punct(')' | ']' | '}') => {
                depth -= 1;
                if depth < 0 {
                    in_decl = false;
                }
            }
            Token::Punct(',') if in_decl && depth == 0 => {
                if let Some(Token::Word(name)) = tokens.get(index + 1) {
                    out.insert(name.clone());
                }
            }
            _ => {}
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Punct(char),
    /// A string, template, number or regular-expression literal.
    Other,
}

/// A light JavaScript tokenizer: words and punctuation outside strings,
/// template literals, comments and regular expressions.
fn js_tokens(source: &str) -> Vec<Token> {
    let mut out = Vec::new();
    scan(source, |event| match event {
        Scan::Word(word, _) => out.push(Token::Word(word.to_string())),
        Scan::Punct(ch) => out.push(Token::Punct(ch)),
        Scan::Literal => out.push(Token::Other),
    });
    out
}

enum Scan<'a> {
    Word(&'a str, usize),
    Punct(char),
    Literal,
}

fn scan<'a>(source: &'a str, mut emit: impl FnMut(Scan<'a>)) {
    let bytes = source.as_bytes();
    let mut pos = 0;
    // Whether a `/` here would start a regular expression.
    let mut regex_ok = true;
    let mut template_depth: Vec<i32> = Vec::new();
    let mut brace_depth = 0i32;
    while pos < bytes.len() {
        let b = bytes[pos];
        if b.is_ascii_whitespace() {
            pos += 1;
            continue;
        }
        if b == b'/' && bytes.get(pos + 1) == Some(&b'/') {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        if b == b'/' && bytes.get(pos + 1) == Some(&b'*') {
            pos += 2;
            while pos + 1 < bytes.len() && !(bytes[pos] == b'*' && bytes[pos + 1] == b'/') {
                pos += 1;
            }
            pos = (pos + 2).min(bytes.len());
            continue;
        }
        if b == b'"' || b == b'\'' {
            pos += 1;
            while pos < bytes.len() && bytes[pos] != b {
                if bytes[pos] == b'\\' {
                    pos += 1;
                }
                if bytes.get(pos) == Some(&b'\n') {
                    break;
                }
                pos += 1;
            }
            pos += 1;
            emit(Scan::Literal);
            regex_ok = false;
            continue;
        }
        if b == b'`' || (b == b'}' && template_depth.last() == Some(&brace_depth)) {
            if b == b'}' {
                template_depth.pop();
            }
            pos += 1;
            loop {
                if pos >= bytes.len() {
                    break;
                }
                if bytes[pos] == b'\\' {
                    pos += 2;
                    continue;
                }
                if bytes[pos] == b'`' {
                    pos += 1;
                    break;
                }
                if bytes[pos] == b'$' && bytes.get(pos + 1) == Some(&b'{') {
                    pos += 2;
                    template_depth.push(brace_depth);
                    break;
                }
                pos += 1;
            }
            emit(Scan::Literal);
            regex_ok = false;
            continue;
        }
        if b == b'/' && regex_ok {
            pos += 1;
            let mut in_class = false;
            while pos < bytes.len() {
                match bytes[pos] {
                    b'\\' => pos += 1,
                    b'[' => in_class = true,
                    b']' => in_class = false,
                    b'/' if !in_class => break,
                    b'\n' => break,
                    _ => {}
                }
                pos += 1;
            }
            pos += 1;
            while pos < bytes.len() && (bytes[pos] as char).is_ascii_alphabetic() {
                pos += 1;
            }
            emit(Scan::Literal);
            regex_ok = false;
            continue;
        }
        if b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b >= 0x80 {
            let start = pos;
            while pos < bytes.len()
                && (bytes[pos].is_ascii_alphanumeric()
                    || bytes[pos] == b'_'
                    || bytes[pos] == b'$'
                    || bytes[pos] >= 0x80)
            {
                pos += 1;
            }
            let word = &source[start..pos];
            regex_ok = matches!(
                word,
                "return"
                    | "typeof"
                    | "instanceof"
                    | "in"
                    | "of"
                    | "new"
                    | "delete"
                    | "void"
                    | "throw"
                    | "case"
                    | "do"
                    | "else"
            );
            emit(Scan::Word(word, start));
            continue;
        }
        if b.is_ascii_digit() {
            while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'.') {
                pos += 1;
            }
            emit(Scan::Literal);
            regex_ok = false;
            continue;
        }
        let ch = b as char;
        match ch {
            '{' => brace_depth += 1,
            '}' => brace_depth -= 1,
            _ => {}
        }
        regex_ok = !matches!(ch, ')' | ']' | '}');
        emit(Scan::Punct(ch));
        pos += 1;
    }
}

/// Replaces the `this` keyword outside nested `function` bodies with
/// `__wf_this` (the script's node).
fn rewrite_this(source: &str) -> String {
    let mut replace_at = Vec::new();
    // Brace depths at which `function` bodies open.
    let mut function_bodies: Vec<i32> = Vec::new();
    let mut depth = 0i32;
    let mut pending_function = false;
    let mut paren = 0i32;
    let mut pending_paren_base = 0i32;
    scan(source, |event| match event {
        Scan::Word("function", _) => {
            pending_function = true;
            pending_paren_base = paren;
        }
        Scan::Word("this", start) => {
            if function_bodies.is_empty() {
                replace_at.push(start);
            }
        }
        Scan::Punct('(') => paren += 1,
        Scan::Punct(')') => paren -= 1,
        Scan::Punct('{') => {
            depth += 1;
            if pending_function && paren == pending_paren_base {
                function_bodies.push(depth);
                pending_function = false;
            }
        }
        Scan::Punct('}') => {
            if function_bodies.last() == Some(&depth) {
                function_bodies.pop();
            }
            depth -= 1;
        }
        _ => {}
    });
    let mut out = String::with_capacity(source.len() + replace_at.len() * 6);
    let mut last = 0;
    for start in replace_at {
        out.push_str(&source[last..start]);
        out.push_str("__wf_this");
        last = start + 4;
    }
    out.push_str(&source[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xfa::locale::LocaleSet;
    use crate::xfa::som::{Limits, SNode};

    fn dom() -> Rc<RefCell<Dom>> {
        let mut dom = Dom::new(Limits::default(), LocaleSet::default());
        let root = dom.add(SNode::new(Class::Subform, "form1", 0), None);
        dom.root = root;
        for (name, value) in [("a", "2"), ("b", "3"), ("out", "")] {
            let mut field = SNode::new(Class::Field, name, 1);
            field.value_kind = if name == "out" { "text" } else { "decimal" }.into();
            field.value = Some(value.to_string()).filter(|v| !v.is_empty());
            field.has_caption = true;
            dom.add(field, Some(root));
        }
        Rc::new(RefCell::new(dom))
    }

    #[test]
    fn rewrites_this_outside_functions_only() {
        assert_eq!(
            rewrite_this("this.rawValue = 'this'; function f() { return this; } this.x // this"),
            "__wf_this.rawValue = 'this'; function f() { return this; } __wf_this.x // this"
        );
        assert_eq!(
            rewrite_this("var r = /this/g; this"),
            "var r = /this/g; __wf_this"
        );
        let names = declared_names("var a = 1, b = f(1, 2), c; function g(x) {} let d");
        for name in ["a", "b", "c", "g", "d"] {
            assert!(names.contains(name), "{name}");
        }
        assert!(!names.contains("x"));
    }

    #[test]
    fn javascript_reads_and_writes_the_som() {
        let dom = dom();
        let mut js = JsRuntime::new(Rc::clone(&dom)).unwrap();
        let out = 3;
        let result = js
            .run(
                out,
                r#"
                var sum = a.rawValue + b.rawValue;
                this.rawValue = "sum=" + sum;
                this.caption.value.text.value = "Total";
                xfa.resolveNode("form1.b").presence = "hidden";
                app.alert("done " + xfa.host.name);
                [1, 2, 3].map(function (v) { return v * 2; }).join(",");
                "#,
            )
            .ok()
            .flatten();
        assert_eq!(result, Some(Val::Str("2,4,6".into())));
        let dom = dom.borrow();
        assert_eq!(dom.nodes[out].value.as_deref(), Some("sum=5"));
        assert_eq!(dom.nodes[out].caption.as_deref(), Some("Total"));
        assert_eq!(dom.nodes[2].presence, "hidden");
        assert_eq!(dom.log, vec!["done Acrobat".to_string()]);
        drop(dom);
        let result = js.run(out, "eval('a.rawValue + 1')").ok().flatten();
        assert_eq!(result, Some(Val::Num(3.0)));
    }

    #[test]
    fn runaway_javascript_is_abandoned() {
        let dom = dom();
        dom.borrow_mut().limits.max_runtime_ms = 200;
        let mut js = JsRuntime::new(Rc::clone(&dom)).unwrap();
        let started = std::time::Instant::now();
        let result = js.run(
            1,
            "function f() { for (var i = 0; i < 900000; i++) {} } for (;;) { f(); }",
        );
        assert!(matches!(
            result,
            Err(JsFailure::Aborted(_)) | Err(JsFailure::Script(_))
        ));
        assert!(started.elapsed().as_secs() < 20);
    }
}
