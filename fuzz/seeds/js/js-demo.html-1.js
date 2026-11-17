
  document.addEventListener("DOMContentLoaded", () => {
    const p = document.createElement("p");
    p.innerHTML = "<em>DOMContentLoaded fired.</em>";
    document.body.appendChild(p);
  });
  setTimeout(() => {
    const p = document.createElement("p");
    p.textContent = "setTimeout(…, 0) fired.";
    p.style.color = "#06c";
    document.body.appendChild(p);
  }, 0);
