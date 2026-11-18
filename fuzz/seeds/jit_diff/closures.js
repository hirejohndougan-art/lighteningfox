function make(n) { const fs = []; for (let i = 0; i < n; i++) fs.push(() => i * 2); return fs; }
let s = 0; for (const f of make(10)) s += f(); s;
