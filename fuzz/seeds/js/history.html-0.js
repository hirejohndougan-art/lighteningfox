
const view = document.getElementById("view"), log = document.getElementById("log");
const say = (m) => { log.textContent += m + "\n"; };
const render = (page) => { view.textContent = "Page: " + page + "  (no reload; location.search = " + (location.search || "none") + ")"; };
const pageOf = () => new URLSearchParams(location.search).get("page") || "home";
for (const p of ["home", "about", "contact"]) {
  document.getElementById(p).addEventListener("click", () => {
    history.pushState({ page: p }, "", "?page=" + p);
    render(p);
    say("pushState → " + location.href.split("/").pop() + "  (history.length " + history.length + ")");
  });
}
window.addEventListener("popstate", (e) => { render(e.state ? e.state.page : pageOf()); say("popstate, state = " + JSON.stringify(e.state)); });
window.addEventListener("hashchange", () => say("hashchange → " + location.hash));
render(pageOf());
say("loaded at " + location.href.split("/").pop());
