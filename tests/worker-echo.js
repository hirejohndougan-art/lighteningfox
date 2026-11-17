// Worker for tests/workers.html.
onmessage = (e) => {
  const d = e.data;
  if (d.kind === "echo") postMessage({ kind: "echo", value: d.value, inWorker: [typeof document, typeof self].join(",") });
  else if (d.kind === "nested") {
    const inner = new Worker(URL.createObjectURL(new Blob(["onmessage = (e) => postMessage(e.data * 2)"])));
    inner.onmessage = (m) => postMessage({ kind: "nested", value: m.data });
    inner.postMessage(3);
  } else if (d.kind === "throw") throw new Error("boom");
};
