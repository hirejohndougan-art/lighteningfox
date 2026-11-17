
const chat = document.getElementById("chat");
for (let i = 1; i <= 12; i++) {
  const m = document.createElement("div");
  m.className = i % 3 ? "msg" : "msg me";
  m.textContent = "Message " + i + (i % 3 ? " from a friend" : " from me");
  chat.appendChild(m);
}
chat.scrollTop = chat.scrollHeight; // like a chat app: show the newest message
const list = document.getElementById("list");
for (let i = 1; i <= 15; i++) { const a = document.createElement("a"); a.href = "#item" + i; a.textContent = "List item " + i; list.appendChild(a); }
document.getElementById("log").textContent =
  `chat: scrollTop ${Math.round(chat.scrollTop)} of ${chat.scrollHeight - chat.clientHeight} (scrollHeight ${chat.scrollHeight}), list max-height box is ${list.offsetHeight}px tall`;
list.addEventListener("click", (e) => { document.getElementById("log").textContent = "Clicked: " + e.target.textContent.trim(); });
