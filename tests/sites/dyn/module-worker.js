// A module worker script (tests/sites/module-worker.html): imports, top-level await.
import { twice } from "./module-worker-lib.js";
const n = await Promise.resolve(21);
let how;
try { importScripts("nothing.js"); how = "classic"; } catch (e) { how = e.name; }
postMessage(twice(n) + " " + how);
