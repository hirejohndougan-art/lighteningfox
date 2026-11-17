chrome.storage.local.set({ level: "dim" });
chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  chrome.storage.local.get("level", (v) => sendResponse({ level: v.level, from: sender.url.split("/").pop(), tab: !!sender.tab }));
  return true;
});
// Say hello to the tab that just asked us something.
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === "local" && changes.seen) {
    chrome.tabs.query({ active: true }, (tabs) => {
      if (tabs.length) chrome.tabs.sendMessage(tabs[0].id, { hello: "from bg" }, (r) => { globalThis.lastReply = r; });
    });
  }
});
