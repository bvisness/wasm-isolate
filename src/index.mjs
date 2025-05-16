/**
 * Converts a mode ID to a human-readable string.
 * @param {number} modeID - The mode ID (0 = record, 1 = replay).
 * @returns {string} A string representation of the mode.
 */
function mode2str(modeID) {
  switch (modeID) {
    case 0: return "record";
    case 1: return "replay";
    default: return "???";
  };
}

/**
 * Verifies that the WebAssembly module was instrumented with the expected mode.
 * @param {WebAssembly.Module} module - The WebAssembly module to check.
 * @param {number} expected - The expected mode ID (0 or 1).
 */
function assertReplayMode(module, expected) {
  const modeSections = WebAssembly.Module.customSections(module, "_replay:mode");
  if (modeSections.length === 0) {
    throw new Error("The given WebAssembly module was not instrumented by `wasm-isolate replay`.");
  }
  const mode = new Uint8Array(modeSections[0])[0];
  if (mode !== expected) {
    throw new Error(`The given WebAssembly module was instrumented for ${mode2str(mode)}, not ${mode2str(expected)}.`);
  }
}

/**
 * Ensures that a global object has not already been polyfilled.
 * @param {string[]} path - The property path to check on the global object.
 * @param {string} key - The polyfill key name.
 */
function assertNotAlreadyPolyfilled(path, key) {
  let thing = globalThis;
  for (const prop of path) {
    thing = thing[prop];
  }

  if (thing.name.includes(`_replay_${key}`)) {
    throw new Error(`${path.join(".")} has already been polyfilled for replay`);
  }
  if (thing.name.includes(`_record_${key}`)) {
    throw new Error(`${path.join(".")} has already been polyfilled for record`);
  }
}

export class Recorder {
  constructor() {
    /** @type {?WebAssembly.Memory} */
    this._callLogMemory = null;

    /** @type {?WebAssembly.Global} */
    this._callLogCursor = null;
  }

  /**
   * Polyfills the WebAssembly API for recording.
   */
  hook() {
    assertNotAlreadyPolyfilled(["WebAssembly", "Instance"], "Instance");
    this._WebAssembly_Instance_orig = WebAssembly.Instance;
    WebAssembly.Instance = function _record_Instance(module, ...args) {
      assertReplayMode(module, 0);
      const instance = new _WebAssembly_Instance_orig(module, ...args);
      this._callLogMemory = instance.exports["_replay:call_log"];
      this._callLogCursor = instance.exports["_replay:call_log_cursor"];
      return instance;
    }.bind(this);
    WebAssembly.Instance.prototype = this._WebAssembly_Instance_orig.prototype;

    assertNotAlreadyPolyfilled(["WebAssembly", "instantiateStreaming"], "instantiateStreaming");
    this._WebAssembly_instantiateStreaming_orig = WebAssembly.instantiateStreaming;
    WebAssembly.instantiateStreaming = async function _record_instantiateStreaming(source, importObject) {
      const res = await this._WebAssembly_instantiateStreaming_orig(source, importObject);
      assertReplayMode(res.module, 0);
      this._callLogMemory = res.instance.exports["_replay:call_log"];
      this._callLogCursor = res.instance.exports["_replay:call_log_cursor"];
      return res;
    }.bind(this);
  }

  /**
   * Restores the original WebAssembly API.
   */
  unhook() {
    WebAssembly.Instance = this._WebAssembly_Instance_orig;
    WebAssembly.instantiateStreaming = this._WebAssembly_instantiateStreaming_orig;
  }

  /**
   * Returns the call log from memory as a Uint8Array.
   * @returns {Uint8Array}
   */
  getCallLog() {
    const cur = this._callLogCursor.value;
    return new Uint8Array(this._callLogMemory.buffer).slice(0, cur);
  }
};

/**
 * @typedef {Object} CallDesc
 * @property {number} func - The function index.
 * @property {number} cursor - The position in the call log.
 */

export class Replayer {
  constructor() {
    /** @type {?WebAssembly.Memory} */
    this._callLogMemory = null;

    /** @type {?WebAssembly.Global} */
    this._callLogCursor = null;

    this._moduleInit = null;
    this._walkCallLog = null;

    /**
     * Call stubs by wasm function index.
     * @type {Object.<number, function()>}
     */
    this._callStubs = {};

    /**
     * All calls to instrumented functions in the order they were called.
     * @type {CallDesc[]}
     */
    this.calls = [];

    /**
     * Calls to instrumented functions, grouped by function index.
     * @type {Object.<number, CallDesc[]>}
     */
    this.callsByFunc = {};
  }

  /**
   * Polyfills the WebAssembly API for replay.
   */
  hook() {
    assertNotAlreadyPolyfilled(["WebAssembly", "instantiateStreaming"], "instantiateStreaming");
    this._WebAssembly_instantiateStreaming_orig = WebAssembly.instantiateStreaming;
    WebAssembly.instantiateStreaming = async function _replay_instantiateStreaming(source, importObject) {
      const res = await this._WebAssembly_instantiateStreaming_orig(source, importObject);
      assertReplayMode(res.module, 1);
      this._callLogMemory = res.instance.exports["_replay:call_log"];
      this._callLogCursor = res.instance.exports["_replay:call_log_cursor"];
      this._moduleInit = res.instance.exports["_replay:module_init"];
      this._walkCallLog = res.instance.exports["_replay:walk_call_log"];
      for (const [name, func] of Object.entries(res.instance.exports)) {
        if (name.startsWith("_replay:callstub_")) {
          const funcIdx = Number(name.substring("_replay:callstub_".length));
          this._callStubs[funcIdx] = func;
        }
      }
      return res;
    }.bind(this);
  }

  /**
   * Restores the original WebAssembly API.
   */
  unhook() {
    WebAssembly.instantiateStreaming = this._WebAssembly_instantiateStreaming_orig;
  }

  /**
   * Loads a call log into memory and loads all call descriptors.
   * @param {Uint8Array} callLog - The call log to load.
   */
  loadCallLog(callLog) {
    const dst = new Uint8Array(this._callLogMemory.buffer);
    dst.set(callLog);
    this._callLogCursor.value = 0;

    this.calls = [];
    this.callsByFunc = {};
    let nextFuncCursor = 0;
    while (nextFuncCursor < callLog.length) {
      const [funcIdx, next] = this._walkCallLog(nextFuncCursor);
      const desc = { func: funcIdx, cursor: nextFuncCursor };
      this.calls.push(desc);
      if (this.callsByFunc[funcIdx] === undefined) {
        this.callsByFunc[funcIdx] = [];
      }
      this.callsByFunc[funcIdx].push({ func: funcIdx, cursor: nextFuncCursor });
      nextFuncCursor = next;
    }
  }

  /**
   * Prepares the module for a given function call, and returns a separate
   * function that will actually perform the call. Separating the two in this
   * way allows for more precise timing of the function call.
   *
   * @param {CallDesc} callDesc - The call descriptor as loaded from calls or callsByFunc.
   * @returns {function()} A function that performs the actual call, returning what the original function returned.
   */
  init(callDesc) {
    this._moduleInit(callDesc.cursor);
    const paramPosition = this._callLogCursor.value;
    return () => {
      if (this._callLogCursor.value !== paramPosition) {
        throw new Error("replay cursor was moved before the call");
      }
      return this._callStubs[callDesc.func]();
    };
  }

  /**
   * Initializes and immediately replays a call. Useful for general replay in
   * cases where timing is not critical.
   *
   * @param {CallDesc} callDesc - The call descriptor as loaded from calls or callsByFunc.
   * @returns {*} The return values of the replayed call.
   */
  initAndCall(callDesc) {
    return this.init(callDesc)();
  }
}
