const out = document.getElementById("out");
const add = (s) => (out.textContent += s + ";");
add("start saw " + startSaw);
add("page ran " + document.body.dataset.ran);
add("chrome " + typeof chrome.runtime.sendMessage + " page sees " + document.body.dataset.chrome);
add("page sees " + document.body.dataset.leaks);
chrome.runtime.onMessage.addListener((m, s, reply) => {
  if (m.poked) { add("poked"); reply({ thanks: "yes" }); }
});
chrome.runtime.sendMessage({ kind: "echo", text: "hi" }).then((r) => {
  add("echo " + r.echo + " " + r.page);
  return browser.storage.local.set({ a: 1 });
}).then(() => chrome.storage.local.get(["a", "missing"])).then((v) => {
  add("storage " + JSON.stringify(v) + " hooked " + (document.body.dataset.hooked || "no"));
  chrome.runtime.sendMessage({ kind: "poke" }, (r) => add("bg " + r));
});
