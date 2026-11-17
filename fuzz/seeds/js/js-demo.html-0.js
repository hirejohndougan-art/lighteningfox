
  document.getElementById("heading").textContent = "Hello from Boa, running inside Ironswift";
  document.title = "JS works";

  const list = document.getElementById("list");
  const langs = ["Rust", "JavaScript", "HTML", "CSS"];
  for (const l of langs) {
    const li = document.createElement("li");
    li.textContent = l + " (" + l.length + " letters)";
    list.appendChild(li);
  }
  document.getElementById("list-box").classList.add("done");

  document.getElementById("qcount").textContent = document.querySelectorAll(".q").length + " paragraphs";
  document.getElementById("html-target").innerHTML = "<b>bold</b> and <i>italic</i> from a string";

  let n = 0;
  document.getElementById("btn").addEventListener("click", (e) => {
    e.preventDefault();
    document.getElementById("counter").textContent = ++n;
  });

  // Fibonacci timing, to watch Boa's speed
  function fib(k) { return k < 2 ? k : fib(k - 1) + fib(k - 2); }
  const t0 = performance.now();
  console.log("fib(20) =", fib(20));
