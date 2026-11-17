// Cancel anything with "ads" in its path; send "old.png" to "new.png".
chrome.webRequest.onBeforeRequest.addListener(
  (d) => {
    if (/\/ads[\/.]/.test(d.url)) return { cancel: true };
    if (d.url.endsWith("/old.png")) return { redirectUrl: d.url.replace("old.png", "new.png") };
    return {};
  },
  { urls: ["<all_urls>"] },
  ["blocking"]
);
