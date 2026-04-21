import init, { convert_pbf } from "./wasm/pkg/osmshrink_wasm.js";

let wasmReady = null;

function ensureWasm() {
  if (!wasmReady) {
    wasmReady = init(new URL("./wasm/pkg/osmshrink_wasm_bg.wasm", import.meta.url));
  }
  return wasmReady;
}

self.onmessage = async (event) => {
  if (event.data?.type !== "convert") {
    return;
  }

  const { id, buffer, spec } = event.data;
  try {
    await ensureWasm();
    const result = convert_pbf(new Uint8Array(buffer), spec);
    self.postMessage({ type: "done", id, result });
  } catch (error) {
    self.postMessage({
      type: "error",
      id,
      error: error instanceof Error ? error.message : String(error)
    });
  }
};
