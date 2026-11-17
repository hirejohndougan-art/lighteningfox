// Mark the page, then ask the background (which reads the setting from storage).
const out = document.getElementById("ext-out");
document.documentElement.setAttribute("data-dark", "on");
const log = (s) => { if (out) out.textContent += s + ";"; };
log("content " + (typeof chrome.runtime.id) + " " + (typeof window.chrome === "undefined" ? "hidden" : "global"));
chrome.storage.local.set({ seen: location.pathname.split("/").pop() }, () => {
  chrome.runtime.sendMessage({ ask: "level" }, (r) => {
    log("bg says " + JSON.stringify(r));
    browser.storage.local.get("seen").then((v) => log("stored " + v.seen));
  });
});
chrome.runtime.onMessage.addListener((msg, sender, reply) => {
  log("tab got " + msg.hello);
  reply({ ok: true });
});
