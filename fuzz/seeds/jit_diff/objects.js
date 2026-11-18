function f(o, n) { let s = 0; for (let i = 0; i < n; i++) { o.k = (o.k || 0) + i; s += o.k; } return s + Object.keys(o).length; }
f({}, 50) + f({ a: 1 }, 3);
