
const out = document.getElementById("out");
const check = (ok, msg) => { const li = document.createElement("li"); li.className = ok ? "pass" : "fail"; li.textContent = (ok ? "PASS " : "FAIL ") + msg; out.appendChild(li); };
const box = document.getElementById("box").getBoundingClientRect();
check(box.width === 330 && box.height === 110, `#box is ${box.width}×${box.height} (expected 330×110)`);
check(box.left === 20, `#box left = ${box.left}`);
const a = document.getElementById("a"), b = document.getElementById("b"), c = document.getElementById("c");
check(a.offsetWidth === b.offsetWidth && b.offsetWidth === c.offsetWidth && a.offsetWidth > 100, `flex items equal width: ${a.offsetWidth}, ${b.offsetWidth}, ${c.offsetWidth}`);
check(b.getBoundingClientRect().left > a.getBoundingClientRect().right, "flex items side by side");
const s = document.getElementById("s").getBoundingClientRect();
check(s.width > 50 && s.height > 10, `inline span is ${Math.round(s.width)}×${Math.round(s.height)}`);
const extra = document.createElement("div"); extra.style.height = "40px"; document.body.insertBefore(extra, document.getElementById("box"));
const moved = document.getElementById("box").getBoundingClientRect();
const e = extra.getBoundingClientRect();
check(e.height === 40 && Math.abs(moved.top - (e.bottom + 15)) < 0.01, `a new 40px block above pushes #box to ${Math.round(moved.top)}px (layout redone on demand)`);
check(window.innerWidth === 700 && document.documentElement.clientWidth === 700, `window is ${innerWidth}px wide`);
check(document.documentElement.scrollHeight > 1500, `page height ${document.documentElement.scrollHeight}px`);
check(matchMedia("(min-width: 600px)").matches && !matchMedia("(max-width: 600px)").matches, "matchMedia width queries");
extra.remove();
