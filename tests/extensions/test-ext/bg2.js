// Background scripts share one global scope, in order.
chrome.webRequest.onBeforeRequest.addListener((d) => ({ cancel: true }), { urls: ["*://blocked.test/*"] }, ["blocking"]);
chrome.webRequest.onBeforeRequest.addListener((d) => ({ redirectUrl: "https://example.org/moved" }), { urls: ["*://moved.test/*"], types: ["script"] }, ["blocking"]);
chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  state.messages++;
  if (msg.kind === "echo") return Promise.resolve({ echo: msg.text, n: state.messages, page: sender.url.split("/").pop() });
  if (msg.kind === "poke") {
    chrome.tabs.query({}, (tabs) => {
      const t = tabs.find((t) => t.url === sender.url);
      chrome.tabs.sendMessage(t.id, { poked: true }, (r) => chrome.storage.local.set({ pokeReply: r && r.thanks }));
    });
    sendResponse("poking");
  }
});
