// Thin wrappers over Tauri's global API (no npm package, no bundler).
const { core, event } = window.__TAURI__;

/** Calls a Rust command. Rejects with the error string Rust returned. */
export const invoke = (command, args) => core.invoke(command, args);

/** Subscribes to an event from Rust. `handler` receives the payload. */
export const listen = (name, handler) => event.listen(name, (e) => handler(e.payload));
