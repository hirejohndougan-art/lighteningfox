
fetch("data.json").then((r) => r.json()).then((d) => {
  document.getElementById("t").textContent = d.title;
  const ul = document.getElementById("list");
  for (const item of d.items) { const li = document.createElement("li"); li.textContent = item; ul.appendChild(li); }
});
const x = new XMLHttpRequest();
x.open("GET", "data.json");
x.onload = () => { document.getElementById("x").textContent = "XMLHttpRequest also got " + JSON.parse(x.responseText).items.length + " items."; };
x.send();
